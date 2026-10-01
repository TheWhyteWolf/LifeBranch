// SPDX-License-Identifier: GPL-3.0-or-later
// The menu as plain state: the query, the filtered list, the selection and the
// scroll window. No Wayland here, so every key's effect is unit-testable.

use crate::matcher;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Keep going (a key that only changed the state).
    Stay,
    /// Enter on a list entry: its index in the input.
    Pick(usize),
    /// Enter with nothing picked: what was typed (dmenu prints it).
    Text(String),
}

pub struct Menu {
    /// What the matcher searches, one per entry (for apps: name + keywords).
    hay: Vec<String>,
    pub query: String,
    /// Indices into the entries, best match first.
    pub shown: Vec<usize>,
    pub sel: usize,
    pub scroll: usize,
    /// List rows on screen.
    pub rows: usize,
    only_match: bool,
}

impl Menu {
    pub fn new(hay: Vec<String>, rows: usize, only_match: bool) -> Menu {
        let mut m = Menu { hay, query: String::new(), shown: Vec::new(), sel: 0, scroll: 0, rows, only_match };
        m.refilter();
        m
    }

    fn refilter(&mut self) {
        self.shown = matcher::filter(&self.query, &self.hay);
        self.sel = 0;
        self.scroll = 0;
    }

    /// Typed text. Control characters (Tab, Return as text, ...) never enter
    /// the query: they would poison the filter.
    pub fn type_str(&mut self, s: &str) {
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if !clean.is_empty() {
            self.query.push_str(&clean);
            self.refilter();
        }
    }

    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.refilter();
        }
    }

    /// Ctrl+Backspace / Ctrl+W: trailing spaces, then the word before them.
    pub fn delete_word(&mut self) {
        let trimmed = self.query.trim_end().len();
        let cut = self.query[..trimmed].rfind(char::is_whitespace).map_or(0, |i| i + 1);
        if cut != self.query.len() {
            self.query.truncate(cut);
            self.refilter();
        }
    }

    pub fn clear(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.refilter();
        }
    }

    pub fn select(&mut self, sel: usize) {
        if self.shown.is_empty() {
            return;
        }
        self.sel = sel.min(self.shown.len() - 1);
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if self.rows > 0 && self.sel >= self.scroll + self.rows {
            self.scroll = self.sel + 1 - self.rows;
        }
    }

    /// Up/Down wrap around, as fuzzel's do.
    pub fn up(&mut self) {
        let n = self.shown.len();
        if n > 0 {
            self.select(if self.sel == 0 { n - 1 } else { self.sel - 1 });
        }
    }

    pub fn down(&mut self) {
        let n = self.shown.len();
        if n > 0 {
            self.select(if self.sel + 1 >= n { 0 } else { self.sel + 1 });
        }
    }

    pub fn page(&mut self, down: bool) {
        let step = self.rows.max(1);
        let target = if down { self.sel + step } else { self.sel.saturating_sub(step) };
        self.select(target);
    }

    /// Mouse wheel: move the window, keeping the selection inside it.
    pub fn scroll_by(&mut self, d: isize) {
        let max = self.shown.len().saturating_sub(self.rows);
        self.scroll = (self.scroll as isize + d).clamp(0, max as isize) as usize;
        self.sel = self.sel.clamp(self.scroll, (self.scroll + self.rows).saturating_sub(1).min(self.shown.len().saturating_sub(1)));
    }

    /// The window of `shown` on screen, as (position in shown, entry index).
    pub fn visible(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.shown.iter().copied().enumerate().skip(self.scroll).take(self.rows)
    }

    pub fn enter(&self) -> Outcome {
        if let Some(&i) = self.shown.get(self.sel).filter(|_| self.rows > 0) {
            return Outcome::Pick(i);
        }
        if self.only_match && !self.hay.is_empty() {
            return Outcome::Stay;
        }
        Outcome::Text(self.query.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(items: &[&str], rows: usize) -> Menu {
        Menu::new(items.iter().map(|s| s.to_string()).collect(), rows, false)
    }

    #[test]
    fn typing_filters_and_resets_selection() {
        let mut m = menu(&["Lock", "Suspend", "Log out", "Reboot", "Power off"], 5);
        m.down();
        assert_eq!(m.sel, 1);
        m.type_str("lo");
        assert_eq!(m.shown, vec![0, 2], "Lock, Log out");
        assert_eq!(m.sel, 0);
        assert_eq!(m.enter(), Outcome::Pick(0));
        m.backspace();
        m.backspace();
        assert_eq!(m.shown.len(), 5);
    }

    #[test]
    fn control_characters_never_reach_the_query() {
        let mut m = menu(&["a"], 1);
        m.type_str("\t");
        m.type_str("x\ny");
        assert_eq!(m.query, "xy");
    }

    #[test]
    fn enter_without_a_match() {
        let mut m = menu(&["alpha"], 3);
        m.type_str("zzz");
        assert_eq!(m.enter(), Outcome::Text("zzz".into()), "dmenu prints the typed text");
        let mut strict = Menu::new(vec!["alpha".into()], 3, true);
        strict.type_str("zzz");
        assert_eq!(strict.enter(), Outcome::Stay, "--only-match");
        let mut pw = Menu::new(Vec::new(), 0, true);
        pw.type_str("hunter2");
        assert_eq!(pw.enter(), Outcome::Text("hunter2".into()), "a bare input box returns its text");
    }

    #[test]
    fn selection_scrolls_the_window_and_wraps() {
        let mut m = menu(&["a", "b", "c", "d", "e"], 2);
        m.down();
        m.down();
        assert_eq!((m.sel, m.scroll), (2, 1));
        m.up();
        m.up();
        m.up();
        assert_eq!((m.sel, m.scroll), (4, 3), "wrapped to the bottom");
        let vis: Vec<_> = m.visible().map(|(_, i)| i).collect();
        assert_eq!(vis, vec![3, 4]);
        m.page(false);
        assert_eq!(m.sel, 2);
        m.scroll_by(-5);
        assert_eq!((m.scroll, m.sel), (0, 1), "selection kept inside the window");
    }

    #[test]
    fn word_and_line_deletion() {
        let mut m = menu(&[], 0);
        m.type_str("open the  ");
        m.delete_word();
        assert_eq!(m.query, "open ");
        m.delete_word();
        assert_eq!(m.query, "");
        m.type_str("abc");
        m.clear();
        assert_eq!(m.query, "");
    }
}
