// SPDX-License-Identifier: GPL-3.0-or-later
// About: read-only facts about this machine and the rice. Every value comes
// through the Runner (`cat` on /proc, /sys, /etc), so it's testable and a
// missing source shows "unknown" instead of failing the panel.

use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] =
    &["device", "os", "kernel", "hostname", "cpu", "memory", "uptime", "niri", "lifeconf"];

pub fn kind(_field: usize) -> RowKind {
    RowKind::Info
}

fn cat(run: Runner, path: &str) -> Option<String> {
    run("cat", &[path]).ok()
}

fn first(v: Option<String>) -> String {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "unknown".into())
}

fn os_name(release: &str) -> Option<String> {
    release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_string())
}

fn cpu(info: &str) -> Option<String> {
    let name = info.lines().find_map(|l| l.strip_prefix("model name")?.split_once(':')).map(|(_, v)| v.trim())?;
    let threads = info.lines().filter(|l| l.starts_with("processor")).count();
    Some(format!("{} ({threads} threads)", name.split_whitespace().collect::<Vec<_>>().join(" ")))
}

fn memory(info: &str) -> Option<String> {
    let kb: f64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.split_whitespace().next()?.parse().ok()?;
    Some(format!("{:.1} GiB", kb / 1024.0 / 1024.0))
}

fn uptime(raw: &str) -> Option<String> {
    let secs = raw.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    Some(match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    })
}

pub fn load(run: Runner) -> Vec<Row> {
    let vals = [
        first(cat(run, "/sys/class/dmi/id/product_name")),
        first(cat(run, "/etc/os-release").as_deref().and_then(os_name)),
        first(run("uname", &["-r"]).ok()),
        first(cat(run, "/etc/hostname")),
        first(cat(run, "/proc/cpuinfo").as_deref().and_then(cpu)),
        first(cat(run, "/proc/meminfo").as_deref().and_then(memory)),
        first(cat(run, "/proc/uptime").as_deref().and_then(uptime)),
        first(run("niri", &["--version"]).ok().and_then(|s| s.lines().next().map(str::to_string))),
        env!("CARGO_PKG_VERSION").to_string(),
    ];
    vals.into_iter().map(|value| Row { value, choices: vec![] }).collect()
}

pub fn apply(_field: usize, _rows: &[Row], _ch: Change, _run: Runner) -> Result<String, String> {
    Err("read-only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_shaped_sources() {
        assert_eq!(os_name("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\n").as_deref(), Some("Arch Linux"));
        let cpuinfo = "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz\nprocessor\t: 1\nmodel name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz\n";
        assert_eq!(cpu(cpuinfo).as_deref(), Some("Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz (2 threads)"));
        assert_eq!(memory("MemTotal:       16257472 kB\n").as_deref(), Some("15.5 GiB"));
        assert_eq!(uptime("2387.78 25088.18").as_deref(), Some("39m"));
        assert_eq!(uptime("200000.0 1").as_deref(), Some("2d 7h 33m"));
        assert_eq!(uptime("7300.0 1").as_deref(), Some("2h 1m"));
    }

    #[test]
    fn missing_sources_say_unknown_and_never_fail() {
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        let rows = load(&none);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "unknown");
        assert!(rows[8].value.chars().next().unwrap().is_ascii_digit(), "lifeconf version is compiled in");
    }

    #[test]
    fn is_read_only() {
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        assert!(apply(0, &[], Change::Toggle, &none).is_err());
    }
}
