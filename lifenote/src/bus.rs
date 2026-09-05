// SPDX-License-Identifier: GPL-3.0-or-later
// The DBus side: org.freedesktop.Notifications (the spec interface every
// notify-send/libnotify client talks to) plus rs.lifenote.Control (what
// `lifenote ctl` talks to), both served at /org/freedesktop/Notifications.
//
// Threading model: the main thread stays a pure calloop loop (app.rs);
// zbus's blocking connection runs its own internal executor thread and
// dispatches method calls there. Everything crossing to the main loop goes
// through the calloop channel as an AppEvent; the only traffic back is
// emit_closed(), and Connection is Arc-cheap and thread-safe.

use crate::state::{ping_waybar, AppEvent, History, HistoryEntry, NoteData, Urgency};
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use zbus::zvariant::Value;

const PATH: &str = "/org/freedesktop/Notifications";
const NAME: &str = "org.freedesktop.Notifications";

pub struct Bus {
    conn: zbus::blocking::Connection,
}

impl Bus {
    /// NotificationClosed signal — the spec requires one per close, with
    /// reason 1 (expired), 2 (dismissed by user), or 3 (CloseNotification).
    pub fn emit_closed(&self, id: u32, reason: u32) {
        let r = self
            .conn
            .emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, reason));
        if let Err(e) = r {
            eprintln!("lifenote: NotificationClosed signal failed: {e}");
        }
    }
}

struct Notifications {
    tx: Sender<AppEvent>,
    next_id: AtomicU32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str, // pure-text rice: icons are ignored wholesale
        summary: &str,
        body: &str,
        _actions: Vec<String>, // not advertised in capabilities
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };
        let urgency = match hints.get("urgency") {
            Some(Value::U8(b)) => Urgency::from_hint(*b),
            _ => Urgency::Normal,
        };
        let note = NoteData {
            id,
            app: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            urgency,
            timeout_ms: expire_timeout,
        };
        // DND is handled on the main thread (it also records history there);
        // the spec return value is just the id, so nothing blocks on it.
        let _ = self.tx.send(AppEvent::Notify(note));
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(AppEvent::Close(id));
    }

    fn get_capabilities(&self) -> Vec<&str> {
        // "body" only: no body-markup (we strip tags), no actions, no icons.
        vec!["body"]
    }

    fn get_server_information(&self) -> (&str, &str, &str, &str) {
        ("lifenote", "olivetree", env!("CARGO_PKG_VERSION"), "1.2")
    }

    // Declared for introspection completeness; emitted via Bus::emit_closed.
    #[zbus(signal)]
    async fn notification_closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    /// Never emitted today (no actions capability) — declared so clients
    /// that bind it at least see a well-formed interface.
    #[zbus(signal)]
    async fn action_invoked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        action_key: String,
    ) -> zbus::Result<()>;
}

struct Control {
    tx: Sender<AppEvent>,
    dnd: Arc<AtomicBool>,
    history: History,
}

#[zbus::interface(name = "rs.lifenote.Control")]
impl Control {
    fn dnd_set(&self, on: bool) -> bool {
        self.dnd.store(on, Ordering::Relaxed);
        let _ = self.tx.send(AppEvent::DndChanged(on));
        on
    }

    fn dnd_toggle(&self) -> bool {
        // Single-writer in practice (dnd-toggle.sh / Mod+N), so a plain
        // load-flip-store is fine.
        let on = !self.dnd.load(Ordering::Relaxed);
        self.dnd.store(on, Ordering::Relaxed);
        let _ = self.tx.send(AppEvent::DndChanged(on));
        on
    }

    fn dnd_get(&self) -> bool {
        self.dnd.load(Ordering::Relaxed)
    }

    fn dismiss_all(&self) {
        let _ = self.tx.send(AppEvent::DismissAll);
    }

    /// Re-read the config file and restyle live. Fire-and-forget: the actual
    /// reload runs on the main thread (app.rs), which re-parses and repaints.
    fn reload(&self) {
        let _ = self.tx.send(AppEvent::Reload);
    }

    /// (app, summary, body), newest first. Reading the ring marks everything
    /// seen: the waybar `#` badge resets and gets a refresh ping.
    fn history(&self) -> Vec<(String, String, String)> {
        let mut h = self.history.lock().unwrap();
        let out = h
            .entries
            .iter()
            .map(|e: &HistoryEntry| (e.app.clone(), e.summary.clone(), e.body.clone()))
            .collect();
        let had_unseen = h.unseen > 0;
        h.unseen = 0;
        drop(h);
        if had_unseen {
            ping_waybar();
        }
        out
    }

    /// History entries added unseen since the last History read — the number
    /// behind the waybar `#` badge.
    fn unseen_count(&self) -> u32 {
        self.history.lock().unwrap().unseen
    }
}

/// Bring up the bus: object tree first, then the well-known name, so no
/// client call can race an empty tree. Fails loudly if the name is owned —
/// almost always a still-running mako.
pub fn start(tx: Sender<AppEvent>, dnd: Arc<AtomicBool>, history: History) -> Result<Bus, String> {
    let notifications = Notifications { tx: tx.clone(), next_id: AtomicU32::new(1) };
    let control = Control { tx, dnd, history };

    let conn = zbus::blocking::connection::Builder::session()
        .map_err(|e| format!("cannot connect to the session bus: {e}"))?
        .serve_at(PATH, notifications)
        .map_err(|e| format!("object server: {e}"))?
        .serve_at(PATH, control)
        .map_err(|e| format!("object server: {e}"))?
        .name(NAME)
        .map_err(|e| format!("bus name: {e}"))?
        .build()
        .map_err(|e| match e {
            zbus::Error::NameTaken => {
                format!("{NAME} is taken — is mako (or another daemon) still running?")
            }
            e => format!("bus setup failed: {e}"),
        })?;

    Ok(Bus { conn })
}
