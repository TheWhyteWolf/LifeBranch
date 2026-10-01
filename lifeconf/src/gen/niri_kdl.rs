// SPDX-License-Identifier: GPL-3.0-or-later
// niri/config.kdl — marker-delimited region rewrite. Unlike the other configs
// this file is mostly hand-written, so lifeconf only owns four fenced regions
// (`// LIFECONF:BEGIN <name>` ... `// LIFECONF:END <name>`) and leaves every
// other byte untouched. The rewrite is staged and must pass `niri validate`
// before it replaces the live file; a rejected rewrite changes nothing.

use crate::cmd;
use crate::gen::Report;
use crate::theme::{hash, plain_name, Theme};

/// Format an f64 without trailing zeros (0.3, 3, 0.14) so the generated flags
/// read like the hand-written defaults.
fn numf(v: f64) -> String {
    format!("{v}")
}

fn gen_animations(t: &Theme) -> String {
    format!(
        "animations {{\n\
         \x20   // Crisper, less floaty (1.0 = niri default speed; lower = faster).\n\
         \x20   slowdown {}\n\
         }}",
        numf(t.animations.slowdown)
    )
}

fn gen_cursor(t: &Theme) -> String {
    format!(
        "cursor {{\n\
         \x20   xcursor-theme \"{}\"\n\
         \x20   xcursor-size {}\n\
         }}",
        plain_name(&t.cursor.theme), t.cursor.size
    )
}

fn gen_idle(t: &Theme) -> String {
    // Each argv token becomes its own quoted KDL string, exactly as hand-written.
    let quoted: Vec<String> =
        cmd::swayidle_argv(t).iter().map(|a| format!("\"{a}\"")).collect();
    format!("spawn-at-startup {}", quoted.join(" "))
}

fn gen_lifewall(t: &Theme) -> String {
    // The shell command has only single quotes inside, so it drops straight into
    // a double-quoted KDL string with no escaping.
    format!("spawn-sh-at-startup \"{}\"", cmd::lifewall_shell_cmd(t))
}

// The three color-bearing blocks inside `layout {}` (indented 4 spaces): the
// focused-window border (the "highlight around the frame"), the tab strip, and
// the drop-target hint. All follow the palette so a re-theme moves them too.
fn gen_border(t: &Theme) -> String {
    let p = &t.palette;
    format!(
        "    border {{\n        width 2\n        active-color \"{}\"\n        inactive-color \"{}\"\n        urgent-color \"{}\"\n    }}",
        hash(&p.accent),
        hash(&p.border),
        hash(&p.urgent),
    )
}

fn gen_tab_indicator(t: &Theme) -> String {
    let p = &t.palette;
    format!(
        "    tab-indicator {{\n        hide-when-single-tab\n        active-color \"{}\"\n        inactive-color \"{}\"\n        urgent-color \"{}\"\n    }}",
        hash(&p.accent),
        hash(&p.border),
        hash(&p.urgent),
    )
}

fn gen_insert_hint(t: &Theme) -> String {
    // Translucent accent (alpha 80), matching the original hand-written value.
    format!("    insert-hint {{\n        color \"{}80\"\n    }}", hash(&t.palette.accent))
}

/// Replace the body between `<prefix>:BEGIN name` / `<prefix>:END name` marker
/// comments with `inner`, leaving the marker lines and everything else intact.
/// Returns None if either marker is missing. LIFECONF regions are the ones
/// lifeconf generates from the theme; LIFEBRANCH regions are the installer's
/// (keyboard, touchpad, …) and are edited by the Settings panels.
pub(crate) fn replace_region_in(src: &str, prefix: &str, name: &str, inner: &str) -> Option<String> {
    let begin = format!("// {prefix}:BEGIN {name}");
    let end = format!("// {prefix}:END {name}");
    let bpos = src.find(&begin)?;
    // Start of replaceable body: the newline after the BEGIN marker line.
    let body_start = src[bpos..].find('\n')? + bpos + 1;
    let epos = src[body_start..].find(&end)? + body_start;
    // Keep indentation before the END marker (the whitespace on its line).
    let end_line_start = src[..epos].rfind('\n')? + 1;
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..body_start]);
    out.push_str(inner);
    out.push('\n');
    out.push_str(&src[end_line_start..]);
    Some(out)
}

fn replace_region(src: &str, name: &str, inner: &str) -> Option<String> {
    replace_region_in(src, "LIFECONF", name, inner)
}

/// The text between a region's markers (None if either is missing).
pub(crate) fn region_body<'a>(src: &'a str, prefix: &str, name: &str) -> Option<&'a str> {
    let begin = format!("// {prefix}:BEGIN {name}");
    let end = format!("// {prefix}:END {name}");
    let bpos = src.find(&begin)?;
    let body_start = src[bpos..].find('\n')? + bpos + 1;
    let epos = src[body_start..].find(&end)? + body_start;
    let end_line_start = src[..epos].rfind('\n')? + 1;
    Some(&src[body_start..end_line_start.max(body_start)])
}

/// Stage `src` next to the real config (through the symlink, so the config
/// stays attached to the repo, and in the same directory so any relative
/// `include` resolves the same way), validate THAT, and only then rename it over
/// the live config. niri hot-reloads config.kdl and reads it at login, so it
/// must never see a truncated or invalid file: on a failed validate the live
/// config is left exactly as it was. The Err text is ready to show.
pub(crate) fn write_validated(path: &str, src: &str) -> Result<(), String> {
    let target = super::resolve_target(std::path::Path::new(path));
    let Some(name) = target.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(format!("! {path}: no file name to write"));
    };
    let tmp = target.with_file_name(format!(".{name}.lifeconf-tmp"));

    let staged = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(src.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("! {path}: write failed: {e}"));
    }

    match std::process::Command::new("niri").arg("validate").arg("--config").arg(&tmp).output() {
        Ok(o) if !o.status.success() => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "! niri validate rejected the rewritten config; {path} left unchanged:\n{}",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Ok(_) => {}
        Err(_) => {} // niri not installed here — skip the guard silently.
    }

    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("! {path}: write failed: {e}")
    })
}

pub fn apply(t: &Theme, path: &str, r: &mut Report) {
    let Ok(mut src) = std::fs::read_to_string(path) else {
        r.notes.push(format!("! {path}: cannot read; skipped niri regions"));
        return;
    };

    let regions = [
        ("animations", gen_animations(t)),
        ("cursor", gen_cursor(t)),
        ("idle-timeouts", gen_idle(t)),
        ("lifewall", gen_lifewall(t)),
        ("niri-border", gen_border(t)),
        ("niri-tab-indicator", gen_tab_indicator(t)),
        ("niri-insert-hint", gen_insert_hint(t)),
    ];
    for (name, inner) in &regions {
        match replace_region(&src, name, inner) {
            Some(next) => src = next,
            None => r.notes.push(format!(
                "! {path}: missing `// LIFECONF:BEGIN {name}` markers; region left as-is"
            )),
        }
    }

    match write_validated(path, &src) {
        Ok(()) => r.written.push(path.to_string()),
        Err(e) => r.notes.push(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lifeconf-niri-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn replace_region_swaps_body_only() {
        let src = "a\n// LIFECONF:BEGIN x\nold\n    // LIFECONF:END x\nb\n";
        let out = replace_region(src, "x", "new").unwrap();
        assert_eq!(out, "a\n// LIFECONF:BEGIN x\nnew\n    // LIFECONF:END x\nb\n");
        assert!(replace_region(src, "missing", "new").is_none());
    }

    #[test]
    fn escaping_holds_for_hostile_theme_values() {
        let mut t = Theme::default();
        t.cursor.theme = "Adwaita\" }\nspawn-at-startup \"x".into();
        t.font.family = "Mono'; touch /tmp/pwned; '".into();
        t.palette.bg = "#12'\"$(id)".into();
        assert!(!gen_cursor(&t).contains("\"x"));
        let wall = gen_lifewall(&t);
        // Only the generator's own quotes survive: one pair around the KDL
        // string, none from the theme values.
        assert_eq!(wall.matches('"').count(), 2);
        assert!(!wall.contains("$("));
        assert!(!wall.contains("touch /tmp/pwned; '"));
    }

    /// The live config must never be replaced by a rewrite niri rejects.
    /// Needs niri on PATH; skipped (passes) without it, as the guard is.
    #[test]
    fn rejected_rewrite_leaves_config_untouched() {
        if std::process::Command::new("niri").arg("--version").output().is_err() {
            return;
        }
        let dir = scratch("reject");
        let cfg = dir.join("config.kdl");
        // Invalid outside any region, so every rewrite fails validation.
        let original = "// LIFECONF:BEGIN cursor\n// LIFECONF:END cursor\nnot-a-niri-node {}\n";
        std::fs::write(&cfg, original).unwrap();
        let mut r = Report::default();
        apply(&Theme::default(), cfg.to_str().unwrap(), &mut r);
        assert!(r.written.is_empty());
        assert!(r.notes.iter().any(|n| n.contains("left unchanged")), "{:?}", r.notes);
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), original);
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "staging file not cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_through_symlink() {
        let dir = scratch("link");
        let real = dir.join("real.kdl");
        let link = dir.join("config.kdl");
        std::fs::write(&real, "// LIFECONF:BEGIN cursor\n// LIFECONF:END cursor\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut r = Report::default();
        apply(&Theme::default(), link.to_str().unwrap(), &mut r);
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let body = std::fs::read_to_string(&real).unwrap();
        assert!(body.contains("xcursor-theme"), "{body}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
