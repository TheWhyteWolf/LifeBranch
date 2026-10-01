// SPDX-License-Identifier: GPL-3.0-or-later
// Region & language panel, over localectl: the system language (LANG) and the
// formats for dates, numbers, money, measurements and paper (the LC_*
// categories, set together), picked from the locales that are generated on
// this machine. A locale that isn't generated yet can be added: it is
// uncommented in /etc/locale.gen and locale-gen runs, both as root through
// pkexec (lifeauth asks for the password).
//
// Changes are system-wide and reach programs at the next login, which the
// state row says. The keyboard layout is the Keyboard panel's.

use super::{pick, Change, Row, RowKind, Runner};

pub const LABELS: &[&str] = &["language", "formats (dates, numbers, money)", "applies", "add a language"];

/// The format categories "formats" sets together; LANG covers the rest.
const FORMATS: &[&str] = &["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER"];

pub fn kind(field: usize) -> RowKind {
    match field {
        0 | 1 => RowKind::Choice,
        2 => RowKind::Info,
        _ => RowKind::Text,
    }
}

/// `localectl status`'s System Locale lines -> (VAR, value) pairs.
pub fn parse_status(out: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut in_locale = false;
    for line in out.lines() {
        let t = line.trim();
        let rest = if let Some(r) = t.strip_prefix("System Locale:") {
            in_locale = true;
            r.trim()
        } else if in_locale && !t.contains(": ") {
            t
        } else {
            in_locale = false;
            continue;
        };
        if let Some((k, val)) = rest.split_once('=') {
            v.push((k.to_string(), val.to_string()));
        }
    }
    v
}

/// The generated locales, by code (what localectl takes). Naming them
/// ("English, United Kingdom") would need ICU's tables; codes it is.
fn choices(locales: &[String]) -> Vec<(String, String)> {
    locales.iter().filter(|l| l.as_str() != "C" && l.as_str() != "POSIX").map(|l| (l.clone(), l.clone())).collect()
}

/// Locale names end up in a root shell line: only the characters they use.
pub fn locale_ok(s: &str) -> bool {
    !s.is_empty() && s.len() < 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
}

/// The /etc/locale.gen line for `name`, from glibc's SUPPORTED list
/// ("de_DE.UTF-8 UTF-8").
pub fn supported_line(supported: &str, name: &str) -> Option<String> {
    supported.lines().map(str::trim).find(|l| l.split_whitespace().next() == Some(name)).map(str::to_string)
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let Ok(status) = run("localectl", &["status"]) else {
        return vec![r("localectl unavailable"), r("-"), r("-"), r("-")];
    };
    let vars = parse_status(&status);
    let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let lang = get("LANG").unwrap_or_else(|| "C".into());
    let formats = get("LC_TIME").unwrap_or_else(|| lang.clone());
    let locales: Vec<String> = run("localectl", &["list-locales"])
        .map(|o| o.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    let c = choices(&locales);
    vec![
        Row { value: lang, choices: c.clone() },
        Row { value: formats, choices: c },
        r("at your next login (system-wide)"),
        r("type a locale like de_DE.UTF-8 to generate it"),
    ]
}

/// The full LANG/LC_* set after changing `keys` to `value`, keeping the rest:
/// localectl set-locale replaces the whole set, so it must be passed whole.
pub fn new_set(current: &[(String, String)], keys: &[&str], value: &str) -> Vec<String> {
    let mut v: Vec<(String, String)> = current.iter().filter(|(k, _)| !keys.contains(&k.as_str())).cloned().collect();
    for k in keys {
        v.push((k.to_string(), value.to_string()));
    }
    // LC_* equal to LANG are noise; drop them.
    let lang = v.iter().find(|(k, _)| k == "LANG").map(|(_, x)| x.clone());
    v.retain(|(k, x)| k == "LANG" || Some(x) != lang.as_ref());
    v.sort();
    v.into_iter().map(|(k, x)| format!("{k}={x}")).collect()
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        0 | 1 => {
            let (_, id) = pick(row, &ch, "locale")?;
            if !locale_ok(id) {
                return Err(format!("odd locale name {id:?}"));
            }
            let current = parse_status(&run("localectl", &["status"])?);
            let keys: &[&str] = if field == 0 { &["LANG"] } else { FORMATS };
            let set = new_set(&current, keys, id);
            let mut args = vec!["set-locale"];
            args.extend(set.iter().map(String::as_str));
            run("localectl", &args)?;
            Ok(format!("{} {id} (at your next login)", if field == 0 { "language" } else { "formats" }))
        }
        3 => {
            let Change::Text(t) = ch else { return Err("type a locale name, then Enter".into()) };
            let name = t.trim();
            if !locale_ok(name) {
                return Err(format!("{name:?} isn't a locale name"));
            }
            let supported = std::fs::read_to_string("/usr/share/i18n/SUPPORTED").map_err(|e| format!("glibc's locale list: {e}"))?;
            let line = supported_line(&supported, name).ok_or_else(|| format!("{name} isn't a locale glibc knows (see /usr/share/i18n/SUPPORTED)"))?;
            // Uncomment it if it's there, append it if not, then generate.
            let script = format!(
                "grep -qx '{line}' /etc/locale.gen || {{ grep -qx '#{line}' /etc/locale.gen && sed -i 's/^#{line}$/{line}/' /etc/locale.gen || echo '{line}' >> /etc/locale.gen; }}; locale-gen"
            );
            run("pkexec", &["sh", "-c", &script])?;
            Ok(format!("{name} generated: pick it above"))
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const STATUS: &str = "System Locale: LANG=en_GB.UTF-8\n               LC_TIME=de_DE.UTF-8\n    VC Keymap: us\n   X11 Layout: (unset)\n";

    #[test]
    fn reads_the_locale_lines_only() {
        assert_eq!(parse_status(STATUS), [("LANG".into(), "en_GB.UTF-8".into()), ("LC_TIME".into(), "de_DE.UTF-8".into())]);
    }

    #[test]
    fn a_change_keeps_the_rest_and_drops_redundant_lc() {
        let cur = parse_status(STATUS);
        assert_eq!(new_set(&cur, &["LANG"], "fr_FR.UTF-8"), ["LANG=fr_FR.UTF-8", "LC_TIME=de_DE.UTF-8"]);
        let f = new_set(&cur, FORMATS, "en_GB.UTF-8");
        assert_eq!(f, ["LANG=en_GB.UTF-8"], "formats equal to LANG need no LC_ lines");
        let f = new_set(&cur, FORMATS, "en_US.UTF-8");
        assert_eq!(f.len(), 6);
        assert!(f.contains(&"LC_PAPER=en_US.UTF-8".to_string()));
    }

    #[test]
    fn language_change_runs_localectl_with_the_whole_set() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(match a.first() {
                Some(&"status") => STATUS.into(),
                Some(&"list-locales") => "C.UTF-8\nen_GB.UTF-8\nfr_FR.UTF-8\n".into(),
                _ => String::new(),
            })
        };
        let rows = load(&rec);
        assert_eq!(rows[0].value, "en_GB.UTF-8");
        assert_eq!(rows[1].value, "de_DE.UTF-8");
        apply(0, &rows, Change::Text("fr_FR.UTF-8".into()), &rec).unwrap();
        assert!(log.borrow().iter().any(|c| c == "localectl set-locale LANG=fr_FR.UTF-8 LC_TIME=de_DE.UTF-8"));
    }

    #[test]
    fn adding_a_locale_is_validated_against_glibc() {
        let sup = "de_DE.UTF-8 UTF-8\nde_DE ISO-8859-1\n";
        assert_eq!(supported_line(sup, "de_DE.UTF-8").as_deref(), Some("de_DE.UTF-8 UTF-8"));
        assert_eq!(supported_line(sup, "xx_XX"), None);
        assert!(!locale_ok("de_DE'; rm -rf /"));
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        let rows = vec![Row::default(); 4];
        assert!(apply(3, &rows, Change::Text("x'y".into()), &boom).is_err());
    }
}
