// SPDX-License-Identifier: GPL-3.0-or-later
// The UI-agnostic editing model shared by the TUI (tui.rs) and the GUI
// (gui/). It owns the working theme, the category/field cursor, the inline
// edit buffer, and every mutation — including live preview and commit. The two
// front-ends only add rendering and key/pointer mapping on top.

use crate::paths::Paths;
use crate::theme::Theme;
use crate::{gen, live, presets};

pub const CATS: &[&str] = &[
    "Presets", "Palette", "Lifewall", "Lifenote", "Lifelock", "Lifegreet", "Idle", "Animations",
    "Cursor", "Font",
];
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
    pub status: String,
    pub dirty: bool,
    pub quit: bool,
}

/// Field labels for a category (order matters — indexes into the match arms).
pub fn field_labels(cat: usize) -> Vec<&'static str> {
    match CATS[cat] {
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
        ],
        "Lifenote" => vec!["border_style", "critical_border_style", "opacity", "position"],
        "Lifelock" | "Lifegreet" => SCREEN_FIELDS.to_vec(),
        "Idle" => vec!["lock_minutes", "screen_off_minutes"],
        "Animations" => vec!["slowdown"],
        "Cursor" => vec!["theme", "size"],
        "Font" => vec!["family", "size"],
        _ => vec![],
    }
}

pub fn kind(cat: usize, field: usize) -> Kind {
    match (CATS[cat], field) {
        ("Presets", _) => Kind::Preset,
        ("Palette", _) => Kind::Hex,
        ("Lifewall", 0) => Kind::Float(0.05),
        ("Lifewall", 1) => Kind::Int(5),
        ("Lifewall", 2) => Kind::Float(0.5),
        ("Lifewall", 3) => Kind::Float(0.02),
        ("Lifewall", 4) => Kind::Enum(GLYPH_MODES),
        ("Lifewall", 5) => Kind::Text, // char / character set
        ("Lifewall", 8) => Kind::Float(10.0), // glider_interval
        ("Lifewall", _) => Kind::Hex, // mature, newborn
        ("Lifenote", 0) | ("Lifenote", 1) => Kind::Enum(STYLES),
        ("Lifenote", 2) => Kind::Float(0.05),
        ("Lifenote", _) => Kind::Enum(ANCHORS),
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
            ("Lifenote", 0) => t.lifenote.border_style.clone(),
            ("Lifenote", 1) => t.lifenote.critical_border_style.clone(),
            ("Lifenote", 2) => fmtf(t.lifenote.opacity),
            ("Lifenote", 3) => t.lifenote.position.clone(),
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
            ("Lifewall", 0) => t.lifewall.tick = s.parse().unwrap_or(t.lifewall.tick).max(0.05),
            ("Lifewall", 1) => t.lifewall.fps = s.parse().unwrap_or(t.lifewall.fps).clamp(1, 240),
            ("Lifewall", 2) => t.lifewall.fade = s.parse().unwrap_or(t.lifewall.fade).max(0.25),
            ("Lifewall", 3) => {
                t.lifewall.density = s.parse().unwrap_or(t.lifewall.density).clamp(0.01, 1.0)
            }
            ("Lifenote", 2) => {
                t.lifenote.opacity = s.parse().unwrap_or(t.lifenote.opacity).clamp(0.0, 1.0)
            }
            ("Idle", 0) => t.idle.lock_minutes = s.parse().unwrap_or(t.idle.lock_minutes).max(1),
            ("Idle", 1) => {
                t.idle.screen_off_minutes = s.parse().unwrap_or(t.idle.screen_off_minutes).max(1)
            }
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
            ("Cursor", 0) => t.cursor.theme = s.to_string(),
            ("Cursor", 1) => t.cursor.size = s.parse().unwrap_or(t.cursor.size).clamp(8, 128),
            ("Font", 0) => t.font.family = s.to_string(),
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
            ("Lifenote", 0) => self.theme.lifenote.border_style = v.into(),
            ("Lifenote", 1) => self.theme.lifenote.critical_border_style = v.into(),
            ("Lifenote", 3) => self.theme.lifenote.position = v.into(),
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
        self.editing = Some(self.value(self.cat, self.field));
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

    pub fn move_down(&mut self) {
        match self.focus {
            Focus::Cats => {
                self.cat = (self.cat + 1) % CATS.len();
                self.field = 0;
            }
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
            Focus::Cats => {
                self.cat = (self.cat + CATS.len() - 1) % CATS.len();
                self.field = 0;
            }
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
