// SPDX-License-Identifier: GPL-3.0-or-later
// swayidle's timeouts are baked into its argv (spawn line doesn't re-run on
// niri reload), so a live change means killing the daemon and relaunching it
// via niri with the new argv. swayidle re-arms immediately; the only cost is a
// reset of the current idle counter.

use crate::cmd;
use crate::gen::Report;
use crate::theme::Theme;
use std::process::Command;

pub fn respawn(t: &Theme, r: &mut Report) {
    let killed = super::silent(Command::new("pkill").args(["-x", "swayidle"]));

    let mut argv = vec!["msg".into(), "action".into(), "spawn".into(), "--".into()];
    argv.extend(cmd::swayidle_argv(t));
    let spawned = super::silent(Command::new("niri").args(&argv));

    if spawned && killed {
        r.notes.push("live: swayidle restarted with new timeouts".into());
    }
}
