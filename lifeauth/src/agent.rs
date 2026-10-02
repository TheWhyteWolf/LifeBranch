// SPDX-License-Identifier: GPL-3.0-or-later
// org.freedesktop.PolicyKit1.AuthenticationAgent: polkitd calls
// BeginAuthentication when something in this session needs admin rights, and
// expects the reply only once the user has authenticated (or given up).
// CancelAuthentication can arrive meanwhile (the requesting program went
// away), so the exchange runs on a blocking thread and the prompt is a child
// process we can kill.
//
// The prompt is lifemenu's password box: one hand-written input box for the
// whole desktop, not a second one to keep in step.

use crate::helper::{self, Ask, End};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use zbus::zvariant::{OwnedValue, Value};
use zeroize::Zeroizing;

/// Tries before giving up, like polkit's own agents.
const TRIES: usize = 3;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
    Failed(String),
}

#[derive(Default)]
struct Pending {
    /// The open prompt's pid, so a cancel can close it.
    prompt: Option<u32>,
    cancelled: bool,
}

type Table = Arc<Mutex<HashMap<String, Pending>>>;

pub struct Agent {
    pending: Table,
    me: u32,
}

impl Agent {
    pub fn new() -> Agent {
        // SAFETY: getuid cannot fail.
        Agent { pending: Arc::default(), me: unsafe { libc::getuid() } }
    }
}

/// Which identity to authenticate as: ourselves when allowed, else the first
/// unix-user offered (root, or the first admin).
pub fn pick_identity(ids: &[(String, HashMap<String, OwnedValue>)], me: u32) -> Option<u32> {
    let uids: Vec<u32> = ids
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, d)| match d.get("uid").map(|v| &**v) {
            Some(Value::U32(u)) => Some(*u),
            _ => None,
        })
        .collect();
    uids.iter().copied().find(|&u| u == me).or_else(|| uids.first().copied())
}

pub fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: getpwuid_r writes into our buffers and sets `out` on success.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return None;
    }
    // SAFETY: pw_name points into `buf`, NUL-terminated, while buf lives.
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

/// Only polkitd may ask, as with the Bluetooth and NetworkManager agents (see
/// prompt::from_owner). The password itself only ever goes to polkit's helper,
/// but anyone else could still raise a real-looking admin prompt with their own
/// wording, or cancel one that is open.
async fn guard(conn: &zbus::Connection, hdr: &zbus::message::Header<'_>) -> Result<(), AgentError> {
    if crate::prompt::from_owner(conn, hdr, "org.freedesktop.PolicyKit1").await {
        Ok(())
    } else {
        Err(AgentError::Failed("not from polkitd".into()))
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)] // the D-Bus signature, plus the sender check
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String, // pure text: no icons
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        guard(conn, &hdr).await?;
        let uid = pick_identity(&identities, self.me)
            .ok_or_else(|| AgentError::Failed("no unix-user identity offered".into()))?;
        let user = user_name(uid).ok_or_else(|| AgentError::Failed(format!("no such user {uid}")))?;
        let as_other = (uid != self.me).then(|| user.clone());
        self.pending.lock().unwrap().insert(cookie.clone(), Pending::default());
        eprintln!("lifeauth: {action_id} as {user}");
        let table = self.pending.clone();
        let c = cookie.clone();
        let result = blocking::unblock(move || authenticate(&user, as_other.as_deref(), &message, &c, &table)).await;
        self.pending.lock().unwrap().remove(&cookie);
        result
    }

    async fn cancel_authentication(
        &self,
        cookie: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        guard(conn, &hdr).await?;
        let mut t = self.pending.lock().unwrap();
        let p = t.get_mut(&cookie).ok_or_else(|| AgentError::Failed("no such authentication".into()))?;
        p.cancelled = true;
        if let Some(pid) = p.prompt.take() {
            // SAFETY: plain kill(2) on a child we spawned and have not reaped.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
        Ok(())
    }
}

/// The setuid helper's stdin and stdout as one stream (pre-126 polkit).
struct Piped {
    child: Child,
    to: ChildStdin,
    from: ChildStdout,
}
impl Read for Piped {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.from.read(b)
    }
}
impl Write for Piped {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.to.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.to.flush()
    }
}
impl Drop for Piped {
    fn drop(&mut self) {
        let _ = self.child.wait();
    }
}

fn authenticate(user: &str, as_other: Option<&str>, message: &str, cookie: &str, table: &Table) -> Result<(), AgentError> {
    let mut note = String::new();
    for _ in 0..TRIES {
        let ask = |a: Ask| prompt(a, message, &note, as_other, cookie, table);
        let end = match UnixStream::connect(helper::SOCKET) {
            Ok(s) => helper::converse(s, format!("{user}\n{cookie}\n").as_bytes(), ask),
            Err(_) => {
                let mut child = Command::new(helper::SETUID_HELPER)
                    .arg(user)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .map_err(|e| AgentError::Failed(format!("polkit helper: {e}")))?;
                let (to, from) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
                helper::converse(Piped { child, to, from }, format!("{cookie}\n").as_bytes(), ask)
            }
        }
        .map_err(|e| AgentError::Failed(format!("polkit helper: {e}")))?;
        match end {
            End::Success => return Ok(()),
            End::Cancelled => return Err(AgentError::Cancelled("dismissed".into())),
            End::Failure(info) => {
                if table.lock().unwrap().get(cookie).is_some_and(|p| p.cancelled) {
                    return Err(AgentError::Cancelled("cancelled".into()));
                }
                note = if info.is_empty() { "Wrong password, try again".into() } else { info };
            }
        }
    }
    Err(AgentError::Failed("authentication failed".into()))
}

/// The text under the prompt: a retry note, PAM's info, the reason, and who
/// we authenticate as when it isn't the user themself.
pub fn mesg(note: &str, info: &str, message: &str, as_other: Option<&str>) -> String {
    let mut parts: Vec<String> = [note, info, message].iter().filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
    if let Some(u) = as_other {
        parts.push(format!("(as {u})"));
    }
    parts.join(" — ")
}

fn prompt(
    a: Ask,
    message: &str,
    note: &str,
    as_other: Option<&str>,
    cookie: &str,
    table: &Table,
) -> Option<Zeroizing<String>> {
    let (text, info, secret) = match a {
        Ask::Secret { prompt, info } => (prompt, info, true),
        Ask::Visible { prompt, info } => (prompt, info, false),
    };
    let m = mesg(note, info, message, as_other);
    let mut p = text.trim_end().to_string();
    p.push(' ');
    let width = (m.chars().count() + 2).clamp(40, 100).to_string();
    let mut cmd = Command::new(crate::prompt::menu_bin());
    cmd.args(["--dmenu", "--prompt-only", &p, "--mesg", &m, "--width", &width]);
    if secret {
        cmd.arg("--password");
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).spawn().ok()?;
    {
        let mut t = table.lock().unwrap();
        let pending = t.get_mut(cookie)?;
        if pending.cancelled {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        pending.prompt = Some(child.id());
    }
    // Reserve up front so reading never reallocates and strands a copy of
    // the password in freed memory.
    let mut buf = Zeroizing::new(Vec::with_capacity(4096));
    let read = child.stdout.take()?.take(4095).read_to_end(&mut buf);
    let ok = child.wait().is_ok_and(|s| s.success());
    if let Some(p) = table.lock().unwrap().get_mut(cookie) {
        p.prompt = None;
    }
    if read.is_err() || !ok {
        return None;
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    let s = String::from_utf8(std::mem::take(&mut *buf)).ok()?;
    Some(Zeroizing::new(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(kind: &str, key: &str, v: u32) -> (String, HashMap<String, OwnedValue>) {
        let mut d = HashMap::new();
        d.insert(key.to_string(), OwnedValue::from(v));
        (kind.to_string(), d)
    }

    #[test]
    fn prefers_ourselves_then_the_first_user() {
        let ids = vec![id("unix-user", "uid", 0), id("unix-group", "gid", 998), id("unix-user", "uid", 1000)];
        assert_eq!(pick_identity(&ids, 1000), Some(1000));
        assert_eq!(pick_identity(&ids, 1001), Some(0));
        assert_eq!(pick_identity(&[id("unix-group", "gid", 10)], 1000), None);
    }

    #[test]
    fn message_line() {
        assert_eq!(mesg("", "", "Authentication is required", None), "Authentication is required");
        assert_eq!(
            mesg("Wrong password, try again", "", "Mount a disk", Some("root")),
            "Wrong password, try again — Mount a disk — (as root)"
        );
    }

    #[test]
    fn resolves_root() {
        assert_eq!(user_name(0).as_deref(), Some("root"));
    }
}
