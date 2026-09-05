// SPDX-License-Identifier: GPL-3.0-or-later
// The wallpaper has no config/IPC (lifewall is flags-only), so a live re-theme
// means killing the running kitty-panel wrapper and relaunching it with new
// flags via niri. The brief reseed matches the existing Mod+Ctrl+G reset UX.
//
// The kill pattern targets the *panel wrapper* ("kitten panel …bin/lifebg"),
// deliberately distinct from lifebg-toggle.sh's pattern, which targets the inner
// lifebg process for SIGSTOP/CONT. They must not overlap.

use crate::cmd;
use crate::gen::Report;
use crate::theme::Theme;
use std::process::Command;

pub fn respawn(t: &Theme, r: &mut Report) {
    let killed =
        super::silent(Command::new("pkill").args(["-f", "kitten panel --edge=background.*bin/lifebg"]));

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
