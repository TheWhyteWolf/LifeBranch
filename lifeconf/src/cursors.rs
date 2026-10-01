// The cursor themes installed on this machine, for Settings > Cursor.
//
// A cursor theme is an icon-theme directory with a `cursors/` subdirectory,
// looked for where libXcursor looks: ~/.local/share/icons, ~/.icons, then each
// $XDG_DATA_DIRS/icons.

use std::path::{Path, PathBuf};

pub fn installed() -> Vec<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let data_dirs = if data_dirs.is_empty() { "/usr/local/share:/usr/share".into() } else { data_dirs };
    let mut roots = vec![data_home.join("icons"), home.join(".icons")];
    roots.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("icons")));
    scan(&roots)
}

/// Theme names under `roots`, sorted and deduplicated. `default` is left out:
/// it only points at another theme.
pub fn scan(roots: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = roots
        .iter()
        .filter_map(|r| std::fs::read_dir(r).ok())
        .flatten()
        .flatten()
        .filter(|e| e.path().join("cursors").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n != "default")
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_themes_with_cursors_across_roots_once_each() {
        let tmp = std::env::temp_dir().join(format!("lifeconf-cursors-{}", std::process::id()));
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        for d in [a.join("Zeta/cursors"), a.join("alpha/cursors"), a.join("default/cursors"), b.join("Zeta/cursors")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(b.join("IconsOnly/48x48")).unwrap();
        assert_eq!(scan(&[a, b, tmp.join("missing")]), ["alpha", "Zeta"]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
