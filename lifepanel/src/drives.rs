// SPDX-License-Identifier: GPL-3.0-or-later
// Removable drives: what lsblk sees, mounted and ejected through udisks
// (udisksctl), so the usual polkit rules apply and a drive lands under
// /run/media/$USER like every other desktop puts it. Replaces udiskie.

use crate::sys::Runner;

#[derive(Debug, Clone, PartialEq)]
pub struct Drive {
    /// The filesystem's block device, e.g. /dev/sdb1.
    pub path: String,
    /// The whole disk it lives on (what gets powered off), e.g. /dev/sdb.
    pub disk: String,
    pub name: String,
    pub size: String,
    pub mountpoint: Option<String>,
}

/// `lsblk -P` values: KEY="value" pairs, with odd bytes escaped as \xNN.
pub fn parse_pairs(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim().to_string();
        let body = &rest[eq + 2..];
        let Some(end) = body.find('"') else { break };
        out.push((key, unescape_hex(&body[..end])));
        rest = &body[end + 1..];
    }
    out
}

fn unescape_hex(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'x') {
            if let Some(v) = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub const LSBLK: &[&str] = &["-P", "-o", "PATH,LABEL,SIZE,FSTYPE,MOUNTPOINT,HOTPLUG,TYPE,PKNAME,MODEL"];

/// Hot-pluggable filesystems: USB sticks, SD cards, external disks. Swap and
/// encrypted containers are left out (a locked LUKS volume needs a
/// passphrase, which is lifefiles' business, not a quick-settings row).
pub fn parse(out: &str) -> Vec<Drive> {
    let rows: Vec<Vec<(String, String)>> = out.lines().map(parse_pairs).collect();
    let get = |r: &[(String, String)], k: &str| r.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let model_of = |disk: &str| {
        rows.iter().find(|r| get(r, "PATH") == disk).map(|r| get(r, "MODEL").trim().to_string()).unwrap_or_default()
    };
    rows.iter()
        .filter(|r| get(r, "HOTPLUG") == "1")
        .filter(|r| matches!(get(r, "TYPE").as_str(), "part" | "disk"))
        .filter(|r| !matches!(get(r, "FSTYPE").as_str(), "" | "swap" | "crypto_LUKS"))
        .map(|r| {
            let path = get(r, "PATH");
            let pk = get(r, "PKNAME");
            let disk = if pk.is_empty() { path.clone() } else { format!("/dev/{pk}") };
            let label = get(r, "LABEL");
            let name = if !label.is_empty() {
                label
            } else {
                let m = model_of(&disk);
                if m.is_empty() { path.trim_start_matches("/dev/").to_string() } else { m }
            };
            let mp = get(r, "MOUNTPOINT");
            Drive { path, disk, name, size: get(r, "SIZE"), mountpoint: (!mp.is_empty()).then_some(mp) }
        })
        .collect()
}

pub fn load(run: Runner) -> Vec<Drive> {
    run("lsblk", LSBLK).map(|o| parse(&o)).unwrap_or_default()
}

/// Device paths end up in argv: only plain /dev/ names.
fn safe(path: &str) -> Result<&str, String> {
    let ok = path.starts_with("/dev/")
        && path.len() > 5
        && path[5..].bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b));
    if ok { Ok(path) } else { Err(format!("refusing odd device path {path:?}")) }
}

/// `Mounted /dev/sdb1 at /run/media/voyd/STICK` -> the mount point.
pub fn parse_mounted(out: &str) -> Option<String> {
    let at = out.find(" at ")?;
    Some(out[at + 4..].trim().trim_end_matches('.').to_string())
}

pub fn mount(path: &str, run: Runner) -> Result<String, String> {
    let out = run("udisksctl", &["mount", "-b", safe(path)?])?;
    Ok(parse_mounted(&out).unwrap_or_else(|| path.to_string()))
}

/// Unmount every mounted filesystem on `disk`, then power it off so it is
/// safe to pull.
pub fn eject(disk: &str, all: &[Drive], run: Runner) -> Result<(), String> {
    for d in all.iter().filter(|d| d.disk == disk && d.mountpoint.is_some()) {
        run("udisksctl", &["unmount", "-b", safe(&d.path)?])?;
    }
    run("udisksctl", &["power-off", "-b", safe(disk)?])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Real shape from this machine, plus a stick with two partitions.
    const OUT: &str = r#"PATH="/dev/nvme0n1" LABEL="" SIZE="465.9G" FSTYPE="" MOUNTPOINT="" HOTPLUG="0" TYPE="disk" PKNAME="" MODEL="APPLE SSD AP0512N"
PATH="/dev/nvme0n1p6" LABEL="" SIZE="247.5G" FSTYPE="btrfs" MOUNTPOINT="/var/log" HOTPLUG="0" TYPE="part" PKNAME="nvme0n1" MODEL=""
PATH="/dev/sdb" LABEL="" SIZE="28.9G" FSTYPE="" MOUNTPOINT="" HOTPLUG="1" TYPE="disk" PKNAME="" MODEL="SanDisk Ultra  "
PATH="/dev/sdb1" LABEL="MY\x20STICK" SIZE="28G" FSTYPE="exfat" MOUNTPOINT="/run/media/voyd/MY STICK" HOTPLUG="1" TYPE="part" PKNAME="sdb" MODEL=""
PATH="/dev/sdb2" LABEL="" SIZE="900M" FSTYPE="vfat" MOUNTPOINT="" HOTPLUG="1" TYPE="part" PKNAME="sdb" MODEL=""
PATH="/dev/sdb3" LABEL="" SIZE="1G" FSTYPE="crypto_LUKS" MOUNTPOINT="" HOTPLUG="1" TYPE="part" PKNAME="sdb" MODEL=""
PATH="/dev/sdc" LABEL="CARD" SIZE="7.4G" FSTYPE="vfat" MOUNTPOINT="" HOTPLUG="1" TYPE="disk" PKNAME="" MODEL="SD"
"#;

    #[test]
    fn lists_only_removable_filesystems() {
        let d = parse(OUT);
        let names: Vec<_> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["MY STICK", "SanDisk Ultra", "CARD"], "label, else the disk's model; no LUKS, no internal");
        assert_eq!(d[0].mountpoint.as_deref(), Some("/run/media/voyd/MY STICK"));
        assert_eq!((d[1].disk.as_str(), d[1].mountpoint.as_deref()), ("/dev/sdb", None));
        assert_eq!(d[2].disk, "/dev/sdc", "a partitionless card is its own disk");
    }

    #[test]
    fn eject_unmounts_that_disk_then_powers_it_off() {
        let all = parse(OUT);
        let log = RefCell::new(Vec::<String>::new());
        let rec = |p: &str, a: &[&str]| -> Result<String, String> {
            log.borrow_mut().push(format!("{p} {}", a.join(" ")));
            Ok(String::new())
        };
        eject("/dev/sdb", &all, &rec).unwrap();
        assert_eq!(*log.borrow(), ["udisksctl unmount -b /dev/sdb1", "udisksctl power-off -b /dev/sdb"]);
        assert!(mount("/dev/sdb1; rm", &rec).is_err());
        assert!(mount("-x", &rec).is_err());
    }

    #[test]
    fn reads_udisksctl_mount_output() {
        assert_eq!(parse_mounted("Mounted /dev/sdb1 at /run/media/voyd/MY STICK\n").as_deref(), Some("/run/media/voyd/MY STICK"));
        assert_eq!(parse_mounted("Mounted /dev/sdc at /run/media/voyd/CARD.\n").as_deref(), Some("/run/media/voyd/CARD"));
    }
}
