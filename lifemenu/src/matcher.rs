// SPDX-License-Identifier: GPL-3.0-or-later
// Fuzzy matching: the query's characters must appear in order (case-
// insensitive). Among matches, a contiguous run beats scattered letters, a hit
// at a word start beats one mid-word, and a prefix beats everything, so "fi"
// puts "Firefox" above "Profile". Greedy and linear: lists here are a few
// hundred entries, typed one key at a time.

/// Score and matched char positions, or None when `query` doesn't match.
pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let hay: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    // to_lowercase can expand a char (İ -> i̇); positions are only used for
    // highlighting, so fall back to no highlight rather than misplace it.
    let same_len = hay.len() == text.chars().count();
    let is_start = |i: usize| i == 0 || !hay[i - 1].is_alphanumeric();

    // Prefer starting at a word start that leads to a full match; try each
    // candidate start and keep the best-scoring greedy walk.
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in 0..hay.len() {
        if hay[start] != needle[0] {
            continue;
        }
        let Some(walk) = walk(&hay, &needle, start, &is_start) else { break };
        if best.as_ref().is_none_or(|b| walk.0 > b.0) {
            best = Some(walk);
        }
    }
    best.map(|(s, pos)| (s, if same_len { pos } else { Vec::new() }))
}

fn walk(hay: &[char], needle: &[char], start: usize, is_start: &dyn Fn(usize) -> bool) -> Option<(i32, Vec<usize>)> {
    let mut pos = Vec::with_capacity(needle.len());
    let mut s = 0i32;
    let mut i = start;
    let mut prev: Option<usize> = None;
    for &c in needle {
        while i < hay.len() && hay[i] != c {
            i += 1;
        }
        if i == hay.len() {
            return None;
        }
        s += 1;
        if prev == Some(i.wrapping_sub(1)) {
            s += 8; // contiguous
        } else if let Some(p) = prev {
            s -= ((i - p - 1) as i32).min(8); // gap
        }
        if is_start(i) {
            s += 6;
        }
        pos.push(i);
        prev = Some(i);
        i += 1;
    }
    if start == 0 {
        s += 12; // prefix
    }
    // Fewer unmatched characters wins a tie: "fi" over "Firefox" for "fi".
    s -= ((hay.len() - needle.len()) as i32 / 4).min(6);
    Some((s, pos))
}

/// Indices of `items` matching `query`, best first. Ties keep input order,
/// which is how the launcher's most-used-first ordering survives filtering.
pub fn filter<S: AsRef<str>>(query: &str, items: &[S]) -> Vec<usize> {
    let mut hits: Vec<(i32, usize)> =
        items.iter().enumerate().filter_map(|(i, t)| score(query, t.as_ref()).map(|(s, _)| (s, i))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_case_insensitive() {
        assert!(score("ffx", "Firefox").is_some());
        assert!(score("FOX", "firefox").is_some());
        assert!(score("xf", "firefox").is_none(), "order matters");
        assert_eq!(score("", "anything").unwrap().0, 0);
    }

    #[test]
    fn prefix_and_word_starts_rank_first() {
        let items = ["Profile Manager", "Firefox", "LibreOffice Impress", "fi"];
        let order: Vec<&str> = filter("fi", &items).into_iter().map(|i| items[i]).collect();
        assert_eq!(order[0], "fi", "exact prefix, shortest");
        assert_eq!(order[1], "Firefox");
        assert!(order[2..].contains(&"Profile Manager"), "mid-word hits after prefixes");
        let items = ["system settings", "kitty terminal"];
        assert_eq!(filter("term", &items), vec![1]);
    }

    #[test]
    fn positions_mark_the_highlight() {
        let (_, pos) = score("lo", "LibreOffice").unwrap();
        assert_eq!(pos, vec![0, 5]);
        let (_, pos) = score("ff", "Firefox").unwrap();
        assert_eq!(pos.len(), 2);
    }

    #[test]
    fn ties_keep_input_order() {
        let items = ["alpha one", "alpha two"];
        assert_eq!(filter("alpha", &items), vec![0, 1]);
    }
}
