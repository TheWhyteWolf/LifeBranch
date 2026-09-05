// SPDX-License-Identifier: GPL-3.0-or-later
// Push colours to running kitty instances via remote control. Each kitty opens
// its own socket under $XDG_RUNTIME_DIR/kitty/ (listen_on in kitty/rice.conf);
// we enumerate them and `set-colors` each. Colours only — font size is a
// separate user preference and is deliberately NOT resized on a re-theme.

use crate::gen::Report;
use crate::paths::Paths;
use std::os::unix::net::UnixStream;
use std::process::Command;

/// Is anything listening on this socket?
///
/// kitty unlinks its socket on a clean exit, but not when it is SIGKILLed or
/// when the compositor dies under it, so $XDG_RUNTIME_DIR/kitty collects dead
/// ones. `kitty @ --to` sits on its own remote-control timeout for each of
/// those, which turns a re-theme into a multi-second stall — and apply_all runs
/// on every preview keystroke. connect(2) answers immediately: ECONNREFUSED
/// means the socket file outlived its kitty, so drop it. (The tmpfiles rule
/// deliberately has no Age; ageing files out here would also delete the socket
/// of any kitty that has merely been open a long time.)
fn is_live(path: &std::path::Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => true,
        Err(_) => {
            let _ = std::fs::remove_file(path);
            false
        }
    }
}

pub fn reload(paths: &Paths, r: &mut Report) {
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else {
        return;
    };
    let dir = format!("{runtime}/kitty");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return; // no sockets => no kitty running (or remote control off)
    };
    let colors = paths.config("kitty/olive.conf");
    let mut n = 0;
    let mut stale = 0;
    for ent in entries.flatten() {
        let path = ent.path();
        if !is_live(&path) {
            stale += 1;
            continue;
        }
        let to = format!("unix:{}", path.display());
        // --all: every window/tab in that instance; --configured: also update
        // the instance's default so new windows open already themed.
        if super::silent(Command::new("kitty").args([
            "@", "--to", &to, "set-colors", "--all", "--configured", &colors,
        ])) {
            n += 1;
        }
    }
    if n > 0 {
        r.notes.push(format!("live: kitty re-themed {n} instance(s)"));
    }
    if stale > 0 {
        r.notes
            .push(format!("live: removed {stale} stale kitty socket(s)"));
    }
}
