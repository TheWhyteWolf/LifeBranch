// SPDX-License-Identifier: GPL-3.0-or-later
// The prompts every agent here shares: a text or password box, a pick from a
// short list, and a notification. All of them are lifemenu (fuzzel when
// lifemenu isn't built), run as a child process whose pid is parked in a
// `Slot`, so a Cancel from the other side can close the box.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// Where an open prompt's pid lives while it is up.
pub type Slot = Arc<Mutex<Option<u32>>>;

/// The prompt binary: lifemenu, else fuzzel (same flags).
pub fn menu_bin() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lifemenu");
    if std::path::Path::new(&local).is_file() {
        return local;
    }
    let on_path = |b: &str| {
        std::env::var("PATH").unwrap_or_default().split(':').any(|d| std::path::Path::new(d).join(b).is_file())
    };
    if on_path("lifemenu") { "lifemenu".into() } else { "fuzzel".into() }
}

/// Close whatever prompt is open in `slot`.
pub fn cancel(slot: &Slot) {
    if let Some(pid) = slot.lock().unwrap().take() {
        // SAFETY: plain kill(2) on a child we spawned and have not reaped.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
}

fn width(m: &str) -> String {
    (m.chars().count() + 2).clamp(40, 100).to_string()
}

/// Run the menu with `input` on stdin; its stdout, or None when dismissed.
fn run(args: &[String], input: &[u8], slot: &Slot) -> Option<Zeroizing<Vec<u8>>> {
    let mut child = Command::new(menu_bin())
        .args(args)
        .stdin(if input.is_empty() { Stdio::null() } else { Stdio::piped() })
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    *slot.lock().unwrap() = Some(child.id());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    // Reserved up front so reading never reallocates and strands a copy of a
    // secret in freed memory.
    let mut buf = Zeroizing::new(Vec::with_capacity(4096));
    let read = child.stdout.take().map(|o| o.take(4095).read_to_end(&mut buf));
    let ok = child.wait().is_ok_and(|s| s.success());
    *slot.lock().unwrap() = None;
    if !ok || !matches!(read, Some(Ok(_))) {
        return None;
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Some(buf)
}

/// A typed answer: `prompt` on the input line, `mesg` under it.
pub fn ask(prompt: &str, mesg: &str, secret: bool, slot: &Slot) -> Option<Zeroizing<String>> {
    let mut p = prompt.trim_end().to_string();
    p.push(' ');
    let mut args: Vec<String> =
        ["--dmenu", "--prompt-only", &p, "--mesg", mesg, "--width", &width(mesg)].iter().map(|s| s.to_string()).collect();
    if secret {
        args.push("--password".into());
    }
    let mut buf = run(&args, b"", slot)?;
    let s = String::from_utf8(std::mem::take(&mut *buf)).ok()?;
    Some(Zeroizing::new(s))
}

/// One of `options`, by index.
pub fn choose(prompt: &str, mesg: &str, options: &[&str], slot: &Slot) -> Option<usize> {
    let args: Vec<String> = ["--dmenu", "--index", "--only-match", "--prompt", &format!("{prompt} "), "--mesg", mesg, "--width", &width(mesg)]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let list: String = options.iter().map(|o| format!("{}\n", o.replace('\n', " "))).collect();
    let buf = run(&args, list.as_bytes(), slot)?;
    String::from_utf8_lossy(&buf).trim().parse().ok().filter(|&i: &usize| i < options.len())
}

/// Yes/no, defaulting to no on anything but an explicit yes.
pub fn confirm(prompt: &str, mesg: &str, yes: &str, slot: &Slot) -> bool {
    choose(prompt, mesg, &[yes, "Cancel"], slot) == Some(0)
}

/// Whether a call came from `service`'s current owner on the bus. The agents
/// hand typed secrets back to their caller, so a call from anyone else (a
/// local process faking a pairing or a wifi prompt to harvest a password) is
/// refused before anything is shown.
pub async fn from_owner(conn: &zbus::Connection, hdr: &zbus::message::Header<'_>, service: &str) -> bool {
    let Some(sender) = hdr.sender() else { return false };
    let owner = async {
        let p = zbus::fdo::DBusProxy::new(conn).await?;
        p.get_name_owner(zbus::names::BusName::try_from(service)?).await.map_err(zbus::Error::from)
    };
    owner.await.is_ok_and(|o| o.as_str() == sender.as_str())
}

/// A desktop notification (lifenote), fire-and-forget.
pub fn notify(summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "lifeauth", summary, body])
        .stdin(Stdio::null())
        .spawn()
        .map(|mut c| std::thread::spawn(move || c.wait()));
}
