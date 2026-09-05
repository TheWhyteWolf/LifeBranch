// SPDX-License-Identifier: GPL-3.0-or-later
// The keyboard-driven settings UI (ratatui). Left: category list. Right: the
// selected category's fields. Every change previews live — cheap consumers are
// regenerated and pushed to the running session on each keystroke; disruptive
// respawns (wallpaper, swayidle) are deferred to commit (save/quit).
//
// All editing logic lives in model::Model; this file is only rendering + keys.
//
// Keys:  j/k ↑/↓ move · Tab switch pane · Enter edit/cycle · +/- or h/l adjust
//        s save · q save+quit · Esc cancel edit · Ctrl+C quit without saving

use crate::model::{field_labels, kind, Focus, Kind, Model, CATS};
use crate::paths::Paths;
use crate::theme::{rgb, Theme};

use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};

pub fn run(paths: Paths, theme: Theme) -> i32 {
    let mut m = Model::new(paths, theme);

    if let Err(e) = enable_raw_mode() {
        eprintln!("lifeconf: cannot enter raw mode ({e}); is this a terminal?");
        return 1;
    }
    let mut out = std::io::stdout();
    let _ = execute!(out, EnterAlternateScreen);
    let backend = ratatui::backend::CrosstermBackend::new(out);
    let mut term = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            eprintln!("lifeconf: terminal init failed: {e}");
            return 1;
        }
    };

    let code = event_loop(&mut term, &mut m);

    let _ = disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen);
    let _ = term.show_cursor();
    code
}

fn event_loop<B: ratatui::backend::Backend>(term: &mut Terminal<B>, m: &mut Model) -> i32 {
    while !m.quit {
        if term.draw(|f| draw(f, m)).is_err() {
            return 1;
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => handle_key(m, k.code, k.modifiers),
            Ok(_) => {}
            Err(_) => return 1,
        }
    }
    0
}

fn handle_key(m: &mut Model, code: KeyCode, mods: KeyModifiers) {
    // Editing mode: typing into a hex/text/number buffer.
    if let Some(buf) = m.editing.as_mut() {
        match code {
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter => {
                let s = m.editing.take().unwrap();
                m.set_text(&s);
            }
            KeyCode::Esc => {
                m.editing = None;
                m.status = "edit cancelled".into();
            }
            _ => {}
        }
        return;
    }

    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        if m.dirty {
            m.revert();
        }
        m.quit = true;
        return;
    }

    match code {
        KeyCode::Char('q') => {
            if m.dirty {
                m.commit();
            }
            m.quit = true;
        }
        KeyCode::Char('s') => m.commit(),
        KeyCode::Tab => m.toggle_focus(),
        KeyCode::Down | KeyCode::Char('j') => m.move_down(),
        KeyCode::Up | KeyCode::Char('k') => m.move_up(),
        KeyCode::Right | KeyCode::Char('l') => {
            if m.focus == Focus::Cats {
                m.focus = Focus::Fields;
            } else {
                m.nudge(1);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if m.focus == Focus::Fields {
                match m.kind_here() {
                    Kind::Hex | Kind::Text => m.focus = Focus::Cats,
                    _ => m.nudge(-1),
                }
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if m.focus == Focus::Fields {
                m.nudge(1);
            }
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            if m.focus == Focus::Fields {
                m.nudge(-1);
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => match m.focus {
            Focus::Cats => m.focus = Focus::Fields,
            Focus::Fields => match m.kind_here() {
                Kind::Hex | Kind::Text | Kind::Float(_) | Kind::Int(_) => m.begin_edit(),
                _ => m.nudge(1),
            },
        },
        _ => {}
    }
}

fn draw(f: &mut Frame, m: &Model) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(20)])
        .split(root[0]);

    draw_cats(f, m, cols[0]);
    draw_fields(f, m, cols[1]);

    let hint = if m.dirty { "  ● unsaved" } else { "" };
    let status = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {}", m.status), Style::default().fg(Color::Rgb(0x7b, 0x8c, 0x5a))),
        Span::styled(hint, Style::default().fg(Color::Rgb(0xc7, 0xd1, 0x7a))),
    ]));
    f.render_widget(status, root[1]);
}

fn accent() -> Style {
    Style::default().fg(Color::Rgb(0xa4, 0xc9, 0x4b)).add_modifier(Modifier::BOLD)
}

fn draw_cats(f: &mut Frame, m: &Model, area: Rect) {
    let items: Vec<ListItem> = CATS.iter().map(|c| ListItem::new(*c)).collect();
    let active = m.focus == Focus::Cats;
    let block = Block::default()
        .borders(Borders::ALL)
        .title("lifeconf")
        .border_style(if active { accent() } else { Style::default() });
    let list = List::default()
        .items(items)
        .block(block)
        .highlight_style(if active { accent() } else { Style::default().add_modifier(Modifier::REVERSED) })
        .highlight_symbol("› ");
    let mut st = ListState::default();
    st.select(Some(m.cat));
    f.render_stateful_widget(list, area, &mut st);
}

fn draw_fields(f: &mut Frame, m: &Model, area: Rect) {
    let active = m.focus == Focus::Fields;
    let labels = field_labels(m.cat);
    let mut rows: Vec<ListItem> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let editing_here = active && i == m.field && m.editing.is_some();
        let val = if editing_here {
            format!("{}▏", m.editing.as_deref().unwrap_or(""))
        } else {
            m.value(m.cat, i)
        };
        let mut spans = vec![
            Span::styled(format!("{label:<22}"), Style::default().fg(Color::Rgb(0x7b, 0x8c, 0x5a))),
            Span::raw(val),
        ];
        if matches!(kind(m.cat, i), Kind::Hex) {
            if let Some((r, g, b)) = rgb(&m.value(m.cat, i)) {
                spans.push(Span::raw("  "));
                spans.push(Span::styled("███", Style::default().fg(Color::Rgb(r, g, b))));
            }
        }
        rows.push(ListItem::new(Line::from(spans)));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(CATS[m.cat])
        .border_style(if active { accent() } else { Style::default() });
    let list = List::default()
        .items(rows)
        .block(block)
        .highlight_style(if active { accent() } else { Style::default() })
        .highlight_symbol("› ");
    let mut st = ListState::default();
    st.select(if active { Some(m.field) } else { None });
    f.render_stateful_widget(list, area, &mut st);
}
