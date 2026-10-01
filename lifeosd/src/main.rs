// SPDX-License-Identifier: GPL-3.0-or-later
// lifeosd — the volume/brightness on-screen display for LifeBranch, replacing
// wob. A labelled pure-text bar at the bottom of the screen:
//
//   volume     ████████████░░░░░░░░░  45%
//
// It owns a FIFO at $XDG_RUNTIME_DIR/lifeosd.fifo; scripts/vol-osd.sh and
// bright-osd.sh write one line per key press (`volume 45`, `volume 45 muted`,
// `brightness 9`, or a bare number as wob took). Between flashes there is no
// surface and no buffer, only a process asleep on the FIFO.
//
// Colours and font come from lifemenu's config, like lifepanel's.

mod msg;
mod ui;

#[path = "../../lifemenu/src/cli.rs"]
#[allow(dead_code)]
mod cli;
#[path = "../../lifemenu/src/render.rs"]
mod render;

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const USAGE: &str = "lifeosd — volume/brightness OSD for LifeBranch (replaces wob)

  lifeosd                      run the OSD (niri starts it at login)
  echo 'volume 45' > $XDG_RUNTIME_DIR/lifeosd.fifo
  echo 'volume 45 muted' > ...   echo 'brightness 9' > ...   echo 72 > ...

Shown for 0.9 s after the last line. Colours and font: lifemenu's config.
";

/// One OSD per session: a second start (a niri config reload) just exits.
fn lock(dir: &str) -> Result<Option<std::fs::File>, String> {
    let path = format!("{dir}/lifeosd.lock");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(None);
    }
    Ok(Some(f))
}

/// A fresh FIFO, opened read-write so it never reports end-of-file when a
/// writer goes away (the `tail -f` wob needed), and non-blocking for the loop.
fn fifo(dir: &str) -> Result<OwnedFd, String> {
    let path = format!("{dir}/lifeosd.fifo");
    let _ = std::fs::remove_file(&path);
    let c = CString::new(path.clone()).map_err(|_| "bad path".to_string())?;
    // SAFETY: mkfifo/open on a NUL-terminated path we built.
    unsafe {
        if libc::mkfifo(c.as_ptr(), 0o600) != 0 {
            return Err(format!("mkfifo {path}: {}", std::io::Error::last_os_error()));
        }
        let fd = libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(format!("{path}: {}", std::io::Error::last_os_error()));
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

fn main() {
    if std::env::args().nth(1).is_some_and(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return;
    }
    let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") else {
        eprintln!("lifeosd: XDG_RUNTIME_DIR is not set");
        std::process::exit(1);
    };
    let _lock = match lock(&dir) {
        Ok(Some(l)) => l,
        Ok(None) => return, // already running
        Err(e) => {
            eprintln!("lifeosd: {e}");
            std::process::exit(1);
        }
    };
    let fd = match fifo(&dir) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("lifeosd: {e}");
            std::process::exit(1);
        }
    };
    let cfg = match cli::parse(&[]) {
        cli::Parsed::Run(c) => c,
        _ => cli::Cfg::default(),
    };
    if let Err(e) = ui::run(cfg, fd) {
        eprintln!("lifeosd: {e}");
        std::process::exit(1);
    }
}
