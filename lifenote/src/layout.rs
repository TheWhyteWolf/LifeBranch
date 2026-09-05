// SPDX-License-Identifier: GPL-3.0-or-later
// Frame composition: notification content -> a styled character grid.
// The frame is real text (box-drawing glyphs), not a pixel border — that is
// the whole point of lifenote. Pure functions; unit-tested by string
// comparison, rasterized by render/.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Single,
    Rounded,
    Heavy,
    Double,
    Ascii,
}

/// What a cell is part of; resolved to a colour per urgency at render time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Border,
    Title,
    Summary,
    Body,
}

struct BorderSet {
    tl: char,
    tr: char,
    bl: char,
    br: char,
    h: char,
    v: char,
}

fn border_set(style: Style) -> BorderSet {
    match style {
        Style::Single => BorderSet { tl: '┌', tr: '┐', bl: '└', br: '┘', h: '─', v: '│' },
        Style::Rounded => BorderSet { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' },
        Style::Heavy => BorderSet { tl: '┏', tr: '┓', bl: '┗', br: '┛', h: '━', v: '┃' },
        Style::Double => BorderSet { tl: '╔', tr: '╗', bl: '╚', br: '╝', h: '═', v: '║' },
        Style::Ascii => BorderSet { tl: '+', tr: '+', bl: '+', br: '+', h: '-', v: '|' },
    }
}

pub type Grid = Vec<Vec<(char, Role)>>;

/// Compose one notification's frame. `summary`/`body` are pre-wrapped to at
/// most `max_width` columns (text::wrap); the frame hugs the longest line,
/// so total width is `inner + 4` (border, pad, text, pad, border).
/// Top border embeds the app name: `┌─ firefox ──────┐`.
pub fn compose(app: &str, summary: &[String], body: &[String], style: Style, max_width: usize) -> Grid {
    let b = border_set(style);
    let content_max = summary
        .iter()
        .chain(body.iter())
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    // Frame title, truncated with an ellipsis if it can't fit.
    let title: Vec<char> = app.trim().chars().collect();
    let want_title = title.len().min(max_width.saturating_sub(2)).max(0);
    let inner = content_max.clamp(want_title + 2, max_width.max(want_title + 2));
    let mut title = title;
    if title.len() > inner.saturating_sub(2) {
        title.truncate(inner.saturating_sub(2).max(1));
        if let Some(last) = title.last_mut() {
            *last = '…';
        }
    }

    let width = inner + 4;
    let mut grid: Grid = Vec::new();

    // Top border: tl h ' ' title ' ' h-fill tr — or a plain rule if no title.
    let mut top: Vec<(char, Role)> = Vec::with_capacity(width);
    top.push((b.tl, Role::Border));
    if title.is_empty() {
        top.extend(std::iter::repeat((b.h, Role::Border)).take(width - 2));
    } else {
        top.push((b.h, Role::Border));
        top.push((' ', Role::Border));
        top.extend(title.iter().map(|&c| (c, Role::Title)));
        top.push((' ', Role::Border));
        let used = 4 + title.len();
        top.extend(std::iter::repeat((b.h, Role::Border)).take(width - 1 - used));
    }
    top.push((b.tr, Role::Border));
    grid.push(top);

    let mut content_row = |text: &str, role: Role| {
        let mut row: Vec<(char, Role)> = Vec::with_capacity(width);
        row.push((b.v, Role::Border));
        row.push((' ', Role::Border));
        let mut n = 0;
        for c in text.chars() {
            row.push((c, role));
            n += 1;
        }
        row.extend(std::iter::repeat((' ', role)).take(inner - n));
        row.push((' ', Role::Border));
        row.push((b.v, Role::Border));
        grid.push(row);
    };

    for line in summary {
        content_row(line, Role::Summary);
    }
    if !body.is_empty() && !summary.is_empty() {
        content_row("", Role::Body); // spacer between summary and body
    }
    for line in body {
        content_row(line, Role::Body);
    }

    let mut bottom: Vec<(char, Role)> = Vec::with_capacity(width);
    bottom.push((b.bl, Role::Border));
    bottom.extend(std::iter::repeat((b.h, Role::Border)).take(width - 2));
    bottom.push((b.br, Role::Border));
    grid.push(bottom);

    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(g: &Grid) -> Vec<String> {
        g.iter().map(|row| row.iter().map(|&(c, _)| c).collect()).collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn single_with_title() {
        let g = compose("firefox", &s(&["Download finished"]), &s(&["rice.png saved"]), Style::Single, 40);
        assert_eq!(
            render(&g),
            vec![
                "┌─ firefox ─────────┐",
                "│ Download finished │",
                "│                   │",
                "│ rice.png saved    │",
                "└───────────────────┘",
            ]
        );
    }

    #[test]
    fn every_style_corners() {
        for (style, tl, br) in [
            (Style::Single, '┌', '┘'),
            (Style::Rounded, '╭', '╯'),
            (Style::Heavy, '┏', '┛'),
            (Style::Double, '╔', '╝'),
            (Style::Ascii, '+', '+'),
        ] {
            let g = compose("app", &s(&["hi"]), &[], style, 40);
            let r = render(&g);
            assert!(r[0].starts_with(tl), "{style:?}: {}", r[0]);
            assert!(r.last().unwrap().ends_with(br), "{style:?}");
        }
    }

    #[test]
    fn no_title_plain_rule() {
        let g = compose("", &s(&["hey"]), &[], Style::Single, 40);
        assert_eq!(render(&g), vec!["┌─────┐", "│ hey │", "└─────┘"]);
    }

    #[test]
    fn no_spacer_without_body() {
        let g = compose("x", &s(&["only summary"]), &[], Style::Single, 40);
        assert_eq!(render(&g).len(), 3);
    }

    #[test]
    fn rows_equal_width() {
        let g = compose("someapp", &s(&["a"]), &s(&["bb", "a much longer body line here"]), Style::Double, 40);
        let r = render(&g);
        let w = r[0].chars().count();
        assert!(r.iter().all(|row| row.chars().count() == w));
    }

    #[test]
    fn long_title_truncated() {
        let g = compose("a-ridiculously-long-application-name-oh-no", &s(&["hi"]), &[], Style::Single, 20);
        let r = render(&g);
        assert!(r[0].chars().count() <= 24);
        assert!(r[0].contains('…'));
    }
}
