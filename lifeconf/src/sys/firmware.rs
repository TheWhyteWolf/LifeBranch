// SPDX-License-Identifier: GPL-3.0-or-later
// Firmware panel: what fwupd can update (UEFI, SSDs, docks, receivers...),
// what LVFS has newer, and a way to install it.
//
// fwupd's daemon is D-Bus activated: it starts when this page asks and costs
// nothing otherwise. Checking downloads LVFS's metadata, so like the Updates
// page it happens only when asked, and the time of the last check is kept in
// ~/.cache/lifeconf/firmware. Updating opens a terminal running
// `fwupdmgr update`: flashing asks before each device, may need the charger
// or a reboot, and its output is worth seeing.
//
// Apple hardware gets its firmware through macOS, not LVFS, so on a Mac this
// page usually shows little or nothing to update.

use super::updates::ago;
use super::{pick, Change, Row, RowKind, Runner, Sel};
use serde_json::Value;

pub const LABELS: &[&str] = &["devices", "updates", "device", "update", "last checked", "check now", "update now"];

static SELECTED: Sel = Sel::new();

pub fn kind(field: usize) -> RowKind {
    match field {
        2 => RowKind::Choice,
        5 | 6 => RowKind::Action,
        _ => RowKind::Info,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub version: String,
    pub updatable: bool,
    /// Newer releases, newest first: (version, summary, urgency).
    pub releases: Vec<(String, String, String)>,
    pub needs_reboot: bool,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn flags(v: &Value) -> Vec<String> {
    v.get("Flags").and_then(Value::as_array).map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// fwupdmgr's `--json` device list (get-devices or get-updates).
pub fn parse(json: &str) -> Result<Vec<Device>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("fwupdmgr's output: {e}"))?;
    let devs = v.get("Devices").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(devs
        .iter()
        .map(|d| {
            let f = flags(d);
            Device {
                id: s(d, "DeviceId"),
                name: s(d, "Name"),
                version: s(d, "Version"),
                updatable: f.iter().any(|x| x == "updatable"),
                needs_reboot: f.iter().any(|x| x == "needs-reboot"),
                releases: d
                    .get("Releases")
                    .and_then(Value::as_array)
                    .map(|rs| rs.iter().map(|r| (s(r, "Version"), s(r, "Summary"), s(r, "Urgency"))).collect())
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// fwupdmgr says "nothing to do" by failing; those aren't errors here.
fn nothing(e: &str) -> bool {
    let e = e.to_lowercase();
    ["no updat", "no upgrades", "nothing to do", "no devices"].iter().any(|m| e.contains(m))
}

fn stamp_path() -> String {
    let home = std::env::var("LIFECONF_HOME").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    format!("{home}/.cache/lifeconf/firmware")
}

fn last_checked() -> Option<u64> {
    let t = std::fs::metadata(stamp_path()).ok()?.modified().ok()?;
    std::time::SystemTime::now().duration_since(t).ok().map(|d| d.as_secs())
}

/// "Name 1.2 → 1.3" for the picker.
pub fn label(d: &Device) -> String {
    match d.releases.first() {
        Some((v, _, _)) => format!("{}  {} -> {v}", d.name, d.version),
        None => format!("{}  {}", d.name, d.version),
    }
}

pub fn load(run: Runner) -> Vec<Row> {
    let r = |v: &str| Row { value: v.into(), choices: vec![] };
    let mut rows = vec![r("-"); LABELS.len()];
    let devices = match run("fwupdmgr", &["get-devices", "--json"]) {
        Ok(out) => parse(&out).unwrap_or_default(),
        Err(e) if e.contains("No such file") || e.contains("not found") => {
            rows[0] = r("fwupd isn't installed (sudo pacman -S fwupd)");
            return rows;
        }
        Err(e) if nothing(&e) => Vec::new(),
        Err(e) => {
            rows[0] = r(&e);
            return rows;
        }
    };
    let updatable = devices.iter().filter(|d| d.updatable).count();
    rows[0] = r(&format!("{updatable} fwupd can update, of {} found", devices.len()));
    let updates = match run("fwupdmgr", &["get-updates", "--json"]) {
        Ok(out) => parse(&out).unwrap_or_default().into_iter().filter(|d| !d.releases.is_empty()).collect(),
        Err(e) if nothing(&e) => Vec::new(),
        Err(e) => {
            rows[1] = r(&e);
            Vec::new()
        }
    };
    let checked = last_checked();
    if rows[1].value == "-" {
        rows[1] = r(&match (updates.len(), checked) {
            (0, None) => "none known yet: check now to ask LVFS".into(),
            (0, Some(_)) => "none: everything is current".into(),
            (n, _) => format!("{n} available"),
        });
    }
    let mut sel = SELECTED.get();
    if !updates.iter().any(|d| d.id == sel) {
        sel = updates.first().map(|d| d.id.clone()).unwrap_or_default();
        SELECTED.set(&sel);
    }
    let chosen = updates.iter().find(|d| d.id == sel);
    rows[2] = Row {
        value: chosen.map(label).unwrap_or_else(|| "-".into()),
        choices: updates.iter().map(|d| (label(d), d.id.clone())).collect(),
    };
    rows[3] = r(&chosen
        .and_then(|d| {
            let (_, summary, urgency) = d.releases.first()?;
            let mut out = summary.clone();
            if !urgency.is_empty() && urgency != "unknown" {
                out.push_str(&format!(" ({urgency} urgency)"));
            }
            if d.needs_reboot {
                out.push_str("; installs at the next reboot");
            }
            Some(out)
        })
        .unwrap_or_else(|| "-".into()));
    rows[4] = r(&checked.map(ago).unwrap_or_else(|| "never".into()));
    rows[5] = r("download LVFS's list");
    rows[6] = r(if updates.is_empty() { "nothing to install" } else { "install the device above (opens a terminal)" });
    rows
}

pub fn apply(field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    match field {
        2 => {
            let (label, id) = pick(rows.get(2).ok_or("no such row")?, &ch, "device")?;
            SELECTED.set(id);
            Ok(label.clone())
        }
        5 => {
            match run("fwupdmgr", &["refresh", "--force"]) {
                Ok(_) => {}
                Err(e) if nothing(&e) => {}
                Err(e) => return Err(e),
            }
            let p = stamp_path();
            if let Some(dir) = std::path::Path::new(&p).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(&p, "").map_err(|e| format!("{p}: {e}"))?;
            Ok("checked LVFS".into())
        }
        6 => {
            if rows.get(6).is_some_and(|r| r.value == "nothing to install") {
                return Err("nothing to install: check now first".into());
            }
            // The device picked above, not every device: fwupdmgr takes the
            // DeviceId. It goes through two shells, so only take ids that
            // can't be shell syntax (fwupd's are hex digests).
            let dev = rows.get(2).ok_or("no such row")?;
            let id = dev
                .choices
                .iter()
                .find(|(label, _)| *label == dev.value)
                .map(|(_, id)| id.as_str())
                .ok_or("pick a device above first")?;
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(format!("odd device id {id:?}"));
            }
            let script = format!("fwupdmgr update {id}; echo; echo 'Done. Press Enter to close.'; read _");
            run("sh", &["-c", &format!("setsid -f kitty --title 'Firmware update' sh -c \"{script}\" >/dev/null 2>&1")])?;
            Ok(format!("updating {} in a terminal", dev.value.split("  ").next().unwrap_or("the device")))
        }
        _ => Err("nothing to change here".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICES: &str = r#"{"Devices":[
        {"Name":"System Firmware","DeviceId":"aa11","Version":"1.20","Flags":["internal","updatable","needs-reboot"]},
        {"Name":"Unifying Receiver","DeviceId":"bb22","Version":"RQR12.10","Flags":["updatable"]},
        {"Name":"Webcam","DeviceId":"cc33","Version":"","Flags":["internal"]}]}"#;
    const UPDATES: &str = r#"{"Devices":[
        {"Name":"System Firmware","DeviceId":"aa11","Version":"1.20","Flags":["internal","updatable","needs-reboot"],
         "Releases":[{"Version":"1.24","Summary":"Lenovo ThinkPad System Firmware","Urgency":"high"}]}]}"#;

    fn fake(devices: &'static str, updates: Result<&'static str, &'static str>) -> impl Fn(&str, &[&str]) -> Result<String, String> {
        move |_: &str, a: &[&str]| match a.first() {
            Some(&"get-devices") => Ok(devices.into()),
            Some(&"get-updates") => updates.map(String::from).map_err(String::from),
            _ => Err("unexpected".into()),
        }
    }

    #[test]
    fn parses_devices_releases_and_flags() {
        let d = parse(UPDATES).unwrap();
        assert_eq!(d[0].releases[0], ("1.24".into(), "Lenovo ThinkPad System Firmware".into(), "high".into()));
        assert!(d[0].needs_reboot && d[0].updatable);
        assert_eq!(label(&d[0]), "System Firmware  1.20 -> 1.24");
        let all = parse(DEVICES).unwrap();
        assert_eq!(all.iter().filter(|d| d.updatable).count(), 2);
        assert!(parse("not json").is_err());
    }

    #[test]
    fn rows_for_an_update_and_for_none() {
        let rows = load(&fake(DEVICES, Ok(UPDATES)));
        assert_eq!(rows[0].value, "2 fwupd can update, of 3 found");
        assert_eq!(rows[1].value, "1 available");
        assert_eq!(rows[2].value, "System Firmware  1.20 -> 1.24");
        assert_eq!(rows[3].value, "Lenovo ThinkPad System Firmware (high urgency); installs at the next reboot");
        // fwupdmgr reports "no updates" by failing: that's an answer, not an error.
        let rows = load(&fake(DEVICES, Err("fwupdmgr: No updatable devices")));
        assert!(rows[1].value.starts_with("none"), "{}", rows[1].value);
        assert_eq!(rows[6].value, "nothing to install");
    }

    #[test]
    fn update_now_installs_only_the_chosen_device() {
        let rows = load(&fake(DEVICES, Ok(UPDATES)));
        let ran = std::cell::RefCell::new(Vec::new());
        let run = |c: &str, a: &[&str]| {
            ran.borrow_mut().push(format!("{c} {}", a.join(" ")));
            Ok(String::new())
        };
        assert_eq!(apply(6, &rows, Change::Toggle, &run).unwrap(), "updating System Firmware in a terminal");
        assert!(ran.borrow()[0].contains("fwupdmgr update aa11;"), "{}", ran.borrow()[0]);
        // An id that could be shell syntax is refused, never run.
        let mut odd = rows.clone();
        odd[2].choices[0].1 = "aa11; rm -rf ~".into();
        assert!(apply(6, &odd, Change::Toggle, &run).is_err());
        assert_eq!(ran.borrow().len(), 1);
    }

    #[test]
    fn missing_fwupd_says_how_to_get_it() {
        let gone = |_: &str, _: &[&str]| Err::<String, String>("fwupdmgr: No such file or directory (os error 2)".into());
        assert!(load(&gone)[0].value.contains("pacman -S fwupd"));
    }
}
