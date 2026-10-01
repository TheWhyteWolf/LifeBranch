// SPDX-License-Identifier: GPL-3.0-or-later
// Drawing. Reads App, and writes back only Hits (where things landed) plus the
// scroll offsets, which depend on the pane height known only at draw time.
// The look is yazi's: columns without borders, selection as a filled row, and
// pure-text markers ("/" for folders) to match the rest of the rice.

use crate::app::{App, Mode, Preview};
use crate::fs::{human_size, Entry};
use crate::theme::Colors;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::path::PathBuf;

/// Truncate to `w` columns with an ellipsis, then pad to exactly `w`.
fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n > w {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        if w > 0 {
            t.push('…');
        }
        t
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

fn row_style(c: &Colors, e: &Entry) -> Style {
    if e.is_dir {
        Style::default().fg(c.accent).add_modifier(Modifier::BOLD)
    } else if e.hidden() {
        Style::default().fg(c.border)
    } else {
        Style::default().fg(c.text)
    }
}

fn entry_line(app: &App, c: &Colors, e: &Entry, w: usize, selected: bool, sizes: bool) -> Line<'static> {
    let marked = app.marks.contains(&e.path);
    let cut = app.clip.as_ref().is_some_and(|(p, cut)| *cut && p.contains(&e.path));
    let drop_here = app.drag.as_ref().is_some_and(|d| d.hover.as_ref() == Some(&e.path));
    let mut name = e.name.clone();
    if e.is_dir {
        name.push('/');
    } else if e.is_link {
        name.push('@');
    }
    let size = if sizes && !e.is_dir { human_size(e.size) } else { String::new() };
    let size_w = if sizes { 7 } else { 0 };
    let name_w = w.saturating_sub(3 + size_w);
    let text = format!(" {}{}{:>sw$} ", if marked { '*' } else { ' ' }, fit(&name, name_w), size, sw = size_w);
    let mut st = row_style(c, e);
    if cut {
        st = st.add_modifier(Modifier::DIM);
    }
    if marked {
        st = st.fg(c.warn);
    }
    if selected {
        st = st.bg(c.border).add_modifier(Modifier::BOLD);
    }
    if drop_here {
        st = st.bg(c.accent).fg(c.surface);
    }
    Line::from(Span::styled(fit(&text, w), st))
}

fn columns(w: u16) -> (u16, u16, u16) {
    let places = if w >= 60 { 16 } else { 0 };
    let parent = if w >= 80 { (w / 5).clamp(14, 28) } else { 0 };
    let preview = if w >= 110 { w * 3 / 10 } else { 0 };
    (places, parent, preview)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let c = Colors::load_cached();
    let area = f.area();
    if area.width < 20 || area.height < 4 {
        return;
    }
    let (pw, parw, prevw) = columns(area.width);
    let body_h = area.height - 2;
    let body_y = 1;
    let list_w = area.width - pw - parw - prevw;
    let list = Rect::new(pw + parw, body_y, list_w, body_h);
    app.hits.list = list;
    app.hits.height = body_h as usize;

    draw_crumbs(f, app, &c, Rect::new(0, 0, area.width, 1));

    // ---- places ----
    app.hits.places.clear();
    if pw > 0 {
        let mut lines = Vec::new();
        for (i, (name, path)) in app.places.iter().enumerate().take(body_h as usize) {
            let active = *path == app.cwd;
            let hover = app.drag.as_ref().is_some_and(|d| d.hover.as_ref() == Some(path));
            let mut st = Style::default().fg(if active { c.accent } else { c.text });
            if active {
                st = st.add_modifier(Modifier::BOLD);
            }
            if hover {
                st = st.bg(c.accent).fg(c.surface);
            }
            lines.push(Line::from(Span::styled(fit(&format!(" {name}"), pw as usize - 1), st)));
            app.hits.places.push((Rect::new(0, body_y + i as u16, pw - 1, 1), path.clone()));
        }
        f.render_widget(Paragraph::new(lines), Rect::new(0, body_y, pw, body_h));
    }

    // ---- parent column ----
    app.hits.parent = Rect::default();
    if parw > 0 {
        let rect = Rect::new(pw, body_y, parw, body_h);
        let cur_name = app.cwd.file_name().map(|n| n.to_string_lossy().into_owned());
        let at = cur_name
            .as_ref()
            .and_then(|n| app.parent_view.iter().position(|&i| &app.parent_entries[i].name == n))
            .unwrap_or(0);
        // Keep the folder we're in visible, roughly centred.
        let scroll = at.saturating_sub(body_h as usize / 2);
        app.hits.parent_scroll = scroll;
        app.hits.parent = Rect::new(rect.x, rect.y, rect.width - 1, rect.height);
        let lines: Vec<Line> = app
            .parent_view
            .iter()
            .skip(scroll)
            .take(body_h as usize)
            .map(|&i| {
                let e = &app.parent_entries[i];
                let sel = Some(&e.name) == cur_name.as_ref();
                entry_line(app, &c, e, parw as usize - 1, sel, false)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), rect);
    }

    // ---- current list ----
    let h = body_h as usize;
    if app.follow {
        if app.sel < app.scroll {
            app.scroll = app.sel;
        } else if app.sel >= app.scroll + h {
            app.scroll = app.sel + 1 - h;
        }
    }
    app.scroll = app.scroll.min(app.view.len().saturating_sub(h));
    let lines: Vec<Line> = if app.view.is_empty() {
        let note = if app.loading {
            "loading…"
        } else if app.filter.is_empty() {
            "(empty)"
        } else {
            "(no match)"
        };
        vec![Line::from(Span::styled(format!(" {note}"), Style::default().fg(c.border)))]
    } else {
        (app.scroll..(app.scroll + h).min(app.view.len()))
            .map(|i| {
                let e = &app.entries[app.view[i]];
                entry_line(app, &c, e, list_w as usize - 1, i == app.sel, true)
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), list);

    // ---- preview ----
    if prevw > 0 {
        let rect = Rect::new(area.width - prevw, body_y, prevw, body_h);
        let w = prevw as usize - 1;
        let dim = Style::default().fg(c.border);
        let lines: Vec<Line> = match &app.preview {
            Preview::None => vec![],
            Preview::Note(n) => vec![Line::from(Span::styled(format!(" {n}"), dim))],
            Preview::Dir(v) => v
                .iter()
                .take(h)
                .map(|n| {
                    let st = if n.ends_with('/') { Style::default().fg(c.accent) } else { Style::default().fg(c.text) };
                    Line::from(Span::styled(fit(&format!(" {n}"), w), st))
                })
                .collect(),
            Preview::Text(v) => v
                .iter()
                .take(h)
                .map(|l| Line::from(Span::styled(fit(&format!(" {l}"), w), Style::default().fg(c.text))))
                .collect(),
        };
        f.render_widget(Paragraph::new(lines), rect);
    }

    draw_status(f, app, &c, Rect::new(0, area.height - 1, area.width, 1));
    draw_overlay(f, app, &c, area);
}

fn draw_crumbs(f: &mut Frame, app: &mut App, c: &Colors, r: Rect) {
    app.hits.crumbs.clear();
    let mut spans = Vec::new();
    let mut x = r.x + 1;
    spans.push(Span::raw(" "));
    let mut acc = PathBuf::from("/");
    let comps: Vec<String> =
        app.cwd.components().skip(1).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let mut segs: Vec<(String, PathBuf)> = vec![("/".into(), acc.clone())];
    for s in comps {
        acc.push(&s);
        segs.push((s, acc.clone()));
    }
    // Long paths: drop leading segments until it fits.
    let budget = r.width as usize - 2;
    let total = |v: &[(String, PathBuf)]| v.iter().map(|(n, _)| n.chars().count() + 1).sum::<usize>();
    let mut skip = 0;
    while total(&segs[skip..]) > budget && skip + 1 < segs.len() {
        skip += 1;
    }
    let last = segs.len() - 1;
    for (i, (name, path)) in segs.into_iter().enumerate().skip(skip) {
        let w = name.chars().count() as u16;
        let hover = app.drag.as_ref().is_some_and(|d| d.hover.as_ref() == Some(&path));
        let mut st = Style::default().fg(if i == last { c.accent } else { c.text });
        if i == last {
            st = st.add_modifier(Modifier::BOLD);
        }
        if hover {
            st = st.bg(c.accent).fg(c.surface);
        }
        app.hits.crumbs.push((Rect::new(x, r.y, w, 1), path));
        spans.push(Span::styled(name.clone(), st));
        x += w;
        if i != last && name != "/" {
            spans.push(Span::styled("/", Style::default().fg(c.border)));
            x += 1;
        } else if i != last {
            // "/" root segment already is the separator
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), r);
}

fn draw_status(f: &mut Frame, app: &App, c: &Colors, r: Rect) {
    let w = r.width as usize;
    let line = match &app.mode {
        Mode::Input(i) => {
            let label = match i.kind {
                crate::app::InputKind::Rename(_) => "rename",
                crate::app::InputKind::Path => "go to",
                crate::app::InputKind::Filter => "filter",
                crate::app::InputKind::NewDir => "new folder",
                crate::app::InputKind::SaveAs => "save as",
            };
            let text: String = i.buf.iter().collect();
            Line::from(vec![
                Span::styled(format!(" {label}: "), Style::default().fg(c.accent)),
                Span::styled(text, Style::default().fg(c.text)),
            ])
        }
        Mode::Confirm(p) => Line::from(Span::styled(
            format!(" delete {} item(s) PERMANENTLY? y/n", p.len()),
            Style::default().fg(c.warn).add_modifier(Modifier::BOLD),
        )),
        Mode::Overwrite(p) => Line::from(Span::styled(
            format!(" {} exists. Replace it? y/n", p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
            Style::default().fg(c.warn).add_modifier(Modifier::BOLD),
        )),
        _ => {
            let left = if !app.msg.is_empty() {
                format!(" {}", app.msg)
            } else {
                let mut s = format!(" {}/{}", if app.view.is_empty() { 0 } else { app.sel + 1 }, app.view.len());
                if !app.marks.is_empty() {
                    s += &format!("  {} marked", app.marks.len());
                }
                if !app.filter.is_empty() {
                    s += &format!("  filter: {}", app.filter);
                }
                if app.show_hidden {
                    s += "  [hidden]";
                }
                if let Some((p, cut)) = &app.clip {
                    s += &format!("  {} {}", if *cut { "cut" } else { "copied" }, p.len());
                }
                s
            };
            let hint = match &app.pick {
                None => "F2 rename  Del trash  Ctrl+L path  / filter  q quit ".to_string(),
                Some(p) => {
                    let what = if p.directory {
                        "Ctrl+S choose this folder"
                    } else if p.save.is_some() {
                        "Ctrl+S save here"
                    } else if p.multiple {
                        "Enter choose  Space mark  Ctrl+S choose marked"
                    } else {
                        "Enter choose"
                    };
                    let title = if p.title.is_empty() { String::new() } else { format!("{}: ", p.title) };
                    format!("{title}{what}  Esc cancel ")
                }
            };
            let gap = w.saturating_sub(left.chars().count() + hint.len());
            let right = if gap > 0 { format!("{}{hint}", " ".repeat(gap)) } else { String::new() };
            let st = if app.msg.is_empty() { Style::default().fg(c.text) } else { Style::default().fg(c.warn) };
            Line::from(vec![Span::styled(left, st), Span::styled(right, Style::default().fg(c.border))])
        }
    };
    f.render_widget(Paragraph::new(line), r);
}

fn draw_overlay(f: &mut Frame, app: &mut App, c: &Colors, area: Rect) {
    app.hits.menu.clear();
    match &app.mode {
        Mode::Input(i) => {
            // Terminal cursor sits at the edit point in the status row.
            let label = match i.kind {
                crate::app::InputKind::Rename(_) => "rename",
                crate::app::InputKind::Path => "go to",
                crate::app::InputKind::Filter => "filter",
                crate::app::InputKind::NewDir => "new folder",
                crate::app::InputKind::SaveAs => "save as",
            };
            let x = (label.len() + 3 + i.cur) as u16;
            f.set_cursor_position((x.min(area.width - 1), area.height - 1));
        }
        Mode::Menu(m) => {
            let w = m.items.iter().map(|(l, _)| l.len()).max().unwrap_or(0) as u16 + 4;
            let h = m.items.len() as u16;
            let x = m.pos.0.min(area.width.saturating_sub(w));
            let y = m.pos.1.min(area.height.saturating_sub(h));
            let rect = Rect::new(x, y, w, h);
            app.hits.menu_rect = rect;
            f.render_widget(Clear, rect);
            let mut lines = Vec::new();
            let mut hits = Vec::new();
            for (i, (label, _)) in m.items.iter().enumerate() {
                let st = if i == m.sel {
                    Style::default().fg(c.accent).bg(c.border).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(c.text).bg(c.surface)
                };
                lines.push(Line::from(Span::styled(fit(&format!("  {label}"), w as usize), st)));
                hits.push((Rect::new(x, y + i as u16, w, 1), i));
            }
            f.render_widget(Paragraph::new(lines), rect);
            app.hits.menu = hits;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;

    fn screen(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn settle(app: &mut App) {
        for _ in 0..200 {
            if app.poll_loader() && !app.loading {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("loader never finished");
    }

    fn click(app: &mut App, x: u16, y: u16, kind: MouseEventKind) {
        app.on_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
    }

    #[test]
    fn draws_listing_and_mouse_hits_map_to_rows() {
        let d = std::env::temp_dir().join(format!("lifefiles-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("folder")).unwrap();
        std::fs::create_dir_all(d.join("other")).unwrap();
        std::fs::write(d.join("note.txt"), "preview me").unwrap();
        let mut app = App::new(d.clone());
        settle(&mut app);
        let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let s = screen(&term);
        assert!(s.contains("folder/") && s.contains("note.txt"));
        assert!(s.contains("Home")); // places sidebar at this width

        // Click the second row (other/), then drag it onto the first (folder/).
        let list = app.hits.list;
        click(&mut app, list.x + 3, list.y + 1, MouseEventKind::Down(MouseButton::Left));
        assert_eq!(app.current().unwrap().name, "other");
        click(&mut app, list.x + 3, list.y, MouseEventKind::Drag(MouseButton::Left));
        click(&mut app, list.x + 3, list.y, MouseEventKind::Up(MouseButton::Left));
        settle(&mut app);
        let canon = d.canonicalize().unwrap();
        assert!(canon.join("folder/other").is_dir(), "drag-drop should move other/ into folder/");

        // Double-click opens the folder.
        term.draw(|f| draw(f, &mut app)).unwrap();
        click(&mut app, list.x + 3, list.y, MouseEventKind::Down(MouseButton::Left));
        click(&mut app, list.x + 3, list.y, MouseEventKind::Up(MouseButton::Left));
        click(&mut app, list.x + 3, list.y, MouseEventKind::Down(MouseButton::Left));
        settle(&mut app);
        assert_eq!(app.cwd, canon.join("folder"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut app = App::new(std::env::temp_dir());
        let mut term = Terminal::new(TestBackend::new(10, 3)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
    }
}
