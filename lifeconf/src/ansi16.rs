// SPDX-License-Identifier: GPL-3.0-or-later
// Derive a full 16-colour terminal palette from the seven role colours.
//
// Only the "custom" path uses this — the three shipped presets carry their own
// hand-tuned [ansi] tables (so olive reproduces kitty/olive.conf exactly). When
// a user hand-edits palette roles, this fills in a consistent, if less lovingly
// tuned, 16-colour set so the terminal never goes un-themed.

use crate::presets::Ansi;
use crate::theme::{rgb, Palette};

fn fmt(c: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

fn lighten(c: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    mix(c, (255, 255, 255), t)
}

fn darken(c: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    mix(c, (0, 0, 0), t)
}

pub fn derive_ansi16(p: &Palette) -> Ansi {
    let z = (0, 0, 0);
    let bg = rgb(&p.bg).unwrap_or(z);
    let surface = rgb(&p.surface).unwrap_or(z);
    let border = rgb(&p.border).unwrap_or(z);
    let text = rgb(&p.text).unwrap_or((123, 140, 90));
    let accent = rgb(&p.accent).unwrap_or((164, 201, 75));
    let warn = rgb(&p.warn).unwrap_or((199, 209, 122));
    let urgent = rgb(&p.urgent).unwrap_or((138, 59, 46));

    Ansi {
        color0: fmt(lighten(bg, 0.1)),
        color8: fmt(border),
        color1: fmt(urgent),
        color9: fmt(lighten(urgent, 0.22)),
        color2: fmt(text),
        color10: fmt(accent),
        color3: fmt(mix(text, warn, 0.5)),
        color11: fmt(warn),
        color4: fmt(mix(text, surface, 0.55)),
        color12: fmt(darken(accent, 0.3)),
        color5: fmt(mix(text, border, 0.35)),
        color13: fmt(lighten(text, 0.15)),
        color6: fmt(mix(text, border, 0.45)),
        color14: fmt(lighten(mix(text, accent, 0.3), 0.1)),
        color7: fmt(text),
        color15: fmt(lighten(text, 0.45)),
    }
}
