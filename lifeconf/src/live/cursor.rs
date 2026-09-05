// SPDX-License-Identifier: GPL-3.0-or-later
// niri picks up the cursor from its own config.kdl (the generator rewrote the
// `cursor {}` region and niri hot-reloads it); GTK apps read gsettings instead,
// so mirror the theme there too — exactly as install.sh does at setup.

use crate::gen::Report;
use crate::theme::Theme;
use std::process::Command;

pub fn apply(t: &Theme, r: &mut Report) {
    let set = |key: &str, val: &str| {
        super::silent(Command::new("gsettings").args(["set", "org.gnome.desktop.interface", key, val]))
    };
    let a = set("cursor-theme", &t.cursor.theme);
    let b = set("cursor-size", &t.cursor.size.to_string());
    if a || b {
        r.notes.push("live: cursor updated (gsettings)".into());
    }
}
