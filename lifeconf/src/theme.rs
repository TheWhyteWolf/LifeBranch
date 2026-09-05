// SPDX-License-Identifier: GPL-3.0-or-later
// The canonical theme model. One TOML file at ~/.config/lifeconf/theme.toml
// is the single source of truth; every generator (src/gen/*) reads a &Theme
// and rewrites one consumer's own native config format. Only lifeconf itself
// ever reads/writes theme.toml — no other app learns TOML.
//
// Defaults here mirror the current hand-tuned olive rice exactly, so a fresh
// `lifeconf --apply` on a stock checkout is a no-visual-diff refactor.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub meta: Meta,
    pub palette: Palette,
    pub lifewall: Lifewall,
    pub lifenote: Lifenote,
    pub idle: Idle,
    pub animations: Animations,
    pub cursor: Cursor,
    pub font: Font,
    // The lock/login screens. #[serde(default)] so older theme.toml files (which
    // predate these sections) still load; `link` keeps them mirroring the palette.
    #[serde(default)]
    pub lifelock: Screen,
    #[serde(default)]
    pub lifegreet: Screen,
}

/// A Game-of-Life screen's five colours (lifelock / lifegreet). When `link` is
/// true these track the palette + wallpaper tints; when false they're the user's
/// own, edited independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screen {
    pub link: bool,
    pub mature: String,
    pub newborn: String,
    pub accent: String,
    pub urgent: String,
    pub text: String,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            link: true,
            mature: "#66744c".into(),
            newborn: "#87a540".into(),
            accent: "#a4c94b".into(),
            urgent: "#8a3b2e".into(),
            text: "#7b8c5a".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: u32,
    /// olive | slate | moss | custom. Selecting a preset overwrites [palette]
    /// (and lifewall mature/newborn); "custom" means the user hand-tuned it.
    pub active_preset: String,
}

/// The seven role colours, each a "#rrggbb" string. Everything visual in the
/// rice ultimately references one of these (plus the two lifewall tints).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    pub bg: String,
    pub surface: String,
    pub border: String,
    pub text: String,
    pub accent: String,
    pub warn: String,
    pub urgent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lifewall {
    pub tick: f64,
    pub fps: u32,
    pub fade: f64,
    pub density: f64,
    /// "custom" (use `char` verbatim) or "kana" (random half-width katakana
    /// per cell). #[serde(default)] so older theme.toml files without this
    /// field still load.
    #[serde(default = "default_glyph_mode")]
    pub glyph_mode: String,
    pub char: String,
    pub mature: String,
    pub newborn: String,
    /// Mean seconds between anti-stagnation glider clusters; 0 disables.
    /// #[serde(default)] so older theme.toml files without this field still
    /// load.
    #[serde(default = "default_glider_interval")]
    pub glider_interval: f64,
}

fn default_glyph_mode() -> String {
    "custom".into()
}

fn default_glider_interval() -> f64 {
    90.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lifenote {
    pub border_style: String,
    pub critical_border_style: String,
    pub opacity: f64,
    pub position: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Idle {
    pub lock_minutes: u32,
    pub screen_off_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Animations {
    pub slowdown: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub theme: String,
    pub size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Font {
    pub family: String,
    pub size: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            meta: Meta { schema_version: 1, active_preset: "olive".into() },
            palette: Palette {
                bg: "#121412".into(),
                surface: "#171a14".into(),
                border: "#39412b".into(),
                text: "#7b8c5a".into(),
                accent: "#a4c94b".into(),
                warn: "#c7d17a".into(),
                urgent: "#8a3b2e".into(),
            },
            lifewall: Lifewall {
                tick: 0.3,
                fps: 30,
                fade: 3.0,
                density: 0.14,
                glyph_mode: "custom".into(),
                char: "#".into(),
                mature: "#66744c".into(),
                newborn: "#87a540".into(),
                glider_interval: 90.0,
            },
            lifenote: Lifenote {
                border_style: "single".into(),
                critical_border_style: "double".into(),
                opacity: 0.85,
                position: "top-right".into(),
            },
            idle: Idle { lock_minutes: 10, screen_off_minutes: 15 },
            animations: Animations { slowdown: 0.6 },
            cursor: Cursor { theme: "phinger-cursors-light".into(), size: 24 },
            font: Font { family: "ShureTechMono Nerd Font".into(), size: 12 },
            lifelock: Screen::default(),
            lifegreet: Screen::default(),
        }
    }
}

impl Theme {
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, body)
    }

    /// Keep each linked screen's colours mirroring the palette + wallpaper tints.
    /// Unlinked screens are left exactly as the user set them. Call before every
    /// generate so the written lifelock/lifegreet configs are current.
    pub fn sync_linked(&mut self) {
        let (mat, new, acc, urg, txt) = (
            self.lifewall.mature.clone(),
            self.lifewall.newborn.clone(),
            self.palette.accent.clone(),
            self.palette.urgent.clone(),
            self.palette.text.clone(),
        );
        for s in [&mut self.lifelock, &mut self.lifegreet] {
            if s.link {
                s.mature = mat.clone();
                s.newborn = new.clone();
                s.accent = acc.clone();
                s.urgent = urg.clone();
                s.text = txt.clone();
            }
        }
    }

    /// Overlay a named preset's palette (and lifewall tints) onto the current
    /// theme, leaving every non-palette setting untouched. Unknown name = no-op
    /// returning false.
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some(p) = crate::presets::preset(name) else {
            return false;
        };
        self.palette = p.palette;
        self.lifewall.mature = p.lifewall.mature;
        self.lifewall.newborn = p.lifewall.newborn;
        self.meta.active_preset = name.to_string();
        self.sync_linked(); // linked screens follow the new palette
        true
    }
}

// --- hex helpers -------------------------------------------------------------

/// "#rrggbb" or "rrggbb" -> "rrggbb" (lowercase, no hash). Falls back to the
/// input trimmed of '#' if it isn't a clean 6-hex string, so a malformed value
/// degrades visibly rather than silently.
pub fn bare(hex: &str) -> String {
    hex.trim().trim_start_matches('#').to_ascii_lowercase()
}

/// "rrggbb" or "#rrggbb" -> "#rrggbb".
pub fn hash(hex: &str) -> String {
    format!("#{}", bare(hex))
}

/// Parse "#rrggbb" into (r,g,b); None if malformed.
pub fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let s = bare(hex);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(&s, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
