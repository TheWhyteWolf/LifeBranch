// SPDX-License-Identifier: GPL-3.0-or-later
// What arrives on the FIFO, and the one line of text it becomes.
//
//   volume 45            label, percent
//   volume 45 muted      ...shown muted
//   brightness 9
//   72                   a bare number, as wob took it (label "level")

/// Characters in the OSD line.
pub const COLS: usize = 34;
/// Where the bar starts, after the label.
const BAR_AT: usize = 11;

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub label: String,
    pub pct: u32,
    pub muted: bool,
}

pub fn parse(line: &str) -> Option<Msg> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (label, rest) = match words.first()?.parse::<f64>() {
        Ok(_) => ("level", &words[..]),
        Err(_) => (words[0], &words[1..]),
    };
    let pct = rest.first()?.trim_end_matches('%').parse::<f64>().ok()?;
    if !pct.is_finite() {
        return None;
    }
    // Printable and short: it is drawn as-is.
    let label: String = label.chars().filter(|c| !c.is_control()).take(BAR_AT - 1).collect();
    Some(Msg { label, pct: pct.round().clamp(0.0, 999.0) as u32, muted: rest.get(1) == Some(&"muted") })
}

/// The last well-formed message in a chunk read from the FIFO: a held key
/// can queue several, and only the newest matters.
pub fn last(chunk: &str) -> Option<Msg> {
    chunk.lines().filter_map(parse).last()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Label,
    Bar,
    Track,
    Value,
}

/// `volume     ██████████░░░░░░░░░░  45%` as (text, role) runs.
pub fn line(m: &Msg) -> Vec<(String, Role)> {
    let label = format!("{:<w$}", m.label, w = BAR_AT);
    let tail = if m.muted { " muted".to_string() } else { format!(" {:>3}%", m.pct) };
    let len = COLS - BAR_AT - tail.chars().count();
    let filled = if m.muted { 0 } else { (m.pct.min(100) as usize * len + 50) / 100 };
    vec![
        (label, Role::Label),
        ("█".repeat(filled), Role::Bar),
        ("░".repeat(len - filled), Role::Track),
        (tail, Role::Value),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labelled_bare_and_muted() {
        assert_eq!(parse("volume 45"), Some(Msg { label: "volume".into(), pct: 45, muted: false }));
        assert_eq!(parse("volume 30 muted\n").map(|m| m.muted), Some(true));
        assert_eq!(parse("72").map(|m| (m.label, m.pct)), Some(("level".into(), 72)));
        assert_eq!(parse("brightness 9.6%").map(|m| m.pct), Some(10));
        assert_eq!(parse(""), None);
        assert_eq!(parse("volume"), None);
        assert_eq!(parse("volume loud"), None);
        assert_eq!(parse("volume NaN"), None);
        assert_eq!(parse("a-very-long-label-indeed 5").map(|m| m.label.len()), Some(10));
    }

    #[test]
    fn only_the_newest_message_counts() {
        assert_eq!(last("volume 40\nvolume 45\ngarbage\n").map(|m| m.pct), Some(45));
    }

    #[test]
    fn the_line_is_always_the_same_width() {
        for (pct, muted) in [(0, false), (45, false), (100, false), (150, false), (30, true)] {
            let l = line(&Msg { label: "volume".into(), pct, muted });
            let n: usize = l.iter().map(|(s, _)| s.chars().count()).sum();
            assert_eq!(n, COLS, "{pct} {muted}");
        }
        let full = line(&Msg { label: "volume".into(), pct: 100, muted: false });
        assert!(full[2].0.is_empty(), "100%: no track left");
        let muted = line(&Msg { label: "volume".into(), pct: 80, muted: true });
        assert!(muted[1].0.is_empty() && muted[3].0 == " muted");
    }
}
