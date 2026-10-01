// SPDX-License-Identifier: GPL-3.0-or-later
// Bluetooth panel over bluetoothctl: power, the known devices, connect (pairing
// first when the device is new) and disconnect, and a background scan so new
// devices show up. As with wifi, picking a device only selects it.
//
// Pairing runs without an agent, so devices that need a PIN/confirmation will
// fail with bluetoothctl's own message; "just works" devices (headphones,
// speakers, most mice) pair fine.

use super::{pick, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] = &["power", "device", "state", "connect", "disconnect", "scan"];

static SELECTED: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Bool,
        1 => RowKind::Choice,
        2 => RowKind::Info,
        _ => RowKind::Action,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dev {
    pub mac: String,
    pub name: String,
}

fn is_mac(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':').all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `Device AA:BB:CC:DD:EE:FF Some Name` lines. Anything else (agent chatter,
/// `[NEW]` events) is ignored; the MAC is validated since it ends up in argv.
pub fn parse_devices(out: &str) -> Vec<Dev> {
    out.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ').unwrap_or((rest, ""));
            is_mac(mac).then(|| Dev { mac: mac.into(), name: name.trim().to_string() })
        })
        .collect()
}

fn shown(d: &Dev) -> &str {
    if d.name.is_empty() { &d.mac } else { &d.name }
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let blank = |why: &str| vec![r(why), r("-"), r("-"), r("-"), r("-"), r("-")];
    let Ok(show) = run("bluetoothctl", &["show"]) else {
        return blank("bluetoothctl unavailable");
    };
    if !show.contains("Controller") {
        return blank("no bluetooth adapter");
    }
    let powered = show.lines().any(|l| l.trim() == "Powered: yes");
    let list = |args: &[&str]| run("bluetoothctl", args).map(|o| parse_devices(&o)).unwrap_or_default();
    let all = list(&["devices"]);
    let paired = list(&["devices", "Paired"]);
    let connected = list(&["devices", "Connected"]);
    let has = |v: &[Dev], d: &Dev| v.iter().any(|x| x.mac == d.mac);

    // Connected first, then paired, then newly discovered; by name within each.
    let mut devs = all;
    devs.sort_by_key(|d| {
        (!has(&connected, d), !has(&paired, d), shown(d).to_lowercase())
    });
    if devs.is_empty() {
        return vec![r(&powered.to_string()), r("none known"), r("-"), r("-"), r("-"), scan_row()];
    }
    let tag = |d: &Dev| {
        let t = if has(&connected, d) {
            "connected"
        } else if has(&paired, d) {
            "paired"
        } else {
            "new"
        };
        format!("{}  [{t}]", shown(d))
    };
    let want = SELECTED.get();
    let sel = devs
        .iter()
        .find(|d| d.mac == want)
        .or_else(|| devs.iter().find(|d| has(&connected, d)))
        .unwrap_or(&devs[0]);
    let (is_conn, is_pair) = (has(&connected, sel), has(&paired, sel));
    let state = if is_conn {
        "connected"
    } else if is_pair {
        "paired, not connected"
    } else {
        "not paired"
    };
    vec![
        r(&powered.to_string()),
        Row { value: tag(sel), choices: devs.iter().map(|d| (tag(d), d.mac.clone())).collect() },
        r(state),
        Row {
            value: if is_pair { format!("connect {}", shown(sel)) } else { format!("pair and connect {}", shown(sel)) },
            choices: vec![(sel.mac.clone(), if is_pair { "paired" } else { "new" }.into())],
        },
        Row {
            value: if is_conn { format!("disconnect {}", shown(sel)) } else { "-".into() },
            choices: vec![(sel.mac.clone(), String::new())],
        },
        scan_row(),
    ]
}

fn scan_row() -> Row {
    Row { value: "scan for 10s, then reselect Bluetooth".into(), choices: vec![] }
}

/// bluetoothctl exits 0 even when the operation failed; the verdict is in text.
fn check(out: &str, ok_marker: &str) -> Result<(), String> {
    if let Some(l) = out.lines().find(|l| l.contains("Failed") || l.contains("not available")) {
        return Err(l.trim().to_string());
    }
    if out.contains(ok_marker) { Ok(()) } else { Err(format!("no confirmation from bluetoothctl: {}", out.trim())) }
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let target = |f: usize| -> Result<(String, String), String> {
        rows.get(f).and_then(|r| r.choices.first().cloned()).ok_or_else(|| "no device selected".to_string())
    };
    match field {
        0 => {
            let on = row.value == "true";
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !on,
            };
            let out = run("bluetoothctl", &["power", if want { "on" } else { "off" }])?;
            check(&out, "succeeded")?;
            Ok(format!("bluetooth {}", if want { "on" } else { "off" }))
        }
        1 => {
            let (label, id) = pick(row, &ch, "device")?;
            SELECTED.set(id);
            Ok(format!("selected {label}"))
        }
        3 => {
            let (mac, flag) = target(3)?;
            if flag == "new" {
                // timeout(1): an unanswered pairing must not hang the window.
                check(&run("timeout", &["20", "bluetoothctl", "pair", &mac])?, "Pairing successful")?;
                // Trusted devices reconnect on their own next time.
                let _ = run("bluetoothctl", &["trust", &mac]);
            }
            check(&run("timeout", &["15", "bluetoothctl", "connect", &mac])?, "Connection successful")?;
            Ok(format!("connected {}", row.value.trim_start_matches("pair and ").trim_start_matches("connect ")))
        }
        4 => {
            let (mac, _) = target(4)?;
            if row.value == "-" {
                return Err("not connected".into());
            }
            check(&run("bluetoothctl", &["disconnect", &mac])?, "Successful disconnected")?;
            Ok("disconnected".into())
        }
        5 => {
            // Backgrounded so the window doesn't freeze for the scan's duration.
            run("sh", &["-c", "bluetoothctl --timeout 10 scan on >/dev/null 2>&1 &"])?;
            Ok("scanning for 10s — reselect Bluetooth to see new devices".into())
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ALL: &str = "Device F0:D1:B8:21:57:5C SYLVANIA A19 T-C-M575C\nDevice F4:4E:FD:52:89:C1 SoundCore mini\nDevice 28:73:F6:13:00:18 Echo Pop-4WF\n[NEW] Device AA:AA:AA:AA:AA:AA chatter\nDevice nothex Bad\n";
    const PAIRED: &str = "Device F4:4E:FD:52:89:C1 SoundCore mini\nDevice 28:73:F6:13:00:18 Echo Pop-4WF\n";
    const CONNECTED: &str = "Device 28:73:F6:13:00:18 Echo Pop-4WF\n";
    const SHOW: &str = "Controller 14:7D:DA:26:07:C9 (public)\n\tPowered: yes\n";

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fake(a: &[&str]) -> Result<String, String> {
        Ok(match a {
            ["show"] => SHOW.into(),
            ["devices"] => ALL.into(),
            ["devices", "Paired"] => PAIRED.into(),
            ["devices", "Connected"] => CONNECTED.into(),
            _ => String::new(),
        })
    }

    #[test]
    fn parses_only_well_formed_device_lines() {
        let d = parse_devices(ALL);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].name, "SYLVANIA A19 T-C-M575C");
        assert!(is_mac("F4:4E:FD:52:89:C1") && !is_mac("F4:4E:FD:52:89") && !is_mac("zz:4E:FD:52:89:C1"));
    }

    #[test]
    fn loads_connected_first_and_states() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let rows = load(&|_, a| fake(a));
        assert_eq!(rows[0].value, "true");
        assert_eq!(rows[1].value, "Echo Pop-4WF  [connected]"); // default selection
        assert_eq!(rows[2].value, "connected");
        let order: Vec<_> = rows[1].choices.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(order[1], "SoundCore mini  [paired]");
        assert_eq!(order[2], "SYLVANIA A19 T-C-M575C  [new]");
        assert_eq!(rows[4].value, "disconnect Echo Pop-4WF");
    }

    #[test]
    fn new_device_pairs_trusts_then_connects() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("F0:D1:B8:21:57:5C");
        let rows = load(&|_, a| fake(a));
        SELECTED.set("");
        assert!(rows[3].value.starts_with("pair and connect"));
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(if a.contains(&"pair") {
                "Pairing successful".into()
            } else if a.contains(&"connect") {
                "Connection successful".into()
            } else {
                String::new()
            })
        };
        apply(3, &rows, Change::Toggle, &rec).unwrap();
        assert_eq!(
            *log.borrow(),
            [
                "timeout 20 bluetoothctl pair F0:D1:B8:21:57:5C",
                "bluetoothctl trust F0:D1:B8:21:57:5C",
                "timeout 15 bluetoothctl connect F0:D1:B8:21:57:5C",
            ]
        );
    }

    #[test]
    fn failure_text_becomes_an_error_even_with_exit_zero() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let rows = load(&|_, a| fake(a));
        let fail = |_: &str, _: &[&str]| -> Result<String, String> { Ok("Failed to connect: org.bluez.Error.Failed\n".into()) };
        let e = apply(3, &rows, Change::Toggle, &fail).unwrap_err();
        assert!(e.contains("Failed to connect"));
    }

    #[test]
    fn scan_is_backgrounded_and_missing_tool_degrades() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        let rows = vec![Row::default(); 6];
        apply(5, &rows, Change::Toggle, &rec).unwrap();
        assert!(log.borrow()[0].ends_with("&"), "scan must not block the UI");
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        let r = load(&none);
        assert_eq!(r.len(), LABELS.len());
        assert_eq!(r[0].value, "bluetoothctl unavailable");
    }
}
