// SPDX-License-Identifier: GPL-3.0-or-later
// Sound panel over wpctl (WirePlumber/PipeWire): default output and input
// device, their volume and mute. Always acts on the *default* sink/source, so
// the volume rows follow whatever the device rows select.

use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] =
    &["output device", "output volume %", "output muted", "input device", "input volume %", "input muted"];

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";
/// wpctl allows overdrive; 150% is where PulseAudio-era UIs stop too.
const MAX_PCT: i32 = 150;

pub fn kind(field: usize) -> RowKind {
    match field {
        0 | 3 => RowKind::Choice,
        1 | 4 => RowKind::Int(5),
        _ => RowKind::Bool,
    }
}

#[derive(Debug, PartialEq)]
pub struct Dev {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Pull the Audio section's "Sinks:" and "Sources:" out of `wpctl status`.
/// The Video section has its own Sinks/Sources, hence the top-level tracking.
pub fn parse_status(out: &str) -> (Vec<Dev>, Vec<Dev>) {
    let (mut sinks, mut sources) = (Vec::new(), Vec::new());
    let mut in_audio = false;
    let mut sect = "";
    for line in out.lines() {
        // Top-level headings are unindented ("Audio", "Video", "Settings").
        if !line.starts_with(' ') && !line.is_empty() {
            in_audio = line.trim() == "Audio";
            sect = "";
            continue;
        }
        if !in_audio {
            continue;
        }
        let t = line.trim_start_matches(|c: char| " │├└─".contains(c));
        match t.trim_end() {
            "Sinks:" => sect = "sinks",
            "Sources:" => sect = "sources",
            // Any other heading ("Devices:", "Sink endpoints:", ...) ends the section.
            h if h.ends_with(':') && parse_dev(line).is_none() => sect = "",
            _ if sect.is_empty() => {}
            _ => {
                if let Some(d) = parse_dev(line) {
                    if sect == "sinks" { sinks.push(d) } else { sources.push(d) }
                }
            }
        }
    }
    (sinks, sources)
}

/// ` │  *   60. Apple Audio Device Pro 2   [vol: 0.98]`
fn parse_dev(line: &str) -> Option<Dev> {
    let t = line.trim_start_matches(|c: char| " │├└─".contains(c));
    let (default, t) = match t.strip_prefix('*') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let t = t.trim_start();
    let (id, rest) = t.split_once('.')?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let name = match rest.rfind('[') {
        Some(i) => &rest[..i],
        None => rest,
    };
    Some(Dev { id: id.into(), name: name.trim().into(), default })
}

/// `Volume: 0.45` or `Volume: 0.45 [MUTED]` -> (percent, muted).
pub fn parse_volume(out: &str) -> Option<(i32, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?;
    let num = rest.split_whitespace().next()?;
    let v: f64 = num.parse().ok()?;
    Some(((v * 100.0).round() as i32, rest.contains("MUTED")))
}

fn device_row(devs: &[Dev]) -> Row {
    Row {
        value: devs.iter().find(|d| d.default).map(|d| d.name.clone()).unwrap_or_else(|| "none".into()),
        choices: devs.iter().map(|d| (d.name.clone(), d.id.clone())).collect(),
    }
}

pub fn load(run: Runner) -> Vec<Row> {
    let unavailable = |why: &str| {
        let row = |v: &str| Row { value: v.into(), choices: vec![] };
        vec![row(why), row("-"), row("-"), row(why), row("-"), row("-")]
    };
    let Ok(status) = run("wpctl", &["status"]) else {
        return unavailable("wpctl unavailable");
    };
    let (sinks, sources) = parse_status(&status);
    let vol = |target: &str| {
        run("wpctl", &["get-volume", target]).ok().and_then(|o| parse_volume(&o))
    };
    let mut rows = vec![device_row(&sinks)];
    for target in [SINK, SOURCE] {
        match vol(target) {
            Some((pct, muted)) => {
                rows.push(Row { value: pct.to_string(), choices: vec![] });
                rows.push(Row { value: muted.to_string(), choices: vec![] });
            }
            None => {
                rows.push(Row { value: "-".into(), choices: vec![] });
                rows.push(Row { value: "-".into(), choices: vec![] });
            }
        }
        if target == SINK {
            rows.push(device_row(&sources));
        }
    }
    rows
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let target = if field < 3 { SINK } else { SOURCE };
    let what = if field < 3 { "output" } else { "input" };
    match kind(field) {
        RowKind::Choice => {
            let row = rows.get(field).ok_or("no devices")?;
            let (name, id) = super::pick(row, &ch, &format!("{what} device"))?;
            run("wpctl", &["set-default", id])?;
            Ok(format!("{what} -> {name}"))
        }
        RowKind::Int(step) => {
            let cur: i32 = rows.get(field).and_then(|r| r.value.parse().ok()).ok_or("volume unavailable")?;
            let next = match ch {
                Change::Step(d) => cur + d * step as i32,
                Change::Text(t) => t.trim().trim_end_matches('%').parse().map_err(|_| format!("not a number: {t:?}"))?,
                Change::Toggle => cur,
            }
            .clamp(0, MAX_PCT);
            run("wpctl", &["set-volume", target, &format!("{:.2}", next as f64 / 100.0)])?;
            Ok(format!("{what} volume {next}%"))
        }
        _ => {
            let cur = rows.get(field).map(|r| r.value == "true").ok_or("mute unavailable")?;
            let want = match ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !cur,
            };
            run("wpctl", &["set-mute", target, if want { "1" } else { "0" }])?;
            Ok(format!("{what} {}", if want { "muted" } else { "unmuted" }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Verbatim from a real `wpctl status` (Video section included on purpose).
    const STATUS: &str = "PipeWire 'pipewire-0' [1.6.9, voyd@Exit, cookie:604631901]
 └─ Clients:
        32. WirePlumber                         [1.6.9, voyd@Exit, pid:2469]

Audio
 ├─ Devices:
 │      52. Navi 10 HDMI Audio                  [alsa]
 │  
 ├─ Sinks:
 │      59. Apple Audio Device Pro              [vol: 0.59]
 │  *   60. Apple Audio Device Pro 2            [vol: 0.98]
 │  
 ├─ Sources:
 │  *   62. Apple Audio Device Pro 1            [vol: 1.00]
 │      63. Apple Audio Device Pro 3            [vol: 1.00 MUTED]
 │  
 ├─ Filters:
 │  
 └─ Streams:

Video
 ├─ Devices:
 │      34. FaceTime HD Camera (Built-in)       [v4l2]
 │  
 ├─ Sinks:
 │  
 ├─ Sources:
 │  *   88. FaceTime HD Camera (Built-in) (V4L2)
 │  
 └─ Streams:
";

    #[test]
    fn parses_audio_devices_only() {
        let (sinks, sources) = parse_status(STATUS);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[1], Dev { id: "60".into(), name: "Apple Audio Device Pro 2".into(), default: true });
        assert!(!sinks[0].default);
        assert_eq!(sources.len(), 2, "the camera in the Video section must not leak in");
        assert_eq!(sources[0].id, "62");
    }

    #[test]
    fn endpoint_sections_are_not_devices() {
        let out = "Audio\n ├─ Sinks:\n │      59. Out   [vol: 0.59]\n │  \n ├─ Sink endpoints:\n │      70. Bogus   [vol: 1.00]\n └─ Streams:\n";
        let (sinks, sources) = parse_status(out);
        assert_eq!(sinks.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["59"]);
        assert!(sources.is_empty());
    }

    #[test]
    fn parses_volume_and_mute() {
        assert_eq!(parse_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_volume("Volume: 1.00 [MUTED]"), Some((100, true)));
        assert_eq!(parse_volume("garbage"), None);
    }

    fn fake(status: &'static str) -> (impl Fn(&str, &[&str]) -> Result<String, String>, RefCell<Vec<String>>) {
        let calls = RefCell::new(Vec::new());
        let f = move |_: &str, args: &[&str]| -> Result<String, String> {
            Ok(match args[0] {
                "status" => status.to_string(),
                "get-volume" if args[1] == SINK => "Volume: 0.98\n".into(),
                "get-volume" => "Volume: 1.00 [MUTED]\n".into(),
                _ => String::new(),
            })
        };
        (f, calls)
    }

    #[test]
    fn loads_six_rows() {
        let (f, _) = fake(STATUS);
        let rows = load(&f);
        let vals: Vec<_> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(vals, ["Apple Audio Device Pro 2", "98", "false", "Apple Audio Device Pro 1", "100", "true"]);
        assert_eq!(rows[0].choices.len(), 2);
    }

    #[test]
    fn missing_wpctl_degrades() {
        let f = |_: &str, _: &[&str]| -> Result<String, String> { Err("wpctl: not found".into()) };
        let rows = load(&f);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "wpctl unavailable");
    }

    #[test]
    fn changes_issue_the_right_commands() {
        let (f, _) = fake(STATUS);
        let rows = load(&f);
        let log = RefCell::new(Vec::<String>::new());
        let rec = |_: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(a.join(" "));
            Ok(String::new())
        };
        apply(1, &rows, Change::Step(1), &rec).unwrap(); // 98 -> 103
        apply(1, &rows, Change::Text("500".into()), &rec).unwrap(); // clamped
        apply(2, &rows, Change::Toggle, &rec).unwrap(); // unmuted -> muted
        apply(5, &rows, Change::Toggle, &rec).unwrap(); // muted -> unmuted
        apply(0, &rows, Change::Step(1), &rec).unwrap(); // wraps to sink 59
        assert_eq!(
            *log.borrow(),
            [
                "set-volume @DEFAULT_AUDIO_SINK@ 1.03",
                "set-volume @DEFAULT_AUDIO_SINK@ 1.50",
                "set-mute @DEFAULT_AUDIO_SINK@ 1",
                "set-mute @DEFAULT_AUDIO_SOURCE@ 0",
                "set-default 59",
            ]
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_command() {
        let (f, _) = fake(STATUS);
        let rows = load(&f);
        let rec = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(apply(1, &rows, Change::Text("loud".into()), &rec).is_err());
        assert!(apply(0, &rows, Change::Text("nonexistent".into()), &rec).is_err());
    }
}
