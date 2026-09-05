// SPDX-License-Identifier: GPL-3.0-or-later
// Live-apply: push a freshly-generated theme to already-running processes so a
// re-theme is instant, kitten-themes style, with no restart. Every function is
// best-effort — a missing/uncooperative target is a note, never a failure, so
// `--apply` always succeeds even on a headless box.
//
// Milestones: M2 waybar + kitty (here); M3 lifenote; M4 lifewall/swayidle/cursor.

pub mod cursor;
pub mod greeter;
pub mod kitty;
pub mod lifenote;
pub mod lifewall;
pub mod swayidle;
pub mod waybar;

use crate::gen::Report;
use crate::paths::Paths;
use crate::theme::Theme;
use std::process::{Command, Stdio};

/// Run a command with its stdout/stderr discarded (so a chatty or failing child
/// never pollutes lifeconf's own report), returning whether it exited 0.
pub(crate) fn silent(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Push the theme to running processes. `restart` gates the *disruptive*
/// respawns (wallpaper reseed, swayidle restart); the colour pushes are cheap
/// and idempotent, safe to spam on every TUI preview keystroke.
pub fn apply_all(theme: &Theme, paths: &Paths, r: &mut Report, restart: bool) {
    // Cheap, non-disruptive: instant colour re-theme. waybar is rate-limited
    // internally (each SIGUSR2 rebuilds its bars); `restart` marks the commit,
    // which always sends so the committed colours are never the throttled ones.
    waybar::reload(r, restart);
    kitty::reload(paths, r);
    lifenote::reload(r);
    cursor::apply(theme, r);

    // Disruptive: only on an explicit apply/commit, not on live preview.
    if restart {
        lifewall::respawn(theme, r);
        swayidle::respawn(theme, r);
    }
}
