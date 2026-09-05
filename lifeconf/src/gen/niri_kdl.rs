// SPDX-License-Identifier: GPL-3.0-or-later
// niri/config.kdl — marker-delimited region rewrite. Unlike the other configs
// this file is mostly hand-written, so lifeconf only owns four fenced regions
// (`// LIFECONF:BEGIN <name>` ... `// LIFECONF:END <name>`) and leaves every
// other byte untouched. After rewriting, `niri validate` is run as a guard.

use crate::cmd;
use crate::gen::Report;
use crate::theme::{hash, Theme};

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
        t.cursor.theme, t.cursor.size
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

/// Replace the body between `BEGIN name`/`END name` markers with `inner`,
/// leaving the marker lines and everything else intact. Returns None (and the
/// caller warns) if either marker is missing.
fn replace_region(src: &str, name: &str, inner: &str) -> Option<String> {
    let begin = format!("// LIFECONF:BEGIN {name}");
    let end = format!("// LIFECONF:END {name}");
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

    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, &src) {
        r.notes.push(format!("! {path}: write failed: {e}"));
        return;
    }
    r.written.push(path.to_string());

    // Guard: validate the rewritten config. M1 only warns (no rollback yet).
    match std::process::Command::new("niri").arg("validate").arg("--config").arg(path).output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => r.notes.push(format!(
            "! niri validate failed on the rewritten config:\n{}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(_) => {} // niri not installed here — skip the guard silently.
    }
}
