// SPDX-License-Identifier: GPL-3.0-or-later
// Keyboard panel: layout, variant, xkb options, numlock and key repeat, written
// into the `keyboard` region of niri's config (see niri_input). Layouts and
// variants are typed as codes (`gb`, `us,ru`) or names ("british", "dvorak")
// and resolved against xkb's own list (see xkb.rs); options must be codes from
// it. Without that list, layouts fall back to `localectl
// list-x11-keymap-layouts`. Either way a typo can't produce a keymap niri
// refuses to start with.

use super::kdl::{self, Node};
use super::niri_input::{config_path, read, write, xkb_name_ok};
use super::xkb::{self as xkbl, Xkb};
use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &["layout", "variant", "options", "numlock", "repeat delay ms", "repeat rate /s"];

// niri's own defaults, shown when the config doesn't set them.
const DEFAULT_DELAY: i32 = 600;
const DEFAULT_RATE: i32 = 25;
const NONE: &str = "(none)";
const SYSTEM: &str = "(system default)";

pub fn kind(field: usize) -> RowKind {
    match field {
        0..=2 => RowKind::Text,
        3 => RowKind::Bool,
        _ => RowKind::Int(if field == 4 { 25 } else { 5 }),
    }
}

pub fn load(_run: Runner) -> Vec<Row> {
    load_at(&config_path())
}

pub fn load_at(path: &str) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let nodes = match read(path, "keyboard") {
        Ok(n) => n,
        Err(e) => return vec![r(&e), r("-"), r("-"), r("-"), r("-"), r("-")],
    };
    let kb = kdl::find(&nodes, "keyboard").and_then(|n| n.children.as_deref()).unwrap_or(&[]);
    let xkb = kdl::find(kb, "xkb").and_then(|n| n.children.as_deref()).unwrap_or(&[]);
    let or = |v: Option<String>, d: &str| r(&v.filter(|s| !s.is_empty()).unwrap_or_else(|| d.into()));
    vec![
        or(kdl::get_str(xkb, "layout"), SYSTEM),
        or(kdl::get_str(xkb, "variant"), NONE),
        or(kdl::get_str(xkb, "options"), NONE),
        r(&kdl::has(kb, "numlock").to_string()),
        or(kdl::get_str(kb, "repeat-delay"), &DEFAULT_DELAY.to_string()),
        or(kdl::get_str(kb, "repeat-rate"), &DEFAULT_RATE.to_string()),
    ]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_at(&config_path(), field, rows, ch, run)
}

pub fn apply_at(path: &str, field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_with(path, field, rows, ch, run, super::xkb::Xkb::load().as_ref())
}

fn apply_with(path: &str, field: usize, rows: &[Row], ch: Change, run: Runner, xkb: Option<&Xkb>) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let mut nodes = read(path, "keyboard")?;
    let msg;
    {
        let kb = kdl::block_mut(&mut nodes, "keyboard");
        match field {
            0..=2 => {
                let Change::Text(t) = ch else { return Err("type a value and press Enter".into()) };
                let t = t.trim();
                let clear = t.is_empty() || t == NONE || t == SYSTEM;
                let (key, what) = [("layout", "layout"), ("variant", "variant"), ("options", "options")][field];
                let layout_now = Some(rows[0].value.as_str()).filter(|v| *v != SYSTEM && !v.contains(' '));
                let mut t = t.to_string();
                let mut note = String::new();
                if !clear {
                    // Names resolve to codes first; the codes are what must be safe.
                    t = match (field, xkb) {
                        (0, Some(x)) => xkbl::layouts(x, &t)?,
                        (1, Some(x)) => match layout_now {
                            Some(l) => xkbl::variants(x, l, &t)?,
                            None => t,
                        },
                        (2, Some(x)) => xkbl::options(x, &t)?,
                        _ => t,
                    };
                    if !xkb_name_ok(&t) {
                        return Err(format!("{what}: only letters, digits and : , + - ( ) _ are allowed"));
                    }
                    if field == 0 && xkb.is_none() {
                        check_layouts(&t, run)?;
                    }
                }
                let xkb_block = kdl::block_mut(kb, "xkb");
                kdl::put(xkb_block, key, (!clear).then(|| Node::str(key, &t)));
                // A new layout keeps its variant only if that variant exists for it.
                if let (0, Some(x), false) = (field, xkb, clear) {
                    if let Some(v) = kdl::get_str(xkb_block, "variant") {
                        if xkbl::variants(x, &t, &v).is_err() {
                            kdl::put(xkb_block, "variant", None);
                            note = format!(" (variant {v} doesn't fit it, so it was reset)");
                        }
                    }
                }
                if xkb_block.is_empty() {
                    kdl::put(kb, "xkb", None); // don't leave an empty xkb {} behind
                }
                msg = format!("{what} {}{note}", if clear { "reset" } else { &t });
            }
            3 => {
                let on = row.value == "true";
                let want = match &ch {
                    Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                    _ => !on,
                };
                kdl::put(kb, "numlock", want.then(|| Node::flag("numlock")));
                msg = format!("numlock {}", if want { "on" } else { "off" });
            }
            _ => {
                let (key, step, lo, hi, def) = if field == 4 {
                    ("repeat-delay", 25, 100, 2000, DEFAULT_DELAY)
                } else {
                    ("repeat-rate", 5, 1, 100, DEFAULT_RATE)
                };
                let cur: i32 = row.value.parse().unwrap_or(def);
                let next = match ch {
                    Change::Step(d) => cur + d * step,
                    Change::Text(t) => t.trim().parse().map_err(|_| format!("not a number: {:?}", t.trim()))?,
                    Change::Toggle => cur,
                }
                .clamp(lo, hi);
                kdl::put(kb, key, Some(Node::num(key, &next.to_string())));
                msg = format!("{} {next}", LABELS[field]);
            }
        }
        if kb.is_empty() {
            // An empty keyboard {} is noise; niri's defaults apply without it.
            kdl::put(&mut nodes, "keyboard", None);
        }
    }
    write(path, "keyboard", &nodes)?;
    Ok(format!("{msg} (niri applies it now)"))
}

fn check_layouts(list: &str, run: Runner) -> Result<(), String> {
    // If localectl is missing we can't verify; the name charset check already ran.
    let Ok(known) = run("localectl", &["list-x11-keymap-layouts"]) else { return Ok(()) };
    for l in list.split(',').map(str::trim).filter(|l| !l.is_empty()) {
        if !known.lines().any(|k| k.trim() == l) {
            return Err(format!("unknown layout {l:?} (see `localectl list-x11-keymap-layouts`)"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::niri_input::test_config;
    use super::*;

    const REGION: &str = "    keyboard {\n        // note\n        xkb {\n            layout \"gb\"\n            model \"pc105\"\n            options \"terminate:ctrl_alt_bksp\"\n        }\n        numlock\n    }";

    fn ap(p: &str, field: usize, rows: &[Row], ch: Change) -> Result<String, String> {
        apply_with(p, field, rows, ch, &layouts, Some(&super::super::xkb::fixture()))
    }

    #[test]
    fn layouts_and_variants_by_name_and_a_stale_variant_is_dropped() {
        let p = test_config("kb-names", "keyboard", REGION);
        let rows = load_at(&p);
        ap(&p, 0, &rows, Change::Text("English (US), russian".into())).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("layout \"us,ru\""));
        let rows = load_at(&p);
        ap(&p, 1, &rows, Change::Text("dvorak,phonetic".into())).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("variant \"dvorak,phonetic\""));
        // German has no dvorak in the fixture: the variant goes with the change.
        let msg = ap(&p, 0, &load_at(&p), Change::Text("de".into())).unwrap();
        assert!(msg.contains("reset"), "{msg}");
        assert_eq!(load_at(&p)[1].value, "(none)");
        let e = ap(&p, 2, &load_at(&p), Change::Text("caps lock".into())).unwrap_err();
        assert!(e.contains("ctrl:nocaps"), "{e}");
    }

    /// The localectl fallback, used when xkb's list is missing.
    #[test]
    fn without_the_xkb_list_layouts_are_checked_by_localectl() {
        let p = test_config("kb-lctl", "keyboard", REGION);
        let rows = load_at(&p);
        assert!(apply_with(&p, 0, &rows, Change::Text("zz".into()), &layouts, None).unwrap_err().contains("unknown layout"));
        apply_with(&p, 0, &rows, Change::Text("de".into()), &layouts, None).unwrap();
    }

    fn layouts(_: &str, a: &[&str]) -> Result<String, String> {
        assert_eq!(a, ["list-x11-keymap-layouts"]);
        Ok("us\ngb\nru\nde\n".into())
    }

    #[test]
    fn loads_values_and_defaults() {
        let p = test_config("kb-load", "keyboard", REGION);
        let rows = load_at(&p);
        let v: Vec<_> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(v, ["gb", "(none)", "terminate:ctrl_alt_bksp", "true", "600", "25"]);
    }

    #[test]
    fn change_keeps_unknown_settings_and_other_regions() {
        let p = test_config("kb-set", "keyboard", REGION);
        let rows = load_at(&p);
        ap(&p, 0, &rows, Change::Text("us,ru".into())).unwrap();
        ap(&p, 4, &rows, Change::Step(1)).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("layout \"us,ru\""));
        assert!(out.contains("model \"pc105\""), "unknown xkb setting kept");
        assert!(out.contains("repeat-delay 625"));
        assert!(out.contains("numlock"));
        assert!(out.contains("mouse {}"), "everything outside the region untouched");
        assert!(out.contains("LIFEBRANCH:BEGIN keyboard") && out.contains("LIFEBRANCH:END keyboard"));
    }

    #[test]
    fn bad_input_is_refused_and_the_file_is_unchanged() {
        let p = test_config("kb-bad", "keyboard", REGION);
        let before = std::fs::read_to_string(&p).unwrap();
        let rows = load_at(&p);
        let e = ap(&p, 0, &rows, Change::Text("zz".into())).unwrap_err();
        assert!(e.contains("no layout"), "{e}");
        let e = ap(&p, 2, &rows, Change::Text("x\" } evil {".into())).unwrap_err();
        assert!(e.contains("no xkb option"), "{e}");
        // Without xkb's list nothing resolves it, and the character check refuses it.
        let e = apply_with(&p, 2, &rows, Change::Text("x\" } evil {".into()), &layouts, None).unwrap_err();
        assert!(e.contains("only letters"));
        assert!(ap(&p, 0, &rows, Change::Step(1)).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn clearing_everything_removes_empty_blocks() {
        let p = test_config("kb-clear", "keyboard", "    keyboard {\n        xkb {\n            layout \"gb\"\n        }\n    }");
        let rows = load_at(&p);
        ap(&p, 0, &rows, Change::Text("(none)".into())).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("keyboard {") && !out.contains("xkb"));
        assert_eq!(load_at(&p)[0].value, "(system default)");
    }

    #[test]
    fn numlock_and_repeat_clamp() {
        let p = test_config("kb-num", "keyboard", REGION);
        let rows = load_at(&p);
        ap(&p, 3, &rows, Change::Toggle).unwrap(); // on -> off
        ap(&p, 5, &rows, Change::Text("9999".into())).unwrap();
        let rows = load_at(&p);
        assert_eq!(rows[3].value, "false");
        assert_eq!(rows[5].value, "100");
    }

    #[test]
    fn region_with_syntax_we_cannot_round_trip_is_left_alone() {
        let p = test_config("kb-odd", "keyboard", "    keyboard { repeat-delay 300; numlock }");
        let rows = load_at(&p);
        assert!(rows[0].value.contains("edit config.kdl by hand"));
        assert!(ap(&p, 3, &rows, Change::Toggle).is_err());
    }

    #[test]
    fn missing_region_degrades() {
        let rows = load_at("/nonexistent/config.kdl");
        assert_eq!(rows.len(), LABELS.len());
    }
}
