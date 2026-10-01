// SPDX-License-Identifier: GPL-3.0-or-later
// Night light panel: wlsunset's settings, which live as one spawn line in the
// `LIFEBRANCH:BEGIN nightlight` region of niri's config (the installer writes
// it with coordinates guessed from the timezone). A change rewrites the region
// (validated by niri before it is kept) and restarts wlsunset, so it applies at
// once rather than at the next login.
//
// Off means no spawn line at all; the settings are parked in
// ~/.config/lifeconf/nightlight.off so turning it back on restores them.

use super::kdl::{Arg, Node};
use super::niri_input;
use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] =
    &["night light", "latitude", "longitude", "night temperature K", "day temperature K", "sunset (HH:MM)", "sunrise (HH:MM)"];

const REGION: &str = "nightlight";

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Bool,
        3 | 4 => RowKind::Int(250),
        _ => RowKind::Text,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Night {
    pub on: bool,
    pub lat: String,
    pub lon: String,
    /// -t and -T; wlsunset's defaults when unset.
    pub low: u32,
    pub high: u32,
    /// Fixed times instead of the sun (-s sunset, -S sunrise).
    pub sunset: String,
    pub sunrise: String,
}

impl Default for Night {
    fn default() -> Self {
        // London, the shipped default, until the installer guesses better.
        Night { on: true, lat: "51.5".into(), lon: "-0.1".into(), low: 4000, high: 6500, sunset: String::new(), sunrise: String::new() }
    }
}

/// Read wlsunset's flags out of a spawn node's arguments.
pub fn from_args(args: &[Arg]) -> Night {
    let mut n = Night::default();
    let a: Vec<&str> = args.iter().map(|x| x.text.as_str()).collect();
    let mut i = 1; // a[0] is "wlsunset"
    while i + 1 < a.len() {
        let v = a[i + 1].to_string();
        match a[i] {
            "-l" => n.lat = v,
            "-L" => n.lon = v,
            "-t" => n.low = v.parse().unwrap_or(n.low),
            "-T" => n.high = v.parse().unwrap_or(n.high),
            "-s" => n.sunset = v,
            "-S" => n.sunrise = v,
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    n
}

pub fn to_args(n: &Night) -> Vec<String> {
    let mut v: Vec<String> = vec!["wlsunset".into()];
    if !n.sunset.is_empty() && !n.sunrise.is_empty() {
        v.extend(["-s".into(), n.sunset.clone(), "-S".into(), n.sunrise.clone()]);
    } else {
        v.extend(["-l".into(), n.lat.clone(), "-L".into(), n.lon.clone()]);
    }
    if n.low != 4000 {
        v.extend(["-t".into(), n.low.to_string()]);
    }
    if n.high != 6500 {
        v.extend(["-T".into(), n.high.to_string()]);
    }
    v
}

fn node(n: &Night) -> Node {
    Node {
        name: "spawn-at-startup".into(),
        args: to_args(n).into_iter().map(|t| Arg { text: t, quoted: true }).collect(),
        children: None,
    }
}

fn parked_path() -> String {
    let home = std::env::var("LIFECONF_HOME").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    format!("{home}/.config/lifeconf/nightlight.off")
}

/// The current settings: the spawn line when on, the parked ones when off.
pub fn read(path: &str) -> Result<Night, String> {
    let nodes = niri_input::read(path, REGION)?;
    let spawn = nodes.iter().find(|n| n.name == "spawn-at-startup" && n.args.first().is_some_and(|a| a.text == "wlsunset"));
    Ok(match spawn {
        Some(s) => from_args(&s.args),
        None => {
            let parked = std::fs::read_to_string(parked_path()).unwrap_or_default();
            let args: Vec<Arg> = parked.split_whitespace().map(|t| Arg { text: t.into(), quoted: true }).collect();
            Night { on: false, ..if args.is_empty() { Night::default() } else { from_args(&args) } }
        }
    })
}

fn valid_coord(s: &str, max: f64) -> bool {
    s.parse::<f64>().is_ok_and(|v| v.is_finite() && v.abs() <= max)
}

fn valid_time(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else { return false };
    h.len() == 2 && m.len() == 2 && h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60)
}

pub fn check(n: &Night) -> Result<(), String> {
    if !valid_coord(&n.lat, 90.0) {
        return Err(format!("latitude {:?}: a number from -90 to 90", n.lat));
    }
    if !valid_coord(&n.lon, 180.0) {
        return Err(format!("longitude {:?}: a number from -180 to 180", n.lon));
    }
    if !(1000..=10000).contains(&n.low) || !(1000..=10000).contains(&n.high) || n.low >= n.high {
        return Err("temperatures: night below day, both 1000-10000 K".into());
    }
    for t in [&n.sunset, &n.sunrise] {
        if !t.is_empty() && !valid_time(t) {
            return Err(format!("{t:?}: a time like 21:30"));
        }
    }
    if n.sunset.is_empty() != n.sunrise.is_empty() {
        return Err("set both sunset and sunrise for fixed times (or clear both to follow the sun)".into());
    }
    Ok(())
}

/// Write `n` and restart wlsunset to match.
pub fn save(path: &str, n: &Night, run: Runner) -> Result<(), String> {
    check(n)?;
    let mut nodes = niri_input::read(path, REGION)?;
    nodes.retain(|x| !(x.name == "spawn-at-startup" && x.args.first().is_some_and(|a| a.text == "wlsunset")));
    if n.on {
        nodes.push(node(n));
    } else {
        let p = parked_path();
        if let Some(d) = std::path::Path::new(&p).parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&p, to_args(n).join(" ")).map_err(|e| format!("{p}: {e}"))?;
    }
    niri_input::write(path, REGION, &nodes)?;
    // Live: the spawn line only runs at login, so restart it by hand.
    let _ = run("pkill", &["-x", "wlsunset"]);
    if n.on {
        let mut argv: Vec<String> = vec!["msg".into(), "action".into(), "spawn".into(), "--".into()];
        argv.extend(to_args(n));
        let a: Vec<&str> = argv.iter().map(String::as_str).collect();
        run("niri", &a)?;
    }
    Ok(())
}

pub fn load(_run: Runner) -> Vec<Row> {
    let r = |v: String| Row { value: v, choices: vec![] };
    match read(&niri_input::config_path()) {
        Ok(n) => vec![
            r(n.on.to_string()),
            r(n.lat.clone()),
            r(n.lon.clone()),
            r(n.low.to_string()),
            r(n.high.to_string()),
            r(if n.sunset.is_empty() { "(follows the sun)".into() } else { n.sunset.clone() }),
            r(if n.sunrise.is_empty() { "(follows the sun)".into() } else { n.sunrise.clone() }),
        ],
        Err(e) => {
            let mut v = vec![r(e)];
            v.resize(LABELS.len(), r("-".into()));
            v
        }
    }
}

pub fn apply(field: usize, _rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_at(&niri_input::config_path(), field, ch, run)
}

pub fn apply_at(path: &str, field: usize, ch: Change, run: Runner) -> Result<String, String> {
    let mut n = read(path)?;
    let text = |c: &Change| match c {
        Change::Text(t) => Ok(t.trim().to_string()),
        _ => Err("type a value, then Enter".to_string()),
    };
    match field {
        0 => {
            n.on = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !n.on,
            };
        }
        1 => n.lat = text(&ch)?,
        2 => n.lon = text(&ch)?,
        3 | 4 => {
            let cur = if field == 3 { n.low } else { n.high };
            let next = match &ch {
                Change::Step(d) => (cur as i32 + d * 250).max(0) as u32,
                Change::Text(t) => t.trim().trim_end_matches(['K', 'k']).parse().map_err(|_| format!("not a number: {t:?}"))?,
                Change::Toggle => cur,
            };
            if field == 3 { n.low = next } else { n.high = next }
        }
        5 | 6 => {
            let t = text(&ch)?;
            let t = if t.starts_with('(') || t == "-" { String::new() } else { t };
            if field == 5 { n.sunset = t } else { n.sunrise = t }
        }
        _ => return Err("read-only".into()),
    }
    save(path, &n, run)?;
    Ok(if n.on { format!("night light {}", to_args(&n)[1..].join(" ")) } else { "night light off".into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::sys::ENV_LOCK as SERIAL;

    fn cfg(tag: &str, line: &str) -> String {
        let dir = std::env::temp_dir().join(format!("lc-night-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config/lifeconf")).unwrap();
        std::env::set_var("LIFECONF_HOME", &dir);
        let p = dir.join("config.kdl");
        std::fs::write(&p, format!("// LIFEBRANCH:BEGIN nightlight\n// a comment\n{line}\n// LIFEBRANCH:END nightlight\n")).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn reads_the_installer_line() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let p = cfg("read", r#"spawn-at-startup "wlsunset" "-l" "54.1500" "-L" "-4.4667""#);
        let n = read(&p).unwrap();
        assert_eq!((n.on, n.lat.as_str(), n.lon.as_str(), n.low, n.high), (true, "54.1500", "-4.4667", 4000, 6500));
    }

    #[test]
    fn checks_values() {
        let ok = Night::default();
        assert!(check(&ok).is_ok());
        assert!(check(&Night { lat: "95".into(), ..ok.clone() }).is_err());
        assert!(check(&Night { lon: "east".into(), ..ok.clone() }).is_err());
        assert!(check(&Night { low: 7000, ..ok.clone() }).is_err(), "night warmer than day");
        assert!(check(&Night { sunset: "21:30".into(), ..ok.clone() }).is_err(), "both or neither");
        assert!(check(&Night { sunset: "25:00".into(), sunrise: "06:00".into(), ..ok.clone() }).is_err());
        assert!(check(&Night { sunset: "21:30".into(), sunrise: "06:15".into(), ..ok }).is_ok());
    }

    #[test]
    fn args_round_trip_and_fixed_times_replace_the_location() {
        let n = Night { low: 3500, sunset: "21:00".into(), sunrise: "07:00".into(), ..Night::default() };
        assert_eq!(to_args(&n), ["wlsunset", "-s", "21:00", "-S", "07:00", "-t", "3500"]);
        let back = from_args(&to_args(&Night::default()).into_iter().map(|t| Arg { text: t, quoted: true }).collect::<Vec<_>>());
        assert_eq!(back, Night::default());
    }

    // The write path runs `niri validate`, so only check what it decides to
    // run, with niri faked out of the picture by an invalid path guard: the
    // region rewrite itself is niri_input's, tested there.
    #[test]
    fn turning_off_parks_the_settings_and_on_restores_them() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if std::process::Command::new("niri").arg("--version").output().is_err() {
            return; // CI without niri: write_validated can't validate
        }
        let p = cfg("park", r#"spawn-at-startup "wlsunset" "-l" "54.1500" "-L" "-4.4667""#);
        let log = RefCell::new(Vec::<String>::new());
        let rec = |pr: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{pr} {}", a.join(" ")));
            Ok(String::new())
        };
        apply_at(&p, 0, Change::Toggle, &rec).unwrap();
        let n = read(&p).unwrap();
        assert!(!n.on);
        assert_eq!(n.lat, "54.1500", "parked, not lost");
        assert!(!std::fs::read_to_string(&p).unwrap().contains("wlsunset"));
        apply_at(&p, 0, Change::Toggle, &rec).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains(r#""-l" "54.1500""#));
        assert_eq!(*log.borrow(), ["pkill -x wlsunset", "pkill -x wlsunset", "niri msg action spawn -- wlsunset -l 54.1500 -L -4.4667"]);
    }
}
