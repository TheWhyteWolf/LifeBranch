// SPDX-License-Identifier: GPL-3.0-or-later
// The pure half of the portal: turning a FileChooser request's options into
// a `lifefiles --pick` command line, and the picked paths into the file://
// URIs the portal answers with. No D-Bus here, so all of it is testable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zbus::zvariant::{OwnedValue, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Open,
    Save,
    /// SaveFiles: pick a folder; each of `names` is saved into it.
    SaveFiles(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Req {
    pub kind: Kind,
    pub title: String,
    pub dir: PathBuf,
    pub multiple: bool,
    pub directory: bool,
    pub name: Option<String>,
}

fn plain<'a, 'v>(mut v: &'a Value<'v>) -> &'a Value<'v> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

fn flag(o: &HashMap<String, OwnedValue>, k: &str) -> bool {
    matches!(o.get(k).map(|v| plain(v)), Some(Value::Bool(true)))
}

fn string(o: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    match o.get(k).map(|v| plain(v)) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// An `ay` option: a NUL-terminated path.
pub fn bytes(v: &Value) -> Option<Vec<u8>> {
    match plain(v) {
        Value::Array(a) => {
            let mut b: Vec<u8> = a.iter().filter_map(|x| if let Value::U8(u) = plain(x) { Some(*u) } else { None }).collect();
            while b.last() == Some(&0) {
                b.pop();
            }
            (!b.is_empty()).then_some(b)
        }
        _ => None,
    }
}

fn path_opt(o: &HashMap<String, OwnedValue>, k: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    o.get(k).and_then(|v| bytes(v)).map(|b| PathBuf::from(std::ffi::OsString::from_vec(b)))
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

/// Build the request from the portal call. Where to start: the app's
/// current_folder, else the current_file's folder (Save), else home.
pub fn request(kind: Kind, title: &str, o: &HashMap<String, OwnedValue>) -> Req {
    let current_file = path_opt(o, "current_file");
    let dir = path_opt(o, "current_folder")
        .or_else(|| current_file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf)))
        .filter(|d| d.is_dir())
        .unwrap_or_else(home);
    let name = match kind {
        Kind::Save => string(o, "current_name")
            .or_else(|| current_file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()))
            .or_else(|| Some(String::new())),
        _ => None,
    };
    Req {
        directory: flag(o, "directory") || matches!(kind, Kind::SaveFiles(_)),
        multiple: flag(o, "multiple") && kind == Kind::Open,
        title: title.to_string(),
        dir,
        name,
        kind,
    }
}

/// SaveFiles' `files` option: the names, as `aay`.
pub fn file_names(o: &HashMap<String, OwnedValue>) -> Vec<String> {
    match o.get("files").map(|v| plain(v)) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(bytes)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            // A name, not a path: never let an app write outside the folder.
            .filter_map(|n| Path::new(&n).file_name().map(|f| f.to_string_lossy().into_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

/// `lifefiles --pick ...` for this request.
pub fn lifefiles_args(r: &Req, out: &Path) -> Vec<String> {
    let mut a = vec!["--pick".to_string()];
    if r.multiple {
        a.push("--multiple".into());
    }
    if r.directory {
        a.push("--directory".into());
    }
    if let Some(n) = &r.name {
        a.extend(["--save".into(), n.clone()]);
    }
    if !r.title.is_empty() {
        a.extend(["--title".into(), r.title.clone()]);
    }
    a.extend(["--out".into(), out.display().to_string(), r.dir.display().to_string()]);
    a
}

/// What lifefiles wrote: NUL-separated absolute paths.
pub fn read_out(b: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    b.split(|c| *c == 0)
        .filter(|p| p.first() == Some(&b'/'))
        .map(|p| PathBuf::from(std::ffi::OsStr::from_bytes(p)))
        .collect()
}

/// A file:// URI, percent-encoding everything outside RFC 3986's unreserved
/// set and '/'.
pub fn uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The URIs to answer with for what was picked.
pub fn uris(r: &Req, picked: &[PathBuf]) -> Vec<String> {
    match &r.kind {
        Kind::SaveFiles(names) => match picked.first() {
            Some(dir) => names.iter().map(|n| uri(&dir.join(n))).collect(),
            None => Vec::new(),
        },
        _ => picked.iter().map(|p| uri(p)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).unwrap())).collect()
    }

    fn ay(s: &str) -> Value<'static> {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        Value::from(b)
    }

    #[test]
    fn open_with_options() {
        let o = opts(vec![("multiple", Value::from(true)), ("current_folder", ay("/tmp"))]);
        let r = request(Kind::Open, "Open Image", &o);
        assert!(r.multiple && !r.directory);
        assert_eq!(r.dir, PathBuf::from("/tmp"));
        let a = lifefiles_args(&r, Path::new("/run/x"));
        assert_eq!(a, ["--pick", "--multiple", "--title", "Open Image", "--out", "/run/x", "/tmp"]);
        let d = request(Kind::Open, "", &opts(vec![("directory", Value::from(true)), ("current_folder", ay("/nonexistent"))]));
        assert!(d.directory);
        assert_eq!(d.dir, home(), "a folder that isn't there falls back to home");
    }

    #[test]
    fn save_takes_the_name_and_folder_from_current_file() {
        let r = request(Kind::Save, "Save As", &opts(vec![("current_file", ay("/tmp/report.odt"))]));
        assert_eq!((r.name.as_deref(), r.dir.as_path()), (Some("report.odt"), Path::new("/tmp")));
        let r = request(Kind::Save, "", &opts(vec![("current_name", Value::from("x.png")), ("multiple", Value::from(true))]));
        assert_eq!(r.name.as_deref(), Some("x.png"));
        assert!(!r.multiple, "a save is one file");
        assert!(lifefiles_args(&r, Path::new("/o")).windows(2).any(|w| w == ["--save", "x.png"]));
    }

    #[test]
    fn save_files_picks_a_folder_and_names_stay_names() {
        let o = opts(vec![("files", Value::from(vec![Value::from(b"a.txt\0".to_vec()), Value::from(b"../../etc/passwd\0".to_vec())]))]);
        let names = file_names(&o);
        assert_eq!(names, ["a.txt", "passwd"]);
        let r = request(Kind::SaveFiles(names), "", &o);
        assert!(r.directory);
        assert_eq!(uris(&r, &[PathBuf::from("/home/u/Out")]), ["file:///home/u/Out/a.txt", "file:///home/u/Out/passwd"]);
    }

    #[test]
    fn output_and_uris() {
        let picked = read_out(b"/home/u/My File.txt\0/home/u/caf\xc3\xa9#1\0relative\0");
        assert_eq!(picked.len(), 2, "relative junk is dropped");
        let r = request(Kind::Open, "", &HashMap::new());
        assert_eq!(uris(&r, &picked), ["file:///home/u/My%20File.txt", "file:///home/u/caf%C3%A9%231"]);
    }
}
