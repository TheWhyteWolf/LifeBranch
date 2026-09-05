// SPDX-License-Identifier: GPL-3.0-or-later
// lifenote re-reads its config on `lifenote ctl reload` (added in M3, over its
// existing rs.lifenote.Control DBus interface). We shell to the installed binary
// rather than duplicate a zbus proxy — same pattern as scripts/dnd-toggle.sh.
// On the laptop (mako, not lifenote) this simply no-ops.

use crate::gen::Report;
use std::process::Command;

/// Prefer ~/.local/bin/lifenote (where install.sh symlinks it; waybar's PATH
/// note shows ~/.local/bin isn't always on PATH), else fall back to PATH.
fn lifenote_bin() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let p = format!("{home}/.local/bin/lifenote");
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    "lifenote".into()
}

pub fn reload(r: &mut Report) {
    if super::silent(Command::new(lifenote_bin()).args(["ctl", "reload"])) {
        r.notes.push("live: lifenote reloaded".into());
    }
}
