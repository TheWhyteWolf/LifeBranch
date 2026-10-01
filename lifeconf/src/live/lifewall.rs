// SPDX-License-Identifier: GPL-3.0-or-later
// The wallpaper has no config/IPC (lifewall is flags-only), so a live re-theme
// means killing the running one and relaunching it with new flags via niri.
// The brief reseed matches the existing Mod+Ctrl+G reset UX.
//
// Two kill patterns: the native `lifebg --layer`, and the kitty-panel wrapper
// that ran it before --layer existed, so the first re-theme after upgrading
// replaces the old wallpaper instead of stacking a second one over it.

use crate::cmd;
use crate::gen::Report;
use crate::theme::Theme;
use std::process::Command;

pub fn respawn(t: &Theme, r: &mut Report) {
    let native = super::silent(Command::new("pkill").args(["-f", "^[^ ]*bin/lifebg --layer"]));
    let panel =
        super::silent(Command::new("pkill").args(["-f", "kitten panel --edge=background.*bin/lifebg"]));
    let killed = native || panel;

    // niri msg action spawn only works inside a running niri session; outside
    // one it just fails and we stay quiet.
    let shell = cmd::lifewall_shell_cmd(t);
    let spawned = super::silent(
        Command::new("niri").args(["msg", "action", "spawn", "--", "sh", "-c", &shell]),
    );

    if spawned {
        let note = if killed {
            "live: wallpaper respawned with new settings"
        } else {
            "live: wallpaper started (none was running)"
        };
        r.notes.push(note.into());
    }
}
