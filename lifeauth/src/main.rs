// SPDX-License-Identifier: GPL-3.0-or-later
// lifeauth — the polkit authentication agent for LifeBranch. When something
// needs admin rights (mounting a disk, `pkexec`, a settings change), polkitd
// asks the session's agent; this one asks you through lifemenu's password box
// and hands the answer to polkit's own helper, which runs PAM as root.
//
// Replaces polkit-kde-agent: the same job without Qt, and the prompt in the
// desktop's own look. Run one per graphical session (niri spawns it).

mod agent;
mod helper;

use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PATH: &str = "/org/lifebranch/AuthenticationAgent";

/// The logind session to register for. Processes niri spawns may live in a
/// user service rather than the session scope, so fall back to the session
/// logind calls the user's display (what libpolkit itself does).
fn session_id(bus: &Connection) -> Result<String, String> {
    if let Some(id) = std::env::var("XDG_SESSION_ID").ok().filter(|s| !s.is_empty()) {
        return Ok(id);
    }
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let reply = bus
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetUser",
            &(uid,),
        )
        .map_err(|e| format!("logind GetUser: {e}"))?;
    let user: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let reply = bus
        .call_method(
            Some("org.freedesktop.login1"),
            user.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.User", "Display"),
        )
        .map_err(|e| format!("logind Display: {e}"))?;
    let v: OwnedValue = reply.body().deserialize().map_err(|e| e.to_string())?;
    let (id, _): (String, OwnedObjectPath) = v.try_into().map_err(|e: zbus::zvariant::Error| e.to_string())?;
    if id.is_empty() {
        return Err("logind reports no graphical session for this user".into());
    }
    Ok(id)
}

fn run() -> Result<(), String> {
    let bus = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("system bus: {e}"))?
        .serve_at(PATH, agent::Agent::new())
        .map_err(|e| format!("object server: {e}"))?
        .build()
        .map_err(|e| format!("system bus: {e}"))?;

    let id = session_id(&bus)?;
    let mut details: HashMap<&str, Value> = HashMap::new();
    details.insert("session-id", Value::from(id.as_str()));
    let subject = ("unix-session", details);
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
    bus.call_method(
        Some("org.freedesktop.PolicyKit1"),
        "/org/freedesktop/PolicyKit1/Authority",
        Some("org.freedesktop.PolicyKit1.Authority"),
        "RegisterAuthenticationAgent",
        &(subject, locale.as_str(), PATH),
    )
    .map_err(|e| {
        let e = e.to_string();
        if e.contains("already exists") {
            format!("another authentication agent (polkit-kde-agent?) already serves session {id}")
        } else {
            format!("registering with polkit: {e}")
        }
    })?;
    eprintln!("lifeauth: agent for session {id}");

    // zbus serves the agent on its own threads; polkitd forgets us when our
    // connection closes, so exiting (SIGTERM) needs no cleanup.
    loop {
        std::thread::park();
    }
}

fn main() {
    if std::env::args().nth(1).is_some_and(|a| a == "-h" || a == "--help") {
        println!("lifeauth — polkit authentication agent (prompts through lifemenu)\nRun once per graphical session; takes no flags.");
        return;
    }
    if let Err(e) = run() {
        eprintln!("lifeauth: {e}");
        std::process::exit(1);
    }
}
