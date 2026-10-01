// SPDX-License-Identifier: GPL-3.0-or-later
// lifeauth — the session's credentials agent for LifeBranch, three in one:
//
//   polkit          admin rights (mounting a disk, `pkexec`, a settings change):
//                   asks your password and hands it to polkit's helper, which
//                   runs PAM as root. Replaces polkit-kde-agent.
//   Bluetooth       pairing that needs a PIN, a passkey or "do the codes
//                   match?". Replaces blueman-applet's agent.
//   NetworkManager  a secret a connection lacks: a new wifi's password, an
//                   enterprise network's login, a VPN's password. Replaces
//                   nm-applet's agent.
//
// Every question is a lifemenu prompt, in the desktop's own look. Run one per
// graphical session (niri spawns it). The polkit part is required; the other
// two register whenever bluetoothd / NetworkManager are there.

mod agent;
mod bluetooth;
mod helper;
mod prompt;
mod secrets;

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

/// Register with a service now, and again every time it (re)starts: both
/// bluetoothd and NetworkManager forget their agents when they restart.
fn keep_registered(bus: &Connection, service: &'static str, what: &'static str, register: fn(&Connection) -> zbus::Result<()>) {
    let bus = bus.clone();
    std::thread::spawn(move || {
        let try_now = |bus: &Connection| match register(bus) {
            Ok(()) => eprintln!("lifeauth: {what} agent registered"),
            Err(e) => eprintln!("lifeauth: no {what} agent ({e})"),
        };
        try_now(&bus);
        let rule = format!("type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='{service}'");
        let Ok(mut it) = zbus::blocking::MessageIterator::for_match_rule(rule.as_str(), &bus, Some(8)) else { return };
        while let Some(Ok(m)) = it.next() {
            if let Ok((_, _, new)) = m.body().deserialize::<(String, String, String)>() {
                if !new.is_empty() {
                    // Give the service a moment to export its manager object.
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    try_now(&bus);
                }
            }
        }
    });
}

fn register_bluetooth(bus: &Connection) -> zbus::Result<()> {
    let path = zbus::zvariant::ObjectPath::try_from(bluetooth::PATH)?;
    bus.call_method(Some("org.bluez"), "/org/bluez", Some("org.bluez.AgentManager1"), "RegisterAgent", &(&path, "KeyboardDisplay"))?;
    bus.call_method(Some("org.bluez"), "/org/bluez", Some("org.bluez.AgentManager1"), "RequestDefaultAgent", &(&path,))?;
    Ok(())
}

fn register_secrets(bus: &Connection) -> zbus::Result<()> {
    bus.call_method(
        Some("org.freedesktop.NetworkManager"),
        "/org/freedesktop/NetworkManager/AgentManager",
        Some("org.freedesktop.NetworkManager.AgentManager"),
        "RegisterWithCapabilities",
        &("org.lifebranch.lifeauth", secrets::VPN_HINTS),
    )?;
    Ok(())
}

fn run() -> Result<(), String> {
    let bus = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("system bus: {e}"))?
        .serve_at(PATH, agent::Agent::new())
        .map_err(|e| format!("object server: {e}"))?
        .serve_at(bluetooth::PATH, bluetooth::Agent::default())
        .map_err(|e| format!("object server: {e}"))?
        .serve_at(secrets::PATH, secrets::Agent::default())
        .map_err(|e| format!("object server: {e}"))?
        .build()
        .map_err(|e| format!("system bus: {e}"))?;
    keep_registered(&bus, "org.bluez", "bluetooth", register_bluetooth);
    keep_registered(&bus, "org.freedesktop.NetworkManager", "network secrets", register_secrets);

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
        println!("lifeauth — polkit, Bluetooth pairing and NetworkManager secret agent (prompts through lifemenu)\nRun once per graphical session; takes no flags.");
        return;
    }
    if let Err(e) = run() {
        eprintln!("lifeauth: {e}");
        std::process::exit(1);
    }
}
