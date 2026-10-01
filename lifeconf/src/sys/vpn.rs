// SPDX-License-Identifier: GPL-3.0-or-later
// VPN panel: every tunnel on the machine in one list, whoever runs it.
//
//   NetworkManager  WireGuard and OpenVPN profiles (nmcli): connect, autoconnect,
//                   import a .conf/.ovpn, delete
//   Tailscale       up/down and the exit node (tailscale; needs `tailscale set
//                   --operator=$USER` once, which the installer offers)
//   Proton VPN      status from NetworkManager (its connections use the protun
//                   plugin); connect through its CLI, else its own app
//   Mullvad         status, connect, location (mullvad)
//
// A provider that isn't installed simply isn't listed. As with wifi and
// bluetooth, picking an entry only selects it; the connect row acts.
//
// lifepanel compiles this file too (quick connect/disconnect), so the logic
// lives here once.

use super::{split_terse, Change, Row, RowKind, Runner, Sel};

pub const LABELS: &[&str] = &["vpn", "state", "connect", "autoconnect", "location", "import file", "delete"];

static SELECTED: Sel = Sel::new();
/// Proton's country for the next connect ("" = fastest anywhere).
static PROTON_COUNTRY: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    match field {
        0 | 4 => RowKind::Choice,
        1 => RowKind::Info,
        3 => RowKind::Bool,
        5 => RowKind::Text,
        // Delete takes the profile's name typed out, so a stray Enter can't.
        6 => RowKind::Text,
        _ => RowKind::Action,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    Nm { uuid: String, autoconnect: bool },
    Tailscale,
    Proton { active_uuid: Option<String> },
    Mullvad,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Up,
    Connecting,
    Down,
    /// Installed but signed out (Tailscale NeedsLogin, Mullvad no account).
    NeedsLogin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Vpn {
    /// Stable id: "nm:<uuid>", "tailscale", "proton", "mullvad".
    pub key: String,
    pub name: String,
    pub provider: Provider,
    pub state: State,
    /// "WireGuard", "exit node: box", "CH#12", ...
    pub detail: String,
}

impl Vpn {
    pub fn is_up(&self) -> bool {
        matches!(self.state, State::Up | State::Connecting)
    }
    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            Provider::Nm { .. } => "NetworkManager",
            Provider::Tailscale => "Tailscale",
            Provider::Proton { .. } => "Proton VPN",
            Provider::Mullvad => "Mullvad",
        }
    }
}

const PROTUN: &str = "org.freedesktop.NetworkManager.protun";

fn is_proton(name: &str, service: &str) -> bool {
    service == PROTUN || name.starts_with("ProtonVPN") || name.starts_with("Proton VPN")
}

/// Installed? Asked of the shell rather than by running the program: Proton's
/// CLI is a Python start-up (and has shipped crashing), far too slow to run on
/// every refresh just to learn it exists.
fn has(run: Runner, program: &str) -> bool {
    run("sh", &["-c", &format!("command -v {program}")]).is_ok_and(|o| !o.trim().is_empty())
}

// ---- NetworkManager ----------------------------------------------------------

/// `nmcli -t -f NAME,UUID,TYPE,AUTOCONNECT connection show` rows that are
/// tunnels: (name, uuid, type, autoconnect).
pub fn parse_nm(out: &str) -> Vec<(String, String, String, bool)> {
    out.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 4 && (f[2] == "vpn" || f[2] == "wireguard"))
        .map(|f| (f[0].clone(), f[1].clone(), f[2].clone(), f[3] == "yes"))
        .collect()
}

/// `nmcli -t -f UUID,STATE connection show --active` -> (uuid, state).
pub fn parse_active(out: &str) -> Vec<(String, String)> {
    out.lines().map(split_terse).filter(|f| f.len() >= 2).map(|f| (f[0].clone(), f[1].clone())).collect()
}

fn nm_kind(kind: &str, service: &str) -> String {
    match (kind, service.rsplit('.').next().unwrap_or("")) {
        ("wireguard", _) => "WireGuard".into(),
        (_, "openvpn") => "OpenVPN".into(),
        (_, "") => "VPN".into(),
        (_, s) => s.to_string(),
    }
}

// ---- Tailscale ---------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
pub struct Tailnet {
    pub state: String,
    /// Peers offering to be an exit node: (host name, first tailnet IP).
    pub exit_options: Vec<(String, String)>,
    /// The exit node in use, by host name.
    pub exit_node: Option<String>,
}

pub fn parse_tailscale(json: &str) -> Option<Tailnet> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = v.get("BackendState")?.as_str()?.to_string();
    let mut t = Tailnet { state, ..Tailnet::default() };
    if let Some(peers) = v.get("Peer").and_then(|p| p.as_object()) {
        let mut opts: Vec<(String, String)> = peers
            .values()
            .filter(|p| p.get("ExitNodeOption").and_then(|b| b.as_bool()) == Some(true))
            .filter_map(|p| {
                let host = p.get("HostName")?.as_str()?.to_string();
                let ip = p.get("TailscaleIPs")?.as_array()?.first()?.as_str()?.to_string();
                Some((host, ip))
            })
            .collect();
        opts.sort();
        t.exit_options = opts;
        t.exit_node = peers
            .values()
            .find(|p| p.get("ExitNode").and_then(|b| b.as_bool()) == Some(true))
            .and_then(|p| p.get("HostName")?.as_str().map(str::to_string));
    }
    Some(t)
}

fn tailscale_state(s: &str) -> State {
    match s {
        "Running" => State::Up,
        "Starting" => State::Connecting,
        "NeedsLogin" | "NeedsMachineAuth" => State::NeedsLogin,
        _ => State::Down,
    }
}

// ---- Mullvad -----------------------------------------------------------------

/// `mullvad status`: the first word is the state; the rest of that line, or a
/// "Relay:" line in newer versions, says where.
pub fn parse_mullvad(out: &str) -> (State, String) {
    let first = out.lines().next().unwrap_or("").trim();
    let state = match first.split_whitespace().next().unwrap_or("") {
        "Connected" => State::Up,
        "Connecting" | "Reconnecting" => State::Connecting,
        _ => State::Down,
    };
    let relay = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("Relay:").map(|r| r.trim().to_string()))
        .or_else(|| first.strip_prefix("Connected to ").map(str::to_string))
        .unwrap_or_default();
    (state, relay)
}

/// `mullvad relay list`: countries are the unindented "Sweden (se)" lines.
pub fn parse_mullvad_countries(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter(|l| !l.starts_with(char::is_whitespace) && !l.trim().is_empty())
        .filter_map(|l| {
            let (name, rest) = l.rsplit_once('(')?;
            let code = rest.trim_end().strip_suffix(')')?;
            (code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())).then(|| (name.trim().to_string(), code.to_string()))
        })
        .collect()
}

/// Proton's locations without its CLI: fastest anywhere, or a country code
/// (any two letters may be typed; these are the common picks).
const PROTON_COUNTRIES: &[&str] = &["US", "GB", "CA", "DE", "NL", "CH", "SE", "FR", "ES", "IT", "JP", "AU", "SG"];

// ---- the list ----------------------------------------------------------------

pub fn list(run: Runner) -> Vec<Vpn> {
    let mut out = Vec::new();

    let mut proton_active: Option<(String, String)> = None;
    if let Ok(profiles) = run("nmcli", &["-t", "-f", "NAME,UUID,TYPE,AUTOCONNECT", "connection", "show"]) {
        let active = run("nmcli", &["-t", "-f", "UUID,STATE", "connection", "show", "--active"])
            .map(|o| parse_active(&o))
            .unwrap_or_default();
        for (name, uuid, kind, auto) in parse_nm(&profiles) {
            let service = if kind == "vpn" {
                run("nmcli", &["-g", "vpn.service-type", "connection", "show", "uuid", &uuid])
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let state = match active.iter().find(|(u, _)| *u == uuid).map(|(_, s)| s.as_str()) {
                Some("activated") => State::Up,
                Some("activating") => State::Connecting,
                _ => State::Down,
            };
            if is_proton(&name, &service) {
                if state != State::Down {
                    proton_active = Some((uuid, name));
                }
                continue; // listed once, as Proton VPN, below
            }
            out.push(Vpn {
                key: format!("nm:{uuid}"),
                detail: nm_kind(&kind, &service),
                provider: Provider::Nm { uuid, autoconnect: auto },
                name,
                state,
            });
        }
    }

    if let Some(t) = run("tailscale", &["status", "--json"]).ok().and_then(|j| parse_tailscale(&j)) {
        let detail = match (&t.exit_node, t.state.as_str()) {
            (_, "NeedsLogin") => "signed out: run `tailscale login`".to_string(),
            (Some(n), _) => format!("exit node: {n}"),
            (None, _) => "no exit node (tailnet only)".into(),
        };
        out.push(Vpn { key: "tailscale".into(), name: "Tailscale".into(), provider: Provider::Tailscale, state: tailscale_state(&t.state), detail });
    }

    if has(run, "protonvpn") || has(run, "protonvpn-app") {
        let (state, detail, active_uuid) = match proton_active {
            Some((uuid, name)) => (State::Up, name, Some(uuid)),
            None => {
                let c = PROTON_COUNTRY.get();
                (State::Down, if c.is_empty() { "fastest server".into() } else { format!("fastest in {c}") }, None)
            }
        };
        out.push(Vpn { key: "proton".into(), name: "Proton VPN".into(), provider: Provider::Proton { active_uuid }, state, detail });
    }

    if let Ok(s) = run("mullvad", &["status"]) {
        let (mut state, relay) = parse_mullvad(&s);
        if s.to_lowercase().contains("not logged in") {
            state = State::NeedsLogin;
        }
        out.push(Vpn { key: "mullvad".into(), name: "Mullvad".into(), provider: Provider::Mullvad, state, detail: relay });
    }
    out
}

// ---- actions -----------------------------------------------------------------

/// Turn a tool's refusal into what to do about it.
fn explain(e: String, v: &Vpn) -> String {
    let low = e.to_lowercase();
    match v.provider {
        Provider::Tailscale if low.contains("access denied") || low.contains("permission") => {
            "Tailscale needs your user as its operator once: sudo tailscale set --operator=$USER".into()
        }
        Provider::Nm { .. } if low.contains("secrets were required") || low.contains("no secrets") => {
            format!("{} needs a password: connect it from quick settings (Mod+A), which asks", v.name)
        }
        _ => e,
    }
}

/// Proton's CLI, when it runs at all (it has shipped broken against new Python).
fn proton_cli(run: Runner) -> bool {
    run("protonvpn", &["--version"]).is_ok()
}

pub fn connect(v: &Vpn, run: Runner) -> Result<String, String> {
    let r = match &v.provider {
        Provider::Nm { uuid, .. } => run("nmcli", &["-w", "20", "connection", "up", "uuid", uuid]).map(|_| format!("{} connected", v.name)),
        Provider::Tailscale => run("tailscale", &["up"]).map(|_| "Tailscale up".into()),
        Provider::Mullvad => run("mullvad", &["connect"]).map(|_| "Mullvad connecting".into()),
        Provider::Proton { .. } => {
            let c = PROTON_COUNTRY.get();
            if proton_cli(run) {
                let mut args = vec!["connect"];
                if !c.is_empty() {
                    args.extend(["--country", c.as_str()]);
                }
                run("protonvpn", &args).map(|_| "Proton VPN connected".into())
            } else {
                // The app is Proton's own window, not ours, but it signs in and
                // picks servers the way the CLI would have.
                run("sh", &["-c", "setsid -f protonvpn-app >/dev/null 2>&1"])
                    .map(|_| "Proton's CLI isn't working here; opened the Proton VPN app to connect".into())
            }
        }
    };
    r.map_err(|e| explain(e, v))
}

pub fn disconnect(v: &Vpn, run: Runner) -> Result<String, String> {
    let r = match &v.provider {
        Provider::Nm { uuid, .. } => run("nmcli", &["connection", "down", "uuid", uuid]).map(|_| format!("{} disconnected", v.name)),
        Provider::Tailscale => run("tailscale", &["down"]).map(|_| "Tailscale down".into()),
        Provider::Mullvad => run("mullvad", &["disconnect"]).map(|_| "Mullvad disconnected".into()),
        Provider::Proton { active_uuid } => {
            if proton_cli(run) {
                run("protonvpn", &["disconnect"]).map(|_| "Proton VPN disconnected".into())
            } else if let Some(u) = active_uuid {
                run("nmcli", &["connection", "down", "uuid", u]).map(|_| "Proton VPN disconnected".into())
            } else {
                Ok("Proton VPN is not connected".into())
            }
        }
    };
    r.map_err(|e| explain(e, v))
}

pub fn toggle(v: &Vpn, run: Runner) -> Result<String, String> {
    if v.is_up() { disconnect(v, run) } else { connect(v, run) }
}

/// Where this VPN can go: (label, id), and the current label.
pub fn locations(v: &Vpn, run: Runner) -> (Vec<(String, String)>, String) {
    match &v.provider {
        Provider::Tailscale => {
            let t = run("tailscale", &["status", "--json"]).ok().and_then(|j| parse_tailscale(&j)).unwrap_or_default();
            let mut c = vec![("none (tailnet only)".to_string(), String::new())];
            c.extend(t.exit_options.iter().map(|(h, ip)| (h.clone(), ip.clone())));
            let cur = t.exit_node.unwrap_or_else(|| "none (tailnet only)".into());
            (c, cur)
        }
        Provider::Mullvad => {
            let c = run("mullvad", &["relay", "list"]).map(|o| parse_mullvad_countries(&o)).unwrap_or_default();
            (c, v.detail.clone())
        }
        Provider::Proton { .. } => {
            let mut c = vec![("fastest anywhere".to_string(), String::new())];
            c.extend(PROTON_COUNTRIES.iter().map(|cc| (cc.to_string(), cc.to_string())));
            let cur = PROTON_COUNTRY.get();
            (c, if cur.is_empty() { "fastest anywhere".into() } else { cur })
        }
        Provider::Nm { .. } => (Vec::new(), "-".into()),
    }
}

pub fn set_location(v: &Vpn, id: &str, run: Runner) -> Result<String, String> {
    if id.starts_with('-') {
        return Err("refusing a location that looks like an option".into());
    }
    match &v.provider {
        Provider::Tailscale => run("tailscale", &["set", &format!("--exit-node={id}")])
            .map(|_| if id.is_empty() { "exit node off".to_string() } else { format!("exit node {id}") })
            .map_err(|e| explain(e, v)),
        Provider::Mullvad => run("mullvad", &["relay", "set", "location", id]).map(|_| format!("Mullvad location {id}")),
        Provider::Proton { .. } => {
            let cc = id.trim().to_uppercase();
            if !(cc.is_empty() || (cc.len() == 2 && cc.bytes().all(|b| b.is_ascii_uppercase()))) {
                return Err("a two-letter country code, e.g. CH".into());
            }
            PROTON_COUNTRY.set(&cc);
            Ok(if cc.is_empty() { "Proton: fastest anywhere next time".into() } else { format!("Proton: fastest in {cc} next time") })
        }
        Provider::Nm { .. } => Err("NetworkManager profiles go where their file says".into()),
    }
}

/// Import a WireGuard .conf or an OpenVPN .ovpn as a NetworkManager profile.
pub fn import(path: &str, run: Runner) -> Result<String, String> {
    let p = path.trim();
    let p = match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    };
    let kind = if p.ends_with(".conf") {
        "wireguard"
    } else if p.ends_with(".ovpn") {
        "openvpn"
    } else {
        return Err("a WireGuard .conf or an OpenVPN .ovpn file".into());
    };
    if p.starts_with('-') || !std::path::Path::new(&p).is_file() {
        return Err(format!("no such file: {p}"));
    }
    let out = run("nmcli", &["connection", "import", "type", kind, "file", &p])?;
    // "Connection 'work' (uuid) successfully added."
    let name = out.split('\'').nth(1).unwrap_or("profile").to_string();
    Ok(format!("imported {name}"))
}

// ---- the lifeconf panel --------------------------------------------------------

fn label(v: &Vpn) -> String {
    let st = match v.state {
        State::Up => "on",
        State::Connecting => "connecting",
        State::Down => "off",
        State::NeedsLogin => "signed out",
    };
    format!("{}  [{}]  {st}", v.name, v.provider_name())
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let vpns = list(run);
    if vpns.is_empty() {
        return vec![
            r("none set up"),
            r("import a WireGuard .conf or OpenVPN .ovpn below, or install Tailscale/Mullvad/Proton"),
            r("-"),
            r("-"),
            r("-"),
            r("path to a .conf or .ovpn"),
            r("-"),
        ];
    }
    let want = SELECTED.get();
    let sel = vpns.iter().find(|v| v.key == want).or_else(|| vpns.iter().find(|v| v.is_up())).unwrap_or(&vpns[0]);
    let (locs, loc_now) = locations(sel, run);
    let nm = matches!(sel.provider, Provider::Nm { .. });
    vec![
        Row { value: label(sel), choices: vpns.iter().map(|v| (label(v), v.key.clone())).collect() },
        r(&format!("{} — {}", if sel.is_up() { "connected" } else { "not connected" }, sel.detail)),
        Row {
            value: format!("{} {}", if sel.is_up() { "disconnect" } else { "connect" }, sel.name),
            choices: vec![(sel.key.clone(), String::new())],
        },
        match &sel.provider {
            Provider::Nm { autoconnect, .. } => r(&autoconnect.to_string()),
            _ => r("-"),
        },
        Row { value: loc_now, choices: locs },
        r("path to a .conf or .ovpn"),
        r(if nm { "type the profile's name to delete it" } else { "-" }),
    ]
}

fn selected(rows: &[Row], run: Runner) -> Result<Vpn, String> {
    let key = rows.get(2).and_then(|r| r.choices.first()).map(|c| c.0.clone()).ok_or("no VPN selected")?;
    list(run).into_iter().find(|v| v.key == key).ok_or_else(|| "that VPN is gone".to_string())
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    let row = rows.get(field).ok_or("no such row")?;
    match field {
        0 => {
            let (label, id) = super::pick(row, &ch, "VPN")?;
            SELECTED.set(id);
            Ok(format!("selected {label}"))
        }
        2 => toggle(&selected(rows, run)?, run),
        3 => {
            let v = selected(rows, run)?;
            let Provider::Nm { uuid, autoconnect } = &v.provider else { return Err("autoconnect is for NetworkManager profiles".into()) };
            let want = match &ch {
                Change::Text(t) => matches!(t.trim(), "true" | "on" | "1" | "yes"),
                _ => !autoconnect,
            };
            run("nmcli", &["connection", "modify", "uuid", uuid, "connection.autoconnect", if want { "yes" } else { "no" }])?;
            Ok(format!("{} autoconnect {}", v.name, if want { "on" } else { "off" }))
        }
        4 => {
            let v = selected(rows, run)?;
            if let (Provider::Proton { .. }, Change::Text(t)) = (&v.provider, &ch) {
                return set_location(&v, t, run); // any country code, typed
            }
            let (_, id) = super::pick(row, &ch, "location")?;
            set_location(&v, id, run)
        }
        5 => match ch {
            Change::Text(t) => import(&t, run),
            _ => Err("type the path to the file, then Enter".into()),
        },
        6 => {
            let v = selected(rows, run)?;
            let Provider::Nm { uuid, .. } = &v.provider else { return Err("only NetworkManager profiles are deleted here".into()) };
            match ch {
                Change::Text(t) if t.trim() == v.name => {
                    run("nmcli", &["connection", "delete", "uuid", uuid])?;
                    SELECTED.set("");
                    Ok(format!("deleted {}", v.name))
                }
                Change::Text(_) => Err(format!("type exactly '{}' to delete it", v.name)),
                _ => Err(format!("type '{}' and Enter to delete it", v.name)),
            }
        }
        _ => Err("read-only".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const PROFILES: &str = "BELL498 2.4:aaa:802-11-wireless:yes\nwork:u-work:vpn:no\nhome-wg:u-wg:wireguard:yes\nProtonVPN CH#12:u-pvpn:vpn:no\ntailscale0:t:tun:no\n";
    const ACTIVE: &str = "aaa:activated\nu-wg:activated\nu-pvpn:activated\n";
    const TS: &str = r#"{"BackendState":"Running","Peer":{"k1":{"HostName":"box","TailscaleIPs":["100.64.0.2","fd7a::2"],"ExitNodeOption":true,"ExitNode":true},"k2":{"HostName":"phone","TailscaleIPs":["100.64.0.3"],"ExitNodeOption":false,"ExitNode":false},"k3":{"HostName":"alpha","TailscaleIPs":["100.64.0.4"],"ExitNodeOption":true,"ExitNode":false}}}"#;

    fn fake(p: &str, a: &[&str]) -> Result<String, String> {
        Ok(match (p, a) {
            ("nmcli", [.., "show"]) => PROFILES.into(),
            ("nmcli", [.., "--active"]) => ACTIVE.into(),
            ("nmcli", ["-g", "vpn.service-type", .., uuid]) => match *uuid {
                "u-pvpn" => format!("{PROTUN}\n"),
                _ => "org.freedesktop.NetworkManager.openvpn\n".into(),
            },
            ("tailscale", ["status", "--json"]) => TS.into(),
            ("sh", ["-c", cmd]) if cmd.starts_with("command -v protonvpn") => "/usr/bin/protonvpn\n".into(),
            ("protonvpn", _) => return Err("protonvpn: Traceback".into()),
            ("mullvad", ["status"]) => "Connected\n    Relay: se-got-wg-001\n".into(),
            ("mullvad", ["relay", "list"]) => "Sweden (se)\n\tGothenburg (got)\n\t\tse-got-wg-001\nSwitzerland (ch)\n".into(),
            _ => return Err(format!("{p}: not faked")),
        })
    }

    #[test]
    fn lists_every_provider_and_files_proton_under_its_own_name() {
        let v = list(&fake);
        let names: Vec<_> = v.iter().map(|v| (v.name.as_str(), v.state, v.detail.as_str())).collect();
        assert_eq!(
            names,
            [
                ("work", State::Down, "OpenVPN"),
                ("home-wg", State::Up, "WireGuard"),
                ("Tailscale", State::Up, "exit node: box"),
                ("Proton VPN", State::Up, "ProtonVPN CH#12"),
                ("Mullvad", State::Up, "se-got-wg-001"),
            ]
        );
        assert!(!v.iter().any(|v| v.name == "tailscale0" || v.name.starts_with("BELL")));
    }

    #[test]
    fn parsers() {
        let t = parse_tailscale(TS).unwrap();
        assert_eq!(t.exit_options, [("alpha".into(), "100.64.0.4".into()), ("box".into(), "100.64.0.2".into())]);
        assert_eq!(t.exit_node.as_deref(), Some("box"));
        assert_eq!(parse_mullvad("Disconnected\n"), (State::Down, String::new()));
        assert_eq!(parse_mullvad("Connected to se-got-wg-001 in Gothenburg, SE\n").1, "se-got-wg-001 in Gothenburg, SE");
        assert_eq!(parse_mullvad_countries("Sweden (se)\n\tGothenburg (got)\nUSA (us)\n"), [("Sweden".into(), "se".into()), ("USA".into(), "us".into())]);
    }

    #[test]
    fn connecting_runs_the_right_tool_and_proton_falls_back_to_its_app() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            match p {
                "protonvpn" => Err("broken".into()),
                _ => fake(p, a).or(Ok(String::new())),
            }
        };
        let v = list(&fake);
        connect(&v[0], &rec).unwrap();
        disconnect(&v[1], &rec).unwrap();
        disconnect(&v[2], &rec).unwrap();
        let msg = connect(&Vpn { state: State::Down, provider: Provider::Proton { active_uuid: None }, ..v[3].clone() }, &rec).unwrap();
        assert!(msg.contains("opened the Proton VPN app"));
        disconnect(&v[3], &rec).unwrap(); // CLI broken: NetworkManager takes it down
        disconnect(&v[4], &rec).unwrap();
        let l: Vec<String> = log.borrow().iter().filter(|c| !c.starts_with("protonvpn ")).cloned().collect();
        assert_eq!(
            l,
            [
                "nmcli -w 20 connection up uuid u-work",
                "nmcli connection down uuid u-wg",
                "tailscale down",
                "sh -c setsid -f protonvpn-app >/dev/null 2>&1",
                "nmcli connection down uuid u-pvpn",
                "mullvad disconnect",
            ]
        );
    }

    #[test]
    fn refusals_say_what_to_do() {
        let v = list(&fake);
        let denied = |_: &str, _: &[&str]| -> Result<String, String> { Err("tailscale: Access denied: prefs write access denied".into()) };
        assert!(disconnect(&v[2], &denied).unwrap_err().contains("--operator=$USER"));
        let secrets = |_: &str, _: &[&str]| -> Result<String, String> { Err("nmcli: Error: Connection activation failed: Secrets were required, but not provided".into()) };
        assert!(connect(&v[0], &secrets).unwrap_err().contains("quick settings"));
    }

    #[test]
    fn panel_rows_select_locate_and_guard_delete() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SELECTED.set("tailscale");
        let rows = load(&fake);
        assert_eq!(rows.len(), LABELS.len());
        assert_eq!(rows[0].value, "Tailscale  [Tailscale]  on");
        assert_eq!(rows[4].value, "box");
        assert_eq!(rows[4].choices[0].1, "", "first choice turns the exit node off");
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            fake(p, a).or(Ok(String::new()))
        };
        apply(4, &rows, Change::Text("alpha".into()), &rec).unwrap();
        assert!(log.borrow().iter().any(|c| c == "tailscale set --exit-node=100.64.0.4"));

        SELECTED.set("nm:u-work");
        let rows = load(&fake);
        assert!(apply(6, &rows, Change::Toggle, &rec).is_err(), "delete needs the name typed");
        assert!(apply(6, &rows, Change::Text("wor".into()), &rec).is_err());
        apply(6, &rows, Change::Text("work".into()), &rec).unwrap();
        assert!(log.borrow().iter().any(|c| c == "nmcli connection delete uuid u-work"));
        SELECTED.set("");
    }

    #[test]
    fn import_picks_the_type_from_the_extension() {
        let dir = std::env::temp_dir().join(format!("lifeconf-vpn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wg = dir.join("home.conf");
        std::fs::write(&wg, "[Interface]\n").unwrap();
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok("Connection 'home' (u-1) successfully added.\n".into())
        };
        assert_eq!(import(wg.to_str().unwrap(), &rec).unwrap(), "imported home");
        assert!(log.borrow()[0].starts_with("nmcli connection import type wireguard file "));
        assert!(import("/etc/passwd", &rec).is_err());
        assert!(import(&dir.join("missing.ovpn").to_string_lossy(), &rec).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read-only look at this machine's VPNs: `cargo test -- --ignored live --nocapture`.
    #[test]
    #[ignore]
    fn live_list() {
        for v in list(&crate::sys::run_real) {
            println!("{:<28} {:<10?} {}", format!("{} [{}]", v.name, v.provider_name()), v.state, v.detail);
            let (locs, now) = locations(&v, &crate::sys::run_real);
            println!("    location: {now}  ({} choices)", locs.len());
        }
    }
}
