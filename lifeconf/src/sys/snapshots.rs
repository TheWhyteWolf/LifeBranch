// SPDX-License-Identifier: GPL-3.0-or-later
// Snapshots panel: the btrfs snapshots snapper keeps of / (before and after
// every pacman transaction, hourly and daily), taking one now, and restoring
// one. Restoring runs scripts/snapshots.sh as root through pkexec, from the
// root-owned copy the installer puts in /usr/local/lib/lifebranch: a script
// in the user's home could be rewritten by anything running as the user
// between opening this page and typing the password.
//
// Listing needs read access, which `snapshots.sh setup` (or `allow-group
// wheel`) grants the wheel group; without it the page says how to get it.

use super::{pick, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] = &[
    "snapshots",
    "hourly and daily",
    "take one now",
    "snapshot",
    "taken",
    "restore (type 'restore')",
    "remove set-aside roots",
];

pub const SCRIPT: &str = "/usr/local/lib/lifebranch/snapshots.sh";
static SELECTED: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    match field {
        0 | 4 => RowKind::Info,
        1 => RowKind::Bool,
        3 => RowKind::Choice,
        5 => RowKind::Text,
        _ => RowKind::Action,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    pub num: u32,
    pub kind: String,
    pub pre: Option<u32>,
    pub date: String,
    pub cleanup: String,
    pub userdata: String,
    pub desc: String,
}

pub const LIST_ARGS: &[&str] =
    &["--iso", "--csvout", "--separator", "|", "-c", "root", "list", "--columns", "number,type,pre-number,date,cleanup,userdata,description"];

/// snapper's CSV (columns as LIST_ARGS asks, a header line first), newest
/// first, without snapshot 0 (that is "the running system", not a snapshot).
pub fn parse(csv: &str) -> Vec<Snap> {
    let mut v: Vec<Snap> = csv
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(7, '|').collect();
            if f.len() < 7 {
                return None;
            }
            let num: u32 = f[0].trim().parse().ok()?;
            (num != 0).then(|| Snap {
                num,
                kind: f[1].trim().into(),
                pre: f[2].trim().parse().ok(),
                date: f[3].trim().into(),
                cleanup: f[4].trim().into(),
                userdata: f[5].trim().into(),
                desc: f[6].trim().trim_matches('"').into(),
            })
        })
        .collect();
    v.sort_by(|a, b| b.num.cmp(&a.num));
    v
}

/// "#412  2026-10-02 00:03  before: pacman -Syu" for the picker.
pub fn label(s: &Snap) -> String {
    let when: String = s.date.chars().take(16).collect();
    let what = match (s.kind.as_str(), s.desc.as_str()) {
        ("pre", d) => format!("before: {d}"),
        ("post", d) => format!("after: {d}"),
        (_, "timeline") => "hourly/daily".into(),
        (_, "") => "(no description)".into(),
        (_, d) => d.into(),
    };
    format!("#{}  {when}  {what}", s.num)
}

fn status(run: Runner) -> std::collections::HashMap<String, String> {
    // The script reads; status needs no root. Fall back to the repo copy so
    // the page can say what's wrong before the installer has run.
    let out = run("bash", &[SCRIPT, "status"]).unwrap_or_default();
    out.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect()
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let mut rows = vec![r("-"); LABELS.len()];
    let st = status(run);
    let get = |k: &str| st.get(k).map(String::as_str).unwrap_or("");
    if st.is_empty() {
        rows[0] = r(&format!("not installed: run the installer, or sudo install -Dm755 scripts/snapshots.sh {SCRIPT}"));
        return rows;
    }
    if get("fs") != "btrfs" {
        rows[0] = r("unavailable: / isn't btrfs");
        return rows;
    }
    if get("snapper") != "yes" {
        rows[0] = r(&format!("off: turn on with sudo {SCRIPT} setup"));
        return rows;
    }
    let list = match run("snapper", LIST_ARGS) {
        Ok(out) => parse(&out),
        Err(_) => {
            rows[0] = r(&format!("can't read them: sudo {SCRIPT} allow-group wheel, then log in again"));
            return rows;
        }
    };
    let timeline = run("snapper", &["-c", "root", "get-config"])
        .ok()
        .and_then(|o| o.lines().find(|l| l.starts_with("TIMELINE_CREATE")).map(|l| l.contains("yes")))
        .unwrap_or(false);
    let booted = get("booted_snapshot");
    rows[0] = r(&if !booted.is_empty() {
        format!("booted from snapshot #{booted} (read-only): restore it to keep it")
    } else if get("restorable") != "yes" {
        format!("{} kept; restoring needs @ and @snapshots side by side", list.len())
    } else {
        format!("{} kept, newest {}", list.len(), list.first().map(|s| s.date.chars().take(16).collect::<String>()).unwrap_or("-".into()))
    });
    rows[1] = r(&timeline.to_string());
    rows[2] = r("snapshot / now");
    let mut sel = SELECTED.get();
    if !list.iter().any(|s| s.num.to_string() == sel) {
        sel = list.first().map(|s| s.num.to_string()).unwrap_or_default();
        SELECTED.set(&sel);
    }
    let chosen = list.iter().find(|s| s.num.to_string() == sel);
    rows[3] = Row {
        value: chosen.map(label).unwrap_or_else(|| "(none yet)".into()),
        choices: list.iter().map(|s| (label(s), s.num.to_string())).collect(),
    };
    rows[4] = r(&chosen
        .map(|s| {
            let pair = match (s.kind.as_str(), s.pre) {
                ("post", Some(p)) => format!(", after #{p}"),
                _ => String::new(),
            };
            format!("{}, {}{pair}{}", s.date, s.kind, if s.cleanup.is_empty() { String::new() } else { format!(", kept by {} rule", s.cleanup) })
        })
        .unwrap_or_else(|| "-".into()));
    rows[5] = r(&chosen.map(|s| format!("(type 'restore' to go back to #{})", s.num)).unwrap_or_else(|| "-".into()));
    rows[6] = r("after a restore, once all is well");
    rows
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        1 => {
            let want = !(row.value == "true");
            let v = format!("TIMELINE_CREATE={}", if want { "yes" } else { "no" });
            run("pkexec", &["snapper", "-c", "root", "set-config", &v])?;
            Ok(format!("hourly and daily snapshots {}", if want { "on" } else { "off" }))
        }
        2 => {
            if !matches!(ch, Change::Toggle | Change::Text(_)) {
                return Err("press Enter to take one".into());
            }
            let args = ["-c", "root", "create", "-d", "taken from Settings", "-c", "number", "--print-number"];
            // Readable for the wheel group means creatable too; else ask.
            let out = run("snapper", &args).or_else(|_| {
                let mut a = vec!["snapper"];
                a.extend(args);
                run("pkexec", &a)
            })?;
            SELECTED.set(out.trim());
            Ok(format!("took snapshot #{}", out.trim()))
        }
        3 => {
            let (label, id) = pick(row, &ch, "snapshot")?;
            SELECTED.set(id);
            Ok(label.clone())
        }
        5 => {
            let n = SELECTED.get();
            if n.is_empty() {
                return Err("no snapshot chosen".into());
            }
            match ch {
                Change::Text(t) if t.trim() == "restore" => {
                    run("pkexec", &[SCRIPT, "restore", &n, "--yes"])?;
                    Ok(format!("snapshot #{n} restored: reboot to start it (the old root is kept until you remove it)"))
                }
                Change::Text(_) => Err(format!("type exactly 'restore' to go back to #{n}")),
                _ => Err("type 'restore' and Enter".into()),
            }
        }
        6 => {
            let out = run("pkexec", &[SCRIPT, "leftovers", "--delete", "--yes"])?;
            Ok(out.lines().last().unwrap_or("done").to_string())
        }
        _ => Err("nothing to change here".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "number|type|pre-number|date|cleanup|userdata|description\n0|single||||||current\n411|pre||2026-10-01 23:59:58|number|important=yes|pacman -Syu\n412|post|411|2026-10-02 00:03:11|number||pacman -Syu\n413|single||2026-10-02 01:00:00|timeline||timeline\n414|single||2026-10-02 01:10:00|||a | b\n";

    fn fake(status: &'static str, csv: &'static str) -> impl Fn(&str, &[&str]) -> Result<String, String> {
        move |p: &str, a: &[&str]| match (p, a.first()) {
            ("bash", _) => Ok(status.into()),
            ("snapper", Some(&"--iso")) => Ok(csv.into()),
            ("snapper", _) if a.contains(&"get-config") => Ok("TIMELINE_CREATE  | yes\n".into()),
            _ => Err(format!("unexpected {p} {a:?}")),
        }
    }

    const OK: &str = "fs=btrfs\nroot_subvol=@\nsnap_subvol=@snapshots\nsnapper=yes\nreadable=yes\nbooted_snapshot=\nrestorable=yes\n";

    #[test]
    fn parses_newest_first_without_zero_and_keeps_bars_in_descriptions() {
        let v = parse(CSV);
        assert_eq!(v.iter().map(|s| s.num).collect::<Vec<_>>(), [414, 413, 412, 411]);
        assert_eq!(v[0].desc, "a | b");
        assert_eq!(v[2].pre, Some(411));
        assert_eq!(label(&v[2]), "#412  2026-10-02 00:03  after: pacman -Syu");
        assert_eq!(label(&v[1]), "#413  2026-10-02 01:00  hourly/daily");
    }

    #[test]
    fn rows_describe_the_list_and_the_newest_is_chosen() {
        let rows = load(&fake(OK, CSV));
        assert_eq!(rows[0].value, "4 kept, newest 2026-10-02 01:10");
        assert_eq!(rows[1].value, "true");
        assert_eq!(rows[3].choices.len(), 4);
        assert!(rows[3].value.starts_with("#414"));
        assert!(rows[5].value.contains("#414"));
    }

    #[test]
    fn says_what_is_missing() {
        let rows = load(&fake("fs=ext4\n", CSV));
        assert_eq!(rows[0].value, "unavailable: / isn't btrfs");
        let rows = load(&fake("fs=btrfs\nsnapper=no\n", CSV));
        assert!(rows[0].value.contains("setup"));
        let unreadable = |p: &str, _: &[&str]| if p == "bash" { Ok(OK.to_string()) } else { Err("No permissions.".to_string()) };
        assert!(load(&unreadable)[0].value.contains("allow-group wheel"));
        let booted = "fs=btrfs\nsnapper=yes\nbooted_snapshot=411\nrestorable=yes\n";
        assert!(load(&fake(booted, CSV))[0].value.contains("booted from snapshot #411"));
    }

    #[test]
    fn restore_needs_the_word_and_runs_the_root_owned_script() {
        let rows = load(&fake(OK, CSV));
        let ran = std::cell::RefCell::new(Vec::new());
        let rec = |p: &str, a: &[&str]| {
            ran.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        assert!(apply(5, &rows, Change::Text("yes".into()), &rec).is_err());
        assert!(apply(5, &rows, Change::Toggle, &rec).is_err());
        assert!(ran.borrow().is_empty(), "nothing runs without the word");
        apply(5, &rows, Change::Text(" restore ".into()), &rec).unwrap();
        assert_eq!(*ran.borrow(), [format!("pkexec {SCRIPT} restore 414 --yes")]);
    }
}
