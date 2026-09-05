// SPDX-License-Identifier: GPL-3.0-or-later
// Notification model: the popup queue, history ring, and the event type the
// bus thread (bus.rs, M3) and test harnesses feed into the calloop channel.

use crate::cli::Cfg;
use crate::layout::{self, Grid};
use crate::text;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

impl Urgency {
    pub fn from_hint(byte: u8) -> Urgency {
        match byte {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// org.freedesktop.Notifications close reasons.
pub const REASON_EXPIRED: u32 = 1;
pub const REASON_DISMISSED: u32 = 2;
pub const REASON_CLOSED: u32 = 3;

/// One notification as it arrives from the bus (or a test harness).
#[derive(Clone, Debug)]
pub struct NoteData {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    /// Spec value: -1 = server default, 0 = never expire, >0 = ms.
    pub timeout_ms: i32,
}

/// Everything the main loop reacts to besides Wayland events.
pub enum AppEvent {
    Notify(NoteData),
    Close(u32),
    DndChanged(bool),
    DismissAll,
    /// `lifenote ctl reload` — re-read the config file and restyle live
    /// (lifeconf pushes this after rewriting ~/.config/lifenote/config).
    Reload,
}

pub struct HistoryEntry {
    pub app: String,
    pub summary: String,
    pub body: String,
}

/// The history ring plus the waybar badge counter.
pub struct HistoryRing {
    pub entries: VecDeque<HistoryEntry>,
    /// Entries that arrived unseen (expired, DND-swallowed, or closed by the
    /// sender — anything but a user dismissal) since `ctl history` last read
    /// the ring. Drives the waybar `#` badge.
    pub unseen: u32,
}

/// Shared with the bus thread: `lifenote ctl history` reads it directly.
pub type History = Arc<Mutex<HistoryRing>>;

pub fn new_history() -> History {
    Arc::new(Mutex::new(HistoryRing { entries: VecDeque::new(), unseen: 0 }))
}

/// Nudge waybar's `#` badge (custom/notif, `"signal": 9` — the dnd-toggle.sh
/// RTMIN+8 pattern). Fire-and-forget; no waybar running is fine.
pub fn ping_waybar() {
    if cfg!(test) {
        return; // unit tests must not signal the live bar
    }
    let _ = std::process::Command::new("pkill")
        .args(["-RTMIN+9", "waybar"])
        .status();
}

pub struct Note {
    pub data: NoteData,
    pub grid: Grid,
    /// Generation key of the expiry timer armed for this note; a stale timer
    /// firing after a replace/dismiss compares unequal and does nothing.
    /// 0 = no live timer (freshly queued or displaced off-screen) — expiry
    /// follows visibility, armed by App::sync_expiries when the note shows.
    pub deadline_gen: u64,
}

/// Wrap + frame one notification per the config (style keyed on urgency).
pub fn compose_note(cfg: &Cfg, d: &NoteData) -> Grid {
    let style = match d.urgency {
        Urgency::Critical => cfg.critical_border_style,
        _ => cfg.border_style,
    };
    let summary = {
        let mut s = text::wrap(&text::strip_markup(&d.summary), cfg.max_width);
        s.truncate(2);
        s
    };
    let body_src = text::strip_markup(&d.body);
    let mut body = if body_src.trim().is_empty() {
        Vec::new()
    } else {
        text::wrap(&body_src, cfg.max_width)
    };
    body.truncate(cfg.max_lines);
    layout::compose(&d.app, &summary, &body, style, cfg.max_width)
}

/// Effective expiry in ms; None = never.
pub fn effective_timeout(cfg: &Cfg, d: &NoteData) -> Option<u64> {
    let ms = match (d.urgency, d.timeout_ms) {
        (Urgency::Critical, _) => cfg.critical_timeout_ms,
        (_, t) if t < 0 => cfg.default_timeout_ms,
        (_, t) => t as u64,
    };
    (ms > 0).then_some(ms)
}

const HISTORY_CAP: usize = 50;

/// Popup queue, newest first. `notes[..max_visible]` are on screen; the rest
/// wait and surface as slots free up.
pub struct Queue {
    pub notes: Vec<Note>,
    pub history: History,
}

impl Queue {
    pub fn new(history: History) -> Queue {
        Queue { notes: Vec::new(), history }
    }

    /// Insert or replace-by-id. Replacement keeps the note's position
    /// (notify-send -r updating a progress popup shouldn't jump the stack).
    /// Returns true if this was a replacement.
    pub fn upsert(&mut self, data: NoteData, grid: Grid) -> bool {
        if let Some(n) = self.notes.iter_mut().find(|n| n.data.id == data.id) {
            n.data = data;
            n.grid = grid;
            true
        } else {
            self.notes.insert(0, Note { data, grid, deadline_gen: 0 });
            false
        }
    }

    /// Remove by id, recording it in history. `seen` says whether the user
    /// actively closed it (click, dismiss-all) — unseen removals (expiry,
    /// DND swallow, sender close) bump the waybar badge counter.
    pub fn remove(&mut self, id: u32, seen: bool) -> bool {
        let Some(pos) = self.notes.iter().position(|n| n.data.id == id) else {
            return false;
        };
        let n = self.notes.remove(pos);
        self.remember(&n.data, seen);
        true
    }

    pub fn remember(&mut self, d: &NoteData, seen: bool) {
        let mut h = self.history.lock().unwrap();
        // Stored one-line and markup-stripped: the ring is read line-per-entry
        // (`ctl history` → fuzzel), and should match what the popup rendered.
        h.entries.push_front(HistoryEntry {
            app: d.app.clone(),
            summary: text::one_line(&d.summary),
            body: text::one_line(&d.body),
        });
        h.entries.truncate(HISTORY_CAP);
        if !seen {
            h.unseen = h.unseen.saturating_add(1);
            drop(h);
            ping_waybar();
        }
    }

    pub fn visible(&self, max: usize) -> &[Note] {
        &self.notes[..self.notes.len().min(max)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32) -> NoteData {
        NoteData {
            id,
            app: "t".into(),
            summary: "s".into(),
            body: String::new(),
            urgency: Urgency::Normal,
            timeout_ms: -1,
        }
    }

    #[test]
    fn upsert_replaces_in_place() {
        let cfg = Cfg::default();
        let mut q = Queue::new(new_history());
        for id in [1, 2, 3] {
            let d = note(id);
            let g = compose_note(&cfg, &d);
            q.upsert(d, g);
        }
        // newest first: 3 2 1 — replacing 2 must not move it.
        let mut d = note(2);
        d.summary = "updated".into();
        let g = compose_note(&cfg, &d);
        assert!(q.upsert(d, g));
        let ids: Vec<u32> = q.notes.iter().map(|n| n.data.id).collect();
        assert_eq!(ids, vec![3, 2, 1]);
        assert_eq!(q.notes[1].data.summary, "updated");
    }

    #[test]
    fn remove_records_history() {
        let cfg = Cfg::default();
        let mut q = Queue::new(new_history());
        let d = note(7);
        let g = compose_note(&cfg, &d);
        q.upsert(d, g);
        assert!(q.remove(7, true));
        assert!(!q.remove(7, true));
        assert_eq!(q.history.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn history_is_one_line_and_counts_unseen() {
        let cfg = Cfg::default();
        let mut q = Queue::new(new_history());
        let mut d = note(1);
        d.summary = "From: bob\nSubject: hi".into();
        d.body = "<b>line1</b>\nline2".into();
        let g = compose_note(&cfg, &d);
        q.upsert(d, g);
        q.remove(1, false); // expired off-screen: unseen
        {
            let h = q.history.lock().unwrap();
            assert_eq!(h.entries[0].summary, "From: bob Subject: hi");
            assert_eq!(h.entries[0].body, "line1 line2");
            assert_eq!(h.unseen, 1);
        }
        let d = note(2);
        let g = compose_note(&cfg, &d);
        q.upsert(d, g);
        q.remove(2, true); // click-dismissed: seen, badge unchanged
        let h = q.history.lock().unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.unseen, 1);
    }

    #[test]
    fn timeouts() {
        let cfg = Cfg::default(); // default 3000, critical 0
        let mut d = note(1);
        assert_eq!(effective_timeout(&cfg, &d), Some(3000));
        d.timeout_ms = 2000;
        assert_eq!(effective_timeout(&cfg, &d), Some(2000));
        d.timeout_ms = 0;
        assert_eq!(effective_timeout(&cfg, &d), None);
        d.urgency = Urgency::Critical;
        d.timeout_ms = 2000; // critical ignores client timeout by default
        assert_eq!(effective_timeout(&cfg, &d), None);
    }
}
