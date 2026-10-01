// SPDX-License-Identifier: GPL-3.0-or-later
// `lifepanel --watch`: the part of udiskie worth keeping. It sleeps on udev's
// netlink broadcast (no polling) and, when a filesystem is plugged in, mounts
// it through udisks and says so with a notification.
//
// udev's group carries events after its rules ran, so the filesystem has
// already been probed (ID_FS_USAGE) by the time we hear about it. Only root
// can send on that socket; we still re-check everything with lsblk before
// acting, and device names pass drives::safe before reaching argv.

use crate::drives::{self, Drive};
use crate::sys::run_real;
use std::time::Duration;

/// libudev's monitor groups: 1 is the kernel's raw events, 2 is udevd's.
const UDEV_GROUP: u32 = 2;

/// The properties of one udev monitor message: "libudev\0", a header whose
/// u32 at offset 16 is where the NUL-separated KEY=VALUE list starts and at
/// 20 how long it is. None for anything else (kernel-format messages).
pub fn parse_udev(buf: &[u8]) -> Option<Vec<(String, String)>> {
    if buf.len() < 40 || &buf[..8] != b"libudev\0" {
        return None;
    }
    let u32_at = |o: usize| u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap()) as usize;
    let (off, len) = (u32_at(16), u32_at(20));
    let props = buf.get(off..off.checked_add(len)?)?;
    Some(
        props
            .split(|b| *b == 0)
            .filter_map(|kv| {
                let s = std::str::from_utf8(kv).ok()?;
                let (k, v) = s.split_once('=')?;
                Some((k.to_string(), v.to_string()))
            })
            .collect(),
    )
}

/// A new filesystem's device node, if this event announces one.
pub fn added_filesystem(props: &[(String, String)]) -> Option<String> {
    let get = |k: &str| props.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    (get("ACTION") == Some("add") && get("SUBSYSTEM") == Some("block") && get("ID_FS_USAGE") == Some("filesystem"))
        .then(|| get("DEVNAME").map(str::to_string))
        .flatten()
}

fn notify(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "lifepanel", summary, body])
        .stdin(std::process::Stdio::null())
        .status();
}

fn find(path: &str) -> Option<Drive> {
    drives::load(&run_real).into_iter().find(|d| d.path == path)
}

fn arrived(path: &str, automount: bool) {
    // udisks hears the same event and may not have caught up yet.
    for _ in 0..5 {
        let Some(d) = find(path) else {
            std::thread::sleep(Duration::from_millis(400));
            continue;
        };
        if d.mountpoint.is_some() {
            return; // something else mounted it
        }
        if !automount {
            return notify(&format!("{} plugged in", d.name), "Open the quick settings to mount it.");
        }
        match drives::mount(&d.path, &run_real) {
            Ok(mp) => return notify(&format!("Mounted {}", d.name), &mp),
            Err(e) if e.contains("not a mountable filesystem") || e.contains("No such") => {
                std::thread::sleep(Duration::from_millis(400));
            }
            Err(e) => return notify(&format!("Could not mount {}", d.name), &e),
        }
    }
}

fn socket() -> Result<i32, String> {
    // SAFETY: plain socket/bind with a zeroed sockaddr_nl we fill in.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT);
        if fd < 0 {
            return Err(format!("netlink socket: {}", std::io::Error::last_os_error()));
        }
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        sa.nl_groups = UDEV_GROUP;
        let r = libc::bind(fd, &sa as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32);
        if r != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(format!("netlink bind: {e}"));
        }
        Ok(fd)
    }
}

pub fn run(automount: bool) -> i32 {
    let fd = match socket() {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("lifepanel --watch: {e}");
            return 1;
        }
    };
    // Whatever was plugged in before login gets the same treatment.
    if automount {
        for d in drives::load(&run_real).into_iter().filter(|d| d.mountpoint.is_none()) {
            if let Err(e) = drives::mount(&d.path, &run_real) {
                eprintln!("lifepanel --watch: {}: {e}", d.path);
            }
        }
    }
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        // SAFETY: recv into our own buffer.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // ENOBUFS: a burst overflowed the socket; events were lost but the
            // socket is fine. Anything else is fatal.
            if e.raw_os_error() == Some(libc::ENOBUFS) {
                continue;
            }
            eprintln!("lifepanel --watch: recv: {e}");
            return 1;
        }
        let Some(props) = parse_udev(&buf[..n as usize]) else { continue };
        if let Some(path) = added_filesystem(&props) {
            // Off the receive loop: a slow mount must not make us drop events.
            std::thread::spawn(move || arrived(&path, automount));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(props: &str) -> Vec<u8> {
        let body: Vec<u8> = props.replace('\n', "\0").into_bytes();
        let mut m = b"libudev\0".to_vec();
        m.extend_from_slice(&0xfeedcafeu32.to_be_bytes());
        m.extend_from_slice(&40u32.to_ne_bytes()); // header size
        m.extend_from_slice(&40u32.to_ne_bytes()); // properties offset
        m.extend_from_slice(&(body.len() as u32).to_ne_bytes());
        m.resize(40, 0);
        m.extend_from_slice(&body);
        m
    }

    #[test]
    fn reads_udev_messages_and_spots_new_filesystems() {
        let m = message("ACTION=add\nDEVPATH=/devices/x/block/sdb/sdb1\nSUBSYSTEM=block\nDEVNAME=/dev/sdb1\nDEVTYPE=partition\nID_FS_USAGE=filesystem\nID_FS_TYPE=exfat\n");
        let p = parse_udev(&m).unwrap();
        assert_eq!(added_filesystem(&p).as_deref(), Some("/dev/sdb1"));

        let disk = parse_udev(&message("ACTION=add\nSUBSYSTEM=block\nDEVNAME=/dev/sdb\nDEVTYPE=disk\n")).unwrap();
        assert_eq!(added_filesystem(&disk), None, "a bare disk has no filesystem to mount");
        let gone = parse_udev(&message("ACTION=remove\nSUBSYSTEM=block\nDEVNAME=/dev/sdb1\nID_FS_USAGE=filesystem\n")).unwrap();
        assert_eq!(added_filesystem(&gone), None);
        let swap = parse_udev(&message("ACTION=add\nSUBSYSTEM=block\nDEVNAME=/dev/sdc2\nID_FS_USAGE=other\n")).unwrap();
        assert_eq!(added_filesystem(&swap), None);
    }

    #[test]
    fn rejects_kernel_format_and_truncated_messages() {
        assert!(parse_udev(b"add@/devices/x/block/sdb\0ACTION=add\0").is_none());
        let mut m = message("ACTION=add\n");
        m[20..24].copy_from_slice(&9999u32.to_ne_bytes());
        assert!(parse_udev(&m).is_none(), "a length past the end is refused");
    }
}
