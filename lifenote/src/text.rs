// SPDX-License-Identifier: GPL-3.0-or-later
// Body text preparation: Pango-markup stripping and word wrap.
// We advertise only the "body" capability (no body-markup), but notify-send
// clients ship markup regardless, so tags are stripped defensively.
// Column math is chars().count() — monospace-naive; CJK/emoji double-width
// is a documented non-goal (see README).

/// Drop `<...>` tag runs and decode the five XML entities plus numeric forms.
/// Unterminated tags are dropped to end-of-string (a truncated tag is noise
/// either way); unknown entities pass through literally.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for t in chars.by_ref() {
                    if t == '>' {
                        break;
                    }
                }
            }
            '&' => {
                // Collect up to the ';' (entities are short; cap the scan).
                let mut ent = String::new();
                let mut terminated = false;
                while let Some(&n) = chars.peek() {
                    if n == ';' {
                        chars.next();
                        terminated = true;
                        break;
                    }
                    if ent.len() >= 8 || n == '&' || n == '<' {
                        break;
                    }
                    ent.push(n);
                    chars.next();
                }
                if !terminated {
                    out.push('&');
                    out.push_str(&ent);
                    continue;
                }
                match ent.as_str() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {
                        let code = ent
                            .strip_prefix("#x")
                            .or_else(|| ent.strip_prefix("#X"))
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                        match code.and_then(char::from_u32) {
                            Some(ch) => out.push(ch),
                            None => {
                                // Not an entity we know — emit verbatim.
                                out.push('&');
                                out.push_str(&ent);
                                out.push(';');
                            }
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// One-line form for the history ring: markup stripped, all whitespace
/// (newlines included) collapsed to single spaces. `ctl history` is consumed
/// line-per-entry (the fuzzel menu), so an embedded '\n' must not split a
/// record and raw Pango tags must not diverge from what the popup showed.
pub fn one_line(s: &str) -> String {
    strip_markup(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Word-wrap to `cols` columns, honoring embedded newlines. Words longer
/// than `cols` are hard-broken. Never returns an empty Vec for empty input —
/// the caller decides whether to render anything.
pub fn wrap(s: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        let mut line_len = 0usize;
        for word in para.split_whitespace() {
            let wlen = word.chars().count();
            if line_len > 0 && line_len + 1 + wlen <= cols {
                line.push(' ');
                line.push_str(word);
                line_len += 1 + wlen;
            } else if line_len == 0 && wlen <= cols {
                line.push_str(word);
                line_len = wlen;
            } else {
                if line_len > 0 {
                    lines.push(std::mem::take(&mut line));
                }
                // Hard-break an over-long word across full lines.
                let mut rest: Vec<char> = word.chars().collect();
                while rest.len() > cols {
                    lines.push(rest.drain(..cols).collect());
                }
                line = rest.into_iter().collect();
                line_len = line.chars().count();
            }
        }
        lines.push(line);
    }
    // Trim trailing blank lines (bodies often end with '\n').
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_entities() {
        assert_eq!(strip_markup("<b>bold</b> &amp; <i>x</i>"), "bold & x");
        assert_eq!(strip_markup("a &lt;tag&gt; &quot;q&quot; &apos;s&apos;"), "a <tag> \"q\" 's'");
        assert_eq!(strip_markup("&#65;&#x42;"), "AB");
        assert_eq!(strip_markup("plain text"), "plain text");
        assert_eq!(strip_markup("<a href=\"x>y\">link</a>"), "y\">link");
    }

    #[test]
    fn passes_unknown_entities_and_bare_amp() {
        assert_eq!(strip_markup("fish &chips; now"), "fish &chips; now");
        assert_eq!(strip_markup("R&D dept"), "R&D dept");
        assert_eq!(strip_markup("trailing &"), "trailing &");
        assert_eq!(strip_markup("&#xZZ;"), "&#xZZ;");
    }

    #[test]
    fn drops_unterminated_tag() {
        assert_eq!(strip_markup("pre <b unterminated"), "pre ");
    }

    #[test]
    fn wraps_words() {
        assert_eq!(wrap("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap("short", 40), vec!["short"]);
    }

    #[test]
    fn wraps_newlines_and_long_words() {
        assert_eq!(wrap("a\nb", 10), vec!["a", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("x abcdefgh y", 4), vec!["x", "abcd", "efgh", "y"]);
    }

    #[test]
    fn empty_and_trailing_blank() {
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("a\n\n", 10), vec!["a"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    }
}
