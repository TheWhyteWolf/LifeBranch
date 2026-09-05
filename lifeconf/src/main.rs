// SPDX-License-Identifier: GPL-3.0-or-later
// lifeconf — one TOML drives the whole olive rice's theming and exposed knobs.
//
// M1 (this milestone): the canonical theme model, three presets, and headless
// regeneration of every consumer's config (`lifeconf --apply`). Live-apply
// (M2-M4), the TUI (M5) and the GUI (M6) land in later milestones; their CLI
// entry points are stubbed here so the shape is stable.

mod ansi16;
mod cmd;
mod gen;
mod gui;
mod live;
mod model;
mod paths;
mod presets;
mod theme;
mod tui;

use paths::Paths;
use std::io::IsTerminal;
use theme::Theme;

const USAGE: &str = "lifeconf — settings/theming for the niri olive rice\n\n\
    lifeconf                 open the interactive UI (TUI in a terminal, else GUI)\n\
    lifeconf --apply         regenerate every config from theme.toml (headless)\n\
    lifeconf --preset NAME   switch palette preset (olive|slate|moss), save + apply\n\
    lifeconf --print         print the resolved theme.toml to stdout\n\
    lifeconf --gui           open the GUI explicitly\n\
    lifeconf --apply-greeter install the staged greeter palette to /etc/lifegreet\n\
    \x20                       (pkexec, else sudo) — the login screen needs root\n\
    lifeconf --help          this text\n\n\
    --no-live                regenerate files only; don't touch running processes\n\
    --no-restart             live-apply colours, but don't respawn wallpaper/idle\n\n\
    Canonical file: ~/.config/lifeconf/theme.toml (created from `olive` on first run).\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::resolve();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return;
    }

    // Optional preset switch, applied before everything else.
    let preset = arg_value(&args, "--preset");

    // Debug-only: render one GUI frame to a PPM offline (no Wayland) — the
    // privacy-safe visual test for the paint path.
    #[cfg(debug_assertions)]
    if let Some(out) = arg_value(&args, "--gui-render-ppm") {
        let theme = load_or_seed(&paths);
        std::process::exit(gui::render_ppm(paths, theme, &out));
    }

    if args.iter().any(|a| a == "--gui") {
        let theme = load_or_seed(&paths);
        std::process::exit(gui::run(paths, theme));
    }

    if args.iter().any(|a| a == "--print") {
        let theme = load_or_seed(&paths);
        match toml::to_string_pretty(&theme) {
            Ok(s) => print!("{s}"),
            Err(e) => {
                eprintln!("lifeconf: serialize failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Force-apply just the greeter config to /etc/lifegreet/config (pkexec/sudo).
    if args.iter().any(|a| a == "--apply-greeter") {
        let mut theme = load_or_seed(&paths);
        theme.sync_linked();
        let _ = gen::generate_all(&theme, &paths); // refresh the staged file first
        std::process::exit(live::greeter::apply_force(&paths));
    }

    let apply = args.iter().any(|a| a == "--apply") || preset.is_some();
    if apply {
        run_apply(&paths, preset.as_deref());
        return;
    }

    // No actionable flag: open the interactive UI — TUI in a terminal, else GUI.
    let theme = load_or_seed(&paths);
    if std::io::stdin().is_terminal() {
        std::process::exit(tui::run(paths, theme));
    }
    std::process::exit(gui::run(paths, theme));
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

/// Load theme.toml, (re)seeding it from the olive default when it's missing,
/// empty, or unparseable — an empty/corrupt file (e.g. a write interrupted by a
/// crash) must be repaired on disk, not just defaulted in memory, or every run
/// keeps failing to parse it.
fn load_or_seed(paths: &Paths) -> Theme {
    let path = paths.theme_toml();
    match std::fs::read_to_string(&path) {
        Ok(body) if !body.trim().is_empty() => match toml::from_str::<Theme>(&body) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("lifeconf: {path}: invalid ({e}); re-seeding from olive");
                seed(&path)
            }
        },
        _ => seed(&path), // missing or empty
    }
}

fn seed(path: &str) -> Theme {
    let theme = Theme::default();
    match theme.save(path) {
        Ok(()) => println!("lifeconf: seeded {path} (olive default)"),
        Err(e) => eprintln!("lifeconf: could not seed {path}: {e}"),
    }
    theme
}

fn run_apply(paths: &Paths, preset: Option<&str>) {
    let mut theme = load_or_seed(paths);

    if let Some(name) = preset {
        if theme.apply_preset(name) {
            if let Err(e) = theme.save(&paths.theme_toml()) {
                eprintln!("lifeconf: could not save theme.toml: {e}");
            }
            println!("lifeconf: preset -> {name}");
        } else {
            eprintln!(
                "lifeconf: unknown preset {name:?} (choices: {})",
                presets::NAMES.join(", ")
            );
            std::process::exit(2);
        }
    }

    theme.sync_linked(); // linked lock/login screens follow the current palette
    let mut report = gen::generate_all(&theme, paths);

    // Live-apply to running processes unless disabled. Skipped automatically when
    // LIFECONF_HOME points at a sandbox (tests must never signal the real
    // waybar/kitty/wallpaper).
    let sandboxed = std::env::var_os("LIFECONF_HOME").is_some();
    let no_live = std::env::args().any(|a| a == "--no-live");
    let no_restart = std::env::args().any(|a| a == "--no-restart");
    if !no_live && !sandboxed {
        live::apply_all(&theme, paths, &mut report, !no_restart);
        live::greeter::apply_if_changed(paths, &mut report);
    }

    for p in &report.written {
        println!("  wrote {p}");
    }
    for n in &report.notes {
        println!("  {n}");
    }
    println!("lifeconf: applied theme ({} files).", report.written.len());
}
