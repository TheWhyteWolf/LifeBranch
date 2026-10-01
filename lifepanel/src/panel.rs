// SPDX-License-Identifier: GPL-3.0-or-later
// The panel as plain state: which sections are loaded, which lists are open,
// what is selected, and the one line of text each row shows. Keys and clicks
// turn into jobs here; the window only draws lines and runs jobs. No Wayland
// in this file, so all of it is unit-testable.

use crate::backend::{Audio, Bt, Join, Job, Loaded, Power, Wifi};
use crate::sys::vpn::Vpn;
use crate::drives::Drive;
use zeroize::Zeroizing;

/// Characters per line.
pub const COLS: usize = 46;
/// Where values start, after the row's name.
const VAL: usize = 11;
/// Networks shown when the wifi list is open; the rest are weaker.
const MAX_APS: usize = 10;
const STEP: i32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Wired,
    Wifi,
    Ap(String),
    /// The password field for `Panel::password`'s network.
    Password,
    Bt,
    BtDev(String),
    /// "vpn": what is connected; opens the list.
    Vpn,
    /// One VPN, by its key ("nm:<uuid>", "tailscale", ...).
    VpnEntry(String),
    Volume,
    /// "to <device>": opens the output list.
    OutputPick,
    Output(String),
    Mic,
    Brightness,
    Profile,
    Battery,
    Dnd,
    Drive(String),
    Settings,
    Sep,
}

impl Item {
    pub fn selectable(&self) -> bool {
        !matches!(self, Item::Wired | Item::Battery | Item::Sep)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Label,
    Value,
    Accent,
    Dim,
}

/// What a click on part of a line does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Toggle,
    Primary,
    Secondary,
    /// A bar from `start` over `len` columns: the click sets a percentage.
    Slider { start: usize, len: usize },
    Choice(usize),
}

#[derive(Debug, Default, PartialEq)]
pub struct Line {
    pub spans: Vec<(String, Role)>,
    /// (first column, past-last column, what it does)
    pub zones: Vec<(usize, usize, Hit)>,
    col: usize,
}

impl Line {
    fn push(&mut self, s: &str, role: Role, hit: Option<Hit>) -> &mut Self {
        let room = COLS.saturating_sub(self.col);
        let n = s.chars().count();
        let text: String = if n > room {
            s.chars().take(room.saturating_sub(1)).chain(std::iter::once('…')).collect()
        } else {
            s.to_string()
        };
        let w = text.chars().count();
        if let Some(h) = hit {
            self.zones.push((self.col, self.col + w, h));
        }
        self.col += w;
        self.spans.push((text, role));
        self
    }
    fn to(&mut self, col: usize) -> &mut Self {
        if col > self.col {
            let pad = " ".repeat(col - self.col);
            self.push(&pad, Role::Dim, None);
        }
        self
    }
    /// Right-align `s` against the line's end.
    fn right(&mut self, s: &str, role: Role, hit: Option<Hit>) -> &mut Self {
        let n = s.chars().count();
        self.to(COLS.saturating_sub(n)).push(s, role, hit)
    }
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.spans.iter().map(|(s, _)| s.as_str()).collect()
    }
    /// Make the whole line do `h` where nothing more specific is.
    fn whole(&mut self, h: Hit) -> &mut Self {
        self.zones.push((0, COLS, h));
        self
    }
    pub fn hit_at(&self, col: usize) -> Option<Hit> {
        // Specific zones were pushed first; `whole` comes last as the fallback.
        self.zones.iter().find(|(a, b, _)| (*a..*b).contains(&col)).map(|z| z.2)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Delete / `e`: eject the selected drive.
    Eject,
    Back,
    Esc,
    Text(String),
}

#[derive(Default)]
pub struct Out {
    pub jobs: Vec<Job>,
    pub close: bool,
}

impl Out {
    fn job(j: Job) -> Out {
        Out { jobs: vec![j], close: false }
    }
    fn close(j: Job) -> Out {
        Out { jobs: vec![j], close: true }
    }
}

pub struct Panel {
    pub wifi: Option<Wifi>,
    pub bt: Option<Bt>,
    pub vpn: Option<Vec<Vpn>>,
    pub audio: Option<Audio>,
    pub power: Option<Power>,
    pub dnd: Option<Option<bool>>,
    pub drives: Option<Vec<Drive>>,
    pub open_wifi: bool,
    pub open_bt: bool,
    pub open_vpn: bool,
    pub open_out: bool,
    pub sel: Item,
    /// The network being joined and its password as typed.
    pub password: Option<(String, Zeroizing<String>)>,
    /// Running jobs that have something to say: (id, "joining X…").
    busy: Vec<(u64, String)>,
    /// The last result: (text, is an error).
    pub status: Option<(String, bool)>,
}

impl Default for Panel {
    fn default() -> Self {
        Panel {
            wifi: None,
            bt: None,
            vpn: None,
            audio: None,
            power: None,
            dnd: None,
            drives: None,
            open_wifi: false,
            open_bt: false,
            open_vpn: false,
            open_out: false,
            sel: Item::Wifi,
            password: None,
            busy: Vec::new(),
            status: None,
        }
    }
}

fn onoff(b: bool) -> &'static str {
    if b { "[on]" } else { "[off]" }
}

fn bar(line: &mut Line, pct: i32, max: i32, quiet: bool) {
    let start = line.col;
    let len = COLS - start - 12;
    let filled = ((pct.clamp(0, max) as usize * len) + (max as usize / 2)) / max as usize;
    let hit = Some(Hit::Slider { start, len });
    let (on, off) = if quiet { (Role::Dim, Role::Dim) } else { (Role::Accent, Role::Dim) };
    line.push(&"█".repeat(filled.min(len)), on, hit);
    line.push(&"░".repeat(len - filled.min(len)), off, hit);
    line.push(&format!(" {pct:>3}%"), Role::Value, None);
}

impl Panel {
    pub fn items(&self) -> Vec<Item> {
        let mut v = Vec::new();
        if self.wifi.as_ref().is_some_and(|w| w.wired.is_some()) {
            v.push(Item::Wired);
        }
        v.push(Item::Wifi);
        if let Some(w) = self.wifi.as_ref().filter(|w| self.open_wifi && w.on) {
            for ap in w.aps.iter().take(MAX_APS) {
                v.push(Item::Ap(ap.ssid.clone()));
                if self.password.as_ref().is_some_and(|(s, _)| *s == ap.ssid) {
                    v.push(Item::Password);
                }
            }
        }
        v.push(Item::Bt);
        if let Some(b) = self.bt.as_ref().filter(|b| self.open_bt && b.on) {
            v.extend(b.devs.iter().map(|d| Item::BtDev(d.mac.clone())));
        }
        // No VPN of any kind on the machine: no row at all.
        if let Some(vpns) = self.vpn.as_ref().filter(|v| !v.is_empty()) {
            v.push(Item::Vpn);
            if self.open_vpn {
                v.extend(vpns.iter().map(|x| Item::VpnEntry(x.key.clone())));
            }
        }
        v.push(Item::Sep);
        v.push(Item::Volume);
        if let Some(a) = &self.audio {
            if a.sinks.len() > 1 {
                v.push(Item::OutputPick);
                if self.open_out {
                    v.extend(a.sinks.iter().map(|s| Item::Output(s.0.clone())));
                }
            }
            if a.inp.is_some() {
                v.push(Item::Mic);
            }
        }
        if self.power.as_ref().is_some_and(|p| p.brightness.is_some()) {
            v.push(Item::Brightness);
        }
        v.push(Item::Sep);
        if self.power.as_ref().is_some_and(|p| !p.profiles.is_empty()) {
            v.push(Item::Profile);
        }
        if self.power.as_ref().is_some_and(|p| p.battery.is_some()) {
            v.push(Item::Battery);
        }
        v.push(Item::Dnd);
        if let Some(d) = self.drives.as_ref().filter(|d| !d.is_empty()) {
            v.push(Item::Sep);
            v.extend(d.iter().map(|d| Item::Drive(d.path.clone())));
        }
        v.push(Item::Sep);
        v.push(Item::Settings);
        v
    }

    /// The footer: what is running, else the last result.
    pub fn footer(&self) -> Option<(String, bool)> {
        match self.busy.last() {
            Some((_, t)) => Some((t.clone(), false)),
            None => self.status.clone(),
        }
    }

    pub fn line(&self, item: &Item) -> Line {
        let mut l = Line::default();
        let loading = "…";
        match item {
            Item::Sep => {}
            Item::Wired => {
                let name = self.wifi.as_ref().and_then(|w| w.wired.clone()).unwrap_or_default();
                l.push("wired", Role::Label, None).to(VAL).push(&format!("connected  {name}"), Role::Value, None);
            }
            Item::Wifi => {
                l.push("wifi", Role::Label, None).to(VAL);
                match &self.wifi {
                    None => {
                        l.push(loading, Role::Dim, None);
                    }
                    Some(w) if w.missing.is_some() => {
                        l.push(w.missing.as_deref().unwrap_or(""), Role::Dim, None);
                    }
                    Some(w) => {
                        l.push(onoff(w.on), if w.on { Role::Accent } else { Role::Dim }, Some(Hit::Toggle)).to(VAL + 7);
                        let what = match (w.on, w.active()) {
                            (false, _) => String::new(),
                            (true, Some(a)) => a.ssid.clone(),
                            (true, None) => "not connected".into(),
                        };
                        l.push(&what, Role::Value, None);
                        if w.on {
                            l.right(if self.open_wifi { "v" } else { ">" }, Role::Dim, None);
                        }
                    }
                }
                l.whole(Hit::Primary);
            }
            Item::Ap(ssid) => {
                let Some(ap) = self.wifi.as_ref().and_then(|w| w.aps.iter().find(|a| &a.ssid == ssid)) else { return l };
                let saved = self.wifi.as_ref().is_some_and(|w| w.profile(ssid).is_some());
                let tag = if ap.in_use {
                    "disconnect".to_string()
                } else if saved {
                    "saved".into()
                } else if crate::sys::net::is_open(&ap.security) {
                    "open".into()
                } else {
                    ap.security.to_lowercase()
                };
                let tail = format!("{tag}  {:>3}%", ap.signal);
                l.push(if ap.in_use { "  ● " } else { "    " }, Role::Accent, None);
                let room = COLS - 4 - tail.chars().count() - 1;
                let name: String = if ssid.chars().count() > room {
                    ssid.chars().take(room - 1).chain(std::iter::once('…')).collect()
                } else {
                    ssid.clone()
                };
                l.push(&name, if ap.in_use { Role::Accent } else { Role::Value }, None);
                l.right(&tail, Role::Dim, None).whole(Hit::Primary);
            }
            Item::Password => {
                // Nothing of the password is drawn, not even its length.
                l.push("    password", Role::Label, None).to(VAL + 4);
                l.push("▏", Role::Accent, None);
                l.right("enter", Role::Dim, Some(Hit::Primary));
            }
            Item::Bt => {
                l.push("bluetooth", Role::Label, None).to(VAL);
                match &self.bt {
                    None => {
                        l.push(loading, Role::Dim, None);
                    }
                    Some(b) if b.missing.is_some() => {
                        l.push(b.missing.as_deref().unwrap_or(""), Role::Dim, None);
                    }
                    Some(b) => {
                        l.push(onoff(b.on), if b.on { Role::Accent } else { Role::Dim }, Some(Hit::Toggle)).to(VAL + 7);
                        let names: Vec<&str> = b.devs.iter().filter(|d| d.connected).map(|d| d.name.as_str()).collect();
                        let what = match (b.on, names.is_empty()) {
                            (false, _) => String::new(),
                            (true, true) => "no device".into(),
                            (true, false) => names.join(", "),
                        };
                        l.push(&what, Role::Value, None);
                        if b.on {
                            l.right(if self.open_bt { "v" } else { ">" }, Role::Dim, None);
                        }
                    }
                }
                l.whole(Hit::Primary);
            }
            Item::BtDev(mac) => {
                let Some(d) = self.bt.as_ref().and_then(|b| b.devs.iter().find(|d| &d.mac == mac)) else { return l };
                let tag = if d.connected {
                    "disconnect"
                } else if d.paired {
                    "connect"
                } else {
                    "pair"
                };
                l.push(if d.connected { "  ● " } else { "    " }, Role::Accent, None);
                let name = if d.name.is_empty() { &d.mac } else { &d.name };
                let room = COLS - 4 - tag.len() - 1;
                let name: String = if name.chars().count() > room {
                    name.chars().take(room - 1).chain(std::iter::once('…')).collect()
                } else {
                    name.clone()
                };
                l.push(&name, if d.connected { Role::Accent } else { Role::Value }, None);
                l.right(tag, Role::Dim, None).whole(Hit::Primary);
            }
            Item::Vpn => {
                l.push("vpn", Role::Label, None).to(VAL);
                let up: Vec<&str> =
                    self.vpn.iter().flatten().filter(|v| v.is_up()).map(|v| v.name.as_str()).collect();
                if up.is_empty() {
                    l.push("off", Role::Dim, None);
                } else {
                    l.push(&up.join(", "), Role::Accent, None);
                }
                l.right(if self.open_vpn { "v" } else { ">" }, Role::Dim, None).whole(Hit::Primary);
            }
            Item::VpnEntry(key) => {
                let Some(v) = self.vpn.iter().flatten().find(|v| &v.key == key) else { return l };
                let tag = if v.is_up() { "disconnect" } else { "connect" };
                l.push(if v.is_up() { "  ● " } else { "    " }, Role::Accent, None);
                let room = COLS - 4 - tag.len() - 1;
                let name: String = if v.name.chars().count() > room {
                    v.name.chars().take(room - 1).chain(std::iter::once('…')).collect()
                } else {
                    v.name.clone()
                };
                l.push(&name, if v.is_up() { Role::Accent } else { Role::Value }, None);
                l.right(tag, Role::Dim, None).whole(Hit::Primary);
            }
            Item::Volume | Item::Mic => {
                let out = *item == Item::Volume;
                l.push(if out { "volume" } else { "mic" }, Role::Label, None).to(VAL);
                let v = self.audio.as_ref().and_then(|a| if out { a.out } else { a.inp });
                match (&self.audio, v) {
                    (None, _) => {
                        l.push(loading, Role::Dim, None);
                    }
                    (Some(a), None) => {
                        l.push(a.missing.as_deref().unwrap_or("no device"), Role::Dim, None);
                    }
                    (Some(_), Some((pct, muted))) => {
                        bar(&mut l, pct, 100, muted);
                        l.right(if muted { "muted" } else { " mute" }, if muted { Role::Accent } else { Role::Dim }, Some(Hit::Toggle));
                    }
                }
            }
            Item::OutputPick => {
                let name = self
                    .audio
                    .as_ref()
                    .and_then(|a| a.sinks.iter().find(|s| s.2).map(|s| s.1.clone()))
                    .unwrap_or_else(|| "none".into());
                l.push("  to", Role::Label, None).to(VAL).push(&name, Role::Value, None);
                l.right(if self.open_out { "v" } else { ">" }, Role::Dim, None).whole(Hit::Primary);
            }
            Item::Output(id) => {
                let Some(s) = self.audio.as_ref().and_then(|a| a.sinks.iter().find(|s| &s.0 == id)) else { return l };
                l.to(VAL - 2).push(if s.2 { "● " } else { "  " }, Role::Accent, None);
                l.push(&s.1, if s.2 { Role::Accent } else { Role::Value }, None).whole(Hit::Primary);
            }
            Item::Brightness => {
                l.push("brightness", Role::Label, None).to(VAL);
                if let Some(p) = self.power.as_ref().and_then(|p| p.brightness) {
                    bar(&mut l, p, 100, false);
                }
            }
            Item::Profile => {
                l.push("power", Role::Label, None).to(VAL);
                if let Some(p) = &self.power {
                    for (i, name) in p.profiles.iter().enumerate() {
                        let cur = p.profile.as_deref() == Some(name.as_str());
                        if i > 0 {
                            l.push("  ", Role::Dim, None);
                        }
                        l.push(name, if cur { Role::Accent } else { Role::Dim }, Some(Hit::Choice(i)));
                    }
                }
            }
            Item::Battery => {
                let b = self.power.as_ref().and_then(|p| p.battery.clone()).unwrap_or_default();
                l.push("battery", Role::Label, None).to(VAL).push(&b, Role::Value, None);
            }
            Item::Dnd => {
                l.push("do not disturb", Role::Label, None).to(VAL + 7);
                match self.dnd {
                    None => {
                        l.push(loading, Role::Dim, None);
                    }
                    Some(None) => {
                        l.push("lifenote not running", Role::Dim, None);
                    }
                    Some(Some(on)) => {
                        l.push(onoff(on), if on { Role::Accent } else { Role::Dim }, Some(Hit::Toggle));
                    }
                }
            }
            Item::Drive(path) => {
                let Some(d) = self.drives.as_ref().and_then(|v| v.iter().find(|d| &d.path == path)) else { return l };
                let acts = if d.mountpoint.is_some() { "open  eject" } else { "mount  eject" };
                let room = COLS - acts.len() - d.size.len() - 3;
                let name: String = if d.name.chars().count() > room {
                    d.name.chars().take(room - 1).chain(std::iter::once('…')).collect()
                } else {
                    d.name.clone()
                };
                l.push(&name, if d.mountpoint.is_some() { Role::Accent } else { Role::Value }, None);
                l.push("  ", Role::Dim, None).push(&d.size, Role::Dim, None);
                let first = if d.mountpoint.is_some() { "open" } else { "mount" };
                l.to(COLS - acts.len()).push(first, Role::Label, Some(Hit::Primary)).push("  ", Role::Dim, None);
                l.push("eject", Role::Label, Some(Hit::Secondary));
                l.whole(Hit::Primary);
            }
            Item::Settings => {
                l.push("all settings…", Role::Label, None).whole(Hit::Primary);
            }
        }
        l
    }

    // ---- results coming in -------------------------------------------------

    pub fn loaded(&mut self, l: Loaded) {
        let before = self.items();
        let at = before.iter().position(|i| *i == self.sel).unwrap_or(0);
        match l {
            Loaded::Wifi(w) => {
                // The network being typed for may drop out of a rescan.
                if self.password.as_ref().is_some_and(|(s, _)| !w.aps.iter().any(|a| &a.ssid == s)) {
                    self.password = None;
                }
                self.wifi = Some(w);
            }
            Loaded::Bt(b) => self.bt = Some(b),
            Loaded::Vpn(v) => self.vpn = Some(v),
            Loaded::Audio(a) => self.audio = Some(a),
            Loaded::Power(p) => self.power = Some(p),
            Loaded::Dnd(d) => self.dnd = Some(d),
            Loaded::Drives(d) => self.drives = Some(d),
        }
        self.keep_selection(at);
    }

    /// The selected row vanished (a network out of range, a drive pulled):
    /// select whatever now sits where it was.
    fn keep_selection(&mut self, at: usize) {
        let items = self.items();
        if items.contains(&self.sel) {
            return;
        }
        let at = at.min(items.len() - 1);
        let pick = items[at..].iter().find(|i| i.selectable()).or_else(|| items[..at].iter().rev().find(|i| i.selectable()));
        self.sel = pick.cloned().unwrap_or(Item::Settings);
    }

    pub fn started(&mut self, id: u64, job: &Job) {
        if let Some(t) = job.busy() {
            self.busy.push((id, t));
        }
    }

    pub fn finished(&mut self, id: u64, r: Result<String, String>) {
        self.busy.retain(|(i, _)| *i != id);
        match r {
            Ok(t) if t.is_empty() => {}
            Ok(t) => self.status = Some((t, false)),
            Err(e) => self.status = Some((e, true)),
        }
    }

    // ---- input ---------------------------------------------------------------

    fn step_sel(&mut self, down: bool) {
        let items = self.items();
        let n = items.len();
        let mut i = items.iter().position(|x| *x == self.sel).unwrap_or(0);
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if items[i].selectable() {
                break;
            }
        }
        self.sel = items[i].clone();
    }

    pub fn key(&mut self, k: Key) -> Out {
        if self.sel == Item::Password {
            match &k {
                Key::Text(t) => {
                    if let Some((_, pw)) = &mut self.password {
                        pw.extend(t.chars().filter(|c| !c.is_control()));
                    }
                    return Out::default();
                }
                Key::Back => {
                    if let Some((_, pw)) = &mut self.password {
                        pw.pop();
                    }
                    return Out::default();
                }
                Key::Esc => {
                    let ssid = self.password.take().map(|(s, _)| s);
                    self.sel = ssid.map(Item::Ap).unwrap_or(Item::Wifi);
                    return Out::default();
                }
                _ => {}
            }
        }
        match k {
            Key::Up => self.step_sel(false),
            Key::Down => self.step_sel(true),
            Key::Esc => return Out { jobs: vec![], close: true },
            Key::Left | Key::Right => return self.adjust(&self.sel.clone(), if k == Key::Right { 1 } else { -1 }),
            Key::Enter => return self.primary(&self.sel.clone()),
            Key::Eject => return self.secondary(&self.sel.clone()),
            Key::Text(t) if t == "e" => return self.secondary(&self.sel.clone()),
            Key::Text(t) if t == "m" && matches!(self.sel, Item::Volume | Item::Mic) => {
                return self.toggle(&self.sel.clone());
            }
            Key::Text(_) | Key::Back => {}
        }
        Out::default()
    }

    /// A click on row `idx` at column `col` (fractional: sliders want it).
    pub fn click(&mut self, item: &Item, col: f64) -> Out {
        if !item.selectable() {
            return Out::default();
        }
        self.sel = item.clone();
        let line = self.line(item);
        match line.hit_at(col.max(0.0) as usize) {
            Some(Hit::Toggle) => self.toggle(item),
            Some(Hit::Primary) => self.primary(item),
            Some(Hit::Secondary) => self.secondary(item),
            Some(Hit::Slider { start, len }) => {
                let pct = (((col - start as f64) / len as f64) * 100.0).round().clamp(0.0, 100.0) as i32;
                self.set_level(item, pct)
            }
            Some(Hit::Choice(i)) => {
                let name = self.power.as_ref().and_then(|p| p.profiles.get(i).cloned());
                name.map(|n| self.set_profile(n)).unwrap_or_default()
            }
            None => Out::default(),
        }
    }

    /// The mouse wheel over a row: sliders move, nothing else does.
    pub fn wheel(&mut self, item: &Item, steps: i32) -> Out {
        match item {
            Item::Volume | Item::Mic | Item::Brightness => {
                self.sel = item.clone();
                self.adjust(item, -steps)
            }
            _ => Out::default(),
        }
    }

    fn level(&self, item: &Item) -> Option<i32> {
        match item {
            Item::Volume => self.audio.as_ref()?.out.map(|v| v.0),
            Item::Mic => self.audio.as_ref()?.inp.map(|v| v.0),
            Item::Brightness => self.power.as_ref()?.brightness,
            _ => None,
        }
    }

    fn set_level(&mut self, item: &Item, pct: i32) -> Out {
        match item {
            Item::Volume | Item::Mic => {
                let sink = *item == Item::Volume;
                let Some(a) = self.audio.as_mut() else { return Out::default() };
                let slot = if sink { &mut a.out } else { &mut a.inp };
                let Some(v) = slot.as_mut() else { return Out::default() };
                // Past 100 only by the keys (the bar shows 0-100); the
                // keys keep going to 150 like wpctl allows, the click stops at 100.
                v.0 = pct.clamp(0, 150);
                Out::job(Job::Volume { sink, pct: v.0 })
            }
            Item::Brightness => {
                let Some(p) = self.power.as_mut().and_then(|p| p.brightness.as_mut()) else { return Out::default() };
                *p = pct.clamp(1, 100);
                Out::job(Job::Brightness(*p))
            }
            _ => Out::default(),
        }
    }

    fn set_profile(&mut self, name: String) -> Out {
        if let Some(p) = &mut self.power {
            p.profile = Some(name.clone());
        }
        Out::job(Job::Profile(name))
    }

    fn adjust(&mut self, item: &Item, dir: i32) -> Out {
        match item {
            Item::Volume | Item::Mic | Item::Brightness => match self.level(item) {
                Some(cur) => {
                    // Snap to the step grid, so 47 goes to 50 not 52.
                    let next = if dir > 0 { (cur / STEP + 1) * STEP } else { (cur + STEP - 1) / STEP * STEP - STEP };
                    let max = if *item == Item::Brightness { 100 } else { 150 };
                    self.set_level(item, next.clamp(0, max))
                }
                None => Out::default(),
            },
            Item::Profile => {
                let Some(p) = &self.power else { return Out::default() };
                let n = p.profiles.len() as i32;
                if n == 0 {
                    return Out::default();
                }
                let cur = p.profiles.iter().position(|x| Some(x.as_str()) == p.profile.as_deref()).unwrap_or(0) as i32;
                let next = p.profiles[(cur + dir).rem_euclid(n) as usize].clone();
                self.set_profile(next)
            }
            Item::Wifi | Item::Bt | Item::Dnd => {
                let on = match item {
                    Item::Wifi => self.wifi.as_ref().filter(|w| w.missing.is_none()).map(|w| w.on),
                    Item::Bt => self.bt.as_ref().filter(|b| b.missing.is_none()).map(|b| b.on),
                    _ => self.dnd.flatten(),
                };
                match on {
                    Some(on) if on != (dir > 0) => self.toggle(item),
                    _ => Out::default(),
                }
            }
            _ => Out::default(),
        }
    }

    fn toggle(&mut self, item: &Item) -> Out {
        match item {
            Item::Wifi => match self.wifi.as_mut().filter(|w| w.missing.is_none()) {
                Some(w) => {
                    w.on = !w.on;
                    if !w.on {
                        w.aps.clear();
                    }
                    Out::job(Job::WifiRadio(w.on))
                }
                None => Out::default(),
            },
            Item::Bt => match self.bt.as_mut().filter(|b| b.missing.is_none()) {
                Some(b) => {
                    b.on = !b.on;
                    Out::job(Job::BtPower(b.on))
                }
                None => Out::default(),
            },
            Item::Dnd => match self.dnd {
                Some(Some(on)) => {
                    self.dnd = Some(Some(!on));
                    Out::job(Job::Dnd(!on))
                }
                _ => Out::default(),
            },
            Item::Volume | Item::Mic => {
                let sink = *item == Item::Volume;
                let Some(a) = self.audio.as_mut() else { return Out::default() };
                let slot = if sink { &mut a.out } else { &mut a.inp };
                let Some(v) = slot.as_mut() else { return Out::default() };
                v.1 = !v.1;
                Out::job(Job::Mute { sink, on: v.1 })
            }
            _ => Out::default(),
        }
    }

    fn primary(&mut self, item: &Item) -> Out {
        match item {
            Item::Wifi => {
                let Some(w) = self.wifi.as_ref().filter(|w| w.missing.is_none()) else { return Out::default() };
                if !w.on {
                    self.open_wifi = true;
                    return self.toggle(item);
                }
                self.open_wifi = !self.open_wifi;
                if self.open_wifi { Out::job(Job::WifiRescan) } else { Out::default() }
            }
            Item::Bt => {
                let Some(b) = self.bt.as_ref().filter(|b| b.missing.is_none()) else { return Out::default() };
                if !b.on {
                    self.open_bt = true;
                    return self.toggle(item);
                }
                self.open_bt = !self.open_bt;
                if self.open_bt { Out::job(Job::BtScan) } else { Out::default() }
            }
            Item::Ap(ssid) => {
                let Some(w) = &self.wifi else { return Out::default() };
                let Some(ap) = w.aps.iter().find(|a| &a.ssid == ssid) else { return Out::default() };
                if ap.in_use {
                    return match &w.device {
                        Some(d) => Out::job(Job::WifiDisconnect(d.clone())),
                        None => Out::default(),
                    };
                }
                let how = match w.profile(ssid) {
                    Some(name) => Join::Saved(name.to_string()),
                    None if crate::sys::net::is_open(&ap.security) => Join::Open,
                    None => {
                        self.password = Some((ssid.clone(), Zeroizing::new(String::new())));
                        self.sel = Item::Password;
                        return Out::default();
                    }
                };
                Out::job(Job::WifiJoin { ssid: ssid.clone(), how })
            }
            Item::Password => {
                let Some((ssid, pw)) = self.password.take() else { return Out::default() };
                self.sel = Item::Ap(ssid.clone());
                if pw.is_empty() {
                    return Out::default();
                }
                Out::job(Job::WifiJoin { ssid, how: Join::Password(pw) })
            }
            Item::Vpn => {
                self.open_vpn = !self.open_vpn;
                Out::default()
            }
            Item::VpnEntry(key) => match self.vpn.iter().flatten().find(|v| &v.key == key) {
                Some(v) => Out::job(Job::Vpn(v.clone())),
                None => Out::default(),
            },
            Item::BtDev(mac) => {
                let Some(d) = self.bt.as_ref().and_then(|b| b.devs.iter().find(|d| &d.mac == mac)) else { return Out::default() };
                let name = if d.name.is_empty() { d.mac.clone() } else { d.name.clone() };
                if d.connected {
                    Out::job(Job::BtDisconnect(mac.clone()))
                } else {
                    Out::job(Job::BtConnect { mac: mac.clone(), name, pair: !d.paired })
                }
            }
            Item::Volume | Item::Mic | Item::Dnd => self.toggle(item),
            Item::OutputPick => {
                self.open_out = !self.open_out;
                Out::default()
            }
            Item::Output(id) => {
                if let Some(a) = &mut self.audio {
                    for s in &mut a.sinks {
                        s.2 = &s.0 == id;
                    }
                }
                self.open_out = false;
                self.sel = Item::OutputPick;
                Out::job(Job::Output(id.clone()))
            }
            Item::Profile => self.adjust(item, 1),
            Item::Drive(path) => {
                let Some(d) = self.drives.as_ref().and_then(|v| v.iter().find(|d| &d.path == path)) else { return Out::default() };
                match &d.mountpoint {
                    Some(mp) => Out::close(Job::Open(mp.clone())),
                    None => Out::job(Job::Mount(path.clone())),
                }
            }
            Item::Settings => Out::close(Job::Settings),
            _ => Out::default(),
        }
    }

    fn secondary(&mut self, item: &Item) -> Out {
        let Item::Drive(path) = item else { return Out::default() };
        let Some(all) = &self.drives else { return Out::default() };
        let Some(d) = all.iter().find(|d| &d.path == path) else { return Out::default() };
        Out::job(Job::Eject(d.disk.clone(), all.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BtDev;
    use crate::sys::net::Ap;

    fn ap(ssid: &str, signal: u32, security: &str, in_use: bool) -> Ap {
        Ap { ssid: ssid.into(), signal, security: security.into(), in_use }
    }

    fn panel() -> Panel {
        let mut p = Panel::default();
        p.loaded(Loaded::Wifi(Wifi {
            missing: None,
            device: Some("wlan0".into()),
            on: true,
            aps: vec![ap("Home", 70, "WPA2", true), ap("Cafe", 40, "", false), ap("Neighbour", 30, "WPA2", false)],
            saved: vec![("Home".into(), "Home 1".into())],
            wired: None,
        }));
        p.loaded(Loaded::Bt(Bt {
            missing: None,
            on: true,
            devs: vec![BtDev { mac: "AA:AA:AA:AA:AA:AA".into(), name: "Buds".into(), paired: true, connected: false }],
        }));
        p.loaded(Loaded::Audio(Audio {
            missing: None,
            sinks: vec![("59".into(), "Speakers".into(), true), ("60".into(), "HDMI".into(), false)],
            out: Some((47, false)),
            inp: Some((100, true)),
        }));
        p.loaded(Loaded::Power(Power {
            profiles: vec!["performance".into(), "balanced".into(), "power-saver".into()],
            profile: Some("balanced".into()),
            brightness: Some(9),
            battery: Some("80% discharging".into()),
        }));
        p.loaded(Loaded::Dnd(Some(false)));
        p.loaded(Loaded::Drives(vec![Drive {
            path: "/dev/sdb1".into(),
            disk: "/dev/sdb".into(),
            name: "STICK".into(),
            size: "28G".into(),
            mountpoint: None,
        }]));
        p
    }

    fn kinds(jobs: &[Job]) -> Vec<String> {
        jobs.iter()
            .map(|j| match j {
                Job::WifiJoin { ssid, how: Join::Saved(n) } => format!("join {ssid} saved {n}"),
                Job::WifiJoin { ssid, how: Join::Open } => format!("join {ssid} open"),
                Job::WifiJoin { ssid, how: Join::Password(pw) } => format!("join {ssid} pw {}", pw.as_str()),
                Job::WifiDisconnect(d) => format!("disconnect {d}"),
                Job::WifiRescan => "rescan".into(),
                Job::WifiRadio(on) => format!("radio {on}"),
                Job::BtConnect { mac, pair, .. } => format!("bt {mac} pair={pair}"),
                Job::BtScan => "btscan".into(),
                Job::Volume { sink, pct } => format!("vol {sink} {pct}"),
                Job::Mute { sink, on } => format!("mute {sink} {on}"),
                Job::Output(id) => format!("out {id}"),
                Job::Brightness(p) => format!("bright {p}"),
                Job::Profile(p) => format!("profile {p}"),
                Job::Dnd(on) => format!("dnd {on}"),
                Job::Mount(p) => format!("mount {p}"),
                Job::Eject(d, _) => format!("eject {d}"),
                Job::Open(d) => format!("open {d}"),
                Job::Settings => "settings".into(),
                _ => "other".into(),
            })
            .collect()
    }

    fn press(p: &mut Panel, k: Key) -> Vec<String> {
        kinds(&p.key(k).jobs)
    }

    #[test]
    fn every_line_fits() {
        let mut p = panel();
        p.open_wifi = true;
        p.open_bt = true;
        p.open_out = true;
        for it in p.items() {
            let l = p.line(&it);
            assert!(l.text().chars().count() <= COLS, "{it:?}: {:?}", l.text());
        }
        assert_eq!(p.line(&Item::Wifi).text().trim_end(), "wifi       [on]   Home                       v", "open: the arrow points down");
    }

    #[test]
    fn opening_wifi_lists_networks_and_joins_the_right_way() {
        let mut p = panel();
        assert_eq!(p.sel, Item::Wifi);
        assert_eq!(press(&mut p, Key::Enter), ["rescan"]);
        assert!(p.items().contains(&Item::Ap("Cafe".into())));
        assert_eq!(press(&mut p, Key::Down), Vec::<String>::new());
        assert_eq!(p.sel, Item::Ap("Home".into()));
        assert_eq!(press(&mut p, Key::Enter), ["disconnect wlan0"], "the connected one disconnects");
        p.key(Key::Down);
        assert_eq!(press(&mut p, Key::Enter), ["join Cafe open"]);
        p.key(Key::Down);
        // Secured and unsaved: a password field opens under it, typing fills it.
        assert!(press(&mut p, Key::Enter).is_empty());
        assert_eq!(p.sel, Item::Password);
        for k in ["h", "u", "n", "t", "e", "r", "2"] {
            p.key(Key::Text(k.into()));
        }
        p.key(Key::Back);
        p.key(Key::Text("e".into())); // typed, not "eject"
        let shown = p.line(&Item::Password).text();
        assert!(!shown.contains('*') && !shown.contains("hunter"), "nothing typed is drawn: {shown}");
        assert_eq!(press(&mut p, Key::Enter), ["join Neighbour pw huntere"]);
        assert!(p.password.is_none() && p.sel == Item::Ap("Neighbour".into()));
    }

    #[test]
    fn escape_leaves_the_password_field_then_closes() {
        let mut p = panel();
        p.open_wifi = true;
        p.sel = Item::Ap("Neighbour".into());
        p.key(Key::Enter);
        p.key(Key::Text("x".into()));
        assert!(!p.key(Key::Esc).close);
        assert!(p.password.is_none());
        assert_eq!(p.sel, Item::Ap("Neighbour".into()));
        assert!(p.key(Key::Esc).close);
    }

    #[test]
    fn sliders_snap_and_mute_toggles() {
        let mut p = panel();
        p.sel = Item::Volume;
        assert_eq!(press(&mut p, Key::Right), ["vol true 50"]);
        assert_eq!(press(&mut p, Key::Left), ["vol true 45"]);
        assert_eq!(press(&mut p, Key::Enter), ["mute true true"]);
        p.sel = Item::Brightness;
        assert_eq!(press(&mut p, Key::Left), ["bright 5"]);
        assert_eq!(press(&mut p, Key::Left), ["bright 1"], "never fully dark");
        assert_eq!(kinds(&p.wheel(&Item::Mic, -1).jobs), ["vol false 105"]);
        p.sel = Item::Profile;
        assert_eq!(press(&mut p, Key::Right), ["profile power-saver"]);
        assert_eq!(press(&mut p, Key::Right), ["profile performance"]);
    }

    #[test]
    fn clicks_hit_the_word_under_the_pointer() {
        let mut p = panel();
        let l = p.line(&Item::Volume);
        let Some((start, _, Hit::Slider { len, .. })) = l.zones.iter().find(|z| matches!(z.2, Hit::Slider { .. })).copied() else {
            panic!("no slider")
        };
        assert_eq!(kinds(&p.click(&Item::Volume, start as f64 + len as f64 / 4.0).jobs), ["vol true 25"]);
        assert_eq!(kinds(&p.click(&Item::Volume, (COLS - 2) as f64).jobs), ["mute true true"]);
        // The profile names are each a target.
        let l = p.line(&Item::Profile);
        let saver = l.zones.iter().find(|z| z.2 == Hit::Choice(2)).unwrap().0;
        assert_eq!(kinds(&p.click(&Item::Profile, saver as f64).jobs), ["profile power-saver"]);
        // The [on] word toggles wifi; the rest of the row opens the list.
        assert_eq!(kinds(&p.click(&Item::Wifi, (VAL + 1) as f64).jobs), ["radio false"]);
        assert_eq!(kinds(&p.click(&Item::Wifi, 30.0).jobs), ["radio true"], "off: the row turns it on");
        assert!(p.open_wifi);
        // Drive: mount, eject.
        assert_eq!(kinds(&p.click(&Item::Drive("/dev/sdb1".into()), 3.0).jobs), ["mount /dev/sdb1"]);
        assert_eq!(kinds(&p.click(&Item::Drive("/dev/sdb1".into()), (COLS - 1) as f64).jobs), ["eject /dev/sdb"]);
    }

    #[test]
    fn selection_skips_info_rows_and_survives_reloads() {
        let mut p = panel();
        p.sel = Item::Bt;
        p.key(Key::Down);
        assert_eq!(p.sel, Item::Volume, "the separator is skipped");
        p.key(Key::Up);
        p.key(Key::Up);
        assert_eq!(p.sel, Item::Wifi);
        p.key(Key::Up);
        assert_eq!(p.sel, Item::Settings, "wraps");
        p.sel = Item::Drive("/dev/sdb1".into());
        p.loaded(Loaded::Drives(vec![]));
        assert_eq!(p.sel, Item::Settings, "the pulled drive's row goes; the next one is selected");
    }

    #[test]
    fn output_pick_and_settings_close() {
        let mut p = panel();
        p.sel = Item::OutputPick;
        p.key(Key::Enter);
        assert!(p.items().contains(&Item::Output("60".into())));
        p.sel = Item::Output("60".into());
        assert_eq!(press(&mut p, Key::Enter), ["out 60"]);
        assert!(!p.open_out && p.sel == Item::OutputPick);
        assert!(p.line(&Item::OutputPick).text().contains("HDMI"));
        p.sel = Item::Settings;
        let o = p.key(Key::Enter);
        assert!(o.close && kinds(&o.jobs) == ["settings"]);
    }

    #[test]
    fn busy_text_then_result() {
        let mut p = panel();
        let j = Job::WifiJoin { ssid: "Cafe".into(), how: Join::Open };
        p.started(7, &j);
        assert_eq!(p.footer(), Some(("joining Cafe…".into(), false)));
        p.finished(7, Err("wrong password".into()));
        assert_eq!(p.footer(), Some(("wrong password".into(), true)));
    }

    #[test]
    fn missing_hardware_rows_say_so_and_do_nothing() {
        let mut p = Panel::default();
        p.loaded(Loaded::Wifi(Wifi { missing: Some("no wifi adapter".into()), ..Wifi::default() }));
        assert!(p.line(&Item::Wifi).text().contains("no wifi adapter"));
        assert!(p.key(Key::Enter).jobs.is_empty());
        assert!(p.key(Key::Right).jobs.is_empty());
        assert!(!p.items().contains(&Item::Brightness), "no backlight: no row");
    }

    #[test]
    fn vpn_row_summarises_and_its_list_toggles() {
        use crate::sys::vpn::{Provider, State, Vpn};
        let mut p = panel();
        assert!(!p.items().contains(&Item::Vpn), "not loaded yet: no row");
        p.loaded(Loaded::Vpn(vec![]));
        assert!(!p.items().contains(&Item::Vpn), "no VPN on the machine: no row");
        let ts = Vpn { key: "tailscale".into(), name: "Tailscale".into(), provider: Provider::Tailscale, state: State::Up, detail: String::new() };
        let wg = Vpn { key: "nm:u1".into(), name: "home-wg".into(), provider: Provider::Nm { uuid: "u1".into(), autoconnect: false }, state: State::Down, detail: "WireGuard".into() };
        p.loaded(Loaded::Vpn(vec![ts, wg]));
        assert!(p.line(&Item::Vpn).text().contains("Tailscale"), "{}", p.line(&Item::Vpn).text());
        p.sel = Item::Vpn;
        p.key(Key::Enter);
        assert!(p.items().contains(&Item::VpnEntry("nm:u1".into())));
        assert!(p.line(&Item::VpnEntry("nm:u1".into())).text().trim_end().ends_with("connect"));
        p.sel = Item::VpnEntry("nm:u1".into());
        let jobs = p.key(Key::Enter).jobs;
        assert!(matches!(&jobs[..], [Job::Vpn(v)] if v.key == "nm:u1"));
        assert_eq!(jobs[0].busy().as_deref(), Some("connecting home-wg…"));
    }
}
