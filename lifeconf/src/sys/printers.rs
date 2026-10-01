// SPDX-License-Identifier: GPL-3.0-or-later
// Printers panel, over CUPS. Printing is off until turned on here (most
// machines print rarely): that enables CUPS's socket (it starts on demand) and
// Avahi, which finds network printers. Modern network printers then simply
// appear: CUPS makes a driverless (IPP Everywhere) queue for each one it sees.
//
// Per printer: status and queued jobs, make it the default (yours, in
// ~/.cups/lpoptions, no root needed), print a test page, cancel its jobs. A
// printer CUPS can't find by itself is added by address, driverless.

use super::{pick, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] =
    &["printing", "printer", "status", "make default", "test page", "cancel jobs", "add by address (ipp://...)"];

static SELECTED: Sel = Sel::new();
const TEST_PAGE: &str = "/usr/share/cups/data/default-testpage.pdf";

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Bool,
        1 => RowKind::Choice,
        2 => RowKind::Info,
        6 => RowKind::Text,
        _ => RowKind::Action,
    }
}

/// `lpstat -e`: one destination per line.
pub fn parse_dests(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains(' ')).map(str::to_string).collect()
}

/// `lpstat -d`: "system default destination: NAME".
pub fn parse_default(out: &str) -> Option<String> {
    out.trim().strip_prefix("system default destination:").map(|s| s.trim().to_string())
}

/// `lpstat -p NAME`: "printer NAME is idle.  enabled since ..." -> "idle".
pub fn parse_state(out: &str) -> String {
    let l = out.lines().next().unwrap_or("");
    if l.contains("is idle") {
        "idle".into()
    } else if l.contains("now printing") {
        "printing".into()
    } else if l.contains("disabled") {
        "disabled (paused)".into()
    } else if l.is_empty() {
        "ready (found on the network)".into()
    } else {
        l.to_string()
    }
}

/// A queue name from an address: ipp://office-printer.local/ipp/print -> office-printer.
pub fn queue_name(uri: &str) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    let host = rest.split(['/', ':']).next()?.trim_end_matches(".local");
    let name: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    (!name.is_empty()).then_some(name)
}

/// Addresses lpadmin may get: ipp/ipps only (driverless), and only the
/// characters printer addresses use (host, port, IPv6 brackets, path, query).
pub fn uri_ok(uri: &str) -> bool {
    (uri.starts_with("ipp://") || uri.starts_with("ipps://"))
        && uri.len() < 200
        && uri.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?=&%[]@".contains(c))
}

fn enabled(run: Runner) -> bool {
    run("systemctl", &["is-enabled", "cups.socket"]).is_ok_and(|o| o.trim() == "enabled")
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: String| Row { value: v, choices: vec![] };
    if run("sh", &["-c", "command -v lpstat"]).map_or(true, |o| o.trim().is_empty()) {
        let mut v = vec![r("CUPS is not installed (pacman -S cups)".into())];
        v.resize(LABELS.len(), r("-".into()));
        return v;
    }
    let on = enabled(run);
    let dests = run("lpstat", &["-e"]).map(|o| parse_dests(&o)).unwrap_or_default();
    let default = run("lpstat", &["-d"]).ok().and_then(|o| parse_default(&o));
    if dests.is_empty() {
        let hint = if on { "none found yet (network printers appear on their own)" } else { "turn printing on first" };
        return vec![r(on.to_string()), r(hint.into()), r("-".into()), r("-".into()), r("-".into()), r("-".into()), r("ipp://printer.local/ipp/print".into())];
    }
    let want = SELECTED.get();
    let sel = dests.iter().find(|d| **d == want).or(default.as_ref().filter(|d| dests.contains(d))).unwrap_or(&dests[0]).clone();
    let label = |d: &String| if Some(d) == default.as_ref() { format!("{d}  (default)") } else { d.clone() };
    let state = run("lpstat", &["-p", &sel]).map(|o| parse_state(&o)).unwrap_or_else(|_| "ready (found on the network)".into());
    let jobs = run("lpstat", &["-o", &sel]).map(|o| o.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    vec![
        r(on.to_string()),
        Row { value: label(&sel), choices: dests.iter().map(|d| (label(d), d.clone())).collect() },
        r(format!("{state}, {jobs} job{} queued", if jobs == 1 { "" } else { "s" })),
        Row { value: if Some(&sel) == default.as_ref() { "already the default".into() } else { format!("make {sel} the default") }, choices: vec![(sel.clone(), String::new())] },
        Row { value: format!("print a test page on {sel}"), choices: vec![(sel.clone(), String::new())] },
        Row { value: if jobs > 0 { format!("cancel {jobs} job(s)") } else { "-".into() }, choices: vec![(sel.clone(), String::new())] },
        r("ipp://printer.local/ipp/print".into()),
    ]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let target = || row.choices.first().map(|c| c.0.clone()).ok_or_else(|| "no printer selected".to_string());
    match field {
        0 => {
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => row.value != "true",
            };
            let verb = if want { "enable" } else { "disable" };
            run("pkexec", &["systemctl", verb, "--now", "cups.socket", "cups.service", "avahi-daemon.service"])?;
            Ok(if want { "printing on: network printers will appear shortly".into() } else { "printing off".into() })
        }
        1 => {
            let (label, id) = pick(row, &ch, "printer")?;
            SELECTED.set(id);
            Ok(format!("selected {label}"))
        }
        3 => {
            let p = target()?;
            run("lpoptions", &["-d", &p])?;
            Ok(format!("{p} is your default printer"))
        }
        4 => {
            let p = target()?;
            run("lp", &["-d", &p, "-t", "Test page", TEST_PAGE])?;
            Ok(format!("test page sent to {p}"))
        }
        5 => {
            let p = target()?;
            run("cancel", &["-a", &p])?;
            Ok(format!("cancelled {p}'s jobs"))
        }
        6 => {
            let Change::Text(t) = ch else { return Err("type the printer's address, then Enter".into()) };
            let uri = t.trim();
            if !uri_ok(uri) {
                return Err("an ipp:// or ipps:// address, e.g. ipp://printer.local/ipp/print".into());
            }
            let name = queue_name(uri).ok_or("can't tell a name from that address")?;
            run("pkexec", &["lpadmin", "-p", &name, "-E", "-v", uri, "-m", "everywhere"])?;
            SELECTED.set(&name);
            Ok(format!("added {name}"))
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn parsers() {
        assert_eq!(parse_dests("HP_OfficeJet\nBrother_HL\n"), ["HP_OfficeJet", "Brother_HL"]);
        assert_eq!(parse_default("system default destination: HP_OfficeJet\n").as_deref(), Some("HP_OfficeJet"));
        assert_eq!(parse_default("no system default destination\n"), None);
        assert_eq!(parse_state("printer HP is idle.  enabled since Thu\n"), "idle");
        assert_eq!(parse_state("printer HP now printing HP-12.  enabled since\n"), "printing");
        assert_eq!(queue_name("ipp://office-printer.local/ipp/print").as_deref(), Some("office-printer"));
        assert_eq!(queue_name("ipps://10.0.0.5:631/ipp/print").as_deref(), Some("10_0_0_5"));
        assert!(uri_ok("ipp://printer.local/ipp/print"));
        assert!(!uri_ok("file:///etc/passwd") && !uri_ok("ipp://a b") && !uri_ok("ipp://x;rm"));
    }

    #[test]
    fn rows_and_commands() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let log = RefCell::new(Vec::<String>::new());
        let fake = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(match (p, a) {
                ("sh", _) => "/usr/bin/lpstat\n".into(),
                ("systemctl", _) => "enabled\n".into(),
                ("lpstat", ["-e"]) => "HP_OfficeJet\nBrother\n".into(),
                ("lpstat", ["-d"]) => "system default destination: Brother\n".into(),
                ("lpstat", ["-p", _]) => "printer Brother is idle.  enabled since x\n".into(),
                ("lpstat", ["-o", _]) => "Brother-3 voyd 1024 Thu\n".into(),
                _ => String::new(),
            })
        };
        let rows = load(&fake);
        assert_eq!(rows[1].value, "Brother  (default)");
        assert_eq!(rows[2].value, "idle, 1 job queued");
        assert_eq!(rows[3].value, "already the default");
        apply(4, &rows, Change::Toggle, &fake).unwrap();
        apply(5, &rows, Change::Toggle, &fake).unwrap();
        apply(6, &rows, Change::Text("ipp://office.local/ipp/print".into()), &fake).unwrap();
        assert!(apply(6, &rows, Change::Text("ipp://x; reboot".into()), &fake).is_err());
        apply(0, &rows, Change::Toggle, &fake).unwrap(); // on -> off
        let l = log.borrow();
        for want in [
            "lp -d Brother -t Test page /usr/share/cups/data/default-testpage.pdf",
            "cancel -a Brother",
            "pkexec lpadmin -p office -E -v ipp://office.local/ipp/print -m everywhere",
            "pkexec systemctl disable --now cups.socket cups.service avahi-daemon.service",
        ] {
            assert!(l.iter().any(|c| c == want), "{want} not in {l:?}");
        }
        SELECTED.set("");
    }
}
