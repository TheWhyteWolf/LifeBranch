// SPDX-License-Identifier: GPL-3.0-or-later
// Generators: each turns a &Theme into one consumer's native config format.
// The pure ones expose render(&Theme) -> String and let mod.rs do the IO;
// niri_kdl and lifegreet have bespoke behaviour (in-place region rewrite;
// privileged staging) and manage their own paths.

pub mod fuzzel;
pub mod kitty;
pub mod lifegreet;
pub mod lifelock;
pub mod lifenote;
pub mod niri_kdl;
pub mod swaylock;
pub mod waybar;

use crate::paths::Paths;
use crate::theme::Theme;

#[derive(Default)]
pub struct Report {
    pub written: Vec<String>,
    pub notes: Vec<String>,
}

/// Resolve `p` to the real file to be replaced, following symlinks.
///
/// install.sh symlinks the generated configs into the repo
/// (~/.config/waybar/style.css -> ~/git/niri/waybar/style.css). An atomic
/// rename onto the *link* path would replace the link with a regular file and
/// quietly detach the config from the repo, so always swap the link's target.
fn resolve_target(p: &std::path::Path) -> std::path::PathBuf {
    if let Ok(real) = std::fs::canonicalize(p) {
        return real;
    }
    // Not created yet, or a dangling link into a not-yet-generated repo file:
    // follow one level by hand so we still write *through* the link.
    if let Ok(link) = std::fs::read_link(p) {
        return if link.is_absolute() {
            link
        } else {
            p.parent().map(|d| d.join(&link)).unwrap_or(link)
        };
    }
    p.to_path_buf()
}

fn write(report: &mut Report, path: String, body: String) {
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            report.notes.push(format!("! {path}: mkdir failed: {e}"));
            return;
        }
    }

    // Atomic replace, not std::fs::write. fs::write truncates in place, so a
    // consumer reading mid-write sees an empty or half-written file — and
    // live::apply_all pokes waybar to re-read style.css immediately after this
    // returns. rename(2) on the same filesystem is atomic: readers see either
    // the old file or the new one, never a partial one.
    let target = resolve_target(std::path::Path::new(&path));
    let Some(name) = target.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        report.notes.push(format!("! {path}: no file name to write"));
        return;
    };
    let tmp = target.with_file_name(format!(".{name}.lifeconf-tmp"));

    let res = (|| -> std::io::Result<()> {
        use std::io::Write;
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(body.as_bytes())?;
            // Durable before the swap: a crash between rename and writeback
            // would otherwise leave a correctly-named but empty file.
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &target)
    })();

    match res {
        Ok(()) => report.written.push(path),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            report.notes.push(format!("! {path}: write failed: {e}"));
        }
    }
}

/// The plain-file consumers: pure String -> file writes, no subprocesses. Fast
/// enough to re-run on every TUI keystroke for live preview.
pub fn generate_theme_files(theme: &Theme, paths: &Paths, r: &mut Report) {
    write(r, paths.config("waybar/style.css"), waybar::render(theme));
    write(r, paths.config("kitty/olive.conf"), kitty::render(theme));
    write(r, paths.config("fuzzel/fuzzel.ini"), fuzzel::render(theme));
    write(r, paths.config("lifenote/config"), lifenote::render(theme));
    write(r, paths.config("swaylock/config"), swaylock::render(theme));
    write(r, paths.config("lifelock/config"), lifelock::render(theme));
}

/// Regenerate every theme-derived config. Returns a report of what changed and
/// any manual follow-ups (e.g. the privileged lifegreet install). The niri
/// rewrite runs `niri validate`, and lifegreet stages a privileged file, so
/// this is the commit path, not the preview path (see generate_theme_files).
pub fn generate_all(theme: &Theme, paths: &Paths) -> Report {
    let mut r = Report::default();
    generate_theme_files(theme, paths, &mut r);
    lifegreet::stage(theme, paths, &mut r);
    niri_kdl::apply(theme, &paths.config("niri/config.kdl"), &mut r);
    r
}
