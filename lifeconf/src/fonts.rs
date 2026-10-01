// SPDX-License-Identifier: GPL-3.0-or-later
// The font families Settings > Font offers: monospace (or dual-width, as the
// non-"Mono" Nerd Fonts are) and covering English. Everything the font
// setting drives, the bar, lifemenu, lifenote and kitty, lays text out on a
// fixed grid, so a proportional font would only look broken there.

pub fn installed_mono() -> Vec<String> {
    let mut text = String::new();
    for spacing in ["mono", "90"] {
        if let Ok(o) = std::process::Command::new("fc-list").args([&format!(":spacing={spacing}:lang=en"), "family"]).output() {
            text.push_str(&String::from_utf8_lossy(&o.stdout));
        }
    }
    families(&text)
}

/// fc-list's `family` output, one face per line with its family's names
/// comma-separated: the first name of each, sorted, once each.
pub fn families(fc_list: &str) -> Vec<String> {
    let mut v: Vec<String> = fc_list
        .lines()
        .filter_map(|l| l.split(',').next())
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    v.sort_by_key(|f| f.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_name_of_each_family_once() {
        let out = "Hack\nAtkynsonMono Nerd Font Mono,AtkynsonMono NFM\nAtkynsonMono Nerd Font Mono,AtkynsonMono NFM,AtkynsonMono NFM Light\nadwaita Mono\n\n";
        assert_eq!(super::families(out), ["adwaita Mono", "AtkynsonMono Nerd Font Mono", "Hack"]);
    }
}
