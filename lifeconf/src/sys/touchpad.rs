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
    "natural scroll",
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
    (5, "click-method", &["clickfinger", "button-areas"]),
    (6, "scroll-method", &["two-finger", "edge", "on-button-down", "no-scroll"]),
    (7, "accel-profile", &["adaptive", "flat"]),
    (9, "tap-button-map", &["left-right-middle", "left-middle-right"]),
];

/// (row, node name) for the on/off rows. `enabled` is the inverse of niri's `off`.
const FLAGS: &[(usize, &str)] = &[(1, "tap"), (2, "natural-scroll"), (3, "dwt"), (4, "disabled-on-external-mouse")];

pub fn kind(field: usize) -> RowKind {
    match field {
        0..=4 => RowKind::Bool,
        8 => RowKind::Int(10),
        _ => RowKind::Choice,
    }
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
    rows[8] = r(&((speed * 100.0).round() as i32).to_string());
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
            ["true", "true", "true", "true", "false", "clickfinger", "two-finger", "adaptive", "0", "default"]
        );
    }

    #[test]
    fn unconfigured_desktop_shows_defaults_and_first_change_creates_the_block() {
        let p = test_config("tp-none", "touchpad", NONE);
        let rows = load_at(&p);
        assert_eq!(rows[1].value, "false");
        assert_eq!(rows[5].value, "default");
        apply_at(&p, 1, &rows, Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("touchpad {\n        tap\n    }"), "{out}");
        assert_eq!(load_at(&p)[1].value, "true");
    }

    #[test]
    fn choices_default_removes_the_node_and_empty_block_disappears() {
        let p = test_config("tp-clr", "touchpad", "    touchpad {\n        click-method \"clickfinger\"\n    }");
        let rows = load_at(&p);
        apply_at(&p, 5, &rows, Change::Step(-1), &nr).unwrap(); // clickfinger -> default
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(!out.contains("touchpad {") && !out.contains("click-method"));
    }

    #[test]
    fn changes_preserve_unknown_nodes() {
        let p = test_config("tp-keep", "touchpad", "    touchpad {\n        tap\n        scroll-factor 2.0\n        drag-lock\n    }");
        let rows = load_at(&p);
        apply_at(&p, 2, &rows, Change::Toggle, &nr).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("scroll-factor 2.0") && out.contains("drag-lock") && out.contains("natural-scroll"));
    }

    #[test]
    fn accel_speed_is_percent_clamped_and_zero_is_removed() {
        let p = test_config("tp-acc", "touchpad", NONE);
        let rows = load_at(&p);
        apply_at(&p, 8, &rows, Change::Text("35".into()), &nr).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("accel-speed 0.35"));
        let rows = load_at(&p);
        assert_eq!(rows[8].value, "35");
        apply_at(&p, 8, &rows, Change::Text("900".into()), &nr).unwrap();
        assert_eq!(load_at(&p)[8].value, "100");
        apply_at(&p, 8, &rows, Change::Text("0".into()), &nr).unwrap();
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
