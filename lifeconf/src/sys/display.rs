// SPDX-License-Identifier: GPL-3.0-or-later
// Display panel over `niri msg`: pick an output, then its mode, scale,
// transform and on/off. These are niri's *temporary* output changes — they
// vanish when niri reloads its config — so a bad choice is always one reload
// (or a restart) from undone. Persisting them into config.kdl is deliberately
// not done here: the per-machine `output` blocks are hand-written.

use super::{pick, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] = &["output", "mode", "scale", "transform", "enabled"];

const SCALES: &[&str] = &["1", "1.25", "1.5", "1.75", "2", "2.5", "3"];
const TRANSFORMS: &[&str] =
    &["normal", "90", "180", "270", "flipped", "flipped-90", "flipped-180", "flipped-270"];
const TEMP: &str = "(temporary: a niri config reload reverts it)";

/// Which output the other rows are about. It's view state, not system state,
/// but a panel reload must not forget it; one panel instance per process.
static SELECTED: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    if field == 4 { RowKind::Bool } else { RowKind::Choice }
}

#[derive(Debug, Clone)]
pub struct Out {
    pub name: String,
    pub enabled: bool,
    /// niri CLI mode strings, e.g. "3072x1920@60.000".
    pub modes: Vec<String>,
    pub current_mode: Option<usize>,
    pub scale: f64,
    pub transform: String,
}

impl Out {
    fn label(&self) -> String {
        if self.enabled { self.name.clone() } else { format!("{} (off)", self.name) }
    }
}

/// `niri msg --json outputs` -> outputs sorted by connector name.
pub fn parse(json: &str) -> Result<Vec<Out>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("niri json: {e}"))?;
    let map = v.as_object().ok_or("niri json: not an object")?;
    let mut outs: Vec<Out> = map
        .iter()
        .map(|(name, o)| {
            let modes = o["modes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|m| {
                            format!(
                                "{}x{}@{:.3}",
                                m["width"].as_u64().unwrap_or(0),
                                m["height"].as_u64().unwrap_or(0),
                                m["refresh_rate"].as_f64().unwrap_or(0.0) / 1000.0
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let logical = &o["logical"];
            Out {
                name: name.clone(),
                // A disabled output has no logical geometry.
                enabled: !logical.is_null(),
                modes,
                current_mode: o["current_mode"].as_u64().map(|i| i as usize),
                scale: logical["scale"].as_f64().unwrap_or(1.0),
                transform: norm_transform(logical["transform"].as_str().unwrap_or("Normal")),
            }
        })
        .collect();
    outs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(outs)
}

/// niri-ipc spells transforms `Normal`, `_90`, `Flipped270`; the CLI wants
/// `normal`, `90`, `flipped-270`.
fn norm_transform(s: &str) -> String {
    let l = s.to_lowercase();
    let l = l.trim_start_matches('_');
    match l.strip_prefix("flipped") {
        Some(rest) if !rest.is_empty() => format!("flipped-{}", rest.trim_start_matches('-')),
        _ => l.to_string(),
    }
}

fn fmt_scale(s: f64) -> String {
    let t = format!("{s:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn choice(label: &str) -> (String, String) {
    (label.to_string(), label.to_string())
}

/// The output the panel is looking at: the remembered one if it still exists,
/// else the first enabled one, else the first.
fn selected(outs: &[Out]) -> Option<&Out> {
    let want = SELECTED.get();
    outs.iter()
        .find(|o| o.name == want)
        .or_else(|| outs.iter().find(|o| o.enabled))
        .or_else(|| outs.first())
}

pub fn load(run: Runner) -> Vec<Row> {
    let blank = |why: &str| {
        let r = |v: &str| Row { value: v.into(), choices: vec![] };
        vec![r(why), r("-"), r("-"), r("-"), r("-")]
    };
    let json = match run("niri", &["msg", "--json", "outputs"]) {
        Ok(j) => j,
        Err(_) => return blank("niri unavailable"),
    };
    let Ok(outs) = parse(&json) else { return blank("unreadable") };
    let Some(sel) = selected(&outs) else { return blank("no outputs") };

    let mut scales: Vec<String> = SCALES.iter().map(|s| s.to_string()).collect();
    let cur_scale = fmt_scale(sel.scale);
    if sel.enabled && !scales.contains(&cur_scale) {
        // A fractional scale set elsewhere (config, wdisplays) must still show.
        scales.push(cur_scale.clone());
        scales.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).total_cmp(&b.parse::<f64>().unwrap_or(0.0)));
    }
    let off = |v: &str| Row { value: v.into(), choices: vec![] };
    vec![
        Row {
            value: sel.label(),
            choices: outs.iter().map(|o| (o.label(), o.name.clone())).collect(),
        },
        if sel.enabled {
            Row {
                value: sel.current_mode.and_then(|i| sel.modes.get(i)).cloned().unwrap_or_else(|| "-".into()),
                choices: sel.modes.iter().map(|m| choice(m)).collect(),
            }
        } else {
            off("-")
        },
        if sel.enabled {
            Row { value: cur_scale, choices: scales.iter().map(|s| choice(s)).collect() }
        } else {
            off("-")
        },
        if sel.enabled {
            Row { value: sel.transform.clone(), choices: TRANSFORMS.iter().map(|t| choice(t)).collect() }
        } else {
            off("-")
        },
        Row { value: sel.enabled.to_string(), choices: vec![] },
    ]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let out_row = rows.first().ok_or("no outputs")?;
    // The output row's value is the *label*; its id is the connector name.
    let name = out_row
        .choices
        .iter()
        .find(|(l, _)| *l == out_row.value)
        .map(|(_, id)| id.clone())
        .ok_or("no outputs")?;
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        0 => {
            let (label, id) = pick(row, &ch, "output")?;
            SELECTED.set(id);
            Ok(format!("showing {label}"))
        }
        1 | 3 => {
            let what = if field == 1 { "mode" } else { "transform" };
            let (label, id) = pick(row, &ch, what)?;
            run("niri", &["msg", "output", &name, what, id])?;
            Ok(format!("{name} {what} {label} {TEMP}"))
        }
        2 => {
            // Beyond the presets, any sane number is fine (niri takes 1.33 etc.).
            let scale = match &ch {
                Change::Text(t) if pick(row, &ch, "scale").is_err() => {
                    let v: f64 = t.trim().parse().map_err(|_| format!("not a number: {:?}", t.trim()))?;
                    fmt_scale(v.clamp(0.25, 8.0))
                }
                _ => pick(row, &ch, "scale")?.1.clone(),
            };
            run("niri", &["msg", "output", &name, "scale", &scale])?;
            Ok(format!("{name} scale {scale} {TEMP}"))
        }
        _ => {
            let on = row.value == "true";
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !on,
            };
            if !want {
                // Turning off the last lit output leaves nothing to click "on" with.
                let lit = out_row.choices.iter().filter(|(l, _)| !l.ends_with("(off)")).count();
                if lit <= 1 {
                    return Err("refusing to turn off the only active output".into());
                }
            }
            run("niri", &["msg", "output", &name, if want { "on" } else { "off" }])?;
            Ok(format!("{name} {} {TEMP}", if want { "on" } else { "off" }))
        }
    }
}

/// The `niri msg` calls that put the selected output back as it is now, built
/// BEFORE a change is applied (the rows still describe the old state). Choosing
/// which output to look at changes nothing on the system, so it has no undo.
pub fn undo(field: usize, rows: &[Row], _ch: &Change) -> Option<Vec<Vec<String>>> {
    let out_row = rows.first()?;
    let name = out_row.choices.iter().find(|(l, _)| *l == out_row.value).map(|(_, id)| id.clone())?;
    let prior = rows.get(field)?.value.clone();
    let cmd = |tail: &[&str]| {
        let mut v: Vec<String> = ["msg", "output", name.as_str()].iter().map(|s| s.to_string()).collect();
        v.extend(tail.iter().map(|s| s.to_string()));
        vec![v]
    };
    match field {
        1 | 3 if prior != "-" => Some(cmd(&[if field == 1 { "mode" } else { "transform" }, &prior])),
        2 if prior != "-" => Some(cmd(&["scale", &prior])),
        4 => {
            // Toggle flips the current state, so the way back is the current state.
            let was_on = prior == "true";
            Some(cmd(&[if was_on { "on" } else { "off" }]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // From a real `niri msg --json outputs` (trimmed), plus a second lit output.
    const JSON: &str = r#"{
      "eDP-1":{"name":"eDP-1","modes":[{"width":3072,"height":1920,"refresh_rate":60000,"is_preferred":true}],
        "current_mode":0,"logical":{"x":0,"y":0,"width":1536,"height":960,"scale":2.0,"transform":"Normal"}},
      "eDP-2":{"name":"eDP-2","modes":[],"current_mode":null,"logical":null},
      "DP-1":{"name":"DP-1","modes":[{"width":2560,"height":1440,"refresh_rate":59951},{"width":2560,"height":1440,"refresh_rate":143912}],
        "current_mode":1,"logical":{"x":1536,"y":0,"width":1706,"height":960,"scale":1.5,"transform":"_90"}}}"#;

    fn fake(json: &'static str) -> impl Fn(&str, &[&str]) -> Result<String, String> {
        move |_, _| Ok(json.to_string())
    }

    // SELECTED is process-global, so tests that touch it must not interleave.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset() -> std::sync::MutexGuard<'static, ()> {
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        g
    }

    #[test]
    fn parses_outputs_modes_and_transforms() {
        let outs = parse(JSON).unwrap();
        let names: Vec<_> = outs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["DP-1", "eDP-1", "eDP-2"]);
        assert_eq!(outs[0].modes, ["2560x1440@59.951", "2560x1440@143.912"]);
        assert_eq!(outs[0].transform, "90");
        assert!(!outs[2].enabled);
        assert_eq!(norm_transform("Flipped270"), "flipped-270");
        assert_eq!(norm_transform("Normal"), "normal");
    }

    #[test]
    fn rows_reflect_the_selected_output() {
        let _g = reset();
        let rows = load(&fake(JSON));
        assert_eq!(rows[0].value, "DP-1"); // first enabled, sorted
        assert_eq!(rows[1].value, "2560x1440@143.912");
        assert_eq!(rows[2].value, "1.5");
        assert_eq!(rows[3].value, "90");
        assert_eq!(rows[4].value, "true");
        assert!(rows[0].choices.iter().any(|(l, id)| l == "eDP-2 (off)" && id == "eDP-2"));
    }

    #[test]
    fn odd_scale_still_listed() {
        let _g = reset();
        let j = JSON.replace("\"scale\":1.5", "\"scale\":1.33");
        let rows = load(&fake(Box::leak(j.into_boxed_str())));
        assert!(rows[2].choices.iter().any(|(l, _)| l == "1.33"));
        assert_eq!(rows[2].value, "1.33");
    }

    #[test]
    fn changes_target_the_selected_output() {
        let _g = reset();
        let rows = load(&fake(JSON));
        let log = RefCell::new(Vec::<String>::new());
        let rec = |_: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(a.join(" "));
            Ok(String::new())
        };
        apply(2, &rows, Change::Text("1.75".into()), &rec).unwrap();
        apply(2, &rows, Change::Text("1.33".into()), &rec).unwrap(); // not a preset
        apply(3, &rows, Change::Step(1), &rec).unwrap(); // 90 -> 180
        apply(1, &rows, Change::Step(1), &rec).unwrap(); // wraps to first mode
        apply(4, &rows, Change::Toggle, &rec).unwrap(); // DP-1 off (eDP-1 stays lit)
        assert_eq!(
            *log.borrow(),
            [
                "msg output DP-1 scale 1.75",
                "msg output DP-1 scale 1.33",
                "msg output DP-1 transform 180",
                "msg output DP-1 mode 2560x1440@59.951",
                "msg output DP-1 off",
            ]
        );
    }

    #[test]
    fn refuses_to_turn_off_the_last_output() {
        let _g = reset();
        let single = r#"{"eDP-1":{"modes":[],"current_mode":null,"logical":{"scale":2.0,"transform":"Normal"}}}"#;
        let rows = load(&fake(single));
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        let e = apply(4, &rows, Change::Toggle, &boom).unwrap_err();
        assert!(e.contains("only active output"));
    }

    #[test]
    fn selecting_an_output_switches_the_other_rows() {
        let _g = reset();
        let rows = load(&fake(JSON));
        let none = |_: &str, _: &[&str]| -> Result<String, String> { panic!("selection runs nothing") };
        apply(0, &rows, Change::Step(1), &none).unwrap(); // DP-1 -> eDP-1
        let rows = load(&fake(JSON));
        assert_eq!(rows[0].value, "eDP-1");
        assert_eq!(rows[2].value, "2");
    }

    #[test]
    fn undo_restores_the_previous_value() {
        let _g = reset();
        let rows = load(&fake(JSON)); // DP-1: 2560x1440@143.912, 1.5, 90, on
        let u = |f: usize| undo(f, &rows, &Change::Step(1)).unwrap()[0].join(" ");
        assert_eq!(u(1), "msg output DP-1 mode 2560x1440@143.912");
        assert_eq!(u(2), "msg output DP-1 scale 1.5");
        assert_eq!(u(3), "msg output DP-1 transform 90");
        assert_eq!(u(4), "msg output DP-1 on");
        assert!(undo(0, &rows, &Change::Step(1)).is_none(), "picking an output changes nothing");
    }

    #[test]
    fn missing_niri_degrades() {
        let _g = reset();
        let f = |_: &str, _: &[&str]| -> Result<String, String> { Err("niri: not found".into()) };
        let rows = load(&f);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "niri unavailable");
    }
}
