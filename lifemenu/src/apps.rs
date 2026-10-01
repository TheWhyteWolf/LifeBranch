// SPDX-License-Identifier: GPL-3.0-or-later
// Launcher mode: the installed applications, from the Desktop Entry files in
// the XDG data dirs. Parsing follows lifeconf/src/sys/desktop.rs and Exec=
// splitting lifegreet/src/sessions.rs (copied, not shared: the crates are
// built independently). Launch counts are remembered so the apps you use
// most come first.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct App {
    /// File name, e.g. `firefox.desktop`.
    pub id: String,
    pub name: String,
    /// Also searched, never shown: GenericName and Keywords.
    pub extra: String,
    pub exec: String,
    pub terminal: bool,
    pub path: Option<String>,
}

impl App {
    /// What the matcher sees: the name first, so name hits rank first.
    pub fn haystack(&self) -> String {
        if self.extra.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.extra)
        }
    }
}

/// Parse the `[Desktop Entry]` group; None for anything that shouldn't be
/// offered (not an application, hidden, NoDisplay, or excluded from
/// `desktop` by OnlyShowIn/NotShowIn).
pub fn parse(id: &str, body: &str, desktop: &[String]) -> Option<App> {
    let mut a = App { id: id.into(), ..App::default() };
    let (mut in_group, mut seen, mut is_app, mut hidden) = (false, false, false, false);
    let (mut only, mut not): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut extra: Vec<String> = Vec::new();
    let list = |v: &str| v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>();
    for line in body.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_group = l == "[Desktop Entry]";
            seen |= in_group;
            continue;
        }
        if !in_group || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "Type" => is_app = v == "Application",
            "Name" => a.name = v.into(),
            "GenericName" => extra.push(v.into()),
            "Keywords" => extra.extend(list(v)),
            "Exec" => a.exec = v.into(),
            "Path" if !v.is_empty() => a.path = Some(v.into()),
            "Terminal" => a.terminal = v == "true",
            "Hidden" | "NoDisplay" => hidden |= v == "true",
            "OnlyShowIn" => only = list(v),
            "NotShowIn" => not = list(v),
            _ => {}
        }
    }
    let shown_here = (only.is_empty() || only.iter().any(|d| desktop.contains(d)))
        && !not.iter().any(|d| desktop.contains(d));
    if !seen || !is_app || hidden || a.exec.is_empty() || !shown_here {
        return None;
    }
    if a.name.is_empty() {
        a.name = id.trim_end_matches(".desktop").into();
    }
    a.extra = extra.join(" ");
    Some(a)
}

/// $XDG_DATA_HOME/applications, then each $XDG_DATA_DIRS/applications.
pub fn dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_home = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()).unwrap_or(format!("{home}/.local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or("/usr/local/share:/usr/share".into());
    std::iter::once(data_home.as_str())
        .chain(data_dirs.split(':'))
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("applications"))
        .collect()
}

/// Every offered app, earlier directories overriding later ones by id
/// (subdirectories give dashed ids, `kde/foo.desktop` -> `kde-foo.desktop`).
pub fn scan(dirs: &[PathBuf], desktop: &[String]) -> Vec<App> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = Vec::new();
    for dir in dirs {
        let mut files = Vec::new();
        collect(dir, "", &mut files);
        files.sort();
        for (id, path) in files {
            if seen.insert(id.clone(), ()).is_some() {
                continue; // overridden (or hidden) by an earlier directory
            }
            if let Some(a) = std::fs::read_to_string(&path).ok().and_then(|b| parse(&id, &b, desktop)) {
                out.push(a);
            }
        }
    }
    out
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
        if p.is_dir() {
            collect(&p, &format!("{prefix}{name}-"), out);
        } else if name.ends_with(".desktop") {
            out.push((format!("{prefix}{name}"), p));
        }
    }
}

/// Split an Exec= value into argv per the Desktop Entry spec: arguments are
/// space-separated, a double-quoted argument keeps its spaces and takes
/// backslash escapes, standalone field codes (%f, %U, ...) are dropped and %%
/// is a literal %. None for an unterminated quote.
pub fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let (mut in_arg, mut quoted) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            '\\' if quoted => cur.push(chars.next()?),
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut cur));
                    in_arg = false;
                }
            }
            c => {
                cur.push(c);
                in_arg = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if in_arg {
        args.push(cur);
    }
    Some(
        args.into_iter()
            .filter(|a| !(a.len() == 2 && a.starts_with('%') && a != "%%"))
            .map(|a| a.replace("%%", "%"))
            .collect(),
    )
}

/// argv for launching `app` (wrapped in the terminal when Terminal=true).
pub fn argv(app: &App, terminal: &str) -> Option<Vec<String>> {
    let cmd = split_exec(&app.exec).filter(|c| !c.is_empty())?;
    if !app.terminal {
        return Some(cmd);
    }
    let mut v = vec![terminal.to_string(), "-e".to_string()];
    v.extend(cmd);
    Some(v)
}

/// Start `argv` detached: its own session, no stdio tied to us, so it
/// outlives lifemenu and never gets our SIGHUP.
pub fn launch(argv: &[String], dir: Option<&str>) -> std::io::Result<()> {
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(d) = dir.filter(|d| Path::new(d).is_dir()) {
        c.current_dir(d);
    }
    // SAFETY: setsid is async-signal-safe; nothing else runs between fork and exec.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    c.spawn().map(|_| ())
}

// ---- launch history: "count<TAB>id" per line ----

pub fn history_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cache")))?;
    Some(Path::new(&base).join("lifemenu/history"))
}

pub fn read_history(body: &str) -> HashMap<String, u32> {
    body.lines()
        .filter_map(|l| {
            let (n, id) = l.split_once('\t')?;
            Some((id.to_string(), n.parse().ok()?))
        })
        .collect()
}

pub fn write_history(h: &HashMap<String, u32>) -> String {
    let mut v: Vec<_> = h.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    v.into_iter().map(|(id, n)| format!("{n}\t{id}\n")).collect()
}

/// Most-launched first, then by name.
pub fn rank(apps: &mut [App], h: &HashMap<String, u32>) {
    apps.sort_by(|a, b| {
        let (na, nb) = (h.get(&a.id).copied().unwrap_or(0), h.get(&b.id).copied().unwrap_or(0));
        nb.cmp(&na).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Count one launch of `id` and save, atomically (write + rename).
pub fn remember(id: &str) {
    let Some(path) = history_path() else { return };
    let mut h = std::fs::read_to_string(&path).map(|b| read_history(&b)).unwrap_or_default();
    *h.entry(id.to_string()).or_insert(0) += 1;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, write_history(&h)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn niri() -> Vec<String> {
        vec!["niri".into()]
    }

    #[test]
    fn parses_an_application() {
        let a = parse(
            "org.kde.dolphin.desktop",
            "[Desktop Entry]\nType=Application\nName=Dolphin\nGenericName=File Manager\nKeywords=files;folders;\n\
             Exec=dolphin %u\nTerminal=false\n[Desktop Action new]\nName=New Window\nExec=dolphin --new-window\n",
            &niri(),
        )
        .unwrap();
        assert_eq!(a.name, "Dolphin");
        assert_eq!(a.exec, "dolphin %u", "the action's Exec must not override");
        assert_eq!(a.haystack(), "Dolphin File Manager files folders");
    }

    #[test]
    fn skips_what_should_not_be_offered() {
        let base = "[Desktop Entry]\nType=Application\nName=X\nExec=x\n";
        assert!(parse("a.desktop", &format!("{base}NoDisplay=true\n"), &niri()).is_none());
        assert!(parse("a.desktop", &format!("{base}Hidden=true\n"), &niri()).is_none());
        assert!(parse("a.desktop", &format!("{base}OnlyShowIn=KDE;\n"), &niri()).is_none());
        assert!(parse("a.desktop", &format!("{base}NotShowIn=niri;\n"), &niri()).is_none());
        assert!(parse("a.desktop", &format!("{base}OnlyShowIn=niri;GNOME;\n"), &niri()).is_some());
        assert!(parse("a.desktop", "[Desktop Entry]\nType=Link\nName=L\nExec=x\n", &niri()).is_none());
    }

    #[test]
    fn exec_quoting_and_terminal_wrap() {
        assert_eq!(split_exec("sh -c \"echo a\\\"b\" %U").unwrap(), ["sh", "-c", "echo a\"b"]);
        assert_eq!(split_exec("run 100%%").unwrap(), ["run", "100%"]);
        assert!(split_exec("x \"open").is_none());
        let a = App { exec: "htop".into(), terminal: true, ..App::default() };
        assert_eq!(argv(&a, "kitty").unwrap(), ["kitty", "-e", "htop"]);
    }

    #[test]
    fn history_round_trips_and_ranks() {
        let mut h = HashMap::new();
        h.insert("b.desktop".to_string(), 3);
        h.insert("c.desktop".to_string(), 1);
        assert_eq!(read_history(&write_history(&h)), h);
        let app = |id: &str, name: &str| App { id: id.into(), name: name.into(), ..App::default() };
        let mut apps = vec![app("a.desktop", "Alpha"), app("c.desktop", "Charlie"), app("b.desktop", "Bravo")];
        rank(&mut apps, &h);
        let ids: Vec<_> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["b.desktop", "c.desktop", "a.desktop"]);
    }

    #[test]
    fn scan_overrides_by_id_across_dirs() {
        let d = std::env::temp_dir().join(format!("lifemenu-scan-{}", std::process::id()));
        let (user, sys) = (d.join("user"), d.join("sys"));
        std::fs::create_dir_all(sys.join("kde")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let entry = |n: &str| format!("[Desktop Entry]\nType=Application\nName={n}\nExec=x\n");
        std::fs::write(sys.join("x.desktop"), entry("System X")).unwrap();
        std::fs::write(user.join("x.desktop"), entry("User X")).unwrap();
        std::fs::write(sys.join("kde/y.desktop"), entry("Y")).unwrap();
        let apps = scan(&[user, sys], &niri());
        let names: Vec<_> = apps.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect();
        assert!(names.contains(&("x.desktop", "User X")) && !names.contains(&("x.desktop", "System X")));
        assert!(names.contains(&("kde-y.desktop", "Y")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
