// SPDX-License-Identifier: GPL-3.0-or-later
// Touchpad panel: the common libinput settings, written into the `touchpad`
// region of niri's config (see niri_input). A setting left at niri's default is
// simply absent from the config, so "default" in a choice row removes the
// node, and when nothing is set the whole block goes away (a desktop with no
// touchpad keeps its "No touchpad configured" shape).

use super::kdl::{self, Node};
use super::niri_input::{config_path, read, write};
use super::{pick, Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &[
    "enabled",
    "tap to click",
    "natural scroll up/down",
    "natural scroll sideways",
    "disable while typing",
    "off with external mouse",
    "click method",
    "scroll method",
    "accel profile",
    "accel speed %",
    "tap button map",
];

const DEFAULT: &str = "default";

/// (row, node name, accepted values) for the choice rows.
const CHOICES: &[(usize, &str, &[&str])] = &[
    (6, "click-method", &["clickfinger", "button-areas"]),
    (7, "scroll-method", &["two-finger", "edge", "on-button-down", "no-scroll"]),
    (8, "accel-profile", &["adaptive", "flat"]),
    (10, "tap-button-map", &["left-right-middle", "left-middle-right"]),
];

/// (row, node name) for the on/off rows. `enabled` is the inverse of niri's `off`.
/// The two natural-scroll rows are handled apart (see `set_natural`).
const FLAGS: &[(usize, &str)] = &[(1, "tap"), (4, "dwt"), (5, "disabled-on-external-mouse")];
const NATURAL_V: usize = 2;
const NATURAL_H: usize = 3;
const SPEED: usize = 9;

pub fn kind(field: usize) -> RowKind {
    match field {
        0..=5 => RowKind::Bool,
        SPEED => RowKind::Int(10),
        _ => RowKind::Choice,
    }
}

// libinput's natural scroll flips both axes at once. Sideways is set apart by
// the sign of niri's horizontal scroll factor: natural-scroll on with
// `scroll-factor horizontal=-1` is natural up/down but traditional sideways.

/// (vertical, horizontal) scroll factors as niri reads them: a bare argument
/// sets both, and a property overrides its axis.
fn factors(tp: &[Node]) -> (f64, f64) {
    let Some(n) = kdl::find(tp, "scroll-factor") else { return (1.0, 1.0) };
    let num = |t: Option<&str>| t.and_then(|t| t.parse::<f64>().ok());
    let base = num(n.args.iter().find(|a| !a.quoted && !a.text.contains('=')).map(|a| a.text.as_str())).unwrap_or(1.0);
    (num(kdl::prop(n, "vertical")).unwrap_or(base), num(kdl::prop(n, "horizontal")).unwrap_or(base))
}

fn natural(tp: &[Node]) -> (bool, bool) {
    let v = kdl::has(tp, "natural-scroll");
    (v, v != (factors(tp).1 < 0.0))
}

fn fmt_factor(f: f64) -> String {
    if f.fract() == 0.0 { format!("{f:.1}") } else { format!("{f}") }
}

/// Write natural scroll per axis, keeping any scroll speed already set.
fn set_natural(tp: &mut Vec<Node>, v: bool, h: bool) {
    let (fv, fh) = factors(tp);
    let fh = if v != h { -fh.abs() } else { fh.abs() };
    kdl::put(tp, "natural-scroll", v.then(|| Node::flag("natural-scroll")));
    let node = if fv == fh {
        (fv != 1.0).then(|| Node::num("scroll-factor", &fmt_factor(fv)))
    } else {
        Some(Node {
            name: "scroll-factor".into(),
            args: [("vertical", fv), ("horizontal", fh)]
                .iter()
                .map(|(k, f)| kdl::Arg { text: format!("{k}={}", fmt_factor(*f)), quoted: false })
                .collect(),
            children: None,
        })
    };
    kdl::put(tp, "scroll-factor", node);
}

fn pad(nodes: &[Node]) -> &[Node] {
    kdl::find(nodes, "touchpad").and_then(|n| n.children.as_deref()).unwrap_or(&[])
}

pub fn load(_run: Runner) -> Vec<Row> {
    load_at(&config_path())
}

pub fn load_at(path: &str) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let nodes = match read(path, "touchpad") {
        Ok(n) => n,
        Err(e) => return std::iter::once(r(&e)).chain((1..LABELS.len()).map(|_| r("-"))).collect(),
    };
    let tp = pad(&nodes);
    let mut rows = vec![r("-"); LABELS.len()];
    rows[0] = r(&(!kdl::has(tp, "off")).to_string());
    for (i, key) in FLAGS {
        rows[*i] = r(&kdl::has(tp, key).to_string());
    }
    for (i, key, opts) in CHOICES {
        let mut choices = vec![(DEFAULT.to_string(), DEFAULT.to_string())];
        choices.extend(opts.iter().map(|o| (o.to_string(), o.to_string())));
        rows[*i] = Row { value: kdl::get_str(tp, key).unwrap_or_else(|| DEFAULT.into()), choices };
    }
    // niri's accel-speed is -1..1; shown as whole percent.
    let speed = kdl::get_str(tp, "accel-speed").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    rows[SPEED] = r(&((speed * 100.0).round() as i32).to_string());
    let (v, h) = natural(tp);
    rows[NATURAL_V] = r(&v.to_string());
    rows[NATURAL_H] = r(&h.to_string());
    rows
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_at(&config_path(), field, rows, ch, run)
}

pub fn apply_at(path: &str, field: usize, rows: &[Row], ch: Change, _run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let mut nodes = read(path, "touchpad")?;
    let msg;
    {
        let tp = kdl::block_mut(&mut nodes, "touchpad");
        let flip = |cur: bool, ch: &Change| match ch {
            Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
            _ => !cur,
        };
        if field == 0 {
            let want = flip(row.value == "true", &ch);
            kdl::put(tp, "off", (!want).then(|| Node::flag("off")));
            msg = format!("touchpad {}", if want { "on" } else { "off" });
        } else if field == NATURAL_V || field == NATURAL_H {
            let want = flip(row.value == "true", &ch);
            let (v, h) = natural(tp);
            let (v, h) = if field == NATURAL_V { (want, h) } else { (v, want) };
            set_natural(tp, v, h);
            msg = format!("{} {}", LABELS[field], if want { "on" } else { "off" });
        } else if let Some((_, key)) = FLAGS.iter().find(|(i, _)| *i == field) {
            let want = flip(row.value == "true", &ch);
            kdl::put(tp, key, want.then(|| Node::flag(key)));
            msg = format!("{} {}", LABELS[field], if want { "on" } else { "off" });
        } else if let Some((_, key, _)) = CHOICES.iter().find(|(i, _, _)| *i == field) {
            let (label, id) = pick(row, &ch, LABELS[field])?;
            kdl::put(tp, key, (id != DEFAULT).then(|| Node::str(key, id)));
            msg = format!("{} {label}", LABELS[field]);
        } else {
            let cur: i32 = row.value.parse().unwrap_or(0);
            let next = match ch {
                Change::Step(d) => cur + d * 10,
                Change::Text(t) => t.trim().trim_end_matches('%').parse().map_err(|_| format!("not a number: {:?}", t.trim()))?,
                Change::Toggle => cur,
            }
            .clamp(-100, 100);
            // 0 is niri's default, so it's removed rather than written.
            let num = format!("{}", next as f64 / 100.0);
            kdl::put(tp, "accel-speed", (next != 0).then(|| Node::num("accel-speed", &num)));
            msg = format!("accel speed {next}%");
        }
        if tp.is_empty() {
            kdl::put(&mut nodes, "touchpad", None);
        }
    }
    write(path, "touchpad", &nodes)?;
    Ok(format!("{msg} (niri applies it now)"))
}

#[cfg(test)]
mod tests {
    use super::super::niri_input::test_config;
    use super::*;

    const MAC: &str = "    touchpad {\n        tap\n        natural-scroll\n        dwt                        // disable-while-typing\n        click-method \"clickfinger\"\n        accel-profile \"adaptive\"\n        scroll-method \"two-finger\"\n    }";
    const NONE: &str = "    // No touchpad configured.";

    fn nr(_: &str, _: &[&str]) -> Result<String, String> {
        Ok(String::new())
    }

    #[test]
    fn loads_the_macbook_block() {
        let p = test_config("tp-load", "touchpad", MAC);
        let v: Vec<String> = load_at(&p).into_iter().map(|r| r.value).collect();
        assert_eq!(
            v,
            ["true", "true", "true", "true", "true", "false", "clickfinger", "two-finger", "adaptive", "0", "default"]
        );
    }

    #[test]
    fn unconfigured_desktop_shows_defaults_and_first_change_creates_the_block() {
        let p = test_config("tp-none", "touchpad", NONE);
        let rows = load_at(&p);
        assert_eq!(rows[1].value, "false");
        assert_eq!(rows[6].value, "default");
        apply_at(&p, 1, &rows, Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("touchpad {\n        tap\n    }"), "{out}");
        assert_eq!(load_at(&p)[1].value, "true");
    }

    #[test]
    fn choices_default_removes_the_node_and_empty_block_disappears() {
        let p = test_config("tp-clr", "touchpad", "    touchpad {\n        click-method \"clickfinger\"\n    }");
        let rows = load_at(&p);
        apply_at(&p, 6, &rows, Change::Step(-1), &nr).unwrap(); // clickfinger -> default
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("touchpad {") && !out.contains("click-method"));
    }

    #[test]
    fn changes_preserve_unknown_nodes() {
        let p = test_config("tp-keep", "touchpad", "    touchpad {\n        tap\n        scroll-factor 2.0\n        drag-lock\n    }");
        let rows = load_at(&p);
        apply_at(&p, 4, &rows, Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("scroll-factor 2.0") && out.contains("drag-lock") && out.contains("dwt"));
    }

    #[test]
    fn natural_scroll_splits_by_axis_and_keeps_the_speed() {
        let p = test_config("tp-nat", "touchpad", "    touchpad {\n        natural-scroll\n        scroll-factor 2.0\n    }");
        let both = |p: &str| {
            let r = load_at(p);
            (r[NATURAL_V].value.clone(), r[NATURAL_H].value.clone())
        };
        assert_eq!(both(&p), ("true".into(), "true".into()));
        apply_at(&p, NATURAL_H, &load_at(&p), Change::Toggle, &nr).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("natural-scroll\n        scroll-factor vertical=2.0 horizontal=-2.0"));
        assert_eq!(both(&p), ("true".into(), "false".into()));
        // Turning vertical off leaves sideways as it was (off): both traditional.
        apply_at(&p, NATURAL_V, &load_at(&p), Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("natural-scroll") && out.contains("scroll-factor 2.0"), "{out}");
        assert_eq!(both(&p), ("false".into(), "false".into()));
        // Sideways alone: natural-scroll on, vertical flipped back by the factor.
        apply_at(&p, NATURAL_H, &load_at(&p), Change::Toggle, &nr).unwrap();
        assert_eq!(both(&p), ("false".into(), "true".into()));
        // Speed 1 and both off again: no scroll-factor node at all.
        let p = test_config("tp-nat1", "touchpad", "    touchpad {\n        natural-scroll\n        scroll-factor vertical=1.0 horizontal=-1.0\n    }");
        assert_eq!(both(&p), ("true".into(), "false".into()));
        apply_at(&p, NATURAL_V, &load_at(&p), Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("scroll-factor") && !out.contains("natural-scroll"), "{out}");
    }

    #[test]
    fn accel_speed_is_percent_clamped_and_zero_is_removed() {
        let p = test_config("tp-acc", "touchpad", NONE);
        let rows = load_at(&p);
        apply_at(&p, SPEED, &rows, Change::Text("35".into()), &nr).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("accel-speed 0.35"));
        let rows = load_at(&p);
        assert_eq!(rows[SPEED].value, "35");
        apply_at(&p, SPEED, &rows, Change::Text("900".into()), &nr).unwrap();
        assert_eq!(load_at(&p)[SPEED].value, "100");
        apply_at(&p, SPEED, &rows, Change::Text("0".into()), &nr).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("accel-speed"));
    }

    #[test]
    fn disabling_the_touchpad_writes_off() {
        let p = test_config("tp-off", "touchpad", MAC);
        let rows = load_at(&p);
        apply_at(&p, 0, &rows, Change::Toggle, &nr).unwrap();
        let rows = load_at(&p);
        assert_eq!(rows[0].value, "false");
        assert!(std::fs::read_to_string(&p).unwrap().contains("        off\n"));
    }
}
