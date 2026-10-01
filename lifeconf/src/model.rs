// SPDX-License-Identifier: GPL-3.0-or-later
// The UI-agnostic editing model shared by the TUI (tui.rs) and the GUI
// (gui/). It owns the working theme, the category/field cursor, the inline
// edit buffer, and every mutation — including live preview and commit. The two
// front-ends only add rendering and key/pointer mapping on top.

use crate::paths::Paths;
use crate::theme::Theme;
use crate::{gen, live, presets, sys};
use std::time::{Duration, Instant};

/// How long a risky system change waits to be confirmed before it is undone.
pub const KEEP_SECS: u64 = 60;

/// A system change applied provisionally: kept only if the user says so in time.
pub struct Pending {
    pub undo: Vec<Vec<String>>,
    pub deadline: Instant,
    pub what: String,
}

pub const CATS: &[&str] = &[
    "Presets", "Palette", "Lifewall", "Notifications", "Lifelock", "Lifegreet", "Idle", "Animations",
    "Cursor", "Font", "Display", "Network", "VPN", "Bluetooth", "Sound", "Keyboard", "Touchpad", "Mouse", "Power", "Night light", "Date & Time", "Apps",
    "Autostart", "About",
];
/// Categories past the theme ones are system panels (sys/): they act on the
/// running system the moment they change, with nothing to Save.
pub fn is_system(cat: usize) -> bool {
    sys::is_panel(CATS[cat])
}
/// The five colour labels shared by the Lifelock/Lifegreet screens (after `link`).
const SCREEN_FIELDS: &[&str] = &["link", "mature", "newborn", "accent", "urgent", "text"];
pub const STYLES: &[&str] = &["single", "rounded", "heavy", "double", "ascii"];
pub const ANCHORS: &[&str] = &["top-right", "top-left", "bottom-right", "bottom-left"];
/// "ascii" (the default) draws random printable ASCII per cell and "kana"
/// random half-width katakana, both ignoring `char`; "custom" uses the `char`
/// field verbatim (see cmd::lifewall_shell_cmd).
pub const GLYPH_MODES: &[&str] = &["ascii", "custom", "kana"];

/// What the current field is, which decides how input drives it.
pub enum Kind {
    Preset,
    Hex,
    Enum(&'static [&'static str]),
    Float(f64), // step
    Int(u32),   // step
    Bool,
    Text,
    /// One of a list read from the system (sys panels); +/- cycles it.
    Choice,
    /// Shown, not editable.
    Info,
    /// Fires when activated; the value text says what it will do.
    Action,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Focus {
    Cats,
    Fields,
}

pub struct Model {
    pub theme: Theme,
    pub saved: Theme, // last committed, for revert-on-cancel
    pub paths: Paths,
    pub cat: usize,
    pub field: usize,
    pub focus: Focus,
    pub editing: Option<String>, // Some(buffer) while typing a value
    /// Sidebar filter: Some while the search box is open or has text.
    pub search: Option<String>,
    /// The search box has keyboard focus (typing edits the filter).
    pub searching: bool,
    /// Current values of the selected system panel, read from the system.
    pub sys_rows: Vec<sys::Row>,
    /// Set while a Display change awaits keep/revert; front ends go modal.
    pub pending: Option<Pending>,
    pub status: String,
    pub dirty: bool,
    pub quit: bool,
}

/// Field labels for a category (order matters — indexes into the match arms).
pub fn field_labels(cat: usize) -> Vec<&'static str> {
    match CATS[cat] {
        c if sys::is_panel(c) => sys::labels(c).to_vec(),
        "Presets" => vec!["preset"],
        "Palette" => vec!["bg", "surface", "border", "text", "accent", "warn", "urgent"],
        "Lifewall" => vec![
            "tick",
            "fps",
            "fade",
            "density",
            "glyph_mode",
            "char",
            "mature",
            "newborn",
            "glider_interval",
            "fps_battery",
        ],
        "Notifications" => vec![
            "border_style",
            "critical_border_style",
            "opacity",
            "position",
            "dismiss_on_click_outside",
            "popup seconds (0 = until dismissed)",
            "max popups on screen",
            "muted apps (comma-separated)",
        ],
        "Lifelock" | "Lifegreet" => SCREEN_FIELDS.to_vec(),
        "Idle" => vec!["lock_minutes", "screen_off_minutes", "suspend_minutes"],
        "Animations" => vec!["slowdown"],
        "Cursor" => vec!["theme", "size"],
        "Font" => vec!["family", "size"],
        _ => vec![],
    }
}

pub fn kind(cat: usize, field: usize) -> Kind {
    match (CATS[cat], field) {
        (c, f) if sys::is_panel(c) => match sys::row_kind(c, f) {
            sys::RowKind::Int(step) => Kind::Int(step),
            sys::RowKind::Bool => Kind::Bool,
            sys::RowKind::Choice => Kind::Choice,
            sys::RowKind::Text => Kind::Text,
            sys::RowKind::Info => Kind::Info,
            sys::RowKind::Action => Kind::Action,
        },
        ("Presets", _) => Kind::Preset,
        ("Palette", _) => Kind::Hex,
        ("Lifewall", 0) => Kind::Float(0.05),
        ("Lifewall", 1) => Kind::Int(5),
        ("Lifewall", 2) => Kind::Float(0.5),
        ("Lifewall", 3) => Kind::Float(0.02),
        ("Lifewall", 4) => Kind::Enum(GLYPH_MODES),
        ("Lifewall", 5) => Kind::Text, // char / character set
        ("Lifewall", 8) => Kind::Float(10.0), // glider_interval
        ("Lifewall", 9) => Kind::Int(1),      // fps_battery: a 5-step overshoots its range
        ("Lifewall", _) => Kind::Hex, // mature, newborn
        ("Notifications", 0) | ("Notifications", 1) => Kind::Enum(STYLES),
        ("Notifications", 2) => Kind::Float(0.05),
        ("Notifications", 4) => Kind::Bool,
        ("Notifications", 5) | ("Notifications", 6) => Kind::Int(1),
        ("Notifications", 7) => Kind::Text,
        ("Notifications", _) => Kind::Enum(ANCHORS),
        ("Lifelock", 0) | ("Lifegreet", 0) => Kind::Bool, // link
        ("Lifelock", _) | ("Lifegreet", _) => Kind::Hex,
        ("Idle", _) => Kind::Int(1),
        ("Animations", _) => Kind::Float(0.05),
        ("Cursor", 0) => Kind::Text,
        ("Cursor", _) => Kind::Int(2),
        ("Font", 0) => Kind::Text,
        ("Font", _) => Kind::Int(1),
        _ => Kind::Text,
    }
}

pub fn fmtf(v: f64) -> String {
    format!("{v}")
}

/// Accept "#rrggbb", "rrggbb", or a partial; keep the old value if unparseable.
pub fn normalize_hex(s: &str, old: &str) -> String {
    let bare = s.trim().trim_start_matches('#');
    if bare.len() == 6 && bare.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", bare.to_ascii_lowercase())
    } else {
        old.to_string()
    }
}

impl Model {
    pub fn new(paths: Paths, mut theme: Theme) -> Model {
        if theme.meta.active_preset.is_empty() {
            theme.meta.active_preset = "custom".into();
        }
        theme.sync_linked(); // linked screens display current palette-derived colours
        Model {
            saved: theme.clone(),
            theme,
            paths,
            cat: 0,
            field: 0,
            focus: Focus::Cats,
            editing: None,
            search: None,
            searching: false,
            sys_rows: Vec::new(),
            pending: None,
            status: "j/k move · Tab pane · Enter edit · +/- adjust · s save · q quit".into(),
            dirty: false,
            quit: false,
        }
    }

    pub fn n_fields(&self) -> usize {
        field_labels(self.cat).len()
    }

    pub fn kind_here(&self) -> Kind {
        kind(self.cat, self.field)
    }

    pub fn value(&self, cat: usize, field: usize) -> String {
        let t = &self.theme;
        match (CATS[cat], field) {
            (c, f) if sys::is_panel(c) => {
                self.sys_rows.get(f).map(|r| r.value.clone()).unwrap_or_default()
            }
            ("Presets", _) => t.meta.active_preset.clone(),
            ("Palette", 0) => t.palette.bg.clone(),
            ("Palette", 1) => t.palette.surface.clone(),
            ("Palette", 2) => t.palette.border.clone(),
            ("Palette", 3) => t.palette.text.clone(),
            ("Palette", 4) => t.palette.accent.clone(),
            ("Palette", 5) => t.palette.warn.clone(),
            ("Palette", 6) => t.palette.urgent.clone(),
            ("Lifewall", 0) => fmtf(t.lifewall.tick),
            ("Lifewall", 1) => t.lifewall.fps.to_string(),
            ("Lifewall", 2) => fmtf(t.lifewall.fade),
            ("Lifewall", 3) => fmtf(t.lifewall.density),
            ("Lifewall", 4) => t.lifewall.glyph_mode.clone(),
            ("Lifewall", 5) => t.lifewall.char.clone(),
            ("Lifewall", 6) => t.lifewall.mature.clone(),
            ("Lifewall", 7) => t.lifewall.newborn.clone(),
            ("Lifewall", 8) => fmtf(t.lifewall.glider_interval),
            ("Lifewall", 9) => t.lifewall.fps_battery.to_string(),
            ("Notifications", 0) => t.lifenote.border_style.clone(),
            ("Notifications", 1) => t.lifenote.critical_border_style.clone(),
            ("Notifications", 2) => fmtf(t.lifenote.opacity),
            ("Notifications", 3) => t.lifenote.position.clone(),
            ("Notifications", 4) => t.lifenote.dismiss_on_click_outside.to_string(),
            ("Notifications", 5) => t.lifenote.timeout_seconds.to_string(),
            ("Notifications", 6) => t.lifenote.max_visible.to_string(),
            ("Notifications", 7) => t.lifenote.muted_apps.clone(),
            ("Lifelock", 0) => t.lifelock.link.to_string(),
            ("Lifelock", 1) => t.lifelock.mature.clone(),
            ("Lifelock", 2) => t.lifelock.newborn.clone(),
            ("Lifelock", 3) => t.lifelock.accent.clone(),
            ("Lifelock", 4) => t.lifelock.urgent.clone(),
            ("Lifelock", 5) => t.lifelock.text.clone(),
            ("Lifegreet", 0) => t.lifegreet.link.to_string(),
            ("Lifegreet", 1) => t.lifegreet.mature.clone(),
            ("Lifegreet", 2) => t.lifegreet.newborn.clone(),
            ("Lifegreet", 3) => t.lifegreet.accent.clone(),
            ("Lifegreet", 4) => t.lifegreet.urgent.clone(),
            ("Lifegreet", 5) => t.lifegreet.text.clone(),
            ("Idle", 0) => t.idle.lock_minutes.to_string(),
            ("Idle", 1) => t.idle.screen_off_minutes.to_string(),
            ("Idle", 2) => t.idle.suspend_minutes.to_string(),
            ("Animations", 0) => fmtf(t.animations.slowdown),
            ("Cursor", 0) => t.cursor.theme.clone(),
            ("Cursor", 1) => t.cursor.size.to_string(),
            ("Font", 0) => t.font.family.clone(),
            ("Font", 1) => t.font.size.to_string(),
            _ => String::new(),
        }
    }

    /// Commit a typed string to the current field (hex/text/number).
    pub fn set_text(&mut self, s: &str) {
        if sys::is_panel(CATS[self.cat]) {
            return self.sys_change(sys::Change::Text(s.to_string()));
        }
        let s = s.trim();
        let t = &mut self.theme;
        let mut touched_palette = false;
        match (CATS[self.cat], self.field) {
            ("Palette", i) => {
                let slot = match i {
                    0 => &mut t.palette.bg,
                    1 => &mut t.palette.surface,
                    2 => &mut t.palette.border,
                    3 => &mut t.palette.text,
                    4 => &mut t.palette.accent,
                    5 => &mut t.palette.warn,
                    _ => &mut t.palette.urgent,
                };
                *slot = normalize_hex(s, &slot.clone());
                touched_palette = true;
            }
            // A single glyph or a whole character set to pick from at random
            // per cell; cmd::lifewall_shell_cmd sanitizes it for the shell/KDL
            // layers it's embedded in, so no filtering is needed here.
            ("Lifewall", 5) => {
                if !s.is_empty() {
                    t.lifewall.char = s.to_string();
                }
            }
            ("Lifewall", 6) => {
                t.lifewall.mature = normalize_hex(s, &t.lifewall.mature.clone());
                touched_palette = true;
            }
            ("Lifewall", 7) => {
                t.lifewall.newborn = normalize_hex(s, &t.lifewall.newborn.clone());
                touched_palette = true;
            }
            // Mean seconds between glider clusters; 0 disables them.
            ("Lifewall", 8) => {
                t.lifewall.glider_interval =
                    s.parse().unwrap_or(t.lifewall.glider_interval).max(0.0)
            }
            // 0 means "same as fps on battery too".
            ("Lifewall", 9) => {
                t.lifewall.fps_battery = s.parse().unwrap_or(t.lifewall.fps_battery).clamp(0, 240)
            }
            ("Lifewall", 0) => t.lifewall.tick = s.parse().unwrap_or(t.lifewall.tick).max(0.05),
            ("Lifewall", 1) => t.lifewall.fps = s.parse().unwrap_or(t.lifewall.fps).clamp(1, 240),
            ("Lifewall", 2) => t.lifewall.fade = s.parse().unwrap_or(t.lifewall.fade).max(0.25),
            ("Lifewall", 3) => {
                t.lifewall.density = s.parse().unwrap_or(t.lifewall.density).clamp(0.01, 1.0)
            }
            ("Notifications", 2) => {
                t.lifenote.opacity = s.parse().unwrap_or(t.lifenote.opacity).clamp(0.0, 1.0)
            }
            ("Notifications", 5) => {
                t.lifenote.timeout_seconds = s.parse().unwrap_or(t.lifenote.timeout_seconds).min(600)
            }
            ("Notifications", 6) => {
                t.lifenote.max_visible = s.parse().unwrap_or(t.lifenote.max_visible).clamp(1, 20)
            }
            // Names as notify-send's -a / the app sends them; tidied to "a, b".
            ("Notifications", 7) => {
                t.lifenote.muted_apps = s
                    .split(',')
                    .map(|a| a.trim().chars().filter(|c| !c.is_control() && *c != '=').collect::<String>())
                    .filter(|a| !a.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            ("Idle", 0) => t.idle.lock_minutes = s.parse().unwrap_or(t.idle.lock_minutes).max(1),
            ("Idle", 1) => {
                t.idle.screen_off_minutes = s.parse().unwrap_or(t.idle.screen_off_minutes).max(1)
            }
            // 0 means "never suspend" and is a real setting, so no .max(1) here.
            ("Idle", 2) => t.idle.suspend_minutes = s.parse().unwrap_or(t.idle.suspend_minutes),
            ("Animations", 0) => {
                t.animations.slowdown = s.parse().unwrap_or(t.animations.slowdown).clamp(0.0, 5.0)
            }
            // Editing any lock/login-screen colour unlinks it from the palette so
            // sync_linked stops overwriting it; the other four keep their values.
            ("Lifelock", i) => {
                t.lifelock.link = false;
                let slot = match i {
                    1 => &mut t.lifelock.mature,
                    2 => &mut t.lifelock.newborn,
                    3 => &mut t.lifelock.accent,
                    4 => &mut t.lifelock.urgent,
                    _ => &mut t.lifelock.text,
                };
                *slot = normalize_hex(s, &slot.clone());
            }
            ("Lifegreet", i) => {
                t.lifegreet.link = false;
                let slot = match i {
                    1 => &mut t.lifegreet.mature,
                    2 => &mut t.lifegreet.newborn,
                    3 => &mut t.lifegreet.accent,
                    4 => &mut t.lifegreet.urgent,
                    _ => &mut t.lifegreet.text,
                };
                *slot = normalize_hex(s, &slot.clone());
            }
            ("Cursor", 0) => t.cursor.theme = crate::theme::plain_name(s),
            ("Cursor", 1) => t.cursor.size = s.parse().unwrap_or(t.cursor.size).clamp(8, 128),
            ("Font", 0) => t.font.family = crate::theme::plain_name(s),
            ("Font", 1) => t.font.size = s.parse().unwrap_or(t.font.size).clamp(6, 48),
            _ => {}
        }
        if touched_palette {
            self.theme.meta.active_preset = "custom".into();
        }
        self.dirty = true;
        self.preview();
    }

    /// Cycle/step the current field by `dir` (+1 / -1).
    pub fn nudge(&mut self, dir: i32) {
        if sys::is_panel(CATS[self.cat]) {
            let ch = match self.kind_here() {
                Kind::Bool => sys::Change::Toggle,
                // Only a forward press fires an action; Left/- must not.
                Kind::Action if dir > 0 => sys::Change::Toggle,
                Kind::Action | Kind::Info | Kind::Text => return,
                _ => sys::Change::Step(dir),
            };
            return self.sys_change(ch);
        }
        match self.kind_here() {
            Kind::Preset => {
                let names = presets::NAMES;
                let cur = names.iter().position(|n| *n == self.theme.meta.active_preset);
                let next = match cur {
                    Some(i) => (i as i32 + dir).rem_euclid(names.len() as i32) as usize,
                    None => 0,
                };
                self.theme.apply_preset(names[next]);
            }
            Kind::Enum(opts) => {
                let cur = self.value(self.cat, self.field);
                let i = opts.iter().position(|o| *o == cur).unwrap_or(0);
                let next = (i as i32 + dir).rem_euclid(opts.len() as i32) as usize;
                self.set_enum(opts[next]);
                return;
            }
            Kind::Bool => match (CATS[self.cat], self.field) {
                ("Notifications", 4) => {
                    let n = &mut self.theme.lifenote;
                    n.dismiss_on_click_outside = !n.dismiss_on_click_outside;
                }
                ("Lifelock", 0) => self.theme.lifelock.link = !self.theme.lifelock.link,
                ("Lifegreet", 0) => self.theme.lifegreet.link = !self.theme.lifegreet.link,
                _ => {}
            },
            Kind::Float(step) => return self.step_num(dir as f64 * step),
            Kind::Int(step) => return self.step_num(dir as f64 * step as f64),
            _ => return,
        }
        self.dirty = true;
        self.preview();
    }

    fn set_enum(&mut self, v: &str) {
        match (CATS[self.cat], self.field) {
            ("Notifications", 0) => self.theme.lifenote.border_style = v.into(),
            ("Notifications", 1) => self.theme.lifenote.critical_border_style = v.into(),
            ("Notifications", 3) => self.theme.lifenote.position = v.into(),
            ("Lifewall", 4) => self.theme.lifewall.glyph_mode = v.into(),
            _ => {}
        }
        self.dirty = true;
        self.preview();
    }

    fn step_num(&mut self, delta: f64) {
        let cur = self.value(self.cat, self.field);
        let base: f64 = cur.parse().unwrap_or(0.0);
        let next = ((base + delta) * 100.0).round() / 100.0;
        self.set_text(&fmtf(next));
    }

    /// Begin editing the current field: seed the buffer with its value.
    pub fn begin_edit(&mut self) {
        let mut v = self.value(self.cat, self.field);
        // A "(none)"/"(system default)" placeholder isn't something to edit.
        if is_system(self.cat) && v.starts_with('(') && v.ends_with(')') {
            v.clear();
        }
        self.editing = Some(v);
        self.status = "type a value · Enter commit · Esc cancel".into();
    }

    /// Regenerate the fast (file-only) consumers and push colours live.
    pub fn preview(&mut self) {
        self.theme.sync_linked();
        let mut r = gen::Report::default();
        gen::generate_theme_files(&self.theme, &self.paths, &mut r);
        live::apply_all(&self.theme, &self.paths, &mut r, false);
    }

    /// Full commit: regenerate everything, respawn the disruptive bits, apply the
    /// greeter (pkexec, only if it changed), and save.
    pub fn commit(&mut self) {
        self.theme.sync_linked();
        let mut r = gen::generate_all(&self.theme, &self.paths);
        live::apply_all(&self.theme, &self.paths, &mut r, true);
        live::greeter::apply_if_changed(&self.paths, &mut r);
        match self.theme.save(&self.paths.theme_toml()) {
            Ok(()) => {
                self.saved = self.theme.clone();
                self.dirty = false;
                // Surface the greeter outcome if there was one, else the generic ok.
                self.status = r
                    .notes
                    .iter()
                    .find(|n| n.starts_with("greeter"))
                    .cloned()
                    .unwrap_or_else(|| "saved + applied".into());
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    /// Revert configs to the last committed theme (cancel path).
    pub fn revert(&mut self) {
        self.theme = self.saved.clone();
        self.preview();
    }

    // --- navigation shared by both front-ends ---

    /// Select a category, (re)reading the system's current values when it is a
    /// system panel — they can change behind our back (volume keys, hotplug).
    pub fn enter_cat(&mut self, i: usize) {
        self.cat = i;
        self.field = 0;
        self.refresh_sys();
    }

    /// Jump to a category by name (case-insensitive), focusing its fields.
    pub fn open_panel(&mut self, name: &str) -> bool {
        match CATS.iter().position(|c| c.eq_ignore_ascii_case(name)) {
            Some(i) => {
                self.enter_cat(i);
                self.focus = Focus::Fields;
                true
            }
            None => false,
        }
    }

    pub fn refresh_sys(&mut self) {
        let name = CATS[self.cat];
        self.sys_rows =
            if sys::is_panel(name) { sys::load(name, &sys::run_real) } else { Vec::new() };
    }

    fn sys_change(&mut self, ch: sys::Change) {
        if self.pending.is_some() {
            self.status = "answer the keep/revert prompt first".into();
            return;
        }
        let name = CATS[self.cat];
        // The undo must be worked out while the rows still show the old state.
        let undo = sys::undo_for(name, self.field, &self.sys_rows, &ch);
        match sys::apply(name, self.field, &self.sys_rows, ch, &sys::run_real) {
            Ok(msg) => {
                if let Some(undo) = undo {
                    let deadline = Instant::now() + Duration::from_secs(KEEP_SECS);
                    self.pending = Some(Pending { undo, deadline, what: msg.clone() });
                }
                self.status = msg;
            }
            Err(e) => self.status = e,
        }
        self.refresh_sys();
    }

    /// Whole seconds left to answer, rounded up (None when nothing is pending).
    pub fn pending_secs(&self, now: Instant) -> Option<u64> {
        self.pending.as_ref().map(|p| p.deadline.saturating_duration_since(now).as_secs_f64().ceil() as u64)
    }

    pub fn keep_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.status = format!("kept: {}", p.what);
        }
    }

    pub fn revert_pending(&mut self, why: &str) {
        self.revert_with(why, &sys::run_real);
    }

    fn revert_with(&mut self, why: &str, run: sys::Runner) {
        let Some(p) = self.pending.take() else { return };
        self.status = match sys::run_undo(&p.undo, run) {
            Ok(()) => format!("{why} — reverted"),
            Err(e) => format!("{why} — revert failed ({e}); a niri config reload will restore it"),
        };
        self.refresh_sys();
    }

    /// Call periodically; reverts once the deadline passes. True if it did.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.tick_with(now, &sys::run_real)
    }

    fn tick_with(&mut self, now: Instant, run: sys::Runner) -> bool {
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            self.revert_with(&format!("no answer in {KEEP_SECS}s"), run);
            return true;
        }
        false
    }

    /// Categories that match the sidebar search (all of them when it's empty):
    /// a hit on the category name or on any of its field labels.
    pub fn visible_cats(&self) -> Vec<usize> {
        let q = self.search.as_deref().unwrap_or("").trim().to_lowercase();
        (0..CATS.len())
            .filter(|&c| {
                q.is_empty()
                    || CATS[c].to_lowercase().contains(&q)
                    || field_labels(c).iter().any(|l| l.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Re-point the selection at a visible category after the filter changed.
    pub fn clamp_to_search(&mut self) {
        let vis = self.visible_cats();
        if !vis.is_empty() && !vis.contains(&self.cat) {
            self.enter_cat(vis[0]);
        }
    }

    fn step_cat(&mut self, delta: isize) {
        let vis = self.visible_cats();
        if vis.is_empty() {
            return;
        }
        let at = vis.iter().position(|&c| c == self.cat).unwrap_or(0) as isize;
        let next = vis[(at + delta).rem_euclid(vis.len() as isize) as usize];
        self.enter_cat(next);
    }

    pub fn move_down(&mut self) {
        match self.focus {
            Focus::Cats => self.step_cat(1),
            Focus::Fields => {
                let n = self.n_fields();
                if n > 0 {
                    self.field = (self.field + 1) % n;
                }
            }
        }
    }

    pub fn move_up(&mut self) {
        match self.focus {
            Focus::Cats => self.step_cat(-1),
            Focus::Fields => {
                let n = self.n_fields();
                if n > 0 {
                    self.field = (self.field + n - 1) % n;
                }
            }
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = if self.focus == Focus::Cats { Focus::Fields } else { Focus::Cats };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        // Nothing here writes: only navigation/search, never preview/commit.
        Model::new(Paths { home: "/nonexistent-lifeconf-test".into() }, Theme::default())
    }

    fn name(c: usize) -> &'static str {
        CATS[c]
    }

    #[test]
    fn empty_search_shows_everything() {
        assert_eq!(model().visible_cats().len(), CATS.len());
    }

    #[test]
    fn search_matches_category_names_and_field_labels() {
        let mut m = model();
        m.search = Some("volume".into()); // a field label inside Sound
        let hits: Vec<_> = m.visible_cats().into_iter().map(name).collect();
        assert_eq!(hits, ["Sound"]);
        m.search = Some("NOTIF".into()); // case-insensitive category name
        let hits: Vec<_> = m.visible_cats().into_iter().map(name).collect();
        assert_eq!(hits, ["Notifications"]);
        m.search = Some("click_outside".into()); // field label with an underscore
        assert!(m.visible_cats().into_iter().map(name).any(|n| n == "Notifications"));
        m.search = Some("zzzz".into());
        assert!(m.visible_cats().is_empty());
    }

    #[test]
    fn clamp_moves_selection_onto_a_hit_and_stepping_stays_in_hits() {
        let mut m = model();
        m.search = Some("cursor".into());
        m.clamp_to_search();
        assert_eq!(name(m.cat), "Cursor");
        m.move_down(); // one hit: wraps onto itself rather than escaping the filter
        assert_eq!(name(m.cat), "Cursor");
    }

    fn pending(secs: u64) -> Pending {
        Pending {
            undo: vec![vec!["msg".into(), "output".into(), "X".into(), "scale".into(), "1".into()]],
            deadline: Instant::now() + Duration::from_secs(secs),
            what: "X scale 2".into(),
        }
    }

    #[test]
    fn silence_reverts_and_an_answer_keeps() {
        let mut m = model();
        m.pending = Some(pending(60));
        assert!(!m.tick(Instant::now()), "not yet");
        assert!(m.pending.is_some());
        m.keep_pending();
        assert!(m.pending.is_none());
        assert!(m.status.starts_with("kept"));

        let log = std::cell::RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        m.pending = Some(pending(60));
        m.revert_with("no answer", &rec);
        assert_eq!(*log.borrow(), ["niri msg output X scale 1"]);
        assert!(m.pending.is_none() && m.status.contains("reverted"));
    }

    #[test]
    fn countdown_rounds_up_and_expiry_fires_tick() {
        let mut m = model();
        assert_eq!(m.pending_secs(Instant::now()), None);
        m.pending = Some(pending(60));
        let t0 = Instant::now();
        assert_eq!(m.pending_secs(t0), Some(60));
        assert_eq!(m.pending_secs(t0 + Duration::from_millis(500)), Some(60));
        assert_eq!(m.pending_secs(t0 + Duration::from_secs(59)), Some(1));
        assert_eq!(m.pending_secs(t0 + Duration::from_secs(61)), Some(0));
        // Expired: the tick reverts, and a failing revert must still clear the
        // prompt (and say how to recover) rather than leave it stuck.
        let fail = |_: &str, _: &[&str]| -> Result<String, String> { Err("niri: gone".into()) };
        assert!(m.tick_with(t0 + Duration::from_secs(61), &fail));
        assert!(m.pending.is_none());
        assert!(m.status.contains("revert failed") && m.status.contains("config reload"));
    }

    #[test]
    fn new_changes_are_refused_while_a_prompt_is_open() {
        let mut m = model();
        m.pending = Some(pending(60));
        m.enter_cat(CATS.iter().position(|c| *c == "Display").unwrap());
        m.sys_change(sys::Change::Step(1));
        assert!(m.status.contains("prompt"));
        assert!(m.pending.is_some());
    }

    #[test]
    fn system_categories_are_flagged() {
        let i = CATS.iter().position(|c| *c == "Sound").unwrap();
        assert!(is_system(i));
        assert!(!is_system(0));
        assert_eq!(field_labels(i).len(), 6);
        assert!(matches!(kind(i, 0), Kind::Choice));
        assert!(matches!(kind(i, 2), Kind::Bool));
    }
}
