// SPDX-License-Identifier: GPL-3.0-or-later
// Session picker: Name=/Exec= from the [Desktop Entry] section of every
// *.desktop file in the wayland-sessions dir (tiny hand-rolled parse, no
// deps). F3 cycles; the selection becomes greetd's start_session cmd.

pub struct SessionEntry {
    pub name: String,
    pub cmd: Vec<String>,
}

pub struct Sessions {
    entries: Vec<SessionEntry>,
    idx: usize,
}

impl Sessions {
    /// Load `dir`, sorted by filename for a stable cycle order. Defaults to
    /// the niri session when present. An empty/unreadable dir falls back to
    /// `fallback_cmd` as the single entry.
    pub fn load(dir: &str, fallback_cmd: &[String]) -> Sessions {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();

        let mut entries: Vec<SessionEntry> = files
            .iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .filter_map(|text| parse_desktop(&text))
            .collect();

        if entries.is_empty() {
            entries.push(SessionEntry {
                name: fallback_cmd.join(" "),
                cmd: fallback_cmd.to_vec(),
            });
        }

        let idx = entries
            .iter()
            .position(|e| e.cmd.iter().any(|w| w.contains("niri-session")))
            .unwrap_or(0);
        Sessions { entries, idx }
    }

    pub fn current(&self) -> &SessionEntry {
        &self.entries[self.idx]
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn cycle(&mut self) {
        self.idx = (self.idx + 1) % self.entries.len();
    }
}

/// One .desktop file -> entry. Only the [Desktop Entry] section is read;
/// Hidden/NoDisplay entries are dropped; %-field codes are stripped (session
/// files shouldn't have them, but be safe — the cmd goes to greetd verbatim).
fn parse_desktop(text: &str) -> Option<SessionEntry> {
    let mut in_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut hidden = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            name.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Exec=") {
            exec.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Hidden=") {
            hidden |= v.trim().eq_ignore_ascii_case("true");
        } else if let Some(v) = line.strip_prefix("NoDisplay=") {
            hidden |= v.trim().eq_ignore_ascii_case("true");
        }
    }
    if hidden {
        return None;
    }
    let exec = exec?;
    let cmd: Vec<String> = exec
        .split_whitespace()
        .filter(|w| !w.starts_with('%'))
        .map(str::to_string)
        .collect();
    if cmd.is_empty() {
        return None;
    }
    Some(SessionEntry { name: name.unwrap_or_else(|| cmd[0].clone()), cmd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_defaults_to_niri() {
        let niri = "[Desktop Entry]\nName=Niri\nComment=x\nExec=niri-session\nType=Application\n";
        let plasma = "[Desktop Entry]\nName=Plasma (Wayland)\nExec=/usr/lib/plasma-dbus-run-session-if-needed /usr/bin/startplasma-wayland\n";
        let a = parse_desktop(plasma).unwrap();
        assert_eq!(a.name, "Plasma (Wayland)");
        assert_eq!(a.cmd.len(), 2);
        let b = parse_desktop(niri).unwrap();
        assert_eq!(b.cmd, vec!["niri-session"]);
    }

    #[test]
    fn hidden_and_actions_sections_skipped() {
        let hidden = "[Desktop Entry]\nName=X\nExec=x\nHidden=true\n";
        assert!(parse_desktop(hidden).is_none());
        let with_action = "[Desktop Entry]\nName=Y\nExec=y %U\n[Desktop Action new]\nName=Z\nExec=z\n";
        let e = parse_desktop(with_action).unwrap();
        assert_eq!(e.name, "Y");
        assert_eq!(e.cmd, vec!["y"]); // %U stripped, action Exec ignored
    }

    #[test]
    fn load_missing_dir_falls_back_and_cycles() {
        let fallback = vec!["niri-session".to_string()];
        let mut s = Sessions::load("/nonexistent-lifegreet-test", &fallback);
        assert_eq!(s.len(), 1);
        assert_eq!(s.current().cmd, fallback);
        s.cycle();
        assert_eq!(s.current().cmd, fallback); // single entry cycles to itself
    }

    #[test]
    fn load_real_dir_and_cycle() {
        let dir = std::env::temp_dir().join(format!("lifegreet-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a-plasma.desktop"), "[Desktop Entry]\nName=Plasma\nExec=startplasma-wayland\n").unwrap();
        std::fs::write(dir.join("niri.desktop"), "[Desktop Entry]\nName=Niri\nExec=niri-session\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a desktop file").unwrap();
        let fallback = vec!["fallback".to_string()];
        let mut s = Sessions::load(dir.to_str().unwrap(), &fallback);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(s.len(), 2);
        assert_eq!(s.current().name, "Niri"); // default despite sort order
        s.cycle();
        assert_eq!(s.current().name, "Plasma");
        s.cycle();
        assert_eq!(s.current().name, "Niri");
    }
}
