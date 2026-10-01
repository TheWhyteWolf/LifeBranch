// SPDX-License-Identifier: GPL-3.0-or-later
// The readings on the right of the bar. Everything that can come straight
// from /proc and /sys does, with no child process: CPU, memory, battery,
// backlight, network. Volume asks wpctl, and the notification badge and DND
// ask lifenote. Those run on a worker thread, never in the paint path.

use std::path::{Path, PathBuf};

fn read(p: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

// ---- CPU ---------------------------------------------------------------------

/// The `cpu` line of /proc/stat: (busy, total) jiffies.
pub fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal: idle + iowait is idle.
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    let total: u64 = v.iter().take(8).sum();
    Some((total - idle, total))
}

/// Usage between two readings, in percent.
pub fn cpu_pct(prev: (u64, u64), now: (u64, u64)) -> u32 {
    let (db, dt) = (now.0.saturating_sub(prev.0), now.1.saturating_sub(prev.1));
    if dt == 0 { 0 } else { ((db * 100 + dt / 2) / dt) as u32 }
}

pub fn read_cpu() -> Option<(u64, u64)> {
    cpu_times(&std::fs::read_to_string("/proc/stat").ok()?)
}

// ---- memory ------------------------------------------------------------------

/// (percent used, used GiB, total GiB) from /proc/meminfo, used meaning
/// total minus available, as waybar's memory module counts it.
pub fn mem(meminfo: &str) -> Option<(u32, f64, f64)> {
    let kb = |k: &str| {
        meminfo.lines().find_map(|l| l.strip_prefix(k)).and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok())
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(avail);
    let gib = |k: u64| k as f64 / (1024.0 * 1024.0);
    Some((((used * 100 + total / 2) / total) as u32, gib(used), gib(total)))
}

pub fn read_mem() -> Option<(u32, f64, f64)> {
    mem(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

// ---- battery -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Charge {
    Discharging,
    Charging,
    /// On AC, not charging (full, or held at a limit).
    Plugged,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub pct: u32,
    pub state: Charge,
}

/// Percent from now/full (charge_* or energy_*), as waybar and upower count
/// it; the kernel's `capacity` is only the fallback, because some drivers
/// (Apple's) report it against the design capacity, which a worn battery
/// never reaches: 65% there was 84% of what this battery can hold.
pub fn battery_pct(now: Option<u64>, full: Option<u64>, capacity: Option<&str>) -> Option<u32> {
    match (now, full) {
        (Some(n), Some(f)) if f > 0 => Some(((n * 100 + f / 2) / f).min(100) as u32),
        _ => capacity?.trim().parse::<u32>().ok().map(|c| c.min(100)),
    }
}

pub fn battery_from(pct: u32, status: &str) -> Option<Battery> {
    let state = match status.trim() {
        "Charging" => Charge::Charging,
        "Discharging" => Charge::Discharging,
        // "Full", "Not charging", "Unknown" while on AC.
        _ => Charge::Plugged,
    };
    Some(Battery { pct: pct.min(100), state })
}

/// The first system battery (type Battery, scope not Device: a mouse's
/// battery is not the laptop's).
pub fn read_battery() -> Option<Battery> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter()
        .filter(|d| read(d.join("type")).as_deref() == Some("Battery"))
        .filter(|d| read(d.join("scope")).as_deref() != Some("Device"))
        .find_map(|d| {
            let num = |f: &str| read(d.join(f)).and_then(|s| s.parse::<u64>().ok());
            let (now, full) = match num("charge_now") {
                Some(n) => (Some(n), num("charge_full")),
                None => (num("energy_now"), num("energy_full")),
            };
            let pct = battery_pct(now, full, read(d.join("capacity")).as_deref())?;
            battery_from(pct, &read(d.join("status")).unwrap_or_default())
        })
}

// ---- backlight ---------------------------------------------------------------

/// The panel's backlight: firmware beats platform beats raw (systemd's
/// order), and keyboard/Touch Bar lights are not the screen.
pub fn pick_backlight(devs: &[(String, String)]) -> Option<String> {
    let rank = |t: &str| match t {
        "firmware" => 3,
        "platform" => 2,
        "raw" => 1,
        _ => 0,
    };
    devs.iter()
        .filter(|(name, _)| !name.contains("kbd") && !name.starts_with("appletb"))
        .max_by_key(|(_, t)| rank(t))
        .map(|(n, _)| n.clone())
}

pub fn backlight_dev() -> Option<PathBuf> {
    let rd = std::fs::read_dir("/sys/class/backlight").ok()?;
    let mut devs: Vec<(String, String)> = rd
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), read(e.path().join("type")).unwrap_or_default()))
        .collect();
    devs.sort();
    pick_backlight(&devs).map(|n| Path::new("/sys/class/backlight").join(n))
}

pub fn read_backlight(dev: &Path) -> Option<u32> {
    let cur: u64 = read(dev.join("brightness"))?.parse().ok()?;
    let max: u64 = read(dev.join("max_brightness"))?.parse().ok()?;
    (max > 0).then(|| ((cur * 100 + max / 2) / max) as u32)
}

// ---- network -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Net {
    /// Wifi link quality in percent.
    Wifi(u32),
    Wired,
    Down,
}

/// Signal strength for `iface` from /proc/net/wireless's level column (dBm),
/// mapped the way waybar and NetworkManager do: 2 x (dBm + 100), so -50 dBm
/// and better is 100% and -100 dBm is 0.
pub fn wifi_quality(table: &str, iface: &str) -> Option<u32> {
    let line = table.lines().find(|l| l.trim_start().starts_with(&format!("{iface}:")))?;
    let dbm: f64 = line.split_whitespace().nth(3)?.trim_end_matches('.').parse().ok()?;
    Some((2.0 * (dbm + 100.0)).round().clamp(0.0, 100.0) as u32)
}

/// The interface the default route leaves by (lowest metric), from
/// /proc/net/route. That is the connection that matters: an up link with no
/// route (the T2 MacBook's internal USB ethernet) is not "online".
pub fn default_iface(route: &str) -> Option<String> {
    route
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() > 7 && f[1] == "00000000" && f[7] == "00000000").then(|| (f[6].parse::<u32>().unwrap_or(u32::MAX), f[0].to_string()))
        })
        .min()
        .map(|(_, i)| i)
}

pub fn read_net() -> Net {
    let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let Some(iface) = default_iface(&route) else { return Net::Down };
    if Path::new("/sys/class/net").join(&iface).join("wireless").exists() {
        let table = std::fs::read_to_string("/proc/net/wireless").unwrap_or_default();
        Net::Wifi(wifi_quality(&table, &iface).unwrap_or(0))
    } else {
        Net::Wired
    }
}

// ---- VPN -----------------------------------------------------------------------

/// Whether a tunnel is up that carries this machine's traffic: a tun or
/// WireGuard interface (ARPHRD_NONE, type 65534) that isn't Tailscale's.
/// tailscale0 is up whenever Tailscale runs and mostly carries only the
/// tailnet, so it doesn't count; WireGuard, OpenVPN, Proton and Mullvad do.
pub fn vpn_up(ifaces: &[(String, String, String)]) -> bool {
    ifaces.iter().any(|(name, kind, oper)| kind == "65534" && name != "tailscale0" && oper != "down")
}

pub fn read_vpn() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else { return false };
    let ifaces: Vec<(String, String, String)> = rd
        .flatten()
        .map(|e| {
            let p = e.path();
            (e.file_name().to_string_lossy().into_owned(), read(p.join("type")).unwrap_or_default(), read(p.join("operstate")).unwrap_or_default())
        })
        .collect();
    vpn_up(&ifaces)
}

// ---- volume, notifications ---------------------------------------------------

/// `Volume: 0.45 [MUTED]` -> (45, true).
pub fn parse_volume(out: &str) -> Option<(u32, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?;
    let v: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some(((v * 100.0).round().max(0.0) as u32, rest.contains("MUTED")))
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn read_volume() -> Option<(u32, bool)> {
    parse_volume(&output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?)
}

pub fn lifenote() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = format!("{home}/.local/bin/lifenote");
    if Path::new(&local).is_file() { local } else { "lifenote".into() }
}

/// (unseen count, do-not-disturb), None when lifenote isn't running.
pub fn read_notes() -> Option<(u32, bool)> {
    let bin = lifenote();
    let unseen = output(&bin, &["ctl", "unseen"])?.trim().parse().unwrap_or(0);
    let dnd = output(&bin, &["ctl", "dnd", "get"]).is_some_and(|o| o.trim() == "on");
    Some((unseen, dnd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_between_two_readings() {
        let a = cpu_times("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        let b = cpu_times("cpu  150 0 150 850 50 0 0 0 0 0\n").unwrap();
        assert_eq!(a, (200, 1000));
        assert_eq!(cpu_pct(a, b), 50, "iowait counts as idle");
        assert_eq!(cpu_pct(a, a), 0);
        assert!(cpu_times("intr 1 2 3").is_none());
    }

    #[test]
    fn memory_is_total_minus_available() {
        let (pct, used, total) = mem("MemTotal:       16257472 kB\nMemFree: 1 kB\nMemAvailable:   11297328 kB\n").unwrap();
        assert_eq!(pct, 31);
        assert!((total - 15.5).abs() < 0.01 && (used - 4.73).abs() < 0.01, "{used} {total}");
    }

    #[test]
    fn battery_states() {
        assert_eq!(battery_from(74, "Discharging\n"), Some(Battery { pct: 74, state: Charge::Discharging }));
        assert_eq!(battery_from(99, "Charging").map(|b| b.state), Some(Charge::Charging));
        assert_eq!(battery_from(100, "Full").map(|b| b.state), Some(Charge::Plugged));
        assert_eq!(battery_from(80, "Not charging").map(|b| b.state), Some(Charge::Plugged));
        // This MacBook: capacity says 65, charge_now/charge_full says 84.
        assert_eq!(battery_pct(Some(5_675_000), Some(6_798_000), Some("65")), Some(83));
        assert_eq!(battery_pct(None, None, Some("65\n")), Some(65));
        assert_eq!(battery_pct(Some(1), Some(0), Some("x")), None);
    }

    #[test]
    fn the_screen_backlight_not_the_touch_bar() {
        let devs = [("appletb_backlight".into(), "raw".into()), ("gmux_backlight".into(), "platform".into())];
        assert_eq!(pick_backlight(&devs).as_deref(), Some("gmux_backlight"));
        let devs = [("intel_backlight".into(), "raw".into()), ("acpi_video0".into(), "firmware".into())];
        assert_eq!(pick_backlight(&devs).as_deref(), Some("acpi_video0"));
        assert_eq!(pick_backlight(&[("tpacpi::kbd_backlight".into(), "raw".into())]), None);
    }

    #[test]
    fn wifi_quality_from_proc() {
        let t = "Inter-| sta-|   Quality        |\n face | tus | link level noise |\n wlan0: 0000   54.  -56.  -256        0      0      0     45      0        0\n";
        assert_eq!(wifi_quality(t, "wlan0"), Some(88), "-56 dBm");
        assert_eq!(wifi_quality(t, "wlan1"), None);
    }

    #[test]
    fn the_default_route_decides() {
        let r = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                 wlan0\t00000000\t0102A8C0\t0003\t0\t0\t600\t00000000\n\
                 eno1\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
                 wlan0\t0002A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n";
        assert_eq!(default_iface(r).as_deref(), Some("eno1"), "lowest metric wins");
        assert_eq!(default_iface("Iface\tDestination\nwlan0\t0002A8C0\t0\t1\t0\t0\t600\t00FFFFFF\n"), None);
    }

    #[test]
    fn a_tunnel_other_than_tailscale_is_a_vpn() {
        let i = |n: &str, t: &str, o: &str| (n.to_string(), t.to_string(), o.to_string());
        let base = vec![i("wlan0", "1", "up"), i("tailscale0", "65534", "unknown"), i("lo", "772", "unknown")];
        assert!(!vpn_up(&base), "Tailscale alone, as on this machine");
        let mut wg = base.clone();
        wg.push(i("wg0-mullvad", "65534", "unknown"));
        assert!(vpn_up(&wg));
        let mut down = base;
        down.push(i("proton0", "65534", "down"));
        assert!(!vpn_up(&down));
    }

    #[test]
    fn volume_text() {
        assert_eq!(parse_volume("Volume: 0.20\n"), Some((20, false)));
        assert_eq!(parse_volume("Volume: 1.00 [MUTED]"), Some((100, true)));
        assert_eq!(parse_volume("nope"), None);
    }
}
