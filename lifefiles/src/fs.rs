// SPDX-License-Identifier: GPL-3.0-or-later
// Filesystem layer: directory listing, natural sort, recursive copy/move and
// an XDG-spec trash (https://specifications.freedesktop.org/trash-spec/) so
// Del is always recoverable from Dolphin/nautilus too. No UI in here.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
}

impl Entry {
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// Read and sort one directory. Hidden files are included; the view filters
/// them, so toggling visibility never re-reads the disk.
pub fn read_dir(path: &Path) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for ent in fs::read_dir(path)? {
        let Ok(ent) = ent else { continue };
        let p = ent.path();
        // symlink_metadata first so a dangling link still lists; then follow
        // the link only to learn whether it points at a directory.
        let Ok(lmeta) = fs::symlink_metadata(&p) else { continue };
        let is_link = lmeta.file_type().is_symlink();
        let meta = if is_link { fs::metadata(&p).unwrap_or(lmeta) } else { lmeta };
        out.push(Entry {
            name: ent.file_name().to_string_lossy().into_owned(),
            path: p,
            is_dir: meta.is_dir(),
            is_link,
            size: meta.len(),
        });
    }
    out.sort_by(cmp_entries);
    Ok(out)
}

pub fn cmp_entries(a: &Entry, b: &Entry) -> Ordering {
    b.is_dir.cmp(&a.is_dir).then_with(|| natural_cmp(&a.name, &b.name))
}

/// Case-insensitive, digit-aware ("file2" < "file10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                let mut nb = String::new();
                while let Some(c) = a.next_if(char::is_ascii_digit) {
                    na.push(c);
                }
                while let Some(c) = b.next_if(char::is_ascii_digit) {
                    nb.push(c);
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}B")
    } else {
        format!("{v:.1}{}", U[i])
    }
}

/// `dir/name`, or `dir/name copy`, `dir/name copy 2`... if taken.
pub fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let suffix = if n == 1 { " copy".to_string() } else { format!(" copy {n}") };
        let p = dir.join(format!("{stem}{suffix}{ext}"));
        if fs::symlink_metadata(&p).is_err() {
            return p;
        }
    }
    unreachable!()
}

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        // Refuse to recurse into ourselves (copying a dir into its own child).
        if let (Ok(s), Ok(d)) = (src.canonicalize(), dst.parent().unwrap_or(dst).canonicalize()) {
            if d.starts_with(&s) {
                return Err(io::Error::other("cannot copy a folder into itself"));
            }
        }
        fs::create_dir(dst)?;
        for ent in fs::read_dir(src)? {
            let ent = ent?;
            copy_recursive(&ent.path(), &dst.join(ent.file_name()))?;
        }
        // After the contents, so a read-only source dir is still fillable.
        fs::set_permissions(dst, meta.permissions())?;
        keep_mtime(dst, &meta);
        Ok(())
    } else {
        fs::copy(src, dst)?; // carries the mode bits
        keep_mtime(dst, &meta);
        Ok(())
    }
}

/// Best effort: a copy that can't take the original's mtime is still a copy.
fn keep_mtime(dst: &Path, src_meta: &fs::Metadata) {
    if let (Ok(t), Ok(f)) = (src_meta.modified(), fs::File::open(dst)) {
        let _ = f.set_modified(t);
    }
}

pub fn remove_recursive(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// Move `src` into `dir`. rename(2) when possible, copy+delete across devices.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf()); // already there
    }
    let dst = unique_dest(dir, &name.to_string_lossy());
    match fs::rename(src, &dst) {
        Ok(()) => Ok(dst),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_recursive(src, &dst)?;
            remove_recursive(src)?;
            Ok(dst)
        }
        Err(e) => Err(e),
    }
}

pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    let dst = unique_dest(dir, &name.to_string_lossy());
    copy_recursive(src, &dst)?;
    Ok(dst)
}

pub fn home_trash() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        Some(d) => PathBuf::from(d).join("Trash"),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".local/share/Trash"),
    }
}

fn pct_encode(p: &str) -> String {
    let mut s = String::new();
    for b in p.bytes() {
        match b {
            b'/' | b'-' | b'_' | b'.' | b'~' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

fn now_local() -> String {
    // SAFETY: localtime_r writes into our zeroed tm; time(NULL) is always valid.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// Nearest existing ancestor of `p` (itself if it exists).
fn existing_ancestor(p: &Path) -> Option<&Path> {
    p.ancestors().find(|a| fs::symlink_metadata(a).is_ok())
}

/// The top directory of the filesystem `abs` lives on: walk up while the
/// parent is still on the same device.
fn volume_top(abs: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let dev = fs::symlink_metadata(abs).ok()?.dev();
    let mut top = abs.parent()?;
    while let Some(up) = top.parent() {
        if fs::metadata(up).ok()?.dev() != dev {
            break;
        }
        top = up;
    }
    Some(top.to_path_buf())
}

/// Per-volume `$topdir/.Trash-$uid` (mode 0700) for something that isn't on the
/// same filesystem as the home trash, so trashing is a rename, not a copy.
fn volume_trash(abs: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let top = volume_top(abs)?;
    // SAFETY: getuid has no preconditions.
    let dir = top.join(format!(".Trash-{}", unsafe { libc::getuid() }));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    Some(dir)
}

/// Move `path` into the trash at `trash` (files/ + info/ per the spec). If it's
/// on another filesystem than `trash`, the volume's own `.Trash-$uid` is used
/// when it can be created, else it falls back to a copy into `trash`.
pub fn trash_to(path: &Path, trash: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let cross = match (fs::symlink_metadata(&abs), existing_ancestor(trash).map(fs::metadata)) {
        (Ok(a), Some(Ok(t))) => a.dev() != t.dev(),
        _ => false,
    };
    if cross {
        if let Some(vt) = volume_trash(&abs) {
            // Volume trashes record Path= relative to the volume's top dir.
            let top = vt.parent().map(Path::to_path_buf);
            return trash_into(&abs, &vt, top.as_deref());
        }
    }
    trash_into(&abs, trash, None)
}

fn trash_into(abs: &Path, trash: &Path, rel_to: Option<&Path>) -> io::Result<()> {
    let abs = abs.to_path_buf();
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let name = abs.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy();
    // Pick a name free in BOTH dirs, and claim the .trashinfo with create_new so
    // two racing trashers can't pick the same slot.
    let mut n = 0u32;
    let (dest, mut info_file) = loop {
        let cand = if n == 0 { name.to_string() } else { format!("{name}.{n}") };
        let info_path = info.join(format!("{cand}.trashinfo"));
        let dest = files.join(&cand);
        if fs::symlink_metadata(&dest).is_err() {
            match fs::OpenOptions::new().write(true).create_new(true).open(&info_path) {
                Ok(f) => break (dest, (f, info_path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        n += 1;
    };
    use io::Write;
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        pct_encode(&rel_to.and_then(|t| abs.strip_prefix(t).ok()).unwrap_or(&abs).to_string_lossy()),
        now_local()
    );
    info_file.0.write_all(body.as_bytes())?;
    let moved = match fs::rename(&abs, &dest) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_recursive(&abs, &dest).and_then(|_| remove_recursive(&abs))
        }
        Err(e) => Err(e),
    };
    if moved.is_err() {
        let _ = fs::remove_file(&info_file.1); // don't leave an orphan record
    }
    moved
}

/// First few KB of a file as text, or None if it looks binary.
pub fn text_preview(path: &Path, max_lines: usize) -> Option<Vec<String>> {
    use io::Read;
    // Opening a FIFO or device node can block forever; only preview regular files.
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut buf = vec![0u8; 8192];
    let n = fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0) {
        return None;
    }
    // A multibyte char can be cut at the 8K boundary; trim back to valid UTF-8.
    let text = match std::str::from_utf8(&buf) {
        Ok(s) => s,
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&buf[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    Some(text.lines().take(max_lines).map(|l| l.replace('\t', "    ")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lifefiles-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn natural_sort_orders_numbers() {
        let mut v = vec!["file10", "File2", "file1", "file02"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["file1", "File2", "file02", "file10"]);
    }

    #[test]
    fn dirs_sort_before_files() {
        let d = scratch("sort");
        fs::write(d.join("a.txt"), "x").unwrap();
        fs::create_dir(d.join("zdir")).unwrap();
        let names: Vec<_> = read_dir(&d).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["zdir", "a.txt"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unique_dest_appends_copy() {
        let d = scratch("uniq");
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_dest(&d, "a.txt"), d.join("a copy.txt"));
        fs::write(d.join("a copy.txt"), "x").unwrap();
        assert_eq!(unique_dest(&d, "a.txt"), d.join("a copy 2.txt"));
        assert_eq!(unique_dest(&d, "new"), d.join("new"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn trash_writes_info_and_handles_name_clash() {
        let d = scratch("trash");
        let trash = d.join("Trash");
        for _ in 0..2 {
            fs::write(d.join("my file.txt"), "x").unwrap();
            trash_to(&d.join("my file.txt"), &trash).unwrap();
            assert!(!d.join("my file.txt").exists());
        }
        assert!(trash.join("files/my file.txt").exists());
        assert!(trash.join("files/my file.txt.1").exists());
        let info = fs::read_to_string(trash.join("info/my file.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath="));
        assert!(info.contains("my%20file.txt"));
        assert!(info.contains("DeletionDate="));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_into_self_is_refused() {
        let d = scratch("self");
        fs::create_dir_all(d.join("a/b")).unwrap();
        assert!(copy_recursive(&d.join("a"), &d.join("a/b/a")).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn move_into_moves_and_renames_on_clash() {
        let d = scratch("move");
        fs::create_dir(d.join("dst")).unwrap();
        fs::write(d.join("f"), "1").unwrap();
        fs::write(d.join("dst/f"), "2").unwrap();
        let out = move_into(&d.join("f"), &d.join("dst")).unwrap();
        assert_eq!(out, d.join("dst/f copy"));
        assert!(!d.join("f").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn text_preview_skips_special_files() {
        let d = scratch("fifo");
        let fifo = std::ffi::CString::new(d.join("pipe").to_str().unwrap()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(text_preview(&d.join("pipe"), 10).is_none()); // would hang if opened
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_keeps_mode_and_mtime() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("meta");
        fs::create_dir(d.join("src")).unwrap();
        fs::write(d.join("src/f"), "x").unwrap();
        fs::set_permissions(d.join("src/f"), fs::Permissions::from_mode(0o751)).unwrap();
        fs::set_permissions(d.join("src"), fs::Permissions::from_mode(0o500)).unwrap();
        copy_recursive(&d.join("src"), &d.join("dst")).unwrap();
        let mode = |p: &str| fs::metadata(d.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode("dst"), mode("dst/f")), (0o500, 0o751));
        assert_eq!(
            fs::metadata(d.join("src/f")).unwrap().modified().unwrap(),
            fs::metadata(d.join("dst/f")).unwrap().modified().unwrap()
        );
        fs::set_permissions(d.join("src"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(d.join("dst"), fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn text_preview_rejects_binary() {
        let d = scratch("prev");
        fs::write(d.join("t"), "a\tb\nc").unwrap();
        fs::write(d.join("b"), [1u8, 0, 2]).unwrap();
        assert_eq!(text_preview(&d.join("t"), 10).unwrap(), ["a    b", "c"]);
        assert!(text_preview(&d.join("b"), 10).is_none());
        let _ = fs::remove_dir_all(&d);
    }
}
