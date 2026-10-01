// SPDX-License-Identifier: GPL-3.0-or-later
// Just enough of the freedesktop Desktop Entry spec for the Apps and Autostart
// panels: read the [Desktop Entry] group's plain keys, scan directories with
// the usual "earlier directory wins" override rule, and rewrite one key in
// place without disturbing the rest of the file.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// File name, e.g. `firefox.desktop` — the id xdg-mime and autostart use.
    pub id: String,
    pub name: String,
    pub exec: String,
    pub mime: Vec<String>,
    pub hidden: bool,
    pub no_display: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
}

fn list(v: &str) -> Vec<String> {
    v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Parse the `[Desktop Entry]` group. Localised keys (`Name[fr]`) are ignored.
/// None for non-applications or files with no such group.
pub fn parse(id: &str, body: &str) -> Option<Entry> {
    let mut e = Entry { id: id.into(), ..Entry::default() };
    let (mut in_group, mut seen_group, mut is_app) = (false, false, true);
    for line in body.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_group = l == "[Desktop Entry]";
            seen_group |= in_group;
            continue;
        }
        if !in_group || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "Type" => is_app = v == "Application",
            "Name" => e.name = v.into(),
            "Exec" => e.exec = v.into(),
            "MimeType" => e.mime = list(v),
            "Hidden" => e.hidden = v == "true",
            "NoDisplay" => e.no_display = v == "true",
            "OnlyShowIn" => e.only_show_in = list(v),
            "NotShowIn" => e.not_show_in = list(v),
            _ => {}
        }
    }
    (seen_group && is_app).then(|| {
        if e.name.is_empty() {
            e.name = id.trim_end_matches(".desktop").to_string();
        }
        e
    })
}

/// Scan `dirs` (highest priority first) for `*.desktop`. A file name seen in an
/// earlier directory hides the same name later — that's how a user entry
/// overrides a system one. Subdirectories are skipped (their ids need the
/// dashed-path rule, which nothing here needs).
pub fn scan(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<_> = rd.flatten().map(|d| d.path()).collect();
        files.sort();
        for p in files {
            let Some(id) = p.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if !id.ends_with(".desktop") || out.iter().any(|e| e.id == id) {
                continue;
            }
            if let Some(e) = std::fs::read_to_string(&p).ok().and_then(|b| parse(&id, &b)) {
                out.push(e);
            }
        }
    }
    out
}

/// Set `key=value` inside the `[Desktop Entry]` group: replace the first
/// existing line there, else insert right under the group header. A same-named
/// key in some other group (an Action section) is never touched.
pub fn set_key(body: &str, key: &str, value: &str) -> String {
    let is_key = |l: &str| l.trim().split_once('=').is_some_and(|(k, _)| k.trim() == key);
    // Pass 1: does the main group already have the key?
    let (mut in_group, mut present) = (false, false);
    for l in body.lines() {
        if l.trim().starts_with('[') {
            in_group = l.trim() == "[Desktop Entry]";
        } else if in_group && is_key(l) {
            present = true;
        }
    }
    // Pass 2: rewrite.
    let mut out = Vec::new();
    let (mut in_group, mut done) = (false, false);
    for line in body.lines() {
        if line.trim().starts_with('[') {
            in_group = line.trim() == "[Desktop Entry]";
            out.push(line.to_string());
            if in_group && !present {
                out.push(format!("{key}={value}"));
                done = true;
            }
        } else if in_group && present && !done && is_key(line) {
            out.push(format!("{key}={value}"));
            done = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !done {
        // No [Desktop Entry] header at all: make one rather than lose the write.
        out.insert(0, format!("[Desktop Entry]\n{key}={value}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Write `body` to `path` via a temp file + rename, creating parents.
pub fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("desktop.lifeconf-tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FF: &str = "[Desktop Entry]\nType=Application\nName=Firefox\nName[fr]=Navigateur\nExec=firefox %u\nMimeType=text/html;x-scheme-handler/http;\n\n[Desktop Action new-window]\nName=New Window\nExec=firefox --new-window\n";

    #[test]
    fn parses_main_group_only() {
        let e = parse("firefox.desktop", FF).unwrap();
        assert_eq!(e.name, "Firefox"); // not the locale or action Name
        assert_eq!(e.exec, "firefox %u");
        assert_eq!(e.mime, ["text/html", "x-scheme-handler/http"]);
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Link\n").is_none());
        assert!(parse("x.desktop", "garbage").is_none());
        assert_eq!(parse("foo.desktop", "[Desktop Entry]\nType=Application\n").unwrap().name, "foo");
    }

    #[test]
    fn set_key_replaces_inserts_and_leaves_other_groups() {
        let r = set_key(FF, "Hidden", "true");
        assert!(r.starts_with("[Desktop Entry]\nHidden=true\nType=Application"));
        let r2 = set_key(&r, "Hidden", "false");
        assert_eq!(r2.matches("Hidden=").count(), 1);
        assert!(r2.contains("Hidden=false"));
        assert!(r2.contains("[Desktop Action new-window]\nName=New Window"));
        // A key that only exists in another group must not be edited in place.
        let tricky = "[Desktop Entry]\nName=A\n\n[Other]\nHidden=true\n";
        let t = set_key(tricky, "Hidden", "false");
        assert!(t.contains("[Desktop Entry]\nHidden=false\nName=A"));
        assert!(t.contains("[Other]\nHidden=true"));
    }

    #[test]
    fn earlier_directory_overrides() {
        let base = std::env::temp_dir().join(format!("lc-desktop-{}", std::process::id()));
        let (a, b) = (base.join("user"), base.join("sys"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.desktop"), "[Desktop Entry]\nType=Application\nName=User X\n").unwrap();
        std::fs::write(b.join("x.desktop"), "[Desktop Entry]\nType=Application\nName=Sys X\n").unwrap();
        std::fs::write(b.join("y.desktop"), "[Desktop Entry]\nType=Application\nName=Y\n").unwrap();
        std::fs::write(b.join("readme.txt"), "x").unwrap();
        let e = scan(&[a, b]);
        let _ = std::fs::remove_dir_all(&base);
        let names: Vec<_> = e.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["User X", "Y"]);
    }
}
