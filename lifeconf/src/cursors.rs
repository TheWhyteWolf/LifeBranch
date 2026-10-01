// The cursor themes installed on this machine, for Settings > Cursor.
//
// A cursor theme is an icon-theme directory with a `cursors/` subdirectory,
// looked for where libXcursor looks: ~/.local/share/icons, ~/.icons, then each
// $XDG_DATA_DIRS/icons.

use std::path::{Path, PathBuf};

pub fn installed() -> Vec<String> {
    scan(&roots())
}

/// Where cursor themes live, in the order libXcursor searches them.
pub fn roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let data_dirs = if data_dirs.is_empty() { "/usr/local/share:/usr/share".into() } else { data_dirs };
    let mut roots = vec![data_home.join("icons"), home.join(".icons")];
    roots.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("icons")));
    roots
}

/// One cursor image: width, height, premultiplied ARGB pixels.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Cursor `name` from `theme`, at the nominal size nearest `size` (the first
/// frame if animated). A theme without it is followed through its
/// index.theme `Inherits=`, a few levels deep, as libXcursor does.
pub fn load(roots: &[PathBuf], theme: &str, name: &str, size: u32) -> Option<Image> {
    let mut theme = theme.to_string();
    for _ in 0..4 {
        for r in roots {
            if let Ok(b) = std::fs::read(r.join(&theme).join("cursors").join(name)) {
                return parse(&b, size);
            }
        }
        theme = roots.iter().find_map(|r| inherits(&r.join(&theme).join("index.theme")))?;
    }
    None
}

fn inherits(index: &Path) -> Option<String> {
    let text = std::fs::read_to_string(index).ok()?;
    let v = text.lines().find_map(|l| l.trim().strip_prefix("Inherits")?.trim_start().strip_prefix('='))?;
    v.split([',', ';']).map(str::trim).find(|s| !s.is_empty()).map(String::from)
}

/// An Xcursor file's image nearest `size`.
pub fn parse(b: &[u8], size: u32) -> Option<Image> {
    let u = |at: usize| b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
    if b.get(..4)? != b"Xcur" {
        return None;
    }
    let ntoc = u(12)? as usize;
    let mut best: Option<(u32, usize)> = None;
    for i in 0..ntoc.min(4096) {
        let at = 16 + 12 * i;
        if u(at)? != 0xfffd_0002 {
            continue;
        }
        let (nominal, pos) = (u(at + 4)?, u(at + 8)? as usize);
        // Strictly nearer only, so the first frame of an animation wins.
        if best.is_none_or(|(n, _)| nominal.abs_diff(size) < n.abs_diff(size)) {
            best = Some((nominal, pos));
        }
    }
    let pos = best?.1;
    let (w, h) = (u(pos + 16)? as usize, u(pos + 20)? as usize);
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    let px = (0..w * h).map(|i| u(pos + 36 + 4 * i)).collect::<Option<Vec<u32>>>()?;
    Some(Image { w, h, px })
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

    fn xcursor(images: &[(u32, u32)]) -> Vec<u8> {
        // (nominal size, pixel value) per image; each image is size×1.
        let mut b = b"Xcur".to_vec();
        let put = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        for v in [16, 0x1_0000, images.len() as u32] {
            put(&mut b, v);
        }
        let mut pos = 16 + 12 * images.len() as u32;
        for (n, _) in images {
            for v in [0xfffd_0002, *n, pos] {
                put(&mut b, v);
            }
            pos += 36 + 4 * n;
        }
        for (n, px) in images {
            for v in [36, 0xfffd_0002, *n, 1, *n, 1, 0, 0, 0] {
                put(&mut b, v);
            }
            for _ in 0..*n {
                put(&mut b, *px);
            }
        }
        b
    }

    #[test]
    fn parse_picks_the_nearest_size_and_the_first_frame() {
        let b = xcursor(&[(24, 1), (24, 2), (48, 3)]);
        let im = parse(&b, 30).unwrap();
        assert_eq!((im.w, im.h, im.px[0]), (24, 1, 1));
        assert_eq!(parse(&b, 40).unwrap().px[0], 3);
        assert!(parse(b"nope", 24).is_none());
        assert!(parse(&b[..60], 24).is_none(), "truncated");
    }

    #[test]
    fn load_follows_inherits() {
        let tmp = std::env::temp_dir().join(format!("lifeconf-cursor-load-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("Base/cursors")).unwrap();
        std::fs::create_dir_all(tmp.join("Child/cursors")).unwrap();
        std::fs::write(tmp.join("Base/cursors/default"), xcursor(&[(24, 7)])).unwrap();
        std::fs::write(tmp.join("Child/index.theme"), "[Icon Theme]\nInherits=Base\n").unwrap();
        let roots = [tmp.clone()];
        assert_eq!(load(&roots, "Child", "default", 24).unwrap().px[0], 7);
        assert!(load(&roots, "Child", "pointer", 24).is_none());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

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
