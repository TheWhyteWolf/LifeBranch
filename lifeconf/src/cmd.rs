// SPDX-License-Identifier: GPL-3.0-or-later
// Command builders shared between the niri config generator (which bakes these
// into spawn-*-at-startup lines) and the live-respawn paths (which hand the
// exact same argv to `niri msg action spawn`). One source of truth so a
// generated startup line and a live respawn can never drift apart.

use crate::theme::{hash, Theme};

/// Format an f64 without trailing zeros (0.3, 3, 0.14).
fn numf(v: f64) -> String {
    format!("{v}")
}

/// Half-width katakana (U+FF66-FF9D) render at a single terminal column,
/// unlike full-width kana/kanji which are double-width and would misalign the
/// Game-of-Life cell grid.
fn kana_charset() -> String {
    ('\u{FF66}'..='\u{FF9D}').collect()
}

/// The glyph string drops into a single-quoted shell arg that itself sits
/// inside an unescaped double-quoted KDL string (see gen_lifewall's "no
/// escaping" invariant) — so strip anything that could break out of either:
/// quotes, backslashes, control characters. Falls back to '#' if that empties
/// a hand-edited theme.toml's value.
fn sanitize_glyphs(s: &str) -> String {
    let clean: String =
        s.chars().filter(|c| !c.is_control() && !matches!(c, '\'' | '"' | '\\')).collect();
    if clean.is_empty() {
        "#".into()
    } else {
        clean
    }
}

/// The shell command that runs the Game-of-Life wallpaper inside a kitty panel.
/// Single-quoted args (font_family, colours) are literal to both KDL's double
/// quotes and sh. font_size is a wallpaper-density knob, not the UI font size,
/// so it stays fixed.
pub fn lifewall_shell_cmd(t: &Theme) -> String {
    let w = &t.lifewall;
    let bg = hash(&t.palette.bg);
    let raw = if w.glyph_mode == "kana" { kana_charset() } else { w.char.clone() };
    let glyph = sanitize_glyphs(&raw);
    format!(
        "kitten panel --edge=background --config NONE \
         -o font_size=8 -o font_family='{family}' -o background='{bg}' \
         ~/.local/bin/lifebg --tick {tick} --fps {fps} --fade {fade} --density {density} \
         --char '{glyph}' --bg '{bg}' --mature '{mature}' --newborn '{newborn}' \
         --glider-interval {glider_interval}",
        family = t.font.family,
        bg = bg,
        tick = numf(w.tick),
        fps = w.fps,
        fade = numf(w.fade),
        density = numf(w.density),
        glyph = glyph,
        mature = hash(&w.mature),
        glider_interval = numf(w.glider_interval),
        newborn = hash(&w.newborn),
    )
}

/// The swayidle process argv (element 0 is "swayidle"). Timeouts come from the
/// theme; the rest of the idle chain is fixed (lifelock at lock, dpms + wallpaper
/// freeze at screen-off, thaw on resume).
pub fn swayidle_argv(t: &Theme) -> Vec<String> {
    let lock = (t.idle.lock_minutes * 60).to_string();
    let off = (t.idle.screen_off_minutes * 60).to_string();
    let lifelock = "systemd-cat -t lifelock ~/.local/bin/lifelock -f".to_string();
    vec![
        "swayidle".into(),
        "-w".into(),
        "timeout".into(),
        lock,
        lifelock.clone(),
        "timeout".into(),
        off,
        "niri msg action power-off-monitors; ~/.local/bin/lifebg-toggle.sh dpms-pause".into(),
        "resume".into(),
        "~/.local/bin/lifebg-toggle.sh dpms-resume".into(),
        "lock".into(),
        lifelock.clone(),
        "before-sleep".into(),
        lifelock,
    ]
}
