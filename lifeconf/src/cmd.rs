// SPDX-License-Identifier: GPL-3.0-or-later
// Command builders shared between the niri config generator (which bakes these
// into spawn-*-at-startup lines) and the live-respawn paths (which hand the
// exact same argv to `niri msg action spawn`). One source of truth so a
// generated startup line and a live respawn can never drift apart.

use crate::theme::{hash, plain_name, Theme};

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

/// Printable ASCII, space excluded. The default: it needs no font beyond the
/// one every terminal already has, and the spread from `.` to `@` gives the
/// board texture that a single repeated glyph cannot. sanitize_glyphs drops
/// the quotes and backslash immediately after, so they are left in here rather
/// than hand-excluded in two places.
fn ascii_charset() -> String {
    ('!'..='~').collect()
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

/// The shell command that runs the Game-of-Life wallpaper on the background
/// layer (lifewall --layer: GPU-drawn, no terminal in between). `exec` so the
/// process IS lifebg, which lifebg-toggle.sh and the reset bind match by name.
/// Single-quoted args (font family, colours) are literal to both KDL's double
/// quotes and sh — plain_name and hash() strip anything that could break out
/// of either, since theme.toml is hand-editable. font-size is a
/// wallpaper-density knob (points, like kitty's), not the UI font size, so it
/// stays fixed.
pub fn lifewall_shell_cmd(t: &Theme) -> String {
    let w = &t.lifewall;
    let bg = hash(&t.palette.bg);
    let raw = match w.glyph_mode.as_str() {
        "ascii" => ascii_charset(),
        "kana" => kana_charset(),
        // "custom", and anything a hand-edited theme.toml puts here.
        _ => w.char.clone(),
    };
    let glyph = sanitize_glyphs(&raw);
    format!(
        "exec ~/.local/bin/lifebg --layer --font-family '{family}' --font-size 8 \
         --tick {tick} --fps {fps} --fade {fade} --density {density} \
         --char '{glyph}' --bg '{bg}' --mature '{mature}' --newborn '{newborn}' \
         --glider-interval {glider_interval} --fps-battery {fps_battery}",
        family = plain_name(&t.font.family),
        bg = bg,
        tick = numf(w.tick),
        fps = w.fps,
        fade = numf(w.fade),
        density = numf(w.density),
        glyph = glyph,
        mature = hash(&w.mature),
        glider_interval = numf(w.glider_interval),
        newborn = hash(&w.newborn),
        fps_battery = w.fps_battery,
    )
}

/// The swayidle process argv (element 0 is "swayidle"). Timeouts come from the
/// theme; the rest of the idle chain is fixed (lifelock at lock, dpms + wallpaper
/// freeze at screen-off, thaw on resume).
///
/// The optional last step is suspend, which is off unless `idle.suspend_minutes`
/// says otherwise. It runs idle-suspend.sh rather than `systemctl suspend`
/// directly: the script no-ops on mains power, so a plugged-in laptop still
/// only locks and blanks. swayidle's own `before-sleep` locks on the way down,
/// so the suspend step deliberately does not lock again.
pub fn swayidle_argv(t: &Theme) -> Vec<String> {
    let lock = (t.idle.lock_minutes * 60).to_string();
    let off = (t.idle.screen_off_minutes * 60).to_string();
    let lifelock = "systemd-cat -t lifelock ~/.local/bin/lifelock -f".to_string();
    let mut argv: Vec<String> = vec![
        "swayidle".into(),
        "-w".into(),
        "timeout".into(),
        lock,
        lifelock.clone(),
        "timeout".into(),
        off,
        "niri msg action power-off-monitors; ~/.local/bin/lifebg-toggle.sh dpms-pause".into(),
        // Binds to the screen-off timeout immediately above: swayidle attaches
        // a `resume` clause to the timeout that precedes it, which is why the
        // suspend step below has to come AFTER this pair and not between them.
        "resume".into(),
        "~/.local/bin/lifebg-toggle.sh dpms-resume".into(),
    ];
    if t.idle.suspend_minutes > 0 {
        argv.push("timeout".into());
        argv.push((t.idle.suspend_minutes * 60).to_string());
        argv.push("~/.local/bin/idle-suspend.sh".into());
    }
    argv.extend(["lock".into(), lifelock.clone(), "before-sleep".into(), lifelock]);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_line_carries_the_battery_frame_rate() {
        let mut t = Theme::default();
        t.lifewall.fps_battery = 8;
        let cmd = lifewall_shell_cmd(&t);
        assert!(cmd.contains("--fps 15 ") && cmd.ends_with("--fps-battery 8"), "{cmd}");
        t.lifewall.fps_battery = 0;
        assert!(lifewall_shell_cmd(&t).ends_with("--fps-battery 0"), "0 = don't throttle");
    }

    fn pos(argv: &[String], needle: &str) -> usize {
        argv.iter().position(|a| a.contains(needle)).unwrap_or_else(|| panic!("{needle:?} not in {argv:?}"))
    }

    /// swayidle attaches a `resume` clause to the timeout immediately before
    /// it, so "resume" must stay adjacent to the screen-off step. Slipping the
    /// suspend timeout between them would silently re-point the wallpaper thaw
    /// at the wrong event and leave the board frozen after a wake.
    #[test]
    fn resume_stays_attached_to_the_screen_off_timeout() {
        for suspend_minutes in [0, 30] {
            let mut t = Theme::default();
            t.idle.suspend_minutes = suspend_minutes;
            let argv = swayidle_argv(&t);
            let off = pos(&argv, "power-off-monitors");
            assert_eq!(argv[off + 1], "resume", "suspend_minutes={suspend_minutes}");
            assert!(argv[off + 2].contains("dpms-resume"));
        }
    }

    #[test]
    fn suspend_step_is_off_by_default_and_lands_after_the_resume_pair() {
        let t = Theme::default();
        assert_eq!(t.idle.suspend_minutes, 0);
        assert!(!swayidle_argv(&t).iter().any(|a| a.contains("idle-suspend")));
        let mut t = Theme::default();
        t.idle.suspend_minutes = 30;
        let argv = swayidle_argv(&t);
        let sus = pos(&argv, "idle-suspend.sh");
        assert_eq!((argv[sus - 2].as_str(), argv[sus - 1].as_str()), ("timeout", "1800"));
        assert!(sus > pos(&argv, "dpms-resume"));
    }
}
