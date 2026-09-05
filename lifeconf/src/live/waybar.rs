// SPDX-License-Identifier: GPL-3.0-or-later
// waybar reloads its config + stylesheet on SIGUSR2. Same "poke waybar" idea as
// scripts/dnd-toggle.sh (which sends RTMIN+8 to refresh a custom module).
//
// Rate-limited, because apply_all runs on every TUI/GUI preview keystroke and
// each SIGUSR2 makes waybar tear down and rebuild its bars. Coalescing the
// preview storm into at most one reload per MIN_INTERVAL keeps the bar doing
// visible work instead of restarting itself. A commit (`force`) always sends,
// so the final state is never the one that got throttled away.

use crate::gen::Report;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MIN_INTERVAL: Duration = Duration::from_millis(250);

fn last_sent() -> &'static Mutex<Option<Instant>> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// Poke waybar to re-read its config and stylesheet.
///
/// `force` bypasses the rate limit — pass it on an explicit apply/commit, not
/// on live preview. A poisoned lock degrades to "send anyway": dropping a
/// reload is worse than sending one too many.
pub fn reload(r: &mut Report, force: bool) {
    let now = Instant::now();
    let send = match last_sent().lock() {
        Ok(mut last) => {
            let due = force
                || last.map_or(true, |t| now.duration_since(t) >= MIN_INTERVAL);
            if due {
                *last = Some(now);
            }
            due
        }
        Err(_) => true,
    };
    if !send {
        return;
    }
    if super::silent(Command::new("pkill").args(["-USR2", "-x", "waybar"])) {
        r.notes.push("live: waybar reloaded (SIGUSR2)".into());
    }
}
