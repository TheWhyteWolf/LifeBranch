// SPDX-License-Identifier: GPL-3.0-or-later
// lifepanel — quick settings for LifeBranch: wifi, bluetooth, volume and
// output, mic, brightness, power profile, battery, do-not-disturb and
// removable drives, in one pure-text popup. Replaces nm-applet, blueman and
// udiskie.
//
//   lifepanel           open the panel (run it again to close it)
//   lifepanel --watch   stay running: mount drives as they are plugged in
//
// The backends are lifeconf's own system panels, compiled in from
// ../lifeconf/src/sys; the look and the font come from lifemenu's config.

mod backend;
mod drives;
mod panel;
mod ui;
mod watch;

// lifeconf's system panels: the parsers and commands the Settings app uses.
#[path = "../../lifeconf/src/sys"]
#[allow(dead_code)]
mod sys {
    mod common;
    pub use self::common::*;
    pub mod bluetooth;
    pub mod net;
    pub mod power;
    pub mod sound;
}

// lifemenu's config reader (colours, font) and its text renderer.
#[path = "../../lifemenu/src/cli.rs"]
#[allow(dead_code)]
mod cli;
#[path = "../../lifemenu/src/render.rs"]
mod render;

use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;

const USAGE: &str = "lifepanel — quick settings for LifeBranch

  lifepanel                 open the panel; running it again closes it
  lifepanel --watch         mount removable drives as they are plugged in
            --no-automount    ...or only say they arrived

In the panel: arrows move and adjust, Enter acts, Delete or e ejects a
drive, Esc closes. The mouse works too: click a word, drag nothing.
Colours and font: lifemenu's config (~/.config/lifemenu/config, else
~/.config/fuzzel/fuzzel.ini).
";

/// One panel at a time: holding the lock means we are it. If another panel
/// holds it, close that one instead (a bar button toggles the panel).
fn claim() -> Result<Option<std::fs::File>, String> {
    let dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| "XDG_RUNTIME_DIR is not set".to_string())?;
    let path = format!("{dir}/lifepanel.lock");
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        if let Ok(pid) = s.trim().parse::<i32>() {
            if pid > 0 {
                // SAFETY: plain kill(2).
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
        return Ok(None);
    }
    let _ = f.set_len(0);
    let _ = f.rewind();
    let _ = write!(f, "{}", std::process::id());
    Ok(Some(f))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("-h") || has("--help") {
        print!("{USAGE}");
        return;
    }
    if let Some(bad) = args.iter().find(|a| !matches!(a.as_str(), "--watch" | "--no-automount")) {
        eprintln!("lifepanel: unknown flag {bad}\n\n{USAGE}");
        std::process::exit(2);
    }
    if has("--watch") {
        std::process::exit(watch::run(!has("--no-automount")));
    }
    let _lock = match claim() {
        Ok(Some(l)) => l,
        Ok(None) => return, // closed the open one
        Err(e) => {
            eprintln!("lifepanel: {e}");
            std::process::exit(1);
        }
    };
    let cfg = match cli::parse(&[]) {
        cli::Parsed::Run(c) => c,
        _ => cli::Cfg::default(),
    };
    if let Err(e) = ui::run(cfg) {
        eprintln!("lifepanel: {e}");
        std::process::exit(1);
    }
}
