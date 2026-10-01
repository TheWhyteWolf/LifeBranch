// SPDX-License-Identifier: GPL-3.0-or-later
// lifebar — the LifeBranch status bar, replacing waybar. Pure text, one bar
// per output:
//
//    1  2   kitty — ~/git          Wed 01 Oct  19:18     # 3  IDLE  NET 77%  VOL 20%  BAT 74%  CPU 4%  MEM 31%  Vesktop
//
// Same modules, wording and clicks as the waybar config it replaces. The
// tray is a StatusNotifierItem host whose items are text labels; their menus
// open in lifemenu.
//
// Signals, so scripts and lifenote can nudge it as they did waybar:
//   SIGRTMIN+8 / +9   re-read do-not-disturb and the notification badge
//   SIGRTMIN+10       re-read volume and brightness
//   SIGUSR2           reload ~/.config/lifebar/config (lifeconf does this)

mod config;
mod model;
mod niri;
mod sys;
mod tray;
mod uevent;
mod ui;

/// lifemenu's renderer wants `crate::cli::Rgba`.
mod cli {
    pub type Rgba = [u8; 4];
}
#[path = "../../lifemenu/src/render.rs"]
mod render;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

pub const SIG_NOTES: u8 = 1;
pub const SIG_LEVELS: u8 = 2;
pub const SIG_RELOAD: u8 = 3;

/// The write end of the self-pipe the signal handler pokes.
static PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(sig: libc::c_int) {
    let code = if sig == libc::SIGUSR2 {
        SIG_RELOAD
    } else if sig == libc::SIGRTMIN() + 10 {
        SIG_LEVELS
    } else {
        SIG_NOTES
    };
    // SAFETY: write(2) is async-signal-safe; the pipe is non-blocking, so a
    // full pipe drops the byte rather than hanging the handler.
    unsafe { libc::write(PIPE.load(Ordering::Relaxed), (&code as *const u8).cast(), 1) };
}

/// Signals become bytes on a pipe the event loop reads, so the handler does
/// nothing but write.
fn signal_pipe() -> Result<OwnedFd, String> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 into a local array, then sigaction with our handler.
    unsafe {
        if libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) != 0 {
            return Err(format!("pipe: {}", std::io::Error::last_os_error()));
        }
        PIPE.store(fds[1], Ordering::Relaxed);
        for sig in [libc::SIGRTMIN() + 8, libc::SIGRTMIN() + 9, libc::SIGRTMIN() + 10, libc::SIGUSR2] {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as *const () as usize;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
        Ok(OwnedFd::from_raw_fd(fds[0]))
    }
}

/// One bar per session: a second start (a config reload, the systemd unit
/// racing a manual run) just exits.
fn lock() -> Option<std::fs::File> {
    let dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let f = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(format!("{dir}/lifebar.lock")).ok()?;
    // SAFETY: flock on a descriptor we own.
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f)
}

fn main() {
    if std::env::args().nth(1).is_some_and(|a| a == "-h" || a == "--help") {
        print!(
            "lifebar — the LifeBranch status bar (replaces waybar)\n\n\
             Takes no flags. Config: ~/.config/lifebar/config (lifeconf writes it).\n\
             Signals: RTMIN+8/+9 notifications, RTMIN+10 volume/brightness, USR2 reload.\n"
        );
        return;
    }
    let Some(_lock) = lock() else {
        eprintln!("lifebar: already running");
        return;
    };
    let pipe = match signal_pipe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lifebar: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = ui::run(config::Config::load(), pipe) {
        eprintln!("lifebar: {e}");
        std::process::exit(1);
    }
}
