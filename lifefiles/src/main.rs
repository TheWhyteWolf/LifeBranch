// SPDX-License-Identifier: GPL-3.0-or-later
// lifefiles — mouse-driven terminal file browser. Three columns like yazi
// (parent | current | preview) plus a places sidebar; click, double-click,
// drag-and-drop, right-click menu and the usual keys. Themed by lifeconf.

mod app;
mod fs;
mod theme;
mod ui;

use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "lifefiles [DIR|FILE]\n\
    Mouse: click select · double-click open · drag to move (Ctrl = copy) ·\n\
           right-click menu · wheel scroll · click breadcrumb/places to jump\n\
    Keys:  arrows/hjkl move · Enter open · Backspace up · Space mark · Ctrl+A all\n\
           Ctrl+C/X/V (or y/x/p) copy/cut/paste · F2 rename · Del trash ·\n\
           Shift+Del delete · F7 new folder · Ctrl+L path · / filter · . hidden ·\n\
           Alt+←/→ history · F5 reload · q quit\n";

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

fn main() {
    let mut start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut select = None;
    if let Some(a) = std::env::args().nth(1) {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return;
        }
        let p = PathBuf::from(&a);
        if p.is_dir() {
            start = p;
        } else if let (Some(par), Some(name)) = (p.parent(), p.file_name()) {
            // A file argument opens its folder with the file highlighted.
            start = if par.as_os_str().is_empty() { start } else { par.to_path_buf() };
            select = Some(name.to_string_lossy().into_owned());
        } else {
            eprintln!("lifefiles: {a}: no such path\n\n{USAGE}");
            std::process::exit(2);
        }
    }
    let start = std::fs::canonicalize(&start).unwrap_or(start);

    if let Err(e) = enable_raw_mode() {
        eprintln!("lifefiles: cannot enter raw mode ({e}); is this a terminal?");
        std::process::exit(1);
    }
    let _ = execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture);
    // A panic (panic=abort in release) must not leave the terminal in raw mode.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| {
        restore();
        prev(i);
    }));

    let mut term = match Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout())) {
        Ok(t) => t,
        Err(e) => {
            restore();
            eprintln!("lifefiles: {e}");
            std::process::exit(1);
        }
    };

    let mut app = app::App::new(start);
    if let Some(name) = select {
        let dir = app.cwd.clone();
        app.go(&dir, Some(name));
    }
    let mut dirty = true;
    while !app.quit {
        if dirty {
            if term.draw(|f| ui::draw(f, &mut app)).is_err() {
                break;
            }
            dirty = false;
        }
        // Short poll only while a load is in flight; otherwise wake rarely.
        let wait = if app.loading { 20 } else { 500 };
        match event::poll(Duration::from_millis(wait)) {
            Ok(true) => {
                match event::read() {
                    Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => app.on_key(k),
                    Ok(Event::Mouse(m)) => app.on_mouse(m),
                    Ok(_) => {}
                    Err(_) => break,
                }
                dirty = true;
            }
            Ok(false) => {}
            Err(_) => break,
        }
        if app.poll_loader() {
            dirty = true;
        }
    }
    restore();
}
