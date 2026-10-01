// SPDX-License-Identifier: GPL-3.0-or-later
// Battery and backlight changes, as the kernel announces them: a netlink
// socket on the kernel's uevent group. A charger plugged in, a battery
// percent ticking down, brightnessctl writing the backlight: each is one
// "change" message, so the bar needs no fast polling for any of them.

/// Whether a kernel uevent (NUL-separated, "ACTION@DEVPATH" first) is about
/// a power supply or a backlight.
pub fn relevant(msg: &[u8]) -> bool {
    msg.split(|b| *b == 0).any(|kv| kv == b"SUBSYSTEM=power_supply" || kv == b"SUBSYSTEM=backlight")
}

pub fn listen(on: impl Fn()) {
    // SAFETY: socket/bind/recv on our own descriptor and buffers.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT);
        if fd < 0 {
            return;
        }
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        sa.nl_groups = 1; // the kernel's own events
        if libc::bind(fd, &sa as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32) != 0 {
            libc::close(fd);
            return;
        }
        let mut buf = vec![0u8; 8192];
        loop {
            let n = libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0);
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted || e.raw_os_error() == Some(libc::ENOBUFS) {
                    continue;
                }
                return;
            }
            if relevant(&buf[..n as usize]) {
                on();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_power_and_backlight_events() {
        assert!(relevant(b"change@/devices/x/power_supply/BAT0\0ACTION=change\0SUBSYSTEM=power_supply\0POWER_SUPPLY_CAPACITY=73\0"));
        assert!(relevant(b"change@/devices/x/backlight/gmux\0ACTION=change\0SUBSYSTEM=backlight\0"));
        assert!(!relevant(b"add@/devices/x/block/sdb\0ACTION=add\0SUBSYSTEM=block\0"));
    }
}
