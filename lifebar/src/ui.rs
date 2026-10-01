// SPDX-License-Identifier: GPL-3.0-or-later
// The bars: one layer-shell surface per output, across the top, reserving
// its height. Each repaint lays out the model's segments and redraws only if
// what that output shows changed; between changes nothing runs but the
// threads asleep on their sockets.
//
// Also here: the calendar popup (click the clock) and the idle inhibitor
// (click IDLE: the bar's own surface holds the idle chain off while WAKE).

use crate::cli::Rgba;
use crate::config::Config;
use crate::model::{Act, Button, Role, Seg, State};
use crate::render::{Atlas, Canvas};
use crate::{model, niri, sys, tray};

use smithay_client_toolkit::reexports::protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            channel::{channel, Event, Sender},
            generic::Generic,
            timer::{TimeoutAction, Timer},
            EventLoop, Interest, Mode, PostAction,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::Duration;

/// Cells between right-hand modules, on top of each one's own padding.
const GAP: usize = 1;

pub enum Msg {
    Niri(serde_json::Value),
    Tray(Vec<tray::Item>),
    Volume(Option<(u32, bool)>),
    Notes(Option<(u32, bool)>),
    Backlight(Option<u32>),
    /// The kernel says a battery or backlight changed.
    Power,
}

/// One segment where it landed: `col` and `width` in cells, padding
/// included; the text starts `pad` cells in.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub col: usize,
    pub width: usize,
    pub pad: usize,
    pub seg: Seg,
}

/// Lay a bar out across `cols` cells: left from the start, the clock centred,
/// the right group against the end. The window title gives way first.
pub fn place(left: Vec<Seg>, center: Vec<Seg>, right: Vec<Seg>, cols: usize) -> Vec<Placed> {
    let n = |s: &Seg| s.text.chars().count();
    let mut out = Vec::new();
    let mut x = cols;
    let mut right_start = cols;
    for s in right.into_iter().rev() {
        let w = n(&s) + 2;
        if x < w {
            break;
        }
        x -= w;
        right_start = x;
        out.push(Placed { col: x, width: w, pad: 1, seg: s });
        x = x.saturating_sub(GAP);
    }
    let cw: usize = center.iter().map(|s| n(s) + 2).sum();
    let mut cx = cols.saturating_sub(cw) / 2;
    // Never under the right group, even on a narrow output.
    cx = cx.min(right_start.saturating_sub(cw + GAP));
    let center_start = cx;
    for s in center {
        let w = n(&s) + 2;
        out.push(Placed { col: cx, width: w, pad: 1, seg: s });
        cx += w;
    }
    let mut lx = 0;
    for mut s in left {
        let room = center_start.saturating_sub(lx + GAP);
        if room == 0 {
            break;
        }
        if n(&s) > room {
            s.text = model::truncate(&s.text, room);
        }
        let w = n(&s);
        out.push(Placed { col: lx, width: w, pad: 0, seg: s });
        lx += w;
    }
    out
}

struct Bar {
    output: wl_output::WlOutput,
    name: String,
    layer: LayerSurface,
    width: u32,
    scale: i32,
    configured: bool,
    pool: Option<SlotPool>,
    placed: Vec<Placed>,
    hover: Option<usize>,
    /// What the last paint showed, so an unchanged bar isn't redrawn.
    painted: Option<(Vec<Placed>, Option<usize>, u32, i32)>,
}

struct Popup {
    layer: LayerSurface,
    year: i32,
    month: u32,
    scale: i32,
    configured: bool,
    focused: bool,
    pool: Option<SlotPool>,
}

struct Ui {
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    layer_shell: LayerShell,
    inhibit_mgr: Option<ZwpIdleInhibitManagerV1>,
    inhibitor: Option<ZwpIdleInhibitorV1>,
    qh: QueueHandle<Ui>,
    tx: Sender<Msg>,
    tray_conn: Option<zbus::blocking::Connection>,

    cfg: Config,
    atlases: HashMap<i32, Atlas>,
    state: State,
    cpu_prev: Option<(u64, u64)>,
    backlight_dev: Option<std::path::PathBuf>,
    bars: Vec<Bar>,
    popup: Option<Popup>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    error: Option<String>,
}

/// Start a program detached (own session), not waited for.
pub fn spawn(argv: &[&str]) {
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
    match c.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("lifebar: {}: {e}", argv[0]),
    }
}

fn local(bin: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.local/bin/{bin}")
}

pub fn run(cfg: Config, signals: OwnedFd) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh: QueueHandle<Ui> = event_queue.handle();
    let mut event_loop: EventLoop<Ui> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Msg>();

    let mut ui = Ui {
        compositor: CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?,
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).map_err(|e| e.to_string())?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|e| format!("wlr-layer-shell not supported: {e}"))?,
        inhibit_mgr: globals.bind(&qh, 1..=1, ()).ok(),
        inhibitor: None,
        qh: qh.clone(),
        tx: tx.clone(),
        tray_conn: None,
        cfg,
        atlases: HashMap::new(),
        state: State::default(),
        cpu_prev: None,
        backlight_dev: sys::backlight_dev(),
        bars: Vec::new(),
        popup: None,
        pointer: None,
        keyboard: None,
        error: None,
    };

    // ---- the sources ----
    let t = tx.clone();
    std::thread::spawn(move || niri::listen(|v| {
        let _ = t.send(Msg::Niri(v));
    }));
    let t = tx.clone();
    match tray::run(move |items| {
        let _ = t.send(Msg::Tray(items));
    }) {
        Ok(c) => ui.tray_conn = Some(c),
        Err(e) => eprintln!("lifebar: no tray: {e}"),
    }
    let t = tx.clone();
    std::thread::spawn(move || crate::uevent::listen(|| {
        let _ = t.send(Msg::Power);
    }));

    event_loop
        .handle()
        .insert_source(rx, |ev, _, ui: &mut Ui| {
            if let Event::Msg(m) = ev {
                ui.on_msg(m);
            }
        })
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(Generic::new(signals, Interest::READ, Mode::Level), |_, fd, ui: &mut Ui| {
            let mut buf = [0u8; 64];
            // SAFETY: read from our own self-pipe.
            let n = unsafe { libc::read(std::os::fd::AsRawFd::as_raw_fd(&std::os::fd::AsFd::as_fd(fd)), buf.as_mut_ptr().cast(), buf.len()) };
            for &s in buf.iter().take(n.max(0) as usize) {
                ui.on_signal(s);
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.to_string())?;
    // The clock, on the minute.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, ui: &mut Ui| {
            ui.state.clock = model::now().1;
            ui.paint_all();
            TimeoutAction::ToDuration(Duration::from_secs(model::to_next_minute()))
        })
        .map_err(|e| e.to_string())?;
    // Load, memory, network, volume: waybar's 5 s.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, ui: &mut Ui| {
            ui.poll_fast();
            TimeoutAction::ToDuration(Duration::from_secs(5))
        })
        .map_err(|e| e.to_string())?;
    // Battery, backlight and the notification badge, as a backstop to the
    // events and signals that normally announce them.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, ui: &mut Ui| {
            ui.poll_power();
            ui.load_notes();
            TimeoutAction::ToDuration(Duration::from_secs(30))
        })
        .map_err(|e| e.to_string())?;

    event_queue.roundtrip(&mut ui).map_err(|e| e.to_string())?;
    let outputs: Vec<wl_output::WlOutput> = ui.output_state.outputs().collect();
    for o in outputs {
        ui.add_bar(o);
    }
    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    while ui.error.is_none() {
        event_loop.dispatch(None, &mut ui).map_err(|e| e.to_string())?;
    }
    Err(ui.error.unwrap())
}

impl Ui {
    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Niri(v) => {
                if !self.state.niri.apply(&v) {
                    return;
                }
            }
            Msg::Tray(t) => self.state.tray = t,
            Msg::Volume(v) => self.state.volume = v,
            Msg::Notes(n) => self.state.notes = n,
            Msg::Backlight(b) => self.state.backlight = b,
            Msg::Power => self.poll_power(),
        }
        self.paint_all();
    }

    pub fn on_signal(&mut self, s: u8) {
        match s {
            crate::SIG_NOTES => self.load_notes(),
            crate::SIG_LEVELS => {
                self.load_volume();
                self.poll_power();
                self.paint_all();
            }
            crate::SIG_RELOAD => {
                self.cfg = Config::load();
                self.atlases.clear();
                for b in &mut self.bars {
                    b.layer.set_size(0, self.cfg.height);
                    b.layer.set_exclusive_zone(self.cfg.height as i32);
                    b.layer.commit();
                    b.painted = None;
                    b.pool = None;
                }
                self.paint_all();
            }
            _ => {}
        }
    }

    fn poll_fast(&mut self) {
        let now = sys::read_cpu();
        if let (Some(p), Some(n)) = (self.cpu_prev, now) {
            self.state.cpu = Some(sys::cpu_pct(p, n));
        }
        self.cpu_prev = now;
        self.state.mem = sys::read_mem().map(|m| m.0);
        self.state.net = Some(sys::read_net());
        self.state.vpn = sys::read_vpn();
        self.load_volume();
        self.paint_all();
    }

    fn poll_power(&mut self) {
        self.state.battery = sys::read_battery();
        self.state.backlight = self.backlight_dev.as_deref().and_then(sys::read_backlight);
        self.paint_all();
    }

    fn load_volume(&self) {
        let t = self.tx.clone();
        std::thread::spawn(move || {
            let _ = t.send(Msg::Volume(sys::read_volume()));
        });
    }

    fn load_notes(&self) {
        let t = self.tx.clone();
        std::thread::spawn(move || {
            let _ = t.send(Msg::Notes(sys::read_notes()));
        });
    }

    // ---- bars ----

    fn add_bar(&mut self, output: wl_output::WlOutput) {
        if self.bars.iter().any(|b| b.output == output) {
            return;
        }
        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default();
        let scale = info.map(|i| i.scale_factor).unwrap_or(1).max(1);
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("lifebar"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, self.cfg.height);
        layer.set_exclusive_zone(self.cfg.height as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.bars.push(Bar { output, name, layer, width: 0, scale, configured: false, pool: None, placed: Vec::new(), hover: None, painted: None });
    }

    fn atlas(&mut self, scale: i32) -> Option<&mut Atlas> {
        if !self.atlases.contains_key(&scale) {
            // The config's size is CSS pixels, as waybar's was; Atlas takes points.
            match Atlas::new(&self.cfg.font, self.cfg.size * 0.75, scale) {
                Ok(a) => {
                    self.atlases.insert(scale, a);
                }
                Err(e) => {
                    self.error = Some(e);
                    return None;
                }
            }
        }
        self.atlases.get_mut(&scale)
    }

    fn color(cfg: &Config, r: Role) -> Rgba {
        match r {
            Role::Text => cfg.text,
            Role::Accent => cfg.accent,
            Role::Warn => cfg.warn,
            Role::Urgent => cfg.urgent,
        }
    }

    fn paint_all(&mut self) {
        for i in 0..self.bars.len() {
            self.paint_bar(i);
        }
    }

    fn paint_bar(&mut self, i: usize) {
        let (scale, width) = {
            let b = &self.bars[i];
            if !b.configured || b.width == 0 {
                return;
            }
            (b.scale, b.width)
        };
        let Some(atlas) = self.atlas(scale) else { return };
        let (cw, ch) = (atlas.cw, atlas.ch);
        let s = scale as usize;
        let (w, h) = (width as usize * s, self.cfg.height as usize * s);
        let margin = cw / 2;
        let cols = (w - 2 * margin) / cw;
        let b = &self.bars[i];
        let placed = place(self.state.left(&b.name), self.state.center(), self.state.right(), cols);
        let key = (placed.clone(), b.hover, width, scale);
        if b.painted.as_ref() == Some(&key) {
            return;
        }
        let cfg = &self.cfg;
        let atlas = self.atlases.get_mut(&scale).expect("atlas");
        let mut px = vec![0u32; w * h];
        let mut cv = Canvas { px: &mut px, w, h };
        cv.fill(0, 0, w, h, cfg.bg);
        cv.fill(0, h - s, w, s, cfg.border);
        let ty = (h - s).saturating_sub(ch) / 2;
        for (k, p) in placed.iter().enumerate() {
            let x0 = margin + p.col * cw;
            let clickable = p.seg.left.is_some() || p.seg.right.is_some() || p.seg.wheel.is_some();
            if b.hover == Some(k) && clickable {
                cv.fill(x0, 0, p.width * cw, h - s, cfg.surface);
            }
            let col = Ui::color(cfg, p.seg.role);
            let col = if b.hover == Some(k) && clickable && p.seg.role == Role::Text { cfg.accent } else { col };
            for (j, c) in p.seg.text.chars().enumerate() {
                cv.glyph(atlas, x0 + (p.pad + j) * cw, ty, c, col);
            }
            if p.seg.mark {
                cv.fill(x0, h - s - 2 * s, p.width * cw, 2 * s, cfg.accent);
            }
        }
        let b = &mut self.bars[i];
        let pool = match &mut b.pool {
            Some(p) => p,
            None => match SlotPool::new(w * h * 4 * 2, &self.shm) {
                Ok(p) => b.pool.insert(p),
                Err(e) => {
                    self.error = Some(format!("shm pool: {e}"));
                    return;
                }
            },
        };
        let Ok((buffer, bytes)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else { return };
        for (dst, p) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&px) {
            *dst = p.to_le_bytes();
        }
        let surface = b.layer.wl_surface();
        surface.set_buffer_scale(scale);
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, w as i32, h as i32);
            surface.commit();
        }
        b.placed = placed;
        b.painted = Some(key);
    }

    /// The placed segment under logical x on bar `i`.
    fn seg_at(&mut self, i: usize, x: f64) -> Option<usize> {
        let scale = self.bars[i].scale;
        let cw = self.atlas(scale)?.cw;
        let px = (x * scale as f64) as usize;
        let col = px.checked_sub(cw / 2)? / cw;
        self.bars[i].placed.iter().position(|p| (p.col..p.col + p.width).contains(&col))
    }

    fn act(&mut self, a: &Act, bar: usize) {
        match a {
            Act::Workspace(id) => niri::focus_workspace(*id),
            Act::Calendar => self.toggle_calendar(bar),
            Act::NotifMenu => spawn(&[&local("notif-menu.sh")]),
            Act::Dnd => spawn(&[&local("dnd-toggle.sh")]),
            Act::Idle => self.toggle_idle(),
            Act::Panel => {
                let lp = local("lifepanel");
                if std::path::Path::new(&lp).is_file() { spawn(&[&lp]) } else { spawn(&[&local("net-menu.sh")]) }
            }
            Act::NetSettings => spawn(&[&local("lifeconf"), "--gui", "--panel", "network"]),
            Act::VpnSettings => spawn(&[&local("lifeconf"), "--gui", "--panel", "vpn"]),
            Act::SoundSettings => spawn(&[&local("lifeconf"), "--gui", "--panel", "sound"]),
            // sysmon.sh focuses the htop window if one is open, else opens it.
            Act::Top => {
                let s = local("sysmon.sh");
                if std::path::Path::new(&s).is_file() { spawn(&[&s]) } else { spawn(&["kitty", "-e", "htop"]) }
            }
            Act::Mute => self.then_refresh(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
            Act::Volume(d) => self.then_refresh(&[&local("vol-osd.sh"), if *d > 0 { "up" } else { "down" }]),
            Act::Brightness(d) => self.then_refresh(&[&local("bright-osd.sh"), if *d > 0 { "up" } else { "down" }]),
            Act::Tray(i, b) => {
                if let (Some(conn), Some(item)) = (&self.tray_conn, self.state.tray.get(*i)) {
                    let how = match b {
                        Button::Left => tray::Click::Activate,
                        Button::Middle => tray::Click::Secondary,
                        Button::Right => tray::Click::Menu,
                    };
                    tray::click(conn, &item.key, how);
                }
            }
        }
    }

    /// Run a quick command, then re-read volume and brightness.
    fn then_refresh(&self, argv: &[&str]) {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        let t = self.tx.clone();
        let dev = self.backlight_dev.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new(&argv[0]).args(&argv[1..]).stdin(std::process::Stdio::null()).status();
            let _ = t.send(Msg::Volume(sys::read_volume()));
            let _ = t.send(Msg::Backlight(dev.as_deref().and_then(sys::read_backlight)));
        });
    }

    fn toggle_idle(&mut self) {
        if let Some(i) = self.inhibitor.take() {
            i.destroy();
            self.state.idle_inhibited = false;
        } else if let (Some(m), Some(b)) = (&self.inhibit_mgr, self.bars.first()) {
            // Held as long as the bar is on screen, which is always.
            self.inhibitor = Some(m.create_inhibitor(b.layer.wl_surface(), &self.qh, ()));
            self.state.idle_inhibited = true;
        }
        self.paint_all();
    }

    // ---- the calendar ----

    fn toggle_calendar(&mut self, bar: usize) {
        if self.popup.take().is_some() {
            return;
        }
        let ((y, m, _), _) = model::now();
        let output = self.bars[bar].output.clone();
        let scale = self.bars[bar].scale;
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("lifebar-calendar"), Some(&output));
        layer.set_anchor(Anchor::TOP);
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        let Some(a) = self.atlas(scale) else { return };
        let (cw, ch) = (a.cw, a.ch);
        let s = scale as usize;
        let (w, h) = ((20 + 4) * cw, (8 * (ch + 2 * s)) + 2 * ch);
        layer.set_size((w / s) as u32, (h / s) as u32);
        layer.commit();
        self.popup = Some(Popup { layer, year: y, month: m, scale, configured: false, focused: false, pool: None });
    }

    fn paint_popup(&mut self) {
        let Some(p) = &self.popup else { return };
        if !p.configured {
            return;
        }
        let (scale, year, month) = (p.scale, p.year, p.month);
        let ((ty, tm, td), _) = model::now();
        let today = (ty == year && tm == month).then_some(td);
        let lines = model::month(year, month, today);
        let Some(a) = self.atlas(scale) else { return };
        let (cw, ch) = (a.cw, a.ch);
        let s = scale as usize;
        let row = ch + 2 * s;
        let (w, h) = ((20 + 4) * cw, 8 * row + 2 * ch);
        let cfg = &self.cfg;
        let atlas = self.atlases.get_mut(&scale).expect("atlas");
        let mut px = vec![0u32; w * h];
        let mut cv = Canvas { px: &mut px, w, h };
        cv.fill(0, 0, w, h, cfg.bg);
        cv.fill(0, 0, w, s, cfg.border);
        cv.fill(0, h - s, w, s, cfg.border);
        cv.fill(0, 0, s, h, cfg.border);
        cv.fill(w - s, 0, s, h, cfg.border);
        for (k, (text, mark)) in lines.iter().enumerate() {
            let y = ch + k * row;
            let col = match k {
                0 => cfg.accent,
                1 => [cfg.text[0], cfg.text[1], cfg.text[2], cfg.text[3] / 2 + 64],
                _ => cfg.text,
            };
            if let Some(m) = mark {
                cv.fill((2 + m) * cw - s, y - s, 2 * cw + 2 * s, row, cfg.surface);
            }
            for (j, c) in text.chars().enumerate() {
                let hit = mark.is_some_and(|m| j == m || j == m + 1);
                cv.glyph(atlas, (2 + j) * cw, y, c, if hit { cfg.accent } else { col });
            }
        }
        let p = self.popup.as_mut().unwrap();
        let pool = match &mut p.pool {
            Some(pl) => pl,
            None => match SlotPool::new(w * h * 4 * 2, &self.shm) {
                Ok(pl) => p.pool.insert(pl),
                Err(_) => return,
            },
        };
        let Ok((buffer, bytes)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else { return };
        for (dst, v) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&px) {
            *dst = v.to_le_bytes();
        }
        let surface = p.layer.wl_surface();
        surface.set_buffer_scale(scale);
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, w as i32, h as i32);
            surface.commit();
        }
    }

    fn popup_month(&mut self, d: i32) {
        if let Some(p) = &mut self.popup {
            let m0 = p.month as i32 - 1 + d;
            p.year += m0.div_euclid(12);
            p.month = (m0.rem_euclid(12) + 1) as u32;
        }
        self.paint_popup();
    }

    fn bar_of(&self, s: &wl_surface::WlSurface) -> Option<usize> {
        self.bars.iter().position(|b| b.layer.wl_surface() == s)
    }
    fn is_popup(&self, s: &wl_surface::WlSurface) -> bool {
        self.popup.as_ref().is_some_and(|p| p.layer.wl_surface() == s)
    }
}

impl LayerShellHandler for Ui {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.popup.as_ref().is_some_and(|p| p.layer == *layer) {
            self.popup = None;
        }
        self.bars.retain(|b| b.layer != *layer);
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        if let Some(p) = self.popup.as_mut().filter(|p| p.layer == *layer) {
            p.configured = true;
            self.paint_popup();
            return;
        }
        if let Some(i) = self.bars.iter().position(|b| b.layer == *layer) {
            let b = &mut self.bars[i];
            if c.new_size.0 != 0 && c.new_size.0 != b.width {
                b.width = c.new_size.0;
                b.pool = None;
                b.painted = None;
            }
            b.configured = true;
            self.paint_bar(i);
        }
    }
}

impl CompositorHandler for Ui {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        let factor = factor.max(1);
        if let Some(i) = self.bar_of(surface) {
            if self.bars[i].scale != factor {
                self.bars[i].scale = factor;
                self.bars[i].pool = None;
                self.bars[i].painted = None;
                self.paint_bar(i);
            }
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Ui {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_bar(output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // The name arrives with the output's info; a bar made before it did
        // would show no workspaces.
        let name = self.output_state.info(&output).and_then(|i| i.name);
        if let (Some(b), Some(n)) = (self.bars.iter_mut().find(|b| b.output == output), name) {
            if b.name != n {
                b.name = n;
                b.painted = None;
            }
        }
        self.paint_all();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.bars.retain(|b| b.output != output);
    }
}

impl PointerHandler for Ui {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            if self.is_popup(&e.surface) {
                match e.kind {
                    PointerEventKind::Axis { vertical, .. } => {
                        let d = if vertical.discrete != 0 { vertical.discrete } else { (vertical.absolute / 15.0) as i32 };
                        if d != 0 {
                            self.popup_month(d.signum());
                        }
                    }
                    PointerEventKind::Press { .. } => self.popup = None,
                    _ => {}
                }
                continue;
            }
            let Some(i) = self.bar_of(&e.surface) else { continue };
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let k = self.seg_at(i, e.position.0);
                    if k != self.bars[i].hover {
                        self.bars[i].hover = k;
                        self.paint_bar(i);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.bars[i].hover.take().is_some() {
                        self.paint_bar(i);
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let b = match button {
                        BTN_LEFT => Button::Left,
                        BTN_MIDDLE => Button::Middle,
                        BTN_RIGHT => Button::Right,
                        _ => continue,
                    };
                    let Some(k) = self.seg_at(i, e.position.0) else { continue };
                    if let Some(a) = self.bars[i].placed[k].seg.act(b).cloned() {
                        self.act(&a, i);
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let d = if vertical.discrete != 0 { vertical.discrete } else { (vertical.absolute / 15.0) as i32 };
                    let Some(k) = self.seg_at(i, e.position.0) else { continue };
                    if let (Some((up, down)), true) = (self.bars[i].placed[k].seg.wheel.clone(), d != 0) {
                        self.act(if d < 0 { &up } else { &down }, i);
                    }
                }
                _ => {}
            }
        }
    }
}

impl KeyboardHandler for Ui {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, s: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        if let Some(p) = self.popup.as_mut().filter(|p| p.layer.wl_surface() == s) {
            p.focused = true;
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, s: &wl_surface::WlSurface, _: u32) {
        // Focus moved on: the calendar closes, like the quick settings panel.
        if self.popup.as_ref().is_some_and(|p| p.focused && p.layer.wl_surface() == s) {
            self.popup = None;
        }
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, ev: KeyEvent) {
        match ev.keysym {
            Keysym::Escape | Keysym::Return => self.popup = None,
            Keysym::Left | Keysym::Up | Keysym::Page_Up => self.popup_month(-1),
            Keysym::Right | Keysym::Down | Keysym::Page_Down => self.popup_month(1),
            _ => {}
        }
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: Modifiers, _: RawModifiers, _: u32) {}
}

impl SeatHandler for Ui {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl ShmHandler for Ui {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Ui {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// The idle-inhibit objects send no events.
impl Dispatch<ZwpIdleInhibitManagerV1, ()> for Ui {
    fn event(_: &mut Self, _: &ZwpIdleInhibitManagerV1, _: <ZwpIdleInhibitManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpIdleInhibitorV1, ()> for Ui {
    fn event(_: &mut Self, _: &ZwpIdleInhibitorV1, _: <ZwpIdleInhibitorV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

smithay_client_toolkit::delegate_compositor!(Ui);
smithay_client_toolkit::delegate_output!(Ui);
smithay_client_toolkit::delegate_shm!(Ui);
smithay_client_toolkit::delegate_seat!(Ui);
smithay_client_toolkit::delegate_pointer!(Ui);
smithay_client_toolkit::delegate_keyboard!(Ui);
smithay_client_toolkit::delegate_layer!(Ui);
smithay_client_toolkit::delegate_registry!(Ui);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Role;

    fn seg(t: &str) -> Seg {
        Seg { text: t.into(), role: Role::Text, mark: false, left: None, middle: None, right: None, wheel: None }
    }

    #[test]
    fn right_group_hugs_the_end_and_the_clock_is_centred() {
        let p = place(vec![seg(" 1 ")], vec![seg("12:00")], vec![seg("CPU 4%"), seg("MEM 9%")], 100);
        let mem = p.iter().find(|x| x.seg.text == "MEM 9%").unwrap();
        assert_eq!(mem.col + mem.width, 100);
        let cpu = p.iter().find(|x| x.seg.text == "CPU 4%").unwrap();
        assert_eq!(cpu.col + cpu.width + GAP, mem.col);
        let clock = p.iter().find(|x| x.seg.text == "12:00").unwrap();
        assert_eq!(clock.col, (100 - 7) / 2);
        assert_eq!(p.iter().find(|x| x.seg.text == " 1 ").unwrap().col, 0);
    }

    #[test]
    fn a_long_title_gives_way_to_the_clock() {
        let title = format!("  {}", "x".repeat(80));
        let p = place(vec![seg(" 1 "), seg(&title)], vec![seg("12:00")], vec![], 60);
        let t = p.iter().find(|x| x.seg.text.starts_with("  x")).unwrap();
        let clock = p.iter().find(|x| x.seg.text == "12:00").unwrap();
        assert!(t.col + t.width < clock.col, "{} {} {}", t.col, t.width, clock.col);
        assert!(t.seg.text.ends_with('…'));
    }
}
