// SPDX-License-Identifier: GPL-3.0-or-later
// Autostart: which XDG autostart entries run at login. This session starts via
// niri-session, so systemd's xdg-autostart-generator runs them. The list is the
// user's ~/.config/autostart over /etc/xdg/autostart, minus entries that
// OnlyShowIn/NotShowIn already exclude from niri (they'd never start, so
// toggling them would be noise). Disabling a system entry writes a user
// override with Hidden=true; nothing under /etc is touched.

use super::desktop::{self, Entry};
use super::{pick, Change, Row, RowKind, Runner, Sel};
use std::path::PathBuf;

pub const LABELS: &[&str] = &["entry", "enabled", "command"];

static SELECTED: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    match field {
        0 => RowKind::Choice,
        1 => RowKind::Bool,
        _ => RowKind::Info,
    }
}

pub struct Dirs {
    pub user: PathBuf,
    pub system: Vec<PathBuf>,
}

pub fn real_dirs() -> Dirs {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let user = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("autostart");
    let sys = std::env::var("XDG_CONFIG_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    Dirs { user, system: sys.split(':').map(|d| PathBuf::from(d).join("autostart")).collect() }
}

/// Would this entry ever be started by this desktop?
fn runs_here(e: &Entry, desktops: &[String]) -> bool {
    let hit = |l: &[String]| l.iter().any(|d| desktops.iter().any(|x| x.eq_ignore_ascii_case(d)));
    (e.only_show_in.is_empty() || hit(&e.only_show_in)) && !hit(&e.not_show_in)
}

fn current_desktops() -> Vec<String> {
    let d = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let v: Vec<String> = d.split(':').filter(|s| !s.is_empty()).map(str::to_string).collect();
    if v.is_empty() { vec!["niri".into()] } else { v }
}

fn entries(dirs: &Dirs, desktops: &[String]) -> Vec<Entry> {
    let mut order = vec![dirs.user.clone()];
    order.extend(dirs.system.iter().cloned());
    let mut v: Vec<Entry> = desktop::scan(&order).into_iter().filter(|e| runs_here(e, desktops)).collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

pub fn load(run: Runner) -> Vec<Row> {
    load_with(run, &real_dirs(), &current_desktops())
}

pub fn load_with(_run: Runner, dirs: &Dirs, desktops: &[String]) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let es = entries(dirs, desktops);
    if es.is_empty() {
        return vec![r("none"), r("-"), r("-")];
    }
    let label = |e: &Entry| {
        if es.iter().filter(|x| x.name == e.name).count() > 1 {
            format!("{} [{}]", e.name, e.id.trim_end_matches(".desktop"))
        } else {
            e.name.clone()
        }
    };
    let want = SELECTED.get();
    let sel = es.iter().find(|e| e.id == want).unwrap_or(&es[0]);
    vec![
        Row { value: label(sel), choices: es.iter().map(|e| (label(e), e.id.clone())).collect() },
        r(&(!sel.hidden).to_string()),
        r(&sel.exec),
    ]
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    apply_with(field, rows, ch, run, &real_dirs(), &current_desktops())
}

pub fn apply_with(
    field: usize,
    rows: &[Row],
    ch: Change,
    _run: Runner,
    dirs: &Dirs,
    desktops: &[String],
) -> Result<String, String> {
    let entry_row = rows.first().ok_or("no entries")?;
    match field {
        0 => {
            let (label, id) = pick(entry_row, &ch, "entry")?;
            SELECTED.set(id);
            Ok(format!("selected {label}"))
        }
        1 => {
            let (label, id) = entry_row
                .choices
                .iter()
                .find(|(l, _)| *l == entry_row.value)
                .ok_or("no entry selected")?;
            let cur = entries(dirs, desktops).into_iter().find(|e| e.id == *id).ok_or("entry vanished")?;
            let want_on = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => cur.hidden, // currently off -> turn on, and vice versa
            };
            set_enabled(dirs, id, want_on).map_err(|e| format!("{id}: {e}"))?;
            Ok(format!("{label} {} at next login", if want_on { "enabled" } else { "disabled" }))
        }
        _ => Err("read-only".into()),
    }
}

fn set_enabled(dirs: &Dirs, id: &str, on: bool) -> std::io::Result<()> {
    let user = dirs.user.join(id);
    let body = match std::fs::read_to_string(&user) {
        Ok(b) => b,
        // No override yet: start from the system file so the user copy is a full
        // entry (a bare Hidden=true file would have no Exec/Type to override).
        Err(_) => {
            let src = dirs
                .system
                .iter()
                .map(|d| d.join(id))
                .find(|p| p.exists())
                .ok_or_else(|| std::io::Error::other("no such entry"))?;
            std::fs::read_to_string(src)?
        }
    };
    desktop::write_atomic(&user, &desktop::set_key(&body, "Hidden", if on { "false" } else { "true" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn setup(tag: &str) -> (PathBuf, Dirs) {
        let base = std::env::temp_dir().join(format!("lc-auto-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (user, sys) = (base.join("user"), base.join("sys"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        let w = |d: &PathBuf, f: &str, name: &str, extra: &str| {
            std::fs::write(d.join(f), format!("[Desktop Entry]\nType=Application\nName={name}\nExec=run-{f}\n{extra}")).unwrap();
        };
        w(&sys, "nm-applet.desktop", "Network Manager Applet", "");
        w(&sys, "baloo.desktop", "Baloo", "");
        w(&sys, "kde-only.desktop", "KDE Only", "OnlyShowIn=KDE;\n");
        w(&sys, "not-niri.desktop", "Not Niri", "NotShowIn=niri;\n");
        w(&user, "mine.desktop", "Mine", "");
        (base, Dirs { user, system: vec![sys] })
    }

    fn nr() -> impl Fn(&str, &[&str]) -> Result<String, String> {
        |_, _| Ok(String::new())
    }

    fn niri() -> Vec<String> {
        vec!["niri".into()]
    }

    #[test]
    fn lists_only_entries_that_would_run_here() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("");
        let (base, d) = setup("list");
        let rows = load_with(&nr(), &d, &niri());
        let _ = std::fs::remove_dir_all(&base);
        let names: Vec<_> = rows[0].choices.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(names, ["Baloo", "Mine", "Network Manager Applet"]);
        assert_eq!(rows[1].value, "true");
    }

    #[test]
    fn disabling_a_system_entry_writes_a_user_override_and_enabling_reverts() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("baloo.desktop");
        let (base, d) = setup("toggle");
        let sys_before = std::fs::read_to_string(d.system[0].join("baloo.desktop")).unwrap();
        let rows = load_with(&nr(), &d, &niri());
        let msg = apply_with(1, &rows, Change::Toggle, &nr(), &d, &niri()).unwrap();
        assert!(msg.contains("disabled"));
        let user = std::fs::read_to_string(d.user.join("baloo.desktop")).unwrap();
        assert!(user.contains("Hidden=true") && user.contains("Exec=run-baloo.desktop"));
        assert_eq!(std::fs::read_to_string(d.system[0].join("baloo.desktop")).unwrap(), sys_before, "system file untouched");

        let rows = load_with(&nr(), &d, &niri());
        assert_eq!(rows[1].value, "false");
        apply_with(1, &rows, Change::Toggle, &nr(), &d, &niri()).unwrap();
        let rows = load_with(&nr(), &d, &niri());
        let _ = std::fs::remove_dir_all(&base);
        SELECTED.set("");
        assert_eq!(rows[1].value, "true");
    }

    #[test]
    fn empty_autostart_degrades() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let d = Dirs { user: "/nonexistent-a".into(), system: vec!["/nonexistent-b".into()] };
        let rows = load_with(&nr(), &d, &niri());
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "none");
    }
}
