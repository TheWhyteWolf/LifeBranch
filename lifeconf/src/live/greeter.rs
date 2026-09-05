// SPDX-License-Identifier: GPL-3.0-or-later
// Apply the greeter theme. lifegreet runs as the `greeter` user off root-owned
// paths, so this is the one privileged corner of lifeconf: installing the
// staged palette to /etc/lifegreet/config and, when the repo build is newer,
// refreshing /usr/local/bin/lifegreet itself (the binary only learned to read
// the config file in M1 — an older installed binary ignores it entirely).
//
// Privilege comes from pkexec: the polkit agent niri spawns pops a password
// dialog, so both the GUI Save button and a TUI save prompt graphically. The
// terminal-only `lifeconf --apply-greeter` falls back to sudo.

use crate::gen::Report;
use crate::paths::Paths;
use std::path::PathBuf;
use std::process::Command;

pub const SYSTEM_CONFIG: &str = "/etc/lifegreet/config";
pub const SYSTEM_BIN: &str = "/usr/local/bin/lifegreet";

/// The repo's own lifegreet build, resolved from lifeconf's binary path
/// (repo/lifeconf/target/release/lifeconf -> repo/lifegreet/target/release/…).
fn repo_greeter_bin() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let repo = exe.ancestors().nth(4)?;
    let bin = repo.join("lifegreet/target/release/lifegreet");
    bin.exists().then_some(bin)
}

/// The shell command installing whatever is out of date, or None if the
/// greeter isn't in use here / everything already matches.
fn install_cmd(staged: &str) -> Option<String> {
    // No installed greeter binary => this machine doesn't use lifegreet.
    if !std::path::Path::new(SYSTEM_BIN).exists() {
        return None;
    }

    let mut steps = Vec::new();

    let want = std::fs::read_to_string(staged).unwrap_or_default();
    let config_stale = !want.is_empty()
        && std::fs::read_to_string(SYSTEM_CONFIG).map(|have| have != want).unwrap_or(true);
    if config_stale {
        steps.push(format!("install -Dm644 '{staged}' '{SYSTEM_CONFIG}'"));
    }

    if let Some(repo_bin) = repo_greeter_bin() {
        let same = match (std::fs::read(&repo_bin), std::fs::read(SYSTEM_BIN)) {
            (Ok(a), Ok(b)) => a == b,
            _ => true, // unreadable — don't guess
        };
        if !same {
            steps.push(format!("install -Dm755 '{}' '{SYSTEM_BIN}'", repo_bin.display()));
        }
    }

    (!steps.is_empty()).then(|| steps.join(" && "))
}

/// Commit path: if the greeter config or binary is out of date, install via
/// pkexec — the polkit agent shows the password prompt. Best-effort: a cancel
/// or missing agent just leaves a note pointing at `lifeconf --apply-greeter`.
pub fn apply_if_changed(paths: &Paths, r: &mut Report) {
    let staged = paths.cache("lifeconf/lifegreet-config");
    let Some(cmd) = install_cmd(&staged) else {
        return;
    };
    if super::silent(Command::new("pkexec").args(["sh", "-c", &cmd])) {
        r.notes.push("greeter: theme applied (visible at next login)".into());
    } else {
        r.notes.push(
            "greeter: not applied (password prompt cancelled/unavailable) — \
             run `lifeconf --apply-greeter` from a terminal"
                .into(),
        );
    }
}

/// The `--apply-greeter` CLI: force-install whatever is stale. Tries pkexec
/// (graphical prompt), then sudo with inherited stdio so the terminal password
/// prompt is visible.
pub fn apply_force(paths: &Paths) -> i32 {
    let staged = paths.cache("lifeconf/lifegreet-config");
    if !std::path::Path::new(SYSTEM_BIN).exists() {
        eprintln!("lifeconf: no {SYSTEM_BIN} — this machine doesn't use the lifegreet greeter");
        return 1;
    }
    let Some(cmd) = install_cmd(&staged) else {
        println!("lifeconf: greeter already up to date");
        return 0;
    };
    let run = |bin: &str| {
        Command::new(bin)
            .args(["sh", "-c", &cmd])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if run("pkexec") || run("sudo") {
        println!("lifeconf: greeter theme applied (takes effect at next login)");
        0
    } else {
        eprintln!("lifeconf: could not apply the greeter theme (pkexec and sudo both failed)");
        1
    }
}
