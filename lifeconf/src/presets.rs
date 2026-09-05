// SPDX-License-Identifier: GPL-3.0-or-later
// Built-in presets, compiled into the binary via include_str! so "reset to
// preset" always uses the shipped values and can never be corrupted by hand
// edits to theme.toml. Each preset carries the seven palette roles, the two
// lifewall tints, and a full hand-tuned 16-colour terminal set.

use crate::theme::Palette;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub palette: Palette,
    pub lifewall: PresetWall,
    pub ansi: Ansi,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PresetWall {
    pub mature: String,
    pub newborn: String,
}

/// The 16 ANSI slots, kitty naming. Order here is display order in olive.conf.
#[derive(Debug, Clone, Deserialize)]
pub struct Ansi {
    pub color0: String,
    pub color8: String,
    pub color1: String,
    pub color9: String,
    pub color2: String,
    pub color10: String,
    pub color3: String,
    pub color11: String,
    pub color4: String,
    pub color12: String,
    pub color5: String,
    pub color13: String,
    pub color6: String,
    pub color14: String,
    pub color7: String,
    pub color15: String,
}

// Every preset's TOML source, keyed by name. NAMES preserves this order, which
// is also the order the "preset" field cycles through in the TUI/GUI — so the
// four fun groups below (vivid, rainbow, pastel, light) stay contiguous when
// stepping through with left/right, right after the three original rices.
macro_rules! preset_files {
    ($($name:literal),+ $(,)?) => {
        pub const NAMES: &[&str] = &[$($name),+];

        fn source(name: &str) -> Option<&'static str> {
            match name {
                $($name => Some(include_str!(concat!("../presets/", $name, ".toml"))),)+
                _ => None,
            }
        }
    };
}

preset_files! {
    // The three original hand-tuned rices.
    "olive", "slate", "moss",
    // Super Saturated — pure R/G/B + CMY accents (255 in one or two channels).
    "vivid-red", "vivid-yellow", "vivid-green", "vivid-cyan", "vivid-blue", "vivid-magenta",
    // Rainbow — a spread of bright hues across the spectrum.
    "rainbow-orange", "rainbow-lime", "rainbow-teal", "rainbow-sky", "rainbow-violet",
    "rainbow-pink",
    // Pastels — soft, low-saturation accents on a dark ground.
    "pastel-pink", "pastel-mint", "pastel-lavender", "pastel-peach", "pastel-butter",
    "pastel-sky",
    // Inverted / light mode — light backgrounds with dark text.
    "light-daylight", "light-paper",
}

pub fn preset(name: &str) -> Option<Preset> {
    let Some(src) = source(name) else {
        return None;
    };
    match toml::from_str(src) {
        Ok(p) => Some(p),
        Err(e) => {
            // A compiled-in preset failing to parse is a build-time bug; surface
            // it loudly rather than silently falling back.
            eprintln!("lifeconf: BUG: preset {name:?} failed to parse: {e}");
            None
        }
    }
}
