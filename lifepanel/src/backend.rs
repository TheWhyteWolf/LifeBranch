// SPDX-License-Identifier: GPL-3.0-or-later
// What the panel shows and what it can do, over the same tools lifeconf's
// panels use (nmcli, bluetoothctl, wpctl, brightnessctl, powerprofilesctl) and
// with their parsers: lifeconf/src/sys is compiled in, not copied.
//
// Everything here blocks (a wifi join can take ten seconds), so the window
// runs loads and jobs on worker threads and only ever sees the results.

use crate::drives::{self, Drive};
use crate::sys::{bluetooth, net, power, sound, split_terse, Runner};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wifi {
    /// Why there is no wifi to show (no adapter, no nmcli).
    pub missing: Option<String>,
    pub device: Option<String>,
    pub on: bool,
    pub aps: Vec<net::Ap>,
    /// (ssid, saved profile name)
    pub saved: Vec<(String, String)>,
    /// A connected wired link's connection name.
    pub wired: Option<String>,
}

impl Wifi {
    pub fn active(&self) -> Option<&net::Ap> {
        self.aps.iter().find(|a| a.in_use)
    }
    pub fn profile(&self, ssid: &str) -> Option<&str> {
        self.saved.iter().find(|(s, _)| s == ssid).map(|(_, n)| n.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BtDev {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bt {
    pub missing: Option<String>,
    pub on: bool,
    pub devs: Vec<BtDev>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Audio {
    pub missing: Option<String>,
    pub sinks: Vec<(String, String, bool)>, // (id, name, default)
    /// (percent, muted) of the default sink and source.
    pub out: Option<(i32, bool)>,
    pub inp: Option<(i32, bool)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Power {
    pub profiles: Vec<String>,
    pub profile: Option<String>,
    pub brightness: Option<i32>,
    pub battery: Option<String>,
}

/// One section's fresh state, as a worker sends it back.
#[derive(Debug)]
pub enum Loaded {
    Wifi(Wifi),
    Bt(Bt),
    Audio(Audio),
    Power(Power),
    Dnd(Option<bool>),
    Drives(Vec<Drive>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section {
    Wifi,
    Bt,
    Audio,
    Power,
    Dnd,
    Drives,
}

pub const ALL: [Section; 6] = [Section::Wifi, Section::Bt, Section::Audio, Section::Power, Section::Dnd, Section::Drives];

pub fn load(s: Section, run: Runner) -> Loaded {
    match s {
        Section::Wifi => Loaded::Wifi(load_wifi(run)),
        Section::Bt => Loaded::Bt(load_bt(run)),
        Section::Audio => Loaded::Audio(load_audio(run)),
        Section::Power => Loaded::Power(load_power(run)),
        Section::Dnd => Loaded::Dnd(load_dnd(run)),
        Section::Drives => Loaded::Drives(drives::load(run)),
    }
}

pub fn load_wifi(run: Runner) -> Wifi {
    let Ok(devs) = run("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]) else {
        return Wifi { missing: Some("nmcli unavailable".into()), ..Wifi::default() };
    };
    let rows: Vec<Vec<String>> = devs.lines().map(split_terse).filter(|f| f.len() >= 4).collect();
    let wired = rows.iter().find(|f| f[1] == "ethernet" && f[2] == "connected").map(|f| f[3].clone());
    let Some(device) = rows.iter().find(|f| f[1] == "wifi").map(|f| f[0].clone()) else {
        return Wifi { missing: Some("no wifi adapter".into()), wired, ..Wifi::default() };
    };
    let on = run("nmcli", &["radio", "wifi"]).is_ok_and(|o| o.trim() == "enabled");
    let mut w = Wifi { device: Some(device), on, wired, ..Wifi::default() };
    if on {
        w.aps = run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"])
            .map(|o| net::parse_aps(&o))
            .unwrap_or_default();
        w.saved = net::saved_ssids(run);
    }
    w
}

pub fn load_bt(run: Runner) -> Bt {
    let Ok(show) = run("bluetoothctl", &["show"]) else {
        return Bt { missing: Some("bluetoothctl unavailable".into()), ..Bt::default() };
    };
    if !show.contains("Controller") {
        return Bt { missing: Some("no bluetooth adapter".into()), ..Bt::default() };
    }
    let on = show.lines().any(|l| l.trim() == "Powered: yes");
    if !on {
        return Bt { on, ..Bt::default() };
    }
    let list = |args: &[&str]| run("bluetoothctl", args).map(|o| bluetooth::parse_devices(&o)).unwrap_or_default();
    let paired = list(&["devices", "Paired"]);
    let connected = list(&["devices", "Connected"]);
    let has = |v: &[bluetooth::Dev], mac: &str| v.iter().any(|x| x.mac == mac);
    let mut devs: Vec<BtDev> = list(&["devices"])
        .into_iter()
        .map(|d| BtDev { paired: has(&paired, &d.mac), connected: has(&connected, &d.mac), name: d.name, mac: d.mac })
        .collect();
    // Connected, then paired, then whatever a scan found; by name within.
    devs.sort_by_key(|d| (!d.connected, !d.paired, d.name.to_lowercase()));
    Bt { missing: None, on, devs }
}

pub fn load_audio(run: Runner) -> Audio {
    let Ok(status) = run("wpctl", &["status"]) else {
        return Audio { missing: Some("wpctl unavailable".into()), ..Audio::default() };
    };
    let (sinks, _) = sound::parse_status(&status);
    let vol = |t: &str| run("wpctl", &["get-volume", t]).ok().and_then(|o| sound::parse_volume(&o));
    Audio {
        missing: None,
        sinks: sinks.into_iter().map(|d| (d.id, d.name, d.default)).collect(),
        out: vol(SINK),
        inp: vol(SOURCE),
    }
}

pub fn load_power(run: Runner) -> Power {
    let profiles = run("powerprofilesctl", &["list"]).map(|o| power::parse_profiles(&o)).unwrap_or_default();
    let profile = run("powerprofilesctl", &["get"]).ok().map(|o| o.trim().to_string()).filter(|p| !p.is_empty());
    let brightness = run("brightnessctl", &["-m"]).ok().and_then(|o| power::parse_brightness(&o));
    let battery = run("upower", &["-i", "/org/freedesktop/UPower/devices/DisplayDevice"])
        .ok()
        .map(|o| power::parse_battery(&o))
        .filter(|b| b != "no battery");
    Power { profiles, profile, brightness, battery }
}

pub fn lifenote() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lifenote");
    if std::path::Path::new(&local).is_file() { local } else { "lifenote".into() }
}

pub fn load_dnd(run: Runner) -> Option<bool> {
    run(&lifenote(), &["ctl", "dnd", "get"]).ok().map(|o| o.trim() == "on")
}

pub const SINK: &str = "@DEFAULT_AUDIO_SINK@";
pub const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

/// How to join a network.
pub enum Join {
    Saved(String),
    Open,
    /// A new secured network: the password goes to nmcli on stdin, never argv.
    Password(Zeroizing<String>),
}

pub enum Job {
    WifiRadio(bool),
    WifiJoin { ssid: String, how: Join },
    WifiDisconnect(String),
    WifiRescan,
    BtPower(bool),
    BtConnect { mac: String, name: String, pair: bool },
    BtDisconnect(String),
    BtScan,
    Volume { sink: bool, pct: i32 },
    Mute { sink: bool, on: bool },
    Output(String),
    Brightness(i32),
    Profile(String),
    Dnd(bool),
    Mount(String),
    Eject(String, Vec<Drive>),
    Open(String),
    Settings,
}

impl Job {
    /// What to reload once it's done.
    pub fn section(&self) -> Option<Section> {
        match self {
            Job::WifiRadio(_) | Job::WifiJoin { .. } | Job::WifiDisconnect(_) | Job::WifiRescan => Some(Section::Wifi),
            Job::BtPower(_) | Job::BtConnect { .. } | Job::BtDisconnect(_) | Job::BtScan => Some(Section::Bt),
            // Sliders and mute are applied locally as they're dragged; a reload
            // racing the next step would make the bar jump back.
            Job::Volume { .. } | Job::Mute { .. } | Job::Brightness(_) => None,
            Job::Output(_) => Some(Section::Audio),
            Job::Profile(_) => Some(Section::Power),
            Job::Dnd(_) => Some(Section::Dnd),
            Job::Mount(_) | Job::Eject(..) => Some(Section::Drives),
            Job::Open(_) | Job::Settings => None,
        }
    }

    /// What the status line says while it runs (quick jobs say nothing).
    pub fn busy(&self) -> Option<String> {
        match self {
            Job::WifiJoin { ssid, .. } => Some(format!("joining {ssid}…")),
            Job::BtConnect { name, pair: true, .. } => Some(format!("pairing {name}…")),
            Job::BtConnect { name, .. } => Some(format!("connecting {name}…")),
            Job::BtScan => Some("looking for devices…".into()),
            Job::Mount(p) => Some(format!("mounting {p}…")),
            Job::Eject(d, _) => Some(format!("ejecting {d}…")),
            _ => None,
        }
    }
}

fn onoff(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// Seconds nmcli may spend on one join.
const WAIT: &str = "15";

/// Run a job; Ok carries a line for the status bar (empty: say nothing).
pub fn run_job(job: Job, run: Runner) -> Result<String, String> {
    match job {
        Job::WifiRadio(on) => run("nmcli", &["radio", "wifi", onoff(on)]).map(|_| format!("wifi {}", onoff(on))),
        Job::WifiJoin { ssid, how } => {
            if ssid.starts_with('-') {
                return Err("refusing a network name that looks like an option".into());
            }
            match how {
                Join::Saved(name) => run("nmcli", &["-w", WAIT, "connection", "up", "id", &name])?,
                Join::Open => run("nmcli", &["-w", WAIT, "device", "wifi", "connect", &ssid])?,
                Join::Password(pw) => join_with_password(&ssid, &pw)?,
            };
            Ok(format!("connected to {ssid}"))
        }
        Job::WifiDisconnect(dev) => run("nmcli", &["device", "disconnect", &dev]).map(|_| "wifi disconnected".into()),
        Job::WifiRescan => {
            // nmcli refuses a rescan right after another one; the list it has
            // is recent then, so that is not an error worth showing.
            let _ = run("nmcli", &["device", "wifi", "rescan"]);
            std::thread::sleep(std::time::Duration::from_secs(3));
            Ok(String::new())
        }
        Job::BtPower(on) => {
            bluetooth::check(&run("bluetoothctl", &["power", onoff(on)])?, "succeeded")?;
            Ok(format!("bluetooth {}", onoff(on)))
        }
        Job::BtConnect { mac, name, pair } => {
            if pair {
                // timeout(1): an unanswered pairing must not hang the worker.
                bluetooth::check(&run("timeout", &["20", "bluetoothctl", "pair", &mac])?, "Pairing successful")?;
                let _ = run("bluetoothctl", &["trust", &mac]);
            }
            bluetooth::check(&run("timeout", &["15", "bluetoothctl", "connect", &mac])?, "Connection successful")?;
            Ok(format!("connected {name}"))
        }
        Job::BtDisconnect(mac) => {
            bluetooth::check(&run("bluetoothctl", &["disconnect", &mac])?, "Successful disconnected")?;
            Ok("disconnected".into())
        }
        Job::BtScan => {
            run("bluetoothctl", &["--timeout", "8", "scan", "on"])?;
            Ok(String::new())
        }
        Job::Volume { sink, pct } => {
            let t = if sink { SINK } else { SOURCE };
            run("wpctl", &["set-volume", t, &format!("{:.2}", pct.clamp(0, 150) as f64 / 100.0)]).map(|_| String::new())
        }
        Job::Mute { sink, on } => {
            run("wpctl", &["set-mute", if sink { SINK } else { SOURCE }, if on { "1" } else { "0" }]).map(|_| String::new())
        }
        Job::Output(id) => run("wpctl", &["set-default", &id]).map(|_| String::new()),
        // Never 0: a dark panel with no way to see the UI to undo it.
        Job::Brightness(p) => run("brightnessctl", &["set", &format!("{}%", p.clamp(1, 100))]).map(|_| String::new()),
        Job::Profile(p) => run("powerprofilesctl", &["set", &p]).map(|_| format!("power profile {p}")),
        Job::Dnd(on) => {
            run(&lifenote(), &["ctl", "dnd", onoff(on)])?;
            // waybar's DND label listens for this.
            let _ = run("pkill", &["-RTMIN+8", "waybar"]);
            Ok(format!("do not disturb {}", onoff(on)))
        }
        Job::Mount(path) => drives::mount(&path, run).map(|mp| format!("mounted at {mp}")),
        Job::Eject(disk, all) => drives::eject(&disk, &all, run).map(|_| format!("{disk} can be removed")),
        Job::Open(dir) => {
            spawn(&["xdg-open", &dir])?;
            Ok(String::new())
        }
        Job::Settings => {
            let home = std::env::var("HOME").unwrap_or_default();
            let local = format!("{home}/.local/bin/lifeconf");
            let bin = if std::path::Path::new(&local).is_file() { local.as_str() } else { "lifeconf" };
            spawn(&[bin, "--gui"])?;
            Ok(String::new())
        }
    }
}

/// Start a program detached from us (its own session), not waited for.
fn spawn(argv: &[&str]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut c = Command::new(argv[0]);
    c.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches nothing else.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|e| format!("{}: {e}", argv[0]))?;
    // Reap it in the background so it never lingers as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn join_with_password(ssid: &str, pw: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("nmcli")
        .args(["--ask", "-w", WAIT, "device", "wifi", "connect", ssid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("nmcli: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let mut line = Zeroizing::new(Vec::with_capacity(pw.len() + 1));
        line.extend_from_slice(pw.as_bytes());
        line.push(b'\n');
        let _ = stdin.write_all(&line);
    }
    let out = child.wait_with_output().map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("failed");
        Err(if last.contains("Secrets were required") || last.contains("property is invalid") {
            "wrong password".to_string()
        } else {
            last.trim_start_matches("Error: ").to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn fake(p: &str, a: &[&str]) -> Result<String, String> {
        Ok(match (p, a.first().copied().unwrap_or("")) {
            ("nmcli", "-t") if a.contains(&"DEVICE,TYPE,STATE,CONNECTION") => {
                "wlan0:wifi:connected:BELL498 2.4\nt2_ncm:ethernet:disconnected:\neno1:ethernet:connected:Wired 1\n".into()
            }
            ("nmcli", "radio") => "enabled\n".into(),
            ("nmcli", "-t") if a.contains(&"IN-USE,SSID,SIGNAL,SECURITY") => "*:BELL498 2.4:60:WPA2\n:Cafe:30:\n".into(),
            ("nmcli", "-t") => "BELL498 2.4:802-11-wireless\n".into(),
            ("nmcli", "--escape") => "BELL498 2.4\n".into(),
            ("bluetoothctl", "show") => "Controller 14:7D:DA:26:07:C9 (public)\n\tPowered: yes\n".into(),
            ("bluetoothctl", "devices") if a.len() == 1 => {
                "Device F4:4E:FD:52:89:C1 SoundCore mini\nDevice 28:73:F6:13:00:18 Echo Pop\nDevice AA:BB:CC:DD:EE:FF Zed\n".into()
            }
            ("bluetoothctl", "devices") if a[1] == "Paired" => "Device F4:4E:FD:52:89:C1 SoundCore mini\nDevice 28:73:F6:13:00:18 Echo Pop\n".into(),
            ("bluetoothctl", "devices") => "Device 28:73:F6:13:00:18 Echo Pop\n".into(),
            _ => return Err(format!("{p}: not faked")),
        })
    }

    #[test]
    fn wifi_state_with_wired_and_saved() {
        let w = load_wifi(&fake);
        assert_eq!(w.device.as_deref(), Some("wlan0"));
        assert_eq!(w.wired.as_deref(), Some("Wired 1"), "the disconnected T2 link is not 'wired'");
        assert!(w.on);
        assert_eq!(w.active().map(|a| a.ssid.as_str()), Some("BELL498 2.4"));
        assert_eq!(w.profile("BELL498 2.4"), Some("BELL498 2.4"));
        assert_eq!(w.profile("Cafe"), None);
    }

    #[test]
    fn no_wifi_adapter_or_nmcli() {
        let eth = |p: &str, a: &[&str]| -> Result<String, String> {
            if p == "nmcli" && a.contains(&"DEVICE,TYPE,STATE,CONNECTION") { Ok("eno1:ethernet:connected:Wired 1\n".into()) } else { Err("x".into()) }
        };
        let w = load_wifi(&eth);
        assert_eq!((w.missing.as_deref(), w.wired.as_deref()), (Some("no wifi adapter"), Some("Wired 1")));
        let none = |_: &str, _: &[&str]| -> Result<String, String> { Err("x".into()) };
        assert_eq!(load_wifi(&none).missing.as_deref(), Some("nmcli unavailable"));
        assert_eq!(load_bt(&none).missing.as_deref(), Some("bluetoothctl unavailable"));
        assert_eq!(load_audio(&none).missing.as_deref(), Some("wpctl unavailable"));
        assert_eq!(load_power(&none), Power::default());
    }

    #[test]
    fn bluetooth_orders_connected_paired_new() {
        let b = load_bt(&fake);
        let order: Vec<_> = b.devs.iter().map(|d| (d.name.as_str(), d.paired, d.connected)).collect();
        assert_eq!(order, [("Echo Pop", true, true), ("SoundCore mini", true, false), ("Zed", false, false)]);
    }

    #[test]
    fn jobs_issue_the_commands_lifeconf_does() {
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(match a {
                [.., "pair", _] => "Pairing successful".into(),
                [.., "connect", _] => "Connection successful".into(),
                ["power", _] => "Changing power on succeeded".into(),
                _ => String::new(),
            })
        };
        let jobs = vec![
            Job::WifiJoin { ssid: "Home".into(), how: Join::Saved("Home 1".into()) },
            Job::WifiJoin { ssid: "Cafe".into(), how: Join::Open },
            Job::BtConnect { mac: "AA:BB:CC:DD:EE:FF".into(), name: "Zed".into(), pair: true },
            Job::BtPower(false),
            Job::Volume { sink: true, pct: 200 },
            Job::Mute { sink: false, on: true },
            Job::Brightness(0),
            Job::Profile("balanced".into()),
        ];
        for j in jobs {
            run_job(j, &rec).unwrap();
        }
        assert_eq!(
            *log.borrow(),
            [
                "nmcli -w 15 connection up id Home 1",
                "nmcli -w 15 device wifi connect Cafe",
                "timeout 20 bluetoothctl pair AA:BB:CC:DD:EE:FF",
                "bluetoothctl trust AA:BB:CC:DD:EE:FF",
                "timeout 15 bluetoothctl connect AA:BB:CC:DD:EE:FF",
                "bluetoothctl power off",
                "wpctl set-volume @DEFAULT_AUDIO_SINK@ 1.50",
                "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ 1",
                "brightnessctl set 1%",
                "powerprofilesctl set balanced",
            ]
        );
        let boom = |_: &str, _: &[&str]| -> Result<String, String> { panic!("must not run") };
        assert!(run_job(Job::WifiJoin { ssid: "-evil".into(), how: Join::Open }, &boom).is_err());
    }
}
