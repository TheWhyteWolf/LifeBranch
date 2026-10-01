// SPDX-License-Identifier: GPL-3.0-or-later
// Mouse panel: libinput's settings for external mice and trackballs, written
// into the `mouse` region of niri's config the same way the Touchpad panel
// writes its own (see niri_input and touchpad.rs). A setting at niri's default
// is absent from the config; "default" in a choice row removes it. Unknown
// nodes in the block (scroll-button, ...) are kept as they are.

use super::kdl::{self, Node};
use super::niri_input::{config_path, read, write};
use super::{pick, Change, Row, RowKind, Runner};

pub const LABELS: &[&str] =
    &["enabled", "natural scroll", "left-handed", "middle-click emulation", "accel profile", "accel speed %", "scroll speed %"];

const DEFAULT: &str = "default";
const FLAGS: &[(usize, &str)] = &[(1, "natural-scroll"), (2, "left-handed"), (3, "middle-emulation")];

pub fn kind(field: usize) -> RowKind {
    match field {
        0..=3 => RowKind::Bool,
        4 => RowKind::Choice,
        _ => RowKind::Int(10),
    }
}

fn block(nodes: &[Node]) -> &[Node] {
    kdl::find(nodes, "mouse").and_then(|n| n.children.as_deref()).unwrap_or(&[])
}

fn num(nodes: &[Node], key: &str, default: f64) -> f64 {
    kdl::get_str(nodes, key).and_then(|s| s.parse().ok()).unwrap_or(default)
}

pub fn load(_run: Runner) -> Vec<Row> {
    load_at(&config_path())
}

pub fn load_at(path: &str) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let nodes = match read(path, "mouse") {
        Ok(n) => n,
        Err(e) => return std::iter::once(r(&e)).chain((1..LABELS.len()).map(|_| r("-"))).collect(),
    };
    let m = block(&nodes);
    let mut rows = vec![r("-"); LABELS.len()];
    rows[0] = r(&(!kdl::has(m, "off")).to_string());
    for (i, key) in FLAGS {
        rows[*i] = r(&kdl::has(m, key).to_string());
    }
    let mut choices = vec![(DEFAULT.to_string(), DEFAULT.to_string())];
    choices.extend(["adaptive", "flat"].iter().map(|o| (o.to_string(), o.to_string())));
    rows[4] = Row { value: kdl::get_str(m, "accel-profile").unwrap_or_else(|| DEFAULT.into()), choices };
    // accel-speed is -1..1 and scroll-factor a multiplier; both shown as percent.
    rows[5] = r(&((num(m, "accel-speed", 0.0) * 100.0).round() as i32).to_string());
    rows[6] = r(&((num(m, "scroll-factor", 1.0) * 100.0).round() as i32).to_string());
    rows
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_at(&config_path(), field, rows, ch, run)
}

pub fn apply_at(path: &str, field: usize, rows: &[Row], ch: Change, _run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let mut nodes = read(path, "mouse")?;
    let msg;
    {
        let m = kdl::block_mut(&mut nodes, "mouse");
        let flip = |cur: bool, ch: &Change| match ch {
            Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
            _ => !cur,
        };
        let step = |cur: i32, ch: &Change| -> Result<i32, String> {
            Ok(match ch {
                Change::Step(d) => cur + d * 10,
                Change::Text(t) => t.trim().trim_end_matches('%').parse().map_err(|_| format!("not a number: {:?}", t.trim()))?,
                Change::Toggle => cur,
            })
        };
        match field {
            0 => {
                let want = flip(row.value == "true", &ch);
                kdl::put(m, "off", (!want).then(|| Node::flag("off")));
                msg = format!("mouse {}", if want { "on" } else { "off" });
            }
            4 => {
                let (label, id) = pick(row, &ch, "accel profile")?;
                kdl::put(m, "accel-profile", (id != DEFAULT).then(|| Node::str("accel-profile", id)));
                msg = format!("accel profile {label}");
            }
            5 => {
                let next = step(row.value.parse().unwrap_or(0), &ch)?.clamp(-100, 100);
                let v = format!("{}", next as f64 / 100.0);
                kdl::put(m, "accel-speed", (next != 0).then(|| Node::num("accel-speed", &v)));
                msg = format!("accel speed {next}%");
            }
            6 => {
                let next = step(row.value.parse().unwrap_or(100), &ch)?.clamp(10, 500);
                let v = format!("{}", next as f64 / 100.0);
                kdl::put(m, "scroll-factor", (next != 100).then(|| Node::num("scroll-factor", &v)));
                msg = format!("scroll speed {next}%");
            }
            _ => {
                let (_, key) = FLAGS.iter().find(|(i, _)| *i == field).ok_or("read-only")?;
                let want = flip(row.value == "true", &ch);
                kdl::put(m, key, want.then(|| Node::flag(key)));
                msg = format!("{} {}", LABELS[field], if want { "on" } else { "off" });
            }
        }
    }
    // An empty `mouse {}` is what the config ships with; keep the block.
    write(path, "mouse", &nodes)?;
    Ok(format!("{msg} (niri applies it now)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An input block whose only mouse node is inside the region (the shared
    /// helper appends a `mouse {}` of its own, made for the other panels).
    fn test_config(tag: &str, _region: &str, inner: &str) -> String {
        let dir = std::env::temp_dir().join(format!("lc-mouse-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.kdl");
        std::fs::write(&p, format!("input {{\n    // LIFEBRANCH:BEGIN mouse\n{inner}\n    // LIFEBRANCH:END mouse\n}}\n")).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn nr(_: &str, _: &[&str]) -> Result<String, String> {
        Ok(String::new())
    }

    #[test]
    fn empty_block_shows_defaults() {
        let p = test_config("ms-empty", "mouse", "    mouse {}");
        let v: Vec<String> = load_at(&p).into_iter().map(|r| r.value).collect();
        assert_eq!(v, ["true", "false", "false", "false", "default", "0", "100"]);
    }

    #[test]
    fn changes_write_nodes_and_defaults_remove_them() {
        let p = test_config("ms-set", "mouse", "    mouse {\n        scroll-button 273\n    }");
        let rows = load_at(&p);
        apply_at(&p, 2, &rows, Change::Toggle, &nr).unwrap();
        apply_at(&p, 4, &load_at(&p), Change::Text("flat".into()), &nr).unwrap();
        apply_at(&p, 5, &load_at(&p), Change::Text("-40".into()), &nr).unwrap();
        apply_at(&p, 6, &load_at(&p), Change::Text("150".into()), &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        for want in ["left-handed", "accel-profile \"flat\"", "accel-speed -0.4", "scroll-factor 1.5", "scroll-button 273"] {
            assert!(out.contains(want), "{want} missing:\n{out}");
        }
        assert_eq!(load_at(&p)[5].value, "-40");
        apply_at(&p, 6, &load_at(&p), Change::Text("100".into()), &nr).unwrap();
        apply_at(&p, 4, &load_at(&p), Change::Text("default".into()), &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("scroll-factor") && !out.contains("accel-profile"));
    }

    #[test]
    fn off_and_ranges() {
        let p = test_config("ms-off", "mouse", "    mouse {}");
        apply_at(&p, 0, &load_at(&p), Change::Toggle, &nr).unwrap();
        assert_eq!(load_at(&p)[0].value, "false");
        apply_at(&p, 6, &load_at(&p), Change::Text("9999".into()), &nr).unwrap();
        assert_eq!(load_at(&p)[6].value, "500");
    }
}
