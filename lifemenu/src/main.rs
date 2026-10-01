// SPDX-License-Identifier: GPL-3.0-or-later
// lifemenu — the launcher and the dmenu for LifeBranch, replacing fuzzel with
// the family's own pure-text look.
//
//   lifemenu                  pick an application and launch it
//   ... | lifemenu --dmenu    pick a line from stdin and print it
//
// Flags are fuzzel's (the subset the scripts use, same meanings and exit
// codes), so every script switches by changing one word.

mod apps;
mod cli;
mod matcher;
mod model;
mod render;
mod ui;

use std::io::{IsTerminal, Read, Write};
use ui::Done;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match cli::parse(&args) {
        cli::Parsed::Run(c) => c,
        cli::Parsed::Help => {
            print!("{}", cli::USAGE);
            return;
        }
        cli::Parsed::Error(e) => {
            eprintln!("lifemenu: {e}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    std::process::exit(if cfg.dmenu { dmenu(cfg) } else { launcher(cfg) });
}

/// Choices from stdin; print the pick (or its index, or the typed text).
fn dmenu(cfg: cli::Cfg) -> i32 {
    // A bare input box (--prompt-only, or a password prompt fed no choices)
    // must not hang waiting on a terminal for lines that will never come.
    let mut input = String::new();
    if !cfg.prompt_only && !std::io::stdin().is_terminal() {
        let _ = std::io::stdin().read_to_string(&mut input);
    }
    let items: Vec<String> = input.lines().map(str::to_string).collect();
    let index = cfg.index;
    let out = match ui::run(cfg, items.clone(), items.clone()) {
        Ok(Done::Picked(i)) if index => i.to_string(),
        Ok(Done::Picked(i)) => items[i].clone(),
        Ok(Done::Text(t)) => t,
        Ok(Done::Cancel) => return 1,
        Err(e) => {
            eprintln!("lifemenu: {e}");
            return 2;
        }
    };
    let mut so = std::io::stdout().lock();
    if writeln!(so, "{out}").and_then(|_| so.flush()).is_err() {
        return 1;
    }
    0
}

/// The installed applications, most-launched first; launch the pick.
fn launcher(cfg: cli::Cfg) -> i32 {
    let desktop: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_else(|_| "niri".into())
        .split(':')
        .map(str::to_string)
        .collect();
    let mut list = apps::scan(&apps::dirs(), &desktop);
    let history = apps::history_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|b| apps::read_history(&b))
        .unwrap_or_default();
    apps::rank(&mut list, &history);
    let labels = list.iter().map(|a| a.name.clone()).collect();
    let hay = list.iter().map(apps::App::haystack).collect();
    let terminal = cfg.terminal.clone();
    let app = match ui::run(cfg, labels, hay) {
        Ok(Done::Picked(i)) => &list[i],
        Ok(_) => return 1,
        Err(e) => {
            eprintln!("lifemenu: {e}");
            return 2;
        }
    };
    let Some(argv) = apps::argv(app, &terminal) else {
        eprintln!("lifemenu: {}: cannot parse Exec={}", app.id, app.exec);
        return 1;
    };
    if let Err(e) = apps::launch(&argv, app.path.as_deref()) {
        eprintln!("lifemenu: {}: {e}", argv[0]);
        return 1;
    }
    apps::remember(&app.id);
    0
}
