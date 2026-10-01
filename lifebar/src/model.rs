// SPDX-License-Identifier: GPL-3.0-or-later
// What the bar says: the state it has heard about, turned into labelled
// segments for one output. Pure: no Wayland, no I/O, so every module's text,
// colour and click is unit-testable. Same modules and wording as the waybar
// config it replaces (NET/BRT/VOL/BAT/CPU/MEM, # badge, DND, IDLE/WAKE).

use crate::niri::Niri;
use crate::sys::{Battery, Charge, Net};
use crate::tray;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Text,
    Accent,
    Warn,
    Urgent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Workspace(u64),
    Calendar,
    NotifMenu,
    Dnd,
    Idle,
    Panel,
    NetSettings,
    VpnSettings,
    Mute,
    SoundSettings,
    Volume(i32),
    Brightness(i32),
    Top,
    Tray(usize, Button),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub text: String,
    pub role: Role,
    /// Drawn with a line under it (the active workspace).
    pub mark: bool,
    pub left: Option<Act>,
    pub middle: Option<Act>,
    pub right: Option<Act>,
    /// (wheel up, wheel down)
    pub wheel: Option<(Act, Act)>,
}

impl Seg {
    fn new(text: impl Into<String>, role: Role) -> Seg {
        Seg { text: text.into(), role, mark: false, left: None, middle: None, right: None, wheel: None }
    }
    fn on(mut self, a: Act) -> Seg {
        self.left = Some(a);
        self
    }
    fn on_right(mut self, a: Act) -> Seg {
        self.right = Some(a);
        self
    }
    fn on_wheel(mut self, up: Act, down: Act) -> Seg {
        self.wheel = Some((up, down));
        self
    }
    pub fn act(&self, b: Button) -> Option<&Act> {
        match b {
            Button::Left => self.left.as_ref(),
            Button::Middle => self.middle.as_ref(),
            Button::Right => self.right.as_ref(),
        }
    }
}

/// Everything the bar has heard. None: not known (yet), so not shown.
#[derive(Default)]
pub struct State {
    pub niri: Niri,
    pub clock: String,
    pub notes: Option<(u32, bool)>,
    pub idle_inhibited: bool,
    pub net: Option<Net>,
    pub vpn: bool,
    pub backlight: Option<u32>,
    pub volume: Option<(u32, bool)>,
    pub battery: Option<Battery>,
    pub cpu: Option<u32>,
    pub mem: Option<u32>,
    pub tray: Vec<tray::Item>,
}

/// Longest window title shown (waybar's max-length).
pub const TITLE_MAX: usize = 60;

fn load_role(pct: u32) -> Role {
    match pct {
        90.. => Role::Urgent,
        70.. => Role::Warn,
        _ => Role::Text,
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() <= max {
        return s;
    }
    if max == 0 {
        return String::new();
    }
    s.chars().take(max - 1).chain(std::iter::once('…')).collect()
}

impl State {
    /// Workspaces, then the focused window's title.
    pub fn left(&self, output: &str) -> Vec<Seg> {
        let mut v: Vec<Seg> = self
            .niri
            .workspaces_on(output)
            .into_iter()
            .map(|w| {
                let role = if w.urgent { Role::Urgent } else if w.active { Role::Accent } else { Role::Text };
                let name = w.name.clone().unwrap_or_else(|| w.idx.to_string());
                let mut s = Seg::new(format!(" {name} "), role).on(Act::Workspace(w.id));
                s.mark = w.active;
                s
            })
            .collect();
        if let Some(t) = self.niri.title_on(output).filter(|t| !t.is_empty()) {
            v.push(Seg::new(format!("  {}", truncate(t, TITLE_MAX)), Role::Text));
        }
        v
    }

    pub fn center(&self) -> Vec<Seg> {
        vec![Seg::new(self.clock.clone(), Role::Accent).on(Act::Calendar)]
    }

    pub fn right(&self) -> Vec<Seg> {
        let mut v = Vec::new();
        if let Some((unseen, dnd)) = self.notes {
            let badge = if unseen > 0 { format!("# {unseen}") } else { "#".into() };
            v.push(Seg::new(badge, Role::Text).on(Act::NotifMenu));
            if dnd {
                v.push(Seg::new("DND", Role::Urgent).on(Act::Dnd));
            }
        }
        v.push(if self.idle_inhibited { Seg::new("WAKE", Role::Warn) } else { Seg::new("IDLE", Role::Text) }.on(Act::Idle));
        if let Some(n) = &self.net {
            let s = match n {
                Net::Wifi(q) => Seg::new(format!("NET {q}%"), Role::Text),
                Net::Wired => Seg::new("NET", Role::Text),
                Net::Down => Seg::new("NET --", Role::Urgent),
            };
            v.push(s.on(Act::Panel).on_right(Act::NetSettings));
        }
        if self.vpn {
            v.push(Seg::new("VPN", Role::Accent).on(Act::Panel).on_right(Act::VpnSettings));
        }
        if let Some(b) = self.backlight {
            v.push(Seg::new(format!("BRT {b}%"), Role::Text).on(Act::Panel).on_wheel(Act::Brightness(1), Act::Brightness(-1)));
        }
        if let Some((pct, muted)) = self.volume {
            let s = if muted { Seg::new("MUTE", Role::Urgent) } else { Seg::new(format!("VOL {pct}%"), Role::Text) };
            v.push(s.on(Act::Mute).on_right(Act::SoundSettings).on_wheel(Act::Volume(1), Act::Volume(-1)));
        }
        if let Some(b) = self.battery {
            let (word, role) = match b.state {
                Charge::Charging => ("CHG", Role::Accent),
                Charge::Plugged => ("AC", Role::Accent),
                Charge::Discharging if b.pct <= 10 => ("BAT", Role::Urgent),
                Charge::Discharging if b.pct <= 20 => ("BAT", Role::Warn),
                Charge::Discharging => ("BAT", Role::Text),
            };
            v.push(Seg::new(format!("{word} {}%", b.pct), role).on(Act::Panel));
        }
        if let Some(c) = self.cpu {
            v.push(Seg::new(format!("CPU {c}%"), load_role(c)).on(Act::Top));
        }
        if let Some(m) = self.mem {
            v.push(Seg::new(format!("MEM {m}%"), load_role(m)).on(Act::Top));
        }
        for (i, t) in self.tray.iter().enumerate() {
            let mut s = Seg::new(t.label.clone(), if t.attention { Role::Urgent } else { Role::Text });
            s.left = Some(Act::Tray(i, Button::Left));
            s.middle = Some(Act::Tray(i, Button::Middle));
            s.right = Some(Act::Tray(i, Button::Right));
            v.push(s);
        }
        v
    }
}

// ---- time ----------------------------------------------------------------------

/// Local time as (year, month 1-12, day, weekday 0=Monday) plus the bar's
/// clock text, "Wed 01 Oct  19:18" (waybar's format).
pub fn now() -> ((i32, u32, u32), String) {
    // SAFETY: time/localtime_r/strftime into our own buffers.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let fmt = c"%a %d %b  %H:%M";
        let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm);
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        ((tm.tm_year + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32), text)
    }
}

/// Seconds until the next minute starts, so the clock ticks on the minute.
pub fn to_next_minute() -> u64 {
    // SAFETY: plain time(2).
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    60 - (t.rem_euclid(60)) as u64
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// Day of the week, 0 = Monday (Sakamoto's method).
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let sun0 = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7);
    ((sun0 + 6) % 7) as u32
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A month as `cal` draws it, Monday first: a centred title, the day names,
/// then the weeks. Each line is (text, the column of `today` if it's on it).
pub fn month(y: i32, m: u32, today: Option<u32>) -> Vec<(String, Option<usize>)> {
    const W: usize = 20;
    let title = format!("{} {y}", MONTHS[(m - 1) as usize]);
    let pad = (W.saturating_sub(title.len())) / 2;
    let mut out = vec![(format!("{}{title}", " ".repeat(pad)), None), ("Mo Tu We Th Fr Sa Su".to_string(), None)];
    let first = weekday(y, m, 1) as usize;
    let mut line = " ".repeat(first * 3);
    let mut mark = None;
    for d in 1..=days_in_month(y, m) {
        if today == Some(d) {
            mark = Some(line.len());
        }
        line.push_str(&format!("{d:>2}"));
        if (first + d as usize) % 7 == 0 {
            out.push((line, mark));
            line = String::new();
            mark = None;
        } else {
            line.push(' ');
        }
    }
    if !line.trim().is_empty() {
        out.push((line.trim_end().to_string(), mark));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut s = State::default();
        let ws = r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":3},{"id":2,"idx":2,"name":null,"output":"eDP-1","is_urgent":true,"is_active":false,"is_focused":false,"active_window_id":null}]}}"#;
        s.niri.apply(&serde_json::from_str(ws).unwrap());
        let win = r#"{"WindowsChanged":{"windows":[{"id":3,"title":"kitty","workspace_id":1,"is_urgent":false}]}}"#;
        s.niri.apply(&serde_json::from_str(win).unwrap());
        s.clock = "Wed 01 Oct  19:18".into();
        s
    }

    fn texts(v: &[Seg]) -> Vec<&str> {
        v.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn left_is_workspaces_then_title() {
        let s = state();
        let l = s.left("eDP-1");
        assert_eq!(texts(&l), [" 1 ", " 2 ", "  kitty"]);
        assert!(l[0].mark && l[0].role == Role::Accent);
        assert_eq!(l[1].role, Role::Urgent);
        assert_eq!(l[1].left, Some(Act::Workspace(2)));
        assert_eq!(texts(&s.left("HDMI-A-1")), Vec::<&str>::new());
    }

    #[test]
    fn right_matches_the_waybar_wording() {
        let mut s = state();
        s.notes = Some((3, true));
        s.net = Some(Net::Wifi(77));
        s.backlight = Some(9);
        s.volume = Some((20, false));
        s.battery = Some(Battery { pct: 74, state: Charge::Discharging });
        s.cpu = Some(4);
        s.mem = Some(75);
        s.tray = vec![tray::Item { key: "k".into(), label: "Vesktop".into(), attention: false }];
        let r = s.right();
        assert_eq!(texts(&r), ["# 3", "DND", "IDLE", "NET 77%", "BRT 9%", "VOL 20%", "BAT 74%", "CPU 4%", "MEM 75%", "Vesktop"]);
        assert_eq!(r[8].role, Role::Warn, "memory past 70%");
        assert_eq!(r[5].wheel, Some((Act::Volume(1), Act::Volume(-1))));
        assert_eq!(r[9].act(Button::Right), Some(&Act::Tray(0, Button::Right)));
        s.volume = Some((20, true));
        s.net = Some(Net::Down);
        s.idle_inhibited = true;
        s.battery = Some(Battery { pct: 8, state: Charge::Discharging });
        s.notes = Some((0, false));
        let r = s.right();
        assert_eq!(texts(&r)[..4], ["#", "WAKE", "NET --", "BRT 9%"]);
        assert!(r.iter().any(|x| x.text == "MUTE" && x.role == Role::Urgent));
        assert!(r.iter().any(|x| x.text == "BAT 8%" && x.role == Role::Urgent));
    }

    #[test]
    fn unknown_readings_are_left_out() {
        let s = State::default();
        assert_eq!(texts(&s.right()), ["IDLE"], "a desktop with nothing loaded yet shows only what it knows");
    }

    #[test]
    fn calendar_matches_cal() {
        assert_eq!(weekday(2026, 10, 1), 3, "1 Oct 2026 is a Thursday");
        assert_eq!(weekday(2024, 2, 29), 3);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        let m = month(2026, 10, Some(1));
        assert_eq!(m[0].0, "    October 2026");
        assert_eq!(m[2], ("          1  2  3  4".to_string(), Some(9)));
        assert_eq!(m.last().unwrap().0, "26 27 28 29 30 31");
        assert_eq!(m.len(), 2 + 5);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab\ncd", 10), "abcd");
    }
}
