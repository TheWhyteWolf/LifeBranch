// SPDX-License-Identifier: GPL-3.0-or-later
// Default applications: one row per role (browser, file manager, …). Each role
// is a set of MIME types that are switched together, so "image viewer" covers
// png/jpeg/gif/webp rather than only the one type. Candidates are the installed
// apps that declare the role's main type; reading and writing go through
// xdg-mime, so ~/.config/mimeapps.list stays the single source of truth.

use super::desktop::{self, Entry};
use super::{pick, Change, Row, RowKind, Runner};
use std::path::PathBuf;

/// (label, the type that decides candidates, every type set together)
const ROLES: &[(&str, &str, &[&str])] = &[
    (
        "web browser",
        "x-scheme-handler/http",
        &["x-scheme-handler/http", "x-scheme-handler/https", "text/html"],
    ),
    ("file manager", "inode/directory", &["inode/directory"]),
    ("text editor", "text/plain", &["text/plain"]),
    ("image viewer", "image/png", &["image/png", "image/jpeg", "image/gif", "image/webp"]),
    ("video player", "video/mp4", &["video/mp4", "video/x-matroska", "video/webm"]),
    (
        "audio player",
        "audio/mpeg",
        &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav"],
    ),
    ("pdf viewer", "application/pdf", &["application/pdf"]),
    (
        "archives",
        "application/zip",
        &["application/zip", "application/x-tar", "application/gzip", "application/x-7z-compressed"],
    ),
    ("email", "x-scheme-handler/mailto", &["x-scheme-handler/mailto"]),
];

pub const LABELS: &[&str] = &[
    "web browser",
    "file manager",
    "text editor",
    "image viewer",
    "video player",
    "audio player",
    "pdf viewer",
    "archives",
    "email",
];

pub fn kind(_field: usize) -> RowKind {
    RowKind::Choice
}

/// Highest priority first, as the XDG spec orders them.
pub fn app_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    vec![
        home.join(".local/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/usr/share/applications"),
        home.join(".local/share/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ]
}

pub fn load(run: Runner) -> Vec<Row> {
    load_with(run, &app_dirs())
}

pub fn load_with(run: Runner, dirs: &[PathBuf]) -> Vec<Row> {
    if run("xdg-mime", &["--version"]).is_err() {
        return ROLES.iter().map(|_| Row { value: "xdg-mime unavailable".into(), choices: vec![] }).collect();
    }
    let apps: Vec<Entry> = desktop::scan(dirs).into_iter().filter(|e| !e.no_display && !e.hidden).collect();
    ROLES
        .iter()
        .map(|(_, primary, _)| {
            let mut cands: Vec<&Entry> = apps.iter().filter(|e| e.mime.iter().any(|m| m == primary)).collect();
            cands.sort_by_key(|e| e.name.to_lowercase());
            // Two apps can share a Name; keep them distinguishable.
            let label = |e: &Entry| {
                if cands.iter().filter(|c| c.name == e.name).count() > 1 {
                    format!("{} [{}]", e.name, e.id.trim_end_matches(".desktop"))
                } else {
                    e.name.clone()
                }
            };
            let current = run("xdg-mime", &["query", "default", primary]).unwrap_or_default();
            let current = current.trim();
            let value = match cands.iter().find(|e| e.id == current) {
                Some(e) => label(e),
                None if current.is_empty() => "none".to_string(),
                // Set to something that doesn't declare the type (e.g. by hand):
                // show it as it is rather than pretend it isn't set.
                None => current.to_string(),
            };
            Row { value, choices: cands.iter().map(|e| (label(e), e.id.clone())).collect() }
        })
        .collect()
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    let (label, mimes) = ROLES.get(field).map(|(l, _, m)| (*l, *m)).ok_or("no such row")?;
    let (name, id) = pick(row, &ch, label)?;
    let mut args = vec!["default", id.as_str()];
    args.extend(mimes.iter().copied());
    run("xdg-mime", &args)?;
    Ok(format!("{label} -> {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn dirs(tag: &str) -> (PathBuf, Vec<PathBuf>) {
        let base = std::env::temp_dir().join(format!("lc-apps-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let w = |f: &str, name: &str, mime: &str, extra: &str| {
            std::fs::write(
                base.join(f),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\nMimeType={mime}\n{extra}"),
            )
            .unwrap();
        };
        w("firefox.desktop", "Firefox", "text/html;x-scheme-handler/http;x-scheme-handler/https;", "");
        w("brave.desktop", "Brave", "x-scheme-handler/http;", "");
        w("hidden.desktop", "Hidden Browser", "x-scheme-handler/http;", "NoDisplay=true\n");
        w("lifefiles.desktop", "lifefiles", "inode/directory;", "");
        w("dolphin.desktop", "Dolphin", "inode/directory;", "");
        let d = vec![base.clone()];
        (base, d)
    }

    fn xdg(a: &[&str]) -> Result<String, String> {
        Ok(match a {
            ["query", "default", "x-scheme-handler/http"] => "firefox.desktop\n".into(),
            ["query", "default", "inode/directory"] => "dolphin.desktop\n".into(),
            _ => String::new(),
        })
    }

    #[test]
    fn candidates_are_apps_declaring_the_type_minus_hidden() {
        let (base, d) = dirs("cand");
        let rows = load_with(&|_, a| xdg(a), &d);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(rows.len(), LABELS.len());
        let names: Vec<_> = rows[0].choices.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(names, ["Brave", "Firefox"]);
        assert_eq!(rows[0].value, "Firefox");
        assert_eq!(rows[1].value, "Dolphin");
        assert_eq!(rows[2].value, "none"); // nothing set, no text editors installed
    }

    #[test]
    fn choosing_sets_every_type_of_the_role_in_one_call() {
        let (base, d) = dirs("set");
        let rows = load_with(&|_, a| xdg(a), &d);
        let _ = std::fs::remove_dir_all(&base);
        let log = RefCell::new(Vec::<String>::new());
        let rec = |_: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(a.join(" "));
            Ok(String::new())
        };
        apply(0, &rows, Change::Step(-1), &rec).unwrap(); // Firefox -> Brave
        apply(1, &rows, Change::Text("lifefiles".into()), &rec).unwrap();
        assert_eq!(
            *log.borrow(),
            [
                "default brave.desktop x-scheme-handler/http x-scheme-handler/https text/html",
                "default lifefiles.desktop inode/directory",
            ]
        );
    }

    #[test]
    fn missing_xdg_mime_degrades() {
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("xdg-mime: not found".into()) };
        let rows = load_with(&none, &[]);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "xdg-mime unavailable");
    }
}
