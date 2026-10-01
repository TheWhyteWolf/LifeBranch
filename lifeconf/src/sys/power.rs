// SPDX-License-Identifier: GPL-3.0-or-later
// Power panel: power profile (power-profiles-daemon), screen brightness
// (brightnessctl), battery state (upower) and what closing the lid does.
//
// The lid policy is read-only on purpose. It lives in logind drop-ins under
// /etc, which are hand-tuned per machine (the T2 MacBook's carries suspend-fix
// notes), and a privileged rewrite of the login path is exactly the kind of
// change that needs a rollback story. Idle lock/screen-off timeouts are in the
// Idle category, where they already live.

use super::{pick, Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &["power profile", "brightness %", "battery", "lid closed", "lid closed (on AC)"];

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Choice,
        1 => RowKind::Int(5),
        _ => RowKind::Info,
    }
}

/// `powerprofilesctl list`: profile names are the lines indented exactly two
/// spaces (or marked `* ` for the active one); deeper lines are their details.
pub fn parse_profiles(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("* ").or_else(|| l.strip_prefix("  "))?;
            let name = rest.strip_suffix(':')?;
            (!name.is_empty() && !name.starts_with(' ') && !name.contains(char::is_whitespace)).then(|| name.to_string())
        })
        .collect()
}

/// `brightnessctl -m` -> `device,class,cur,pct%,max`; the percentage.
pub fn parse_brightness(out: &str) -> Option<i32> {
    out.lines().next()?.split(',').nth(3)?.trim_end_matches('%').trim().parse().ok()
}

/// upower's DisplayDevice summary: "27% charging, 2.9 hours to full".
pub fn parse_battery(out: &str) -> String {
    let get = |k: &str| out.lines().find_map(|l| l.trim().strip_prefix(k)).map(|v| v.trim().to_string());
    if get("present:").as_deref() != Some("yes") {
        return "no battery".into();
    }
    let pct = get("percentage:")
        .and_then(|p| p.trim_end_matches('%').parse::<f64>().ok())
        .map(|p| format!("{}%", p.round() as i32))
        .unwrap_or_else(|| "?".into());
    let state = get("state:").unwrap_or_default();
    let eta = get("time to full:").map(|t| format!(", {t} to full")).or_else(|| get("time to empty:").map(|t| format!(", {t} left")));
    format!("{pct} {state}{}", eta.unwrap_or_default())
}

/// `s "suspend"` from busctl -> suspend.
fn parse_busctl_str(out: &str) -> Option<String> {
    out.trim().strip_prefix("s ").map(|v| v.trim_matches('"').to_string())
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let profile = match (run("powerprofilesctl", &["list"]), run("powerprofilesctl", &["get"])) {
        (Ok(list), Ok(cur)) => {
            let names = parse_profiles(&list);
            Row { value: cur.trim().to_string(), choices: names.iter().map(|n| (n.clone(), n.clone())).collect() }
        }
        _ => r("unavailable"),
    };
    let brightness = run("brightnessctl", &["-m"])
        .ok()
        .and_then(|o| parse_brightness(&o))
        .map(|p| r(&p.to_string()))
        .unwrap_or_else(|| r("unavailable"));
    let battery = r(&run("upower", &["-i", "/org/freedesktop/UPower/devices/DisplayDevice"])
        .map(|o| parse_battery(&o))
        .unwrap_or_else(|_| "unknown".into()));
    let lid = |prop: &str| {
        let out = run(
            "busctl",
            &["get-property", "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager", prop],
        );
        r(&out.ok().and_then(|o| parse_busctl_str(&o)).unwrap_or_else(|| "unknown".into()))
    };
    vec![profile, brightness, battery, lid("HandleLidSwitch"), lid("HandleLidSwitchExternalPower")]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        0 => {
            let (label, id) = pick(row, &ch, "power profile")?;
            run("powerprofilesctl", &["set", id])?;
            Ok(format!("power profile {label}"))
        }
        1 => {
            let cur: i32 = row.value.parse().map_err(|_| "brightness unavailable".to_string())?;
            let next = match ch {
                Change::Step(d) => cur + d * 5,
                Change::Text(t) => t.trim().trim_end_matches('%').parse().map_err(|_| format!("not a number: {:?}", t.trim()))?,
                Change::Toggle => cur,
            }
            // Never 0: a fully dark panel with no way to see the UI to undo it.
            .clamp(1, 100);
            run("brightnessctl", &["set", &format!("{next}%")])?;
            Ok(format!("brightness {next}%"))
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const PROFILES: &str = "  performance:\n    CpuDriver:\tintel_pstate\n    Degraded:   no\n\n  balanced:\n    CpuDriver:\tintel_pstate\n    PlatformDriver:\tplaceholder\n\n* power-saver:\n    CpuDriver:\tintel_pstate\n    PlatformDriver:\tplaceholder\n";
    const UPOWER: &str = "  power supply:         yes\n  battery\n    present:             yes\n    state:               charging\n    time to full:        2.9 hours\n    percentage:          27.6405%\n";

    fn fake(p: &str, a: &[&str]) -> Result<String, String> {
        Ok(match (p, a[0]) {
            ("powerprofilesctl", "list") => PROFILES.into(),
            ("powerprofilesctl", "get") => "power-saver\n".into(),
            ("brightnessctl", "-m") => "gmux_backlight,backlight,5746,9%,65535\n".into(),
            ("upower", _) => UPOWER.into(),
            ("busctl", _) => "s \"suspend\"\n".into(),
            _ => String::new(),
        })
    }

    #[test]
    fn parses_real_shaped_outputs() {
        assert_eq!(parse_profiles(PROFILES), ["performance", "balanced", "power-saver"]);
        assert_eq!(parse_brightness("gmux_backlight,backlight,5746,9%,65535"), Some(9));
        assert_eq!(parse_battery(UPOWER), "28% charging, 2.9 hours to full");
        assert_eq!(parse_battery("  battery\n    present:             no\n"), "no battery");
        assert_eq!(parse_busctl_str("s \"ignore\"\n").as_deref(), Some("ignore"));
    }

    #[test]
    fn loads_rows() {
        let v: Vec<String> = load(&fake).into_iter().map(|r| r.value).collect();
        assert_eq!(v, ["power-saver", "9", "28% charging, 2.9 hours to full", "suspend", "suspend"]);
    }

    #[test]
    fn changes_run_the_right_commands_and_brightness_never_hits_zero() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        let rows = load(&fake);
        apply(0, &rows, Change::Step(-1), &rec).unwrap(); // power-saver -> balanced
        apply(1, &rows, Change::Step(1), &rec).unwrap(); // 9 -> 14
        apply(1, &rows, Change::Text("0".into()), &rec).unwrap(); // clamped to 1
        apply(1, &rows, Change::Text("250%".into()), &rec).unwrap();
        assert_eq!(
            *log.borrow(),
            ["powerprofilesctl set balanced", "brightnessctl set 14%", "brightnessctl set 1%", "brightnessctl set 100%"]
        );
        assert!(apply(3, &rows, Change::Toggle, &rec).is_err(), "lid policy is read-only");
    }

    #[test]
    fn missing_tools_degrade_per_row() {
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        let rows = load(&none);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!((rows[0].value.as_str(), rows[1].value.as_str()), ("unavailable", "unavailable"));
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(apply(1, &rows, Change::Step(1), &boom).is_err());
    }
}
