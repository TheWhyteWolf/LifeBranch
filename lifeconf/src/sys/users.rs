// SPDX-License-Identifier: GPL-3.0-or-later
// Users panel: your account. Full name and password are changed as root
// through pkexec, so lifeauth asks for your current password first, which is
// the "old password" check, done by polkit rather than trusted to us.
//
// The new password is asked twice in lifemenu's password box and reaches
// chpasswd on stdin, never on a command line; it is wiped from memory after.
// The login screen is pure text, so there is no picture to set.

use super::{Change, Row, RowKind, Runner};
use zeroize::Zeroizing;

pub const LABELS: &[&str] = &["account", "full name", "groups", "administrator", "change password"];

pub fn kind(field: usize) -> RowKind {
    match field {
        1 => RowKind::Text,
        4 => RowKind::Action,
        _ => RowKind::Info,
    }
}

fn me() -> String {
    std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "root".into())
}

/// `getent passwd` line -> (uid, full name). GECOS may carry ",room,phone".
pub fn parse_passwd(line: &str) -> Option<(String, String)> {
    let f: Vec<&str> = line.trim().split(':').collect();
    (f.len() >= 7).then(|| (f[2].to_string(), f[4].split(',').next().unwrap_or("").to_string()))
}

/// A full name usermod can store: no ':' or ',' (passwd field separators), no
/// control characters, not absurdly long.
pub fn name_ok(s: &str) -> bool {
    s.len() <= 64 && !s.chars().any(|c| c == ':' || c == ',' || c.is_control())
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: String| Row { value: v, choices: vec![] };
    let user = me();
    let (uid, full) = run("getent", &["passwd", &user]).ok().and_then(|o| parse_passwd(&o)).unwrap_or_default();
    let groups = run("id", &["-nG", &user]).map(|o| o.trim().to_string()).unwrap_or_default();
    let admin = groups.split_whitespace().any(|g| g == "wheel");
    vec![
        r(format!("{user} (uid {uid})")),
        r(if full.is_empty() { "(not set)".into() } else { full }),
        r(groups),
        r(if admin { "yes (wheel: can use sudo)".into() } else { "no".into() }),
        r("set a new password".into()),
    ]
}

fn menu_bin() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lifemenu");
    if std::path::Path::new(&local).is_file() { local } else { "lifemenu".into() }
}

/// lifemenu's password box; None when dismissed.
fn ask(prompt: &str, mesg: &str) -> Option<Zeroizing<String>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(menu_bin())
        .args(["--dmenu", "--password", "--prompt-only", prompt, "--mesg", mesg, "--width", "50"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut buf = Zeroizing::new(Vec::with_capacity(4096));
    child.stdout.take()?.take(4095).read_to_end(&mut buf).ok()?;
    if !child.wait().ok()?.success() {
        return None;
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    String::from_utf8(std::mem::take(&mut *buf)).ok().map(Zeroizing::new)
}

/// Check a new password pair; Err says why not.
pub fn check_new(a: &str, b: &str) -> Result<(), String> {
    if a != b {
        return Err("the two passwords didn't match; nothing changed".into());
    }
    if a.chars().count() < 6 {
        return Err("at least 6 characters, please; nothing changed".into());
    }
    if a.contains(['\n', '\r', ':']) {
        return Err("a password can't contain ':' or a line break; nothing changed".into());
    }
    Ok(())
}

/// `user:password` to `pkexec chpasswd` on stdin.
fn set_password(user: &str, pw: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("pkexec")
        .arg("chpasswd")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pkexec: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let mut line = Zeroizing::new(Vec::with_capacity(user.len() + pw.len() + 2));
        line.extend_from_slice(user.as_bytes());
        line.push(b':');
        line.extend_from_slice(pw.as_bytes());
        line.push(b'\n');
        let _ = stdin.write_all(&line);
    }
    let out = child.wait_with_output().map_err(|e| format!("pkexec: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(()),
        // pkexec: 126 = the password prompt was dismissed, 127 = not authorized.
        Some(126 | 127) => Err("not changed (authentication was cancelled or refused)".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("chpasswd failed").to_string()),
    }
}

pub fn apply(field: usize, _rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let user = me();
    match field {
        1 => {
            let Change::Text(t) = ch else { return Err("type the name, then Enter".into()) };
            let name = t.trim();
            if !name_ok(name) {
                return Err("a name without ':' or ',' (and under 64 characters)".into());
            }
            run("pkexec", &["usermod", "-c", name, &user])?;
            Ok(if name.is_empty() { "full name cleared".into() } else { format!("full name: {name}") })
        }
        4 => {
            let a = ask("new password ", &format!("New password for {user}")).ok_or("not changed")?;
            let b = ask("again ", "Type the new password once more").ok_or("not changed")?;
            check_new(&a, &b)?;
            set_password(&user, &a)?;
            Ok("password changed: use it from your next login or unlock".into())
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_passwd_line() {
        assert_eq!(parse_passwd("voyd:x:1000:1000::/home/voyd:/bin/bash"), Some(("1000".into(), String::new())));
        assert_eq!(parse_passwd("ana:x:1001:1001:Ana Lima,Room 3,555:/home/ana:/bin/zsh").map(|p| p.1), Some("Ana Lima".into()));
        assert_eq!(parse_passwd("broken"), None);
    }

    #[test]
    fn names_and_passwords_are_checked_before_anything_runs() {
        assert!(name_ok("Ana Lima") && name_ok(""));
        assert!(!name_ok("evil:0:0") && !name_ok("a,b") && !name_ok("a\nb"));
        assert!(check_new("secret1", "secret1").is_ok());
        assert!(check_new("secret1", "secret2").unwrap_err().contains("didn't match"));
        assert!(check_new("abc", "abc").is_err());
        assert!(check_new("abc:def!", "abc:def!").is_err(), "':' would split chpasswd's line");
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(apply(1, &[], Change::Text("x:y".into()), &boom).is_err());
    }
}
