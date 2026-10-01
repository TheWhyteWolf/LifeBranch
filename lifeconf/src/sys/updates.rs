// SPDX-License-Identifier: GPL-3.0-or-later
// Updates panel: what pacman and the AUR have newer, and a way to install it.
//
// Checking talks to the mirrors and takes seconds, so the page shows the last
// check (cached in ~/.cache/lifeconf/updates) and only checks again when asked.
// checkupdates (pacman-contrib) syncs a private copy of the databases, so a
// check never half-updates the system; without it, `pacman -Qu` reads the last
// real sync and the page says so.
//
// Updating opens a terminal running `yay -Syu` (pacman when there is no yay):
// an upgrade asks questions (replacements, conflicts, the sudo password) and
// its output is worth seeing, so it is not hidden behind a button.

use super::{Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &["official repos", "AUR", "packages", "last checked", "check now", "update now"];

pub fn kind(field: usize) -> RowKind {
    match field {
        0..=3 => RowKind::Info,
        _ => RowKind::Action,
    }
}

/// One pending update.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub name: String,
    pub from: String,
    pub to: String,
}

/// `name old -> new` lines (checkupdates, pacman -Qu, yay -Qua; yay appends
/// an age in brackets, which is dropped).
pub fn parse(out: &str) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4 && f[2] == "->").then(|| Update { name: f[0].into(), from: f[1].into(), to: f[3].into() })
        })
        .collect()
}

fn cache_path() -> String {
    let home = std::env::var("LIFECONF_HOME").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    format!("{home}/.cache/lifeconf/updates")
}

/// The cache: a header line `checked <unix secs> <source>` then `repo`/`aur`
/// tagged update lines.
#[derive(Debug, Default, PartialEq)]
pub struct Check {
    pub when: u64,
    pub source: String,
    pub repo: Vec<Update>,
    pub aur: Vec<Update>,
}

pub fn render(c: &Check) -> String {
    let mut s = format!("checked {} {}\n", c.when, c.source);
    for (tag, list) in [("repo", &c.repo), ("aur", &c.aur)] {
        for u in list {
            s.push_str(&format!("{tag} {} {} -> {}\n", u.name, u.from, u.to));
        }
    }
    s
}

pub fn read_cache(body: &str) -> Option<Check> {
    let mut lines = body.lines();
    let head: Vec<&str> = lines.next()?.split_whitespace().collect();
    if head.first() != Some(&"checked") {
        return None;
    }
    let mut c = Check { when: head.get(1)?.parse().ok()?, source: head.get(2).unwrap_or(&"").to_string(), ..Check::default() };
    for l in lines {
        let (tag, rest) = l.split_once(' ')?;
        let u = parse(rest).pop()?;
        if tag == "aur" { c.aur.push(u) } else { c.repo.push(u) }
    }
    Some(c)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "3 hours ago" for the last-checked row.
pub fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

/// Ask the mirrors (and the AUR) and cache the answer.
pub fn check(run: Runner) -> Result<Check, String> {
    // checkupdates exits 2 for "nothing to update", which the runner reports
    // as a failure; its stdout is empty then, so treat that as none.
    let (repo, source) = match run("checkupdates", &[]) {
        Ok(o) => (parse(&o), "checkupdates"),
        Err(e) if e.contains("No such file") || e.contains("not found") => {
            (run("pacman", &["-Qu"]).map(|o| parse(&o)).unwrap_or_default(), "last-sync")
        }
        Err(_) => (Vec::new(), "checkupdates"),
    };
    let aur = run("yay", &["-Qua"]).map(|o| parse(&o)).unwrap_or_default();
    let c = Check { when: now(), source: source.into(), repo, aur };
    let p = cache_path();
    if let Some(d) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&p, render(&c)).map_err(|e| format!("{p}: {e}"))?;
    Ok(c)
}

fn summary(list: &[Update]) -> String {
    match list.len() {
        0 => "up to date".into(),
        1 => "1 update".into(),
        n => format!("{n} updates"),
    }
}

pub fn load(_run: Runner) -> Vec<Row> {
    let r = |v: String| Row { value: v, choices: vec![] };
    let cached = std::fs::read_to_string(cache_path()).ok().and_then(|b| read_cache(&b));
    let Some(c) = cached else {
        return vec![
            r("not checked yet".into()),
            r("-".into()),
            r("-".into()),
            r("never".into()),
            r("check for updates".into()),
            r("update everything (opens a terminal)".into()),
        ];
    };
    let names: Vec<&str> = c.repo.iter().chain(&c.aur).map(|u| u.name.as_str()).collect();
    let shown = if names.is_empty() {
        "-".to_string()
    } else if names.len() <= 6 {
        names.join(", ")
    } else {
        format!("{}, +{} more", names[..6].join(", "), names.len() - 6)
    };
    let stale = if c.source == "last-sync" { " (from the last sync; install pacman-contrib for a live check)" } else { "" };
    vec![
        r(format!("{}{stale}", summary(&c.repo))),
        r(summary(&c.aur)),
        r(shown),
        r(ago(now().saturating_sub(c.when))),
        r("check again".into()),
        r("update everything (opens a terminal)".into()),
    ]
}

pub fn apply(field: usize, _rows: &[Row], _ch: Change, run: Runner) -> Result<String, String> {
    match field {
        4 => {
            let c = check(run)?;
            Ok(format!("repos: {}, AUR: {}", summary(&c.repo), summary(&c.aur)))
        }
        5 => {
            let tool = if run("sh", &["-c", "command -v yay"]).is_ok_and(|o| !o.trim().is_empty()) { "yay -Syu" } else { "sudo pacman -Syu" };
            // The window stays until Enter, so the result can be read; then
            // the cache is refreshed so the page shows the new state.
            let script = format!("{tool}; echo; read -rp 'Done. Enter closes this window. ' _");
            run("sh", &["-c", &format!("setsid -f kitty --title 'System update' sh -c \"{script}\" >/dev/null 2>&1")])?;
            let _ = std::fs::remove_file(cache_path());
            Ok("update running in a terminal; check again when it's done".into())
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_tools_lines() {
        let yay = "brave-bin 1:1.96.59-1 -> 1:1.96.60-1 [1d1h]\nclaude-code 2.1.285-1 -> 2.1.287-1 [2h38m]\n";
        let u = parse(yay);
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].name.as_str(), u[0].to.as_str()), ("brave-bin", "1:1.96.60-1"));
        assert_eq!(parse("linux 6.10.1-1 -> 6.10.2-1\n:: warning\n").len(), 1);
    }

    #[test]
    fn cache_round_trips() {
        let c = Check {
            when: 1_790_000_000,
            source: "checkupdates".into(),
            repo: parse("linux 6.10.1-1 -> 6.10.2-1\n"),
            aur: parse("yay 12-1 -> 13-1\n"),
        };
        assert_eq!(read_cache(&render(&c)), Some(c));
        assert_eq!(read_cache("garbage"), None);
    }

    #[test]
    fn ages_and_summaries() {
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(7200), "2 h ago");
        assert_eq!(ago(3 * 86400), "3 days ago");
        assert_eq!(summary(&[]), "up to date");
    }

    #[test]
    fn check_falls_back_without_pacman_contrib_and_caches() {
        let _g = crate::sys::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("lc-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LIFECONF_HOME", &dir);
        let fake = |p: &str, _: &[&str]| -> Result<String, String> {
            match p {
                "checkupdates" => Err("checkupdates: No such file or directory (os error 2)".into()),
                "pacman" => Ok("mesa 1-1 -> 2-1\n".into()),
                "yay" => Ok("brave-bin 1 -> 2 [1d]\n".into()),
                _ => Ok(String::new()),
            }
        };
        let c = check(&fake).unwrap();
        assert_eq!((c.repo.len(), c.aur.len(), c.source.as_str()), (1, 1, "last-sync"));
        let rows = load(&fake);
        assert!(rows[0].value.starts_with("1 update (from the last sync"));
        assert_eq!(rows[2].value, "mesa, brave-bin");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
