// SPDX-License-Identifier: GPL-3.0-or-later
// Date & Time panel over timedatectl: timezone and automatic (NTP) sync. Setting
// either goes through logind's timedated, which asks polkit — the polkit agent
// niri starts shows the password prompt, and this blocks until it's answered.
// The timezone is typed; a bare city ("Toronto") resolves to its zone when only
// one zone ends in that name.

use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &["timezone", "automatic time", "clock synchronised", "local time"];

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Text,
        1 => RowKind::Bool,
        _ => RowKind::Info,
    }
}

fn prop<'a>(out: &'a str, key: &str) -> Option<&'a str> {
    out.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let Ok(show) = run("timedatectl", &["show", "-p", "Timezone", "-p", "NTP", "-p", "NTPSynchronized", "-p", "CanNTP"])
    else {
        return vec![r("timedatectl unavailable"), r("-"), r("-"), r("-")];
    };
    let yn = |k| prop(&show, k).unwrap_or("no") == "yes";
    let now = run("date", &["+%a %Y-%m-%d %H:%M:%S %Z"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "-".into());
    vec![
        r(prop(&show, "Timezone").unwrap_or("unknown")),
        // Without an NTP service installed, the switch can't be turned on.
        r(&if yn("CanNTP") { yn("NTP").to_string() } else { "unavailable".into() }),
        r(if yn("NTPSynchronized") { "yes" } else { "no" }),
        r(&now),
    ]
}

/// Map what the user typed to a real zone name from `timedatectl list-timezones`.
pub fn resolve_tz(input: &str, zones: &[&str]) -> Result<String, String> {
    let want = input.trim().replace(' ', "_");
    if want.is_empty() {
        return Err("type a timezone, e.g. Europe/London or just London".into());
    }
    if let Some(z) = zones.iter().find(|z| z.eq_ignore_ascii_case(&want)) {
        return Ok(z.to_string());
    }
    let suffix = format!("/{}", want.to_lowercase());
    let hits: Vec<&&str> = zones.iter().filter(|z| z.to_lowercase().ends_with(&suffix)).collect();
    match hits.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("no timezone matching {:?}", input.trim())),
        many => Err(format!(
            "{:?} is ambiguous: {}",
            input.trim(),
            many.iter().take(4).map(|z| **z).collect::<Vec<_>>().join(", ")
        )),
    }
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    match field {
        0 => {
            let Change::Text(t) = ch else { return Err("type a timezone and press Enter".into()) };
            let list = run("timedatectl", &["list-timezones"])?;
            let zones: Vec<&str> = list.lines().map(str::trim).collect();
            let zone = resolve_tz(&t, &zones)?;
            run("timedatectl", &["set-timezone", &zone])?;
            Ok(format!("timezone {zone}"))
        }
        1 => {
            let row = rows.get(1).ok_or("no such row")?;
            if row.value == "unavailable" {
                return Err("no NTP service available (install systemd-timesyncd or chrony)".into());
            }
            let on = row.value == "true";
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !on,
            };
            run("timedatectl", &["set-ntp", if want { "true" } else { "false" }])?;
            Ok(format!("automatic time {}", if want { "on" } else { "off" }))
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const SHOW: &str = "Timezone=Europe/Isle_of_Man\nCanNTP=yes\nNTP=yes\nNTPSynchronized=yes\n";
    const ZONES: &[&str] = &["America/Toronto", "Europe/London", "Europe/Isle_of_Man", "America/Indiana/Indianapolis", "Asia/Kolkata", "America/Port-au-Prince", "Europe/Kyiv", "Pacific/Auckland"];

    fn fake(_: &str, a: &[&str]) -> Result<String, String> {
        Ok(match a[0] {
            "show" => SHOW.into(),
            "list-timezones" => ZONES.join("\n"),
            _ => "Thu 2026-10-01 03:25:41 BST\n".into(),
        })
    }

    #[test]
    fn loads_rows() {
        let v: Vec<String> = load(&fake).into_iter().map(|r| r.value).collect();
        assert_eq!(v, ["Europe/Isle_of_Man", "true", "yes", "Thu 2026-10-01 03:25:41 BST"]);
    }

    #[test]
    fn resolves_exact_city_and_ambiguity() {
        assert_eq!(resolve_tz("europe/london", ZONES).unwrap(), "Europe/London");
        assert_eq!(resolve_tz("Toronto", ZONES).unwrap(), "America/Toronto");
        assert_eq!(resolve_tz("isle of man", ZONES).unwrap(), "Europe/Isle_of_Man");
        assert!(resolve_tz("Nowhere", ZONES).unwrap_err().contains("no timezone"));
        assert!(resolve_tz("  ", ZONES).is_err());
        let dup = ["America/Indiana/Knox", "America/North_Dakota/Knox"];
        assert!(resolve_tz("Knox", &dup).unwrap_err().contains("ambiguous"));
    }

    #[test]
    fn set_timezone_validates_before_running() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            fake(p, a)
        };
        let rows = load(&fake);
        apply(0, &rows, Change::Text("Toronto".into()), &rec).unwrap();
        assert!(apply(0, &rows, Change::Text("Atlantis".into()), &rec).is_err());
        assert_eq!(*log.borrow(), ["timedatectl list-timezones", "timedatectl set-timezone America/Toronto", "timedatectl list-timezones"]);
    }

    #[test]
    fn ntp_toggle_and_unavailable() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            fake(p, a)
        };
        let rows = load(&fake);
        apply(1, &rows, Change::Toggle, &rec).unwrap();
        assert_eq!(*log.borrow(), ["timedatectl set-ntp false"]);
        let no_ntp = |_: &str, a: &[&str]| -> Result<String, String> {
            Ok(if a[0] == "show" { "Timezone=UTC\nCanNTP=no\nNTP=no\nNTPSynchronized=no\n".into() } else { String::new() })
        };
        let rows = load(&no_ntp);
        assert_eq!(rows[1].value, "unavailable");
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(apply(1, &rows, Change::Toggle, &boom).is_err());
    }

    #[test]
    fn missing_timedatectl_degrades() {
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        let rows = load(&none);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "timedatectl unavailable");
    }
}
