// SPDX-License-Identifier: GPL-3.0-or-later
// The tray: a StatusNotifierItem host, and the watcher too when nothing else
// is (which, once waybar is gone, is always). Apps like vesktop, Element and
// ProtonVPN register an item; the bar shows its title as a text label, in
// keeping with the rest of the bar. Left click activates the app, middle
// click is its secondary action, and right click opens its menu
// (com.canonical.dbusmenu) as a lifemenu list.
//
// All of this is D-Bus, through zbus on its own threads: the bar's window
// only ever sees a list of labels.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use zbus::blocking::Connection;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// What the bar draws for one item.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// "bus/path", how the item is addressed.
    pub key: String,
    pub label: String,
    pub attention: bool,
}

pub enum Click {
    Activate,
    Secondary,
    Menu,
}

/// "bus/path" or a bare bus name (path /StatusNotifierItem), as items
/// register themselves and watchers list them.
pub fn split(entry: &str) -> (String, String) {
    match entry.find('/') {
        Some(i) => (entry[..i].to_string(), entry[i..].to_string()),
        None => (entry.to_string(), "/StatusNotifierItem".to_string()),
    }
}

/// The label for an item: its title, else its id ("vesktop_status_icon"
/// reads as "vesktop"), trimmed to fit a bar.
pub fn label(title: &str, id: &str) -> String {
    let t = if title.trim().is_empty() { id.trim().split(['_', '-']).next().unwrap_or("") } else { title.trim() };
    let t: String = t.chars().filter(|c| !c.is_control()).collect();
    if t.chars().count() > 14 {
        t.chars().take(13).chain(std::iter::once('…')).collect()
    } else {
        t
    }
}

// ---- the watcher -------------------------------------------------------------

struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
    changed: Sender<()>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        // Items pass either their bus name or just an object path, in which
        // case the sender is the bus name.
        let entry = if service.starts_with('/') { format!("{sender}{service}") } else if service.is_empty() { sender } else { service };
        {
            let mut items = self.items.lock().unwrap();
            if items.contains(&entry) {
                return;
            }
            items.push(entry.clone());
        }
        let _ = self.changed.send(());
        let _ = zbus::block_on(Watcher::status_notifier_item_registered(&emitter, &entry));
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true // we are one
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ---- reading items -----------------------------------------------------------

fn props(conn: &Connection, bus: &str, path: &str) -> Option<HashMap<String, OwnedValue>> {
    let reply = conn.call_method(Some(bus), path, Some("org.freedesktop.DBus.Properties"), "GetAll", &(ITEM,)).ok()?;
    reply.body().deserialize().ok()
}

fn s(p: &HashMap<String, OwnedValue>, k: &str) -> String {
    match p.get(k).map(|v| &**v) {
        Some(Value::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn read_item(conn: &Connection, entry: &str) -> Option<Item> {
    let (bus, path) = split(entry);
    let p = props(conn, &bus, &path)?;
    let status = s(&p, "Status");
    if status == "Passive" {
        return None; // the app asked to be hidden for now
    }
    Some(Item { key: entry.to_string(), label: label(&s(&p, "Title"), &s(&p, "Id")), attention: status == "NeedsAttention" })
}

/// Run the tray: become (or find) the watcher, then keep `send` fed with the
/// current item list. Returns only on a fatal D-Bus error.
pub fn run(send: impl Fn(Vec<Item>) + Send + 'static) -> Result<Connection, String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
    let items: Arc<Mutex<Vec<String>>> = Arc::default();
    let (tx, rx) = std::sync::mpsc::channel::<()>();

    let own = conn
        .request_name_with_flags(WATCHER, RequestNameFlags::DoNotQueue.into())
        .is_ok_and(|r| matches!(r, zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner));
    if own {
        conn.object_server()
            .at(WATCHER_PATH, Watcher { items: items.clone(), changed: tx.clone() })
            .map_err(|e| format!("watcher: {e}"))?;
        if let Ok(i) = conn.object_server().interface::<_, Watcher>(WATCHER_PATH) {
            let _ = zbus::block_on(Watcher::status_notifier_host_registered(i.signal_emitter()));
        }
    } else {
        // Another watcher (waybar, during a switch-over): be a host of it.
        let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = conn.request_name(host.as_str());
        let _ = conn.call_method(Some(WATCHER), WATCHER_PATH, Some(WATCHER), "RegisterStatusNotifierHost", &(host.as_str(),));
        let r = conn.call_method(Some(WATCHER), WATCHER_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(WATCHER, "RegisteredStatusNotifierItems"));
        if let Ok(v) = r.and_then(|m| m.body().deserialize::<OwnedValue>().map_err(Into::into)) {
            if let Ok(list) = Vec::<String>::try_from(v) {
                *items.lock().unwrap() = list;
            }
        }
    }

    // Signals: items changing title/status, items (un)registering with a
    // foreign watcher, and bus names vanishing (an app quit).
    let sig_conn = conn.clone();
    let sig_items = items.clone();
    let sig_tx = tx.clone();
    let watcher_iface = own;
    std::thread::spawn(move || {
        let rules = [
            format!("type='signal',interface='{ITEM}'"),
            format!("type='signal',interface='{WATCHER}'"),
            "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged'".to_string(),
        ];
        let mut its = Vec::new();
        for r in rules {
            if let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(r.as_str(), &sig_conn, Some(64)) {
                its.push(it);
            }
        }
        // One thread per stream: MessageIterator blocks.
        for mut it in its {
            let (items, tx, conn) = (sig_items.clone(), sig_tx.clone(), sig_conn.clone());
            std::thread::spawn(move || {
                while let Some(Ok(m)) = it.next() {
                    let h = m.header();
                    let member = h.member().map(|m| m.to_string()).unwrap_or_default();
                    match member.as_str() {
                        "NameOwnerChanged" => {
                            let Ok((name, _old, new)) = m.body().deserialize::<(String, String, String)>() else { continue };
                            if !new.is_empty() {
                                continue;
                            }
                            let gone: Vec<String> = {
                                let mut v = items.lock().unwrap();
                                let gone = v.iter().filter(|e| split(e).0 == name).cloned().collect();
                                v.retain(|e| split(e).0 != name);
                                gone
                            };
                            if !gone.is_empty() {
                                if watcher_iface {
                                    if let Ok(i) = conn.object_server().interface::<_, Watcher>(WATCHER_PATH) {
                                        for g in &gone {
                                            let _ = zbus::block_on(Watcher::status_notifier_item_unregistered(i.signal_emitter(), g));
                                        }
                                    }
                                }
                                let _ = tx.send(());
                            }
                        }
                        "StatusNotifierItemRegistered" if !watcher_iface => {
                            if let Ok(e) = m.body().deserialize::<String>() {
                                let mut v = items.lock().unwrap();
                                if !v.contains(&e) {
                                    v.push(e);
                                }
                            }
                            let _ = tx.send(());
                        }
                        "StatusNotifierItemUnregistered" if !watcher_iface => {
                            if let Ok(e) = m.body().deserialize::<String>() {
                                items.lock().unwrap().retain(|x| *x != e);
                            }
                            let _ = tx.send(());
                        }
                        "NewTitle" | "NewStatus" | "NewToolTip" => {
                            let _ = tx.send(());
                        }
                        _ => {}
                    }
                }
            });
        }
    });

    // The publisher: re-read every item on any change. Items are few and
    // changes rare; a full re-read keeps this simple and never stale.
    let pub_conn = conn.clone();
    std::thread::spawn(move || {
        let mut last: Vec<Item> = Vec::new();
        let publish = |last: &mut Vec<Item>| {
            let entries = items.lock().unwrap().clone();
            let now: Vec<Item> = entries.iter().filter_map(|e| read_item(&pub_conn, e)).collect();
            if now != *last {
                *last = now.clone();
                send(now);
            }
        };
        publish(&mut last);
        while rx.recv().is_ok() {
            // Coalesce a burst (an app registering and setting its title).
            std::thread::sleep(std::time::Duration::from_millis(50));
            while rx.try_recv().is_ok() {}
            publish(&mut last);
        }
    });
    Ok(conn)
}

// ---- clicks and menus ----------------------------------------------------------

/// One entry of a dbusmenu, flattened: submenus become "Parent › Child".
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
}

/// dbusmenu labels mark mnemonics with `_` and a literal underscore as `__`.
pub fn strip_mnemonic(l: &str) -> String {
    let mut out = String::new();
    let mut it = l.chars().peekable();
    while let Some(c) = it.next() {
        if c == '_' {
            if it.peek() == Some(&'_') {
                out.push('_');
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Strip variant wrappers: encoders differ in how deep they nest them.
fn plain<'a, 'v>(mut v: &'a Value<'v>) -> &'a Value<'v> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

fn prop<'a>(p: &'a HashMap<String, OwnedValue>, k: &str) -> Option<&'a Value<'static>> {
    p.get(k).map(|v| plain(v))
}

/// Walk a GetLayout node: (id, properties, children as variants).
pub fn flatten(id: i32, p: &HashMap<String, OwnedValue>, children: &[OwnedValue], prefix: &str, out: &mut Vec<MenuEntry>) {
    let visible = !matches!(prop(p, "visible"), Some(Value::Bool(false)));
    let enabled = !matches!(prop(p, "enabled"), Some(Value::Bool(false)));
    let sep = matches!(prop(p, "type"), Some(Value::Str(t)) if t.as_str() == "separator");
    if !visible || sep {
        return;
    }
    let label = match prop(p, "label") {
        Some(Value::Str(s)) => strip_mnemonic(s.as_str()),
        _ => String::new(),
    };
    let toggle = match (prop(p, "toggle-type"), prop(p, "toggle-state")) {
        (Some(Value::Str(t)), Some(Value::I32(st))) if !t.is_empty() => {
            if *st == 1 { "[x] " } else { "[ ] " }
        }
        _ => "",
    };
    let here = if prefix.is_empty() { format!("{toggle}{label}") } else { format!("{prefix} › {toggle}{label}") };
    if children.is_empty() {
        if id != 0 && enabled && !label.is_empty() {
            out.push(MenuEntry { id, label: here });
        }
        return;
    }
    let next = if id == 0 { String::new() } else { here };
    for c in children {
        if let Value::Structure(st) = plain(c) {
            let f = st.fields();
            if let (Some(Value::I32(cid)), Some(Value::Dict(d)), Some(Value::Array(a))) = (f.first().map(plain), f.get(1).map(plain), f.get(2).map(plain)) {
                let cp: HashMap<String, OwnedValue> = d
                    .iter()
                    .filter_map(|(k, v)| {
                        let k = match k {
                            Value::Str(s) => s.to_string(),
                            _ => return None,
                        };
                        let v = OwnedValue::try_from(plain(v)).ok()?;
                        Some((k, v))
                    })
                    .collect();
                let kids: Vec<OwnedValue> = a.iter().filter_map(|v| OwnedValue::try_from(v).ok()).collect();
                flatten(*cid, &cp, &kids, &next, out);
            }
        }
    }
}

fn menu_bin() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lifemenu");
    if std::path::Path::new(&local).is_file() { local } else { "lifemenu".into() }
}

/// Show `entries` in lifemenu, return the picked one's index.
fn pick(title: &str, entries: &[MenuEntry]) -> Option<usize> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(menu_bin())
        .args(["--dmenu", "--index", "--prompt", &format!("{title}> "), "--width", "44"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        for e in entries {
            let _ = writeln!(stdin, "{}", e.label.replace('\n', " "));
        }
    }
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn show_menu(conn: &Connection, bus: &str, menu_path: &str, title: &str) -> Result<(), String> {
    let _ = conn.call_method(Some(bus), menu_path, Some(MENU), "AboutToShow", &(0i32,));
    let reply = conn
        .call_method(Some(bus), menu_path, Some(MENU), "GetLayout", &(0i32, -1i32, Vec::<String>::new()))
        .map_err(|e| e.to_string())?;
    let (_rev, (id, p, kids)): (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) =
        reply.body().deserialize().map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    flatten(id, &p, &kids, "", &mut entries);
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(i) = pick(title, &entries) {
        if let Some(e) = entries.get(i) {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
            conn.call_method(Some(bus), menu_path, Some(MENU), "Event", &(e.id, "clicked", Value::from(0i32), ts))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Act on a click, off the UI thread.
pub fn click(conn: &Connection, key: &str, how: Click) {
    let conn = conn.clone();
    let key = key.to_string();
    std::thread::spawn(move || {
        let (bus, path) = split(&key);
        let p = props(&conn, &bus, &path).unwrap_or_default();
        let menu = match p.get("Menu").map(|v| &**v) {
            Some(Value::ObjectPath(o)) => Some(o.to_string()),
            _ => None,
        };
        let is_menu = matches!(p.get("ItemIsMenu").map(|v| &**v), Some(Value::Bool(true)));
        let title = label(&s(&p, "Title"), &s(&p, "Id"));
        let call = |m: &str| conn.call_method(Some(bus.as_str()), path.as_str(), Some(ITEM), m, &(0i32, 0i32)).is_ok();
        let r = match how {
            Click::Activate if !is_menu => {
                if call("Activate") {
                    Ok(())
                } else if let Some(m) = &menu {
                    show_menu(&conn, &bus, m, &title)
                } else {
                    Ok(())
                }
            }
            Click::Secondary => {
                call("SecondaryActivate");
                Ok(())
            }
            Click::Activate | Click::Menu => match &menu {
                Some(m) => show_menu(&conn, &bus, m, &title),
                // No dbusmenu: the item draws its own (Qt/GTK) menu.
                None => {
                    call("ContextMenu");
                    Ok(())
                }
            },
        };
        if let Err(e) = r {
            eprintln!("lifebar: tray {title}: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Structure;

    #[test]
    fn entries_split_into_bus_and_path() {
        assert_eq!(split(":1.45/org/ayatana/NotificationItem/x"), (":1.45".into(), "/org/ayatana/NotificationItem/x".into()));
        assert_eq!(split("org.kde.StatusNotifierItem-5089-1"), ("org.kde.StatusNotifierItem-5089-1".into(), "/StatusNotifierItem".into()));
    }

    #[test]
    fn labels_prefer_title_and_stay_short() {
        assert_eq!(label("Vesktop", "vesktop"), "Vesktop");
        assert_eq!(label("  ", "element"), "element");
        assert_eq!(label("", "vesktop_status_icon"), "vesktop");
        assert_eq!(label("Proton VPN is connected", ""), "Proton VPN is…");
    }

    #[test]
    fn mnemonics() {
        assert_eq!(strip_mnemonic("_Quit"), "Quit");
        assert_eq!(strip_mnemonic("snake__case"), "snake_case");
    }

    fn node(id: i32, props: &[(&str, Value<'static>)], kids: Vec<OwnedValue>) -> OwnedValue {
        // As on the wire: a{sv}, each value wrapped once as a variant.
        let d: HashMap<String, Value<'static>> = props.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let kids: Vec<Value<'static>> = kids.into_iter().map(Value::from).collect();
        let st = Structure::from((id, d, kids));
        OwnedValue::try_from(Value::Structure(st)).unwrap()
    }

    #[test]
    fn menus_flatten_with_submenus_toggles_and_skips() {
        let kids = vec![
            node(1, &[("label", Value::from("_Show"))], vec![]),
            node(2, &[("type", Value::from("separator"))], vec![]),
            node(3, &[("label", Value::from("Hidden")), ("visible", Value::from(false))], vec![]),
            node(4, &[("label", Value::from("Off")), ("enabled", Value::from(false))], vec![]),
            node(5, &[("label", Value::from("Mute")), ("toggle-type", Value::from("checkmark")), ("toggle-state", Value::from(1i32))], vec![]),
            node(6, &[("label", Value::from("Status")), ("children-display", Value::from("submenu"))], vec![
                node(7, &[("label", Value::from("Online"))], vec![]),
            ]),
        ];
        let mut out = Vec::new();
        flatten(0, &HashMap::new(), &kids, "", &mut out);
        let got: Vec<(i32, &str)> = out.iter().map(|e| (e.id, e.label.as_str())).collect();
        assert_eq!(got, [(1, "Show"), (5, "[x] Mute"), (7, "Status › Online")]);
    }
}
