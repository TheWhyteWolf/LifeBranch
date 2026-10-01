// SPDX-License-Identifier: GPL-3.0-or-later
// Colours come from ~/.config/lifefiles/theme (key=#hex), which lifeconf
// generates from the palette — the same "other apps never parse theme.toml"
// arrangement lifenote uses. Olive defaults apply if the file is missing.

use ratatui::style::Color;

#[derive(Clone, Copy)]
pub struct Colors {
    pub surface: Color,
    pub border: Color,
    pub text: Color,
    pub accent: Color,
    pub warn: Color,
    pub urgent: Color,
}

fn hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            surface: Color::Rgb(0x17, 0x1a, 0x14),
            border: Color::Rgb(0x39, 0x41, 0x2b),
            text: Color::Rgb(0x7b, 0x8c, 0x5a),
            accent: Color::Rgb(0xa4, 0xc9, 0x4b),
            warn: Color::Rgb(0xc7, 0xd1, 0x7a),
            urgent: Color::Rgb(0x8a, 0x3b, 0x2e),
        }
    }
}

impl Colors {
    pub fn parse(body: &str) -> Colors {
        let mut c = Colors::default();
        for line in body.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let Some(col) = hex(v.trim()) else { continue };
            match k.trim() {
                "surface" => c.surface = col,
                "border" => c.border = col,
                "text" => c.text = col,
                "accent" => c.accent = col,
                "warn" => c.warn = col,
                "urgent" => c.urgent = col,
                _ => {}
            }
        }
        c
    }

    pub fn load() -> Colors {
        let home = std::env::var("HOME").unwrap_or_default();
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{home}/.config"));
        std::fs::read_to_string(format!("{base}/lifefiles/theme"))
            .map(|b| Colors::parse(&b))
            .unwrap_or_default()
    }
}

impl Colors {
    /// Loaded once per process; the file only changes via lifeconf, which
    /// restarts nothing, so a relaunch picks up a new theme.
    pub fn load_cached() -> Colors {
        use std::sync::OnceLock;
        static C: OnceLock<Colors> = OnceLock::new();
        *C.get_or_init(Colors::load)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_keys_and_ignores_junk() {
        let c = Colors::parse("accent=#ff0000\nbogus=#00ff00\ntext=nothex\n# c\n");
        assert_eq!(c.accent, Color::Rgb(255, 0, 0));
        assert_eq!(c.text, Colors::default().text);
    }
}
