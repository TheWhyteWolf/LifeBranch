// SPDX-License-Identifier: GPL-3.0-or-later
// lifeportal — the FileChooser backend of xdg-desktop-portal for LifeBranch.
// When an app (Brave, Element, anything sandboxed or GTK/Qt set to use
// portals) opens an Open or Save dialog, xdg-desktop-portal routes it here
// (xdg/portals.conf), and the dialog is lifefiles in a floating kitty window:
// the same browser as Mod+E, in `--pick` mode.
//
// Installed per user: ~/.local/share/xdg-desktop-portal/portals/lifebranch.portal
// names this service; D-Bus activation starts it on the first dialog.

mod chooser;

use chooser::Kind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NAME: &str = "org.freedesktop.impl.portal.desktop.lifebranch";
const PATH: &str = "/org/freedesktop/portal/desktop";

const OK: u32 = 0;
const CANCELLED: u32 = 1;
const FAILED: u32 = 2;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// org.freedesktop.impl.portal.Request at the call's handle: the frontend
/// calls Close when the app gives up on the dialog.
struct Request {
    pid: Arc<Mutex<Option<u32>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        if let Some(pid) = *self.pid.lock().unwrap() {
            // SAFETY: plain kill(2) on the picker we started.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
}

fn local(bin: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let p = format!("{home}/.local/bin/{bin}");
    if std::path::Path::new(&p).is_file() { p } else { bin.to_string() }
}

/// Run the picker and wait: kitty with lifefiles inside, or
/// $LIFEPORTAL_PICKER (given lifefiles' arguments) for tests.
fn pick(req: &chooser::Req, pid: &Arc<Mutex<Option<u32>>>) -> Result<Option<Vec<PathBuf>>, String> {
    let dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| "XDG_RUNTIME_DIR is not set".to_string())?;
    let out = PathBuf::from(format!("{dir}/lifeportal-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let args = chooser::lifefiles_args(req, &out);
    let mut cmd = match std::env::var("LIFEPORTAL_PICKER") {
        Ok(p) if !p.is_empty() => {
            let mut c = std::process::Command::new(p);
            c.args(&args);
            c
        }
        _ => {
            let mut c = std::process::Command::new("kitty");
            let title = if req.title.is_empty() { "Choose a file".to_string() } else { req.title.clone() };
            // The class gives niri its window rule (floating, dialog-sized).
            c.args(["--class", "lifefiles-picker", "--title", &title, "-e", &local("lifefiles")]).args(&args);
            c
        }
    };
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start the picker: {e}"))?;
    *pid.lock().unwrap() = Some(child.id());
    let status = child.wait();
    *pid.lock().unwrap() = None;
    let got = std::fs::read(&out).ok();
    let _ = std::fs::remove_file(&out);
    match (status, got) {
        (Ok(s), Some(b)) if s.success() => Ok(Some(chooser::read_out(&b))),
        (Ok(_), _) => Ok(None), // cancelled (or closed)
        (Err(e), _) => Err(e.to_string()),
    }
}

struct FileChooser;

impl FileChooser {
    async fn run(server: &ObjectServer, handle: OwnedObjectPath, req: chooser::Req) -> (u32, HashMap<String, OwnedValue>) {
        let pid: Arc<Mutex<Option<u32>>> = Arc::default();
        let _ = server.at(&handle, Request { pid: pid.clone() }).await;
        let r = req.clone();
        let result = blocking::unblock(move || pick(&r, &pid)).await;
        let _ = server.remove::<Request, _>(&handle).await;
        let mut res: HashMap<String, OwnedValue> = HashMap::new();
        let code = match result {
            Ok(Some(paths)) if !paths.is_empty() => {
                let uris = chooser::uris(&req, &paths);
                if let Ok(v) = OwnedValue::try_from(Value::from(uris)) {
                    res.insert("uris".into(), v);
                }
                OK
            }
            Ok(_) => CANCELLED,
            Err(e) => {
                eprintln!("lifeportal: {e}");
                FAILED
            }
        };
        (code, res)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[zbus(out_args("response", "results"))]
    async fn open_file(
        &self,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        FileChooser::run(server, handle, chooser::request(Kind::Open, &title, &options)).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_file(
        &self,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        FileChooser::run(server, handle, chooser::request(Kind::Save, &title, &options)).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_files(
        &self,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let names = chooser::file_names(&options);
        FileChooser::run(server, handle, chooser::request(Kind::SaveFiles(names), &title, &options)).await
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

fn main() {
    if std::env::args().nth(1).is_some_and(|a| a == "-h" || a == "--help") {
        println!("lifeportal — xdg-desktop-portal FileChooser backend (lifefiles as the dialog)\nStarted by D-Bus activation; takes no flags.");
        return;
    }
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(NAME))
        .and_then(|b| b.serve_at(PATH, FileChooser))
        .and_then(|b| b.build());
    match conn {
        Ok(_c) => loop {
            std::thread::park();
        },
        Err(e) => {
            eprintln!("lifeportal: {e}");
            std::process::exit(1);
        }
    }
}
