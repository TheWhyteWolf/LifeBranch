// SPDX-License-Identifier: GPL-3.0-or-later
// Workspaces and window titles from niri's IPC socket ($NIRI_SOCKET): one
// request, "EventStream", and then a JSON event per line for as long as niri
// runs. The bar keeps its own model of workspaces and windows from those
// events, the same way waybar's niri modules do. No `niri msg` child process.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub id: u64,
    pub idx: u64,
    pub name: Option<String>,
    pub output: Option<String>,
    pub active: bool,
    pub focused: bool,
    pub urgent: bool,
    pub active_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: u64,
    pub title: String,
    pub workspace: Option<u64>,
    pub urgent: bool,
}

#[derive(Debug, Default)]
pub struct Niri {
    pub workspaces: Vec<Workspace>,
    pub windows: HashMap<u64, Window>,
}

fn workspace(v: &Value) -> Option<Workspace> {
    Some(Workspace {
        id: v.get("id")?.as_u64()?,
        idx: v.get("idx").and_then(Value::as_u64).unwrap_or(0),
        name: v.get("name").and_then(Value::as_str).map(str::to_string),
        output: v.get("output").and_then(Value::as_str).map(str::to_string),
        active: v.get("is_active").and_then(Value::as_bool).unwrap_or(false),
        focused: v.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
        urgent: v.get("is_urgent").and_then(Value::as_bool).unwrap_or(false),
        active_window: v.get("active_window_id").and_then(Value::as_u64),
    })
}

fn window(v: &Value) -> Option<Window> {
    Some(Window {
        id: v.get("id")?.as_u64()?,
        title: v.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
        workspace: v.get("workspace_id").and_then(Value::as_u64),
        urgent: v.get("is_urgent").and_then(Value::as_bool).unwrap_or(false),
    })
}

impl Niri {
    /// Apply one event line. False for events the bar has no use for.
    pub fn apply(&mut self, ev: &Value) -> bool {
        let Some((kind, body)) = ev.as_object().and_then(|o| o.iter().next()) else { return false };
        match kind.as_str() {
            "WorkspacesChanged" => {
                let list = body.get("workspaces").and_then(Value::as_array);
                self.workspaces = list.map(|l| l.iter().filter_map(workspace).collect()).unwrap_or_default();
            }
            "WorkspaceActivated" => {
                let (Some(id), focused) = (body.get("id").and_then(Value::as_u64), body.get("focused").and_then(Value::as_bool).unwrap_or(false)) else {
                    return false;
                };
                let output = self.workspaces.iter().find(|w| w.id == id).and_then(|w| w.output.clone());
                for w in &mut self.workspaces {
                    if w.output == output {
                        w.active = w.id == id;
                    }
                    if focused {
                        w.focused = w.id == id;
                    }
                }
            }
            "WorkspaceActiveWindowChanged" => {
                let Some(id) = body.get("workspace_id").and_then(Value::as_u64) else { return false };
                let win = body.get("active_window_id").and_then(Value::as_u64);
                if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) {
                    w.active_window = win;
                }
            }
            "WorkspaceUrgencyChanged" => {
                let Some(id) = body.get("id").and_then(Value::as_u64) else { return false };
                if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) {
                    w.urgent = body.get("urgent").and_then(Value::as_bool).unwrap_or(false);
                }
            }
            "WindowsChanged" => {
                let list = body.get("windows").and_then(Value::as_array);
                self.windows = list.map(|l| l.iter().filter_map(window).map(|w| (w.id, w)).collect()).unwrap_or_default();
            }
            "WindowOpenedOrChanged" => {
                let Some(w) = body.get("window").and_then(window) else { return false };
                self.windows.insert(w.id, w);
            }
            "WindowClosed" => {
                let Some(id) = body.get("id").and_then(Value::as_u64) else { return false };
                self.windows.remove(&id);
            }
            "WindowUrgencyChanged" => {
                let Some(id) = body.get("id").and_then(Value::as_u64) else { return false };
                if let Some(w) = self.windows.get_mut(&id) {
                    w.urgent = body.get("urgent").and_then(Value::as_bool).unwrap_or(false);
                }
            }
            _ => return false,
        }
        true
    }

    /// The workspaces on one output, in order.
    pub fn workspaces_on(&self, output: &str) -> Vec<&Workspace> {
        let mut v: Vec<&Workspace> = self.workspaces.iter().filter(|w| w.output.as_deref() == Some(output)).collect();
        v.sort_by_key(|w| w.idx);
        v
    }

    /// The title of the window that has the focus on `output`'s visible
    /// workspace (waybar's "separate-outputs").
    pub fn title_on(&self, output: &str) -> Option<&str> {
        let ws = self.workspaces.iter().find(|w| w.active && w.output.as_deref() == Some(output))?;
        Some(self.windows.get(&ws.active_window?)?.title.as_str())
    }
}

/// The socket niri's IPC listens on.
fn socket() -> Option<String> {
    std::env::var("NIRI_SOCKET").ok().filter(|s| !s.is_empty())
}

/// Read the event stream forever, calling `on` with each parsed line. If
/// niri goes away (a restart), try again every second.
pub fn listen(mut on: impl FnMut(Value)) -> ! {
    loop {
        if let Some(path) = socket() {
            if let Ok(mut s) = UnixStream::connect(&path) {
                if s.write_all(b"\"EventStream\"\n").is_ok() {
                    let mut lines = BufReader::new(s).lines();
                    let _ = lines.next(); // {"Ok":"Handled"}
                    for line in lines.map_while(Result::ok) {
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            on(v);
                        }
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Switch to a workspace (a click on its number).
pub fn focus_workspace(id: u64) {
    std::thread::spawn(move || {
        let Some(path) = socket() else { return };
        let Ok(mut s) = UnixStream::connect(path) else { return };
        let req = format!("{{\"Action\":{{\"FocusWorkspace\":{{\"reference\":{{\"Id\":{id}}}}}}}}}\n");
        if s.write_all(req.as_bytes()).is_ok() {
            let mut reply = String::new();
            let _ = BufReader::new(s).read_line(&mut reply);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // Shapes from a real `niri msg --json event-stream`.
    const WS: &str = r#"{"WorkspacesChanged":{"workspaces":[{"id":2,"idx":2,"name":null,"output":"eDP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":null},{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":3},{"id":5,"idx":1,"name":"web","output":"HDMI-A-1","is_urgent":false,"is_active":true,"is_focused":false,"active_window_id":7}]}}"#;
    const WIN: &str = r#"{"WindowsChanged":{"windows":[{"id":7,"title":"Discord","app_id":"vesktop","pid":5089,"workspace_id":5,"is_focused":false,"is_floating":false,"is_urgent":false},{"id":3,"title":"kitty","app_id":"kitty","pid":3690,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false}]}}"#;

    #[test]
    fn titles_and_workspaces_per_output() {
        let mut n = Niri::default();
        assert!(n.apply(&ev(WS)) && n.apply(&ev(WIN)));
        let ids: Vec<u64> = n.workspaces_on("eDP-1").iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 2], "by index, not by arrival");
        assert_eq!(n.title_on("eDP-1"), Some("kitty"));
        assert_eq!(n.title_on("HDMI-A-1"), Some("Discord"), "each output shows its own window");
        assert_eq!(n.title_on("DP-9"), None);
    }

    #[test]
    fn incremental_events_keep_the_model_current() {
        let mut n = Niri::default();
        n.apply(&ev(WS));
        n.apply(&ev(WIN));
        n.apply(&ev(r#"{"WindowOpenedOrChanged":{"window":{"id":3,"title":"vim","workspace_id":1,"is_urgent":false}}}"#));
        assert_eq!(n.title_on("eDP-1"), Some("vim"));
        n.apply(&ev(r#"{"WorkspaceActivated":{"id":2,"focused":true}}"#));
        assert_eq!(n.title_on("eDP-1"), None, "the empty workspace has no title");
        assert!(n.workspaces_on("HDMI-A-1")[0].active, "another output's active workspace is untouched");
        assert!(!n.workspaces.iter().find(|w| w.id == 1).unwrap().focused);
        n.apply(&ev(r#"{"WorkspaceActiveWindowChanged":{"workspace_id":2,"active_window_id":3}}"#));
        assert_eq!(n.title_on("eDP-1"), Some("vim"));
        n.apply(&ev(r#"{"WindowClosed":{"id":3}}"#));
        assert_eq!(n.title_on("eDP-1"), None);
        n.apply(&ev(r#"{"WorkspaceUrgencyChanged":{"id":5,"urgent":true}}"#));
        assert!(n.workspaces_on("HDMI-A-1")[0].urgent);
        assert!(!n.apply(&ev(r#"{"KeyboardLayoutsChanged":{"keyboard_layouts":{"names":[],"current_idx":0}}}"#)));
    }
}
