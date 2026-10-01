// SPDX-License-Identifier: GPL-3.0-or-later
// xkb's own list of layouts, variants and options (evdev.lst, from
// xkeyboard-config), so the Keyboard panel can take a layout by name
// ("british", "dvorak") as well as by code, and check variants and options
// rather than only their characters.

pub const LIST: &str = "/usr/share/X11/xkb/rules/evdev.lst";

#[derive(Default)]
pub struct Xkb {
    /// (code, description)
    pub layouts: Vec<(String, String)>,
    /// (layout, code, description)
    pub variants: Vec<(String, String, String)>,
    /// (code, description); group headers like `grp` are left out.
    pub options: Vec<(String, String)>,
}

impl Xkb {
    pub fn load() -> Option<Xkb> {
        Some(Xkb::parse(&std::fs::read_to_string(LIST).ok()?))
    }

    pub fn parse(text: &str) -> Xkb {
        let mut x = Xkb::default();
        let mut section = "";
        for line in text.lines() {
            if let Some(s) = line.strip_prefix("! ") {
                section = s.trim();
                continue;
            }
            let line = line.trim();
            let Some((code, desc)) = line.split_once(char::is_whitespace) else { continue };
            let desc = desc.trim().to_string();
            match section {
                "layout" => x.layouts.push((code.into(), desc)),
                "variant" => {
                    if let Some((layout, d)) = desc.split_once(": ") {
                        x.variants.push((layout.into(), code.into(), d.into()));
                    }
                }
                "option" if code.contains(':') => x.options.push((code.into(), desc)),
                _ => {}
            }
        }
        x
    }
}

/// Pick one of `items` (code, description) for `want`: the code itself, else
/// the one description matching exactly, starting with it or containing it
/// (ignoring case). Several at the best level is an error naming a few.
fn resolve<'a>(want: &str, items: impl Iterator<Item = (&'a str, &'a str)> + Clone, what: &str) -> Result<String, String> {
    if let Some((c, _)) = items.clone().find(|(c, _)| *c == want) {
        return Ok(c.to_string());
    }
    let w = want.to_lowercase();
    let tests: [&dyn Fn(&str) -> bool; 3] = [&|d| d == w, &|d| d.starts_with(&w), &|d| d.contains(&w)];
    for t in tests {
        let hits: Vec<(&str, &str)> = items.clone().filter(|(_, d)| t(&d.to_lowercase())).collect();
        match hits.as_slice() {
            [] => continue,
            [(c, _)] => return Ok(c.to_string()),
            many => {
                let shown: Vec<String> = many.iter().take(4).map(|(c, d)| format!("{c} ({d})")).collect();
                let more = if many.len() > 4 { format!(" and {} more", many.len() - 4) } else { String::new() };
                return Err(format!("{want:?} could be {}{more}", shown.join(", ")));
            }
        }
    }
    Err(format!("no {what} {want:?}"))
}

/// A comma list of layouts, each a code or a name, as codes.
pub fn layouts(x: &Xkb, input: &str) -> Result<String, String> {
    let items = x.layouts.iter().map(|(c, d)| (c.as_str(), d.as_str()));
    let v: Result<Vec<String>, String> = input.split(',').map(|p| resolve(p.trim(), items.clone(), "layout")).collect();
    Ok(v?.join(","))
}

/// Variants, comma-parallel to `layouts` (an empty part keeps that layout's
/// default), each a code or a name within its own layout.
pub fn variants(x: &Xkb, layouts: &str, input: &str) -> Result<String, String> {
    let ls: Vec<&str> = layouts.split(',').map(str::trim).collect();
    let parts: Vec<&str> = input.split(',').map(str::trim).collect();
    if parts.len() > ls.len() {
        return Err(format!("{} variants for {} layout(s): one per layout, comma-separated", parts.len(), ls.len()));
    }
    let mut out = Vec::new();
    for (p, l) in parts.iter().zip(&ls) {
        if p.is_empty() {
            out.push(String::new());
            continue;
        }
        let items = x.variants.iter().filter(|(lay, _, _)| lay == l).map(|(_, c, d)| (c.as_str(), d.as_str()));
        out.push(resolve(p, items, &format!("{l} variant")).map_err(|e| format!("{e} (layout {l})"))?);
    }
    Ok(out.join(","))
}

/// Options must be codes (`ctrl:nocaps`): names repeat across groups. An
/// unknown one is refused with the codes whose names mention it.
pub fn options(x: &Xkb, input: &str) -> Result<String, String> {
    for p in input.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if x.options.iter().any(|(c, _)| c == p) {
            continue;
        }
        let w = p.to_lowercase();
        let near: Vec<&str> =
            x.options.iter().filter(|(_, d)| d.to_lowercase().contains(&w)).take(4).map(|(c, _)| c.as_str()).collect();
        return Err(if near.is_empty() {
            format!("no xkb option {p:?} (see {LIST})")
        } else {
            format!("no xkb option {p:?}; did you mean {}?", near.join(", "))
        });
    }
    Ok(input.split(',').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(","))
}

#[cfg(test)]
pub fn fixture() -> Xkb {
    Xkb::parse(
        "! model\n  pc105           Generic 105-key PC\n\n! layout\n  us              English (US)\n  gb              English (UK)\n  ru              Russian\n  de              German\n\n! variant\n  dvorak          us: English (Dvorak)\n  intl            us: English (US, intl., with dead keys)\n  dvorak          gb: English (UK, Dvorak)\n  extd            gb: English (UK, extended, Windows)\n  phonetic        ru: Russian (phonetic)\n\n! option\n  ctrl                 Ctrl position\n  ctrl:nocaps          Caps Lock as Ctrl\n  ctrl:swapcaps        Swap Ctrl and Caps Lock\n  compose:ralt         Right Alt\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_sections() {
        let x = fixture();
        assert_eq!(x.layouts.len(), 4);
        assert_eq!(x.variants[2], ("gb".into(), "dvorak".into(), "English (UK, Dvorak)".into()));
        assert_eq!(x.options.len(), 3, "the `ctrl` group header is not an option");
    }

    #[test]
    fn layouts_by_code_or_name() {
        let x = fixture();
        assert_eq!(layouts(&x, "gb").unwrap(), "gb");
        assert_eq!(layouts(&x, "english (uk), russian").unwrap(), "gb,ru");
        assert_eq!(layouts(&x, "germ").unwrap(), "de");
        let e = layouts(&x, "english").unwrap_err();
        assert!(e.contains("us (English (US))") && e.contains("gb (English (UK))"), "{e}");
        assert!(layouts(&x, "zz").unwrap_err().contains("no layout"));
    }

    #[test]
    fn variants_follow_their_layouts() {
        let x = fixture();
        assert_eq!(variants(&x, "gb", "dvorak").unwrap(), "dvorak");
        assert_eq!(variants(&x, "gb,ru", "extended,phonetic").unwrap(), "extd,phonetic");
        assert_eq!(variants(&x, "us,ru", ",phonetic").unwrap(), ",phonetic");
        assert!(variants(&x, "ru", "dvorak").unwrap_err().contains("layout ru"));
        assert!(variants(&x, "gb", "dvorak,intl").unwrap_err().contains("one per layout"));
    }

    #[test]
    fn options_are_codes_with_suggestions() {
        let x = fixture();
        assert_eq!(options(&x, "ctrl:nocaps, compose:ralt").unwrap(), "ctrl:nocaps,compose:ralt");
        let e = options(&x, "caps lock").unwrap_err();
        assert!(e.contains("ctrl:nocaps") && e.contains("ctrl:swapcaps"), "{e}");
        assert!(options(&x, "ctrl:nope").unwrap_err().contains("no xkb option"));
    }
}
