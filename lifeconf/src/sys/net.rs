// SPDX-License-Identifier: GPL-3.0-or-later
// Network panel over nmcli: wifi on/off, the visible networks, connect to the
// one you picked, disconnect. Picking a network only *selects* it; the
// separate "connect" row acts, so cycling the list never joins anything.
//
// A secured network that has no saved profile needs a password, and a password
// must not travel on a command line (visible to every process via /proc). That
// case is refused here with a pointer to scripts/net-menu.sh, which feeds it to
// nmcli on stdin.

use super::{pick, split_terse, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] = &["wifi", "status", "network", "connect", "disconnect"];

static SELECTED: Sel = Sel::new();

/// Seconds nmcli may spend on one connect before we give up and report.
const WAIT: &str = "10";

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Bool,
        1 => RowKind::Info,
        2 => RowKind::Choice,
        _ => RowKind::Action,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ap {
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
}

/// `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list`: hidden networks
/// (empty SSID) dropped, one entry per SSID keeping its strongest signal, the
/// connected one first, then by signal.
pub fn parse_aps(out: &str) -> Vec<Ap> {
    let mut aps: Vec<Ap> = Vec::new();
    for line in out.lines() {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let ap = Ap {
            in_use: f[0].trim() == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            security: f[3].clone(),
        };
        match aps.iter_mut().find(|a| a.ssid == ap.ssid) {
            Some(old) => {
                let keep_use = old.in_use || ap.in_use;
                if ap.signal > old.signal {
                    *old = ap;
                }
                old.in_use = keep_use;
            }
            None => aps.push(ap),
        }
    }
    aps.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    aps
}

/// Names of saved wifi profiles from `nmcli -t -f NAME,TYPE connection show`.
pub fn parse_saved(out: &str) -> Vec<String> {
    out.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 2 && f[1] == "802-11-wireless")
        .map(|f| f[0].clone())
        .collect()
}

/// Saved wifi profiles as (ssid, profile name). A profile's name needn't match
/// its SSID ("Home 1", a custom name), so each one is asked which SSID it joins.
pub fn saved_ssids(run: Runner) -> Vec<(String, String)> {
    run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])
        .map(|o| parse_saved(&o))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| {
            let ssid = run(
                "nmcli",
                &["--escape", "no", "-g", "802-11-wireless.ssid", "connection", "show", "id", &name],
            )
            .ok()?;
            Some((ssid.trim_end_matches('\n').to_string(), name))
        })
        .collect()
}

pub fn is_open(security: &str) -> bool {
    security.is_empty() || security == "--"
}

fn label(ap: &Ap, saved: bool) -> String {
    let sec = if is_open(&ap.security) { "open" } else { ap.security.as_str() };
    format!("{}  {}%  {}{}", ap.ssid, ap.signal, sec, if saved { "  saved" } else { "" })
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let blank = |why: &str| vec![r(why), r("-"), r("-"), r("-"), r("-")];
    let Ok(radio) = run("nmcli", &["radio", "wifi"]) else {
        return blank("nmcli unavailable");
    };
    let on = radio.trim() == "enabled";
    if !on {
        return vec![r("false"), r("wifi is off"), r("-"), r("-"), r("-")];
    }
    let aps = run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"])
        .map(|o| parse_aps(&o))
        .unwrap_or_default();
    let saved = saved_ssids(run);
    let profile_of = |a: &Ap| saved.iter().find(|(s, _)| *s == a.ssid).map(|(_, n)| n.as_str());
    let active = aps.iter().find(|a| a.in_use);
    let status = match active {
        Some(a) => format!("connected: {} ({}%)", a.ssid, a.signal),
        None => "not connected".to_string(),
    };
    if aps.is_empty() {
        return vec![r("true"), r(&status), r("none visible"), r("-"), r("-")];
    }

    let want = SELECTED.get();
    let sel = aps
        .iter()
        .find(|a| a.ssid == want)
        .or(active)
        .unwrap_or(&aps[0]);
    let is_saved = |a: &Ap| profile_of(a).is_some();
    let flag = if let Some(name) = profile_of(sel) {
        format!("saved:{name}")
    } else if is_open(&sel.security) {
        "open".to_string()
    } else {
        "secured".to_string()
    };
    vec![
        r("true"),
        r(&status),
        Row {
            value: label(sel, is_saved(sel)),
            choices: aps.iter().map(|a| (label(a, is_saved(a)), a.ssid.clone())).collect(),
        },
        // The connect row carries what apply() needs to act on the selection.
        Row { value: format!("connect to {}", sel.ssid), choices: vec![(sel.ssid.clone(), flag)] },
        if active.is_some() { r("disconnect wifi") } else { r("-") },
    ]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        0 => {
            let on = row.value == "true";
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !on,
            };
            run("nmcli", &["radio", "wifi", if want { "on" } else { "off" }])?;
            Ok(format!("wifi {}", if want { "on" } else { "off" }))
        }
        2 => {
            let (label, id) = pick(row, &ch, "network")?;
            SELECTED.set(id);
            Ok(format!("selected {label}"))
        }
        3 => {
            let (ssid, flag) = row.choices.first().ok_or("no network selected")?;
            if ssid.starts_with('-') {
                return Err("refusing an SSID that looks like an option".into());
            }
            match flag.as_str() {
                f if f.starts_with("saved:") => {
                    run("nmcli", &["-w", WAIT, "connection", "up", "id", &f["saved:".len()..]])?
                }
                "open" => run("nmcli", &["-w", WAIT, "device", "wifi", "connect", ssid])?,
                _ => {
                    return Err(format!(
                        "{ssid} is secured and not saved yet: join it once from quick settings (Mod+A) or net-menu.sh, which ask for the password"
                    ))
                }
            };
            Ok(format!("connected to {ssid}"))
        }
        4 => {
            let devs = run("nmcli", &["-t", "-f", "DEVICE,TYPE", "device"])?;
            let dev = devs
                .lines()
                .map(split_terse)
                .find(|f| f.len() >= 2 && f[1] == "wifi")
                .map(|f| f[0].clone())
                .ok_or("no wifi device")?;
            run("nmcli", &["device", "disconnect", &dev])?;
            Ok(format!("disconnected {dev}"))
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Shape from real `nmcli -t` output, plus an escaped colon and a duplicate AP.
    const APS: &str = "*:BELL498 2.4:60:WPA2\n :agaspar-5G:47:WPA2\n:Cafe\\: Free:35:\n :agaspar-5G:52:WPA2\n:::WPA2\n:Weak:12:WPA2\n";
    const SAVED: &str = "BELL498 2.4:802-11-wireless\ntailscale0:tun\nWeak (2):802-11-wireless\n";

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fake(radio: &'static str) -> impl Fn(&str, &[&str]) -> Result<String, String> {
        move |_, a| {
            Ok(match a[0] {
                "radio" => radio.into(),
                // Profile -> SSID; "Weak (2)" is deliberately named unlike its SSID.
                "--escape" => match *a.last().unwrap() {
                    "Weak (2)" => "Weak\n".into(),
                    n => format!("{n}\n"),
                },
                "-t" if a.contains(&"IN-USE,SSID,SIGNAL,SECURITY") => APS.into(),
                "-t" if a.contains(&"NAME,TYPE") => SAVED.into(),
                "-t" => "wlan0:wifi\nlo:loopback\n".into(),
                _ => String::new(),
            })
        }
    }

    #[test]
    fn parses_aps_dedups_and_orders() {
        let aps = parse_aps(APS);
        let names: Vec<_> = aps.iter().map(|a| a.ssid.as_str()).collect();
        assert_eq!(names, ["BELL498 2.4", "agaspar-5G", "Cafe: Free", "Weak"]);
        assert_eq!(aps[1].signal, 52, "strongest duplicate wins");
        assert!(aps[0].in_use);
        assert!(is_open(&aps[2].security));
    }

    #[test]
    fn saved_profiles_are_wifi_only() {
        assert_eq!(parse_saved(SAVED), ["BELL498 2.4", "Weak (2)"]);
    }

    #[test]
    fn loads_rows_with_status_and_connect_target() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let rows = load(&fake("enabled"));
        assert_eq!(rows[0].value, "true");
        assert_eq!(rows[1].value, "connected: BELL498 2.4 (60%)");
        assert_eq!(rows[2].value, "BELL498 2.4  60%  WPA2  saved");
        assert_eq!(rows[3].choices[0], ("BELL498 2.4".into(), "saved:BELL498 2.4".into()));
        assert_eq!(rows[4].value, "disconnect wifi");
    }

    #[test]
    fn wifi_off_and_missing_nmcli_degrade() {
        let off = load(&fake("disabled"));
        assert_eq!((off[0].value.as_str(), off[1].value.as_str()), ("false", "wifi is off"));
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("nmcli: not found".into()) };
        let rows = load(&none);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "nmcli unavailable");
    }

    #[test]
    fn selecting_does_not_connect_and_connect_picks_the_right_command() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let rows = load(&fake("enabled"));
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("selecting must run nothing") };
        apply(2, &rows, Change::Step(1), &boom).unwrap(); // -> agaspar-5G
        let rows = load(&fake("enabled"));
        assert!(rows[2].value.starts_with("agaspar-5G"));

        let log = RefCell::new(Vec::<String>::new());
        let rec = |_: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(a.join(" "));
            Ok(if a.contains(&"DEVICE,TYPE") { "wlan0:wifi\n".into() } else { String::new() })
        };
        // Secured + unsaved: refused, nothing run (no password on a command line).
        let e = apply(3, &rows, Change::Toggle, &rec).unwrap_err();
        assert!(e.contains("net-menu.sh"));
        assert!(log.borrow().is_empty());

        SELECTED.set("Weak");
        let rows = load(&fake("enabled"));
        apply(3, &rows, Change::Toggle, &rec).unwrap(); // saved -> connection up
        SELECTED.set("Cafe: Free");
        let rows = load(&fake("enabled"));
        apply(3, &rows, Change::Toggle, &rec).unwrap(); // open -> device wifi connect
        apply(4, &rows, Change::Toggle, &rec).unwrap();
        apply(0, &rows, Change::Toggle, &rec).unwrap();
        SELECTED.set("");
        assert_eq!(
            *log.borrow(),
            [
                "-w 10 connection up id Weak (2)",
                "-w 10 device wifi connect Cafe: Free",
                "-t -f DEVICE,TYPE device",
                "device disconnect wlan0",
                "radio wifi off",
            ]
        );
    }

    #[test]
    fn option_looking_ssid_is_refused() {
        let rows = vec![
            Row::default(),
            Row::default(),
            Row::default(),
            Row { value: String::new(), choices: vec![("--evil".into(), "open".into())] },
        ];
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(apply(3, &rows, Change::Toggle, &boom).is_err());
    }
}
