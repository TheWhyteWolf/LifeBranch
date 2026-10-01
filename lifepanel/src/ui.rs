// SPDX-License-Identifier: GPL-3.0-or-later
// The window: a layer-shell surface in the top-right corner, under the bar.
// It takes the keyboard on demand and closes when focus moves elsewhere (a
// click on a window), on Esc, or when `lifepanel` is run again.
//
// Loads and jobs run on worker threads and report back over a channel, so a
// ten-second wifi join never freezes the panel; sliders and toggles change
// the shown state at once and the command catches up.

use crate::backend::{self, Loaded, Section};
use crate::cli::{Cfg, Rgba};
use crate::panel::{Item, Key, Out, Panel, Role, COLS};
use crate::render::{Atlas, Canvas};
use crate::sys::run_real;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            channel::{channel, Event, Sender},
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use std::time::Duration;

/// How often the open panel refreshes what changes on its own (scan
/// results, a drive plugged in, the battery).
const REFRESH: Duration = Duration::from_secs(4);
/// Gap to the screen edge, logical pixels.
const MARGIN: i32 = 6;

enum Msg {
    Loaded(Loaded),
    Done(u64, Result<String, String>),
}

/// Row positions in buffer pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Geo {
    pub w: usize,
    pub h: usize,
    pub pad_x: usize,
    pub pad_y: usize,
    pub row_h: usize,
    pub border: usize,
    /// (top, height) of each item, then the footer lines.
    pub rows: Vec<(usize, usize)>,
}

pub fn layout(cfg: &Cfg, cw: usize, ch: usize, scale: usize, items: &[Item], footer_lines: usize) -> Geo {
    let border = cfg.border_width * scale;
    let (pad_x, pad_y) = (cfg.pad_h * scale + border, cfg.pad_v * scale + border);
    let row_h = ch + 4 * scale;
    let mut y = pad_y;
    let mut rows = Vec::with_capacity(items.len() + footer_lines);
    for it in items {
        let h = if *it == Item::Sep { row_h / 2 } else { row_h };
        rows.push((y, h));
        y += h;
    }
    for _ in 0..footer_lines {
        rows.push((y, row_h));
        y += row_h;
    }
    let up = |v: usize| v.div_ceil(scale) * scale;
    Geo { w: up(COLS * cw + 2 * pad_x), h: up(y + pad_y), pad_x, pad_y, row_h, border, rows }
}

/// A job that finished after the panel closed: only failures and real news
/// (a join, a mount) are worth a notification.
fn report(r: Result<String, String>) {
    let (summary, body) = match r {
        Ok(t) if t.is_empty() => return,
        Ok(t) => (t, String::new()),
        Err(e) => ("Quick settings".to_string(), e),
    };
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "lifepanel", &summary, &body])
        .stdin(std::process::Stdio::null())
        .status();
}

/// Wrap the footer to at most two lines.
fn wrap(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= COLS {
        return vec![s.to_string()];
    }
    let cut = chars[..COLS].iter().rposition(|c| *c == ' ').filter(|&i| i > COLS / 2).unwrap_or(COLS);
    let first: String = chars[..cut].iter().collect();
    let rest: Vec<char> = chars[cut..].iter().copied().skip_while(|c| *c == ' ').collect();
    let second: String = if rest.len() > COLS {
        rest[..COLS - 1].iter().chain(std::iter::once(&'…')).collect()
    } else {
        rest.iter().collect()
    };
    vec![first, second]
}

struct Ui {
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    layer_shell: LayerShell,
    loop_handle: LoopHandle<'static, Ui>,
    qh: QueueHandle<Ui>,
    tx: Sender<Msg>,

    cfg: Cfg,
    panel: Panel,
    items: Vec<Item>,
    atlas: Option<Atlas>,
    scale: i32,
    geo: Option<Geo>,
    /// The logical size last asked of the compositor.
    asked: (u32, u32),
    layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    canvas: Vec<u32>,
    configured: bool,
    focused: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    mods: Modifiers,
    next_id: u64,
    /// Jobs still running.
    pending: usize,
    done: bool,
    error: Option<String>,
}

pub fn run(cfg: Cfg) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh: QueueHandle<Ui> = event_queue.handle();
    let mut event_loop: EventLoop<Ui> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("wlr-layer-shell not supported: {e}"))?;
    let (tx, rx) = channel::<Msg>();

    let mut ui = Ui {
        compositor: CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?,
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).map_err(|e| e.to_string())?,
        layer_shell,
        loop_handle: event_loop.handle(),
        qh: qh.clone(),
        tx,
        cfg,
        panel: Panel::default(),
        items: Vec::new(),
        atlas: None,
        scale: 1,
        geo: None,
        asked: (0, 0),
        layer: None,
        pool: None,
        canvas: Vec::new(),
        configured: false,
        focused: false,
        keyboard: None,
        pointer: None,
        mods: Modifiers::default(),
        next_id: 0,
        pending: 0,
        done: false,
        error: None,
    };
    for s in backend::ALL {
        ui.load(s);
    }

    event_loop
        .handle()
        .insert_source(rx, |ev, _, ui: &mut Ui| {
            if let Event::Msg(m) = ev {
                match m {
                    Msg::Loaded(l) => ui.panel.loaded(l),
                    Msg::Done(id, r) => {
                        ui.pending -= 1;
                        if ui.done {
                            // The panel is gone; say how it went elsewhere.
                            return report(r);
                        }
                        ui.panel.finished(id, r);
                    }
                }
                if !ui.done {
                    ui.refresh();
                }
            }
        })
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(Timer::from_duration(REFRESH), |_, _, ui: &mut Ui| {
            if ui.done {
                return TimeoutAction::Drop;
            }
            for s in [Section::Power, Section::Drives] {
                ui.load(s);
            }
            if ui.panel.open_wifi {
                ui.load(Section::Wifi);
            }
            if ui.panel.open_bt {
                ui.load(Section::Bt);
            }
            TimeoutAction::ToDuration(REFRESH)
        })
        .map_err(|e| e.to_string())?;

    event_queue.roundtrip(&mut ui).map_err(|e| e.to_string())?;
    ui.scale = ui.output_state.outputs().filter_map(|o| ui.output_state.info(&o)).map(|i| i.scale_factor).max().unwrap_or(1).max(1);
    ui.atlas = Some(Atlas::new(&ui.cfg.font, ui.cfg.font_size, ui.scale)?);
    ui.relayout();
    ui.create_surface();

    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    while !ui.done && ui.error.is_none() {
        event_loop.dispatch(None, &mut ui).map_err(|e| e.to_string())?;
    }
    // Closed with a join or a mount still running: take the panel off the
    // screen now, and stay until those finish so their results are not lost.
    if let Some(l) = ui.layer.take() {
        drop(l);
        event_loop.dispatch(Some(Duration::ZERO), &mut ui).map_err(|e| e.to_string())?;
    }
    ui.done = true;
    while ui.pending > 0 {
        event_loop.dispatch(None, &mut ui).map_err(|e| e.to_string())?;
    }
    match ui.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Ui {
    fn load(&self, s: Section) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Loaded(backend::load(s, &run_real)));
        });
    }

    fn start(&mut self, out: Out) {
        for job in out.jobs {
            self.next_id += 1;
            self.pending += 1;
            let id = self.next_id;
            self.panel.started(id, &job);
            let tx = self.tx.clone();
            let reload = job.section();
            std::thread::spawn(move || {
                let r = backend::run_job(job, &run_real);
                let _ = tx.send(Msg::Done(id, r));
                if let Some(s) = reload {
                    let _ = tx.send(Msg::Loaded(backend::load(s, &run_real)));
                }
            });
        }
        if out.close {
            self.done = true;
        }
        self.refresh();
    }

    fn footer(&self) -> Vec<(String, bool)> {
        match self.panel.footer() {
            Some((t, err)) => wrap(&t).into_iter().map(|l| (l, err)).collect(),
            None => Vec::new(),
        }
    }

    fn relayout(&mut self) {
        self.items = self.panel.items();
        let Some(a) = &self.atlas else { return };
        let g = layout(&self.cfg, a.cw, a.ch, self.scale as usize, &self.items, self.footer().len());
        let size = ((g.w / self.scale as usize) as u32, (g.h / self.scale as usize) as u32);
        self.geo = Some(g);
        if size != self.asked {
            self.asked = size;
            if let Some(l) = &self.layer {
                // A new size takes effect with the next configure; paint then.
                l.set_size(size.0, size.1);
                l.commit();
                self.configured = false;
            }
        }
    }

    /// State changed: lay out again and repaint.
    fn refresh(&mut self) {
        self.relayout();
        self.paint();
    }

    fn create_surface(&mut self) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("lifepanel"), None);
        layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
        layer.set_margin(MARGIN, MARGIN, 0, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        layer.set_size(self.asked.0, self.asked.1);
        layer.commit();
        self.layer = Some(layer);
    }

    fn on_key(&mut self, ev: KeyEvent) {
        let k = match ev.keysym {
            Keysym::Escape => Key::Esc,
            Keysym::Up | Keysym::ISO_Left_Tab => Key::Up,
            Keysym::Down | Keysym::Tab => Key::Down,
            Keysym::Left => Key::Left,
            Keysym::Right => Key::Right,
            Keysym::Return | Keysym::KP_Enter | Keysym::space if self.panel.sel != Item::Password || ev.keysym != Keysym::space => Key::Enter,
            Keysym::BackSpace => Key::Back,
            Keysym::Delete => Key::Eject,
            _ => {
                if self.mods.ctrl || self.mods.alt || self.mods.logo {
                    return;
                }
                match ev.utf8 {
                    Some(t) if !t.is_empty() => Key::Text(t),
                    _ => return,
                }
            }
        };
        let out = self.panel.key(k);
        self.start(out);
    }

    /// The item under buffer-pixel y and the column under x.
    fn hit(&self, x: f64, y: f64) -> Option<(Item, f64)> {
        let g = self.geo.as_ref()?;
        let a = self.atlas.as_ref()?;
        let (x, y) = (x * self.scale as f64, y * self.scale as f64);
        let i = g.rows.iter().take(self.items.len()).position(|&(top, h)| (top as f64..(top + h) as f64).contains(&y))?;
        Some((self.items[i].clone(), (x - g.pad_x as f64) / a.cw as f64))
    }

    fn color(&self, r: Role, selected: bool) -> Rgba {
        let c = &self.cfg;
        match r {
            Role::Label if selected => c.selection_text,
            Role::Label => c.text,
            Role::Value => c.input,
            Role::Accent => c.prompt_color,
            Role::Dim => [c.text[0], c.text[1], c.text[2], (c.text[3] as u32 * 6 / 10) as u8],
        }
    }

    fn paint(&mut self) {
        if !self.configured {
            return;
        }
        let Some(g) = self.geo.clone() else { return };
        let footer = self.footer();
        let lines: Vec<_> = self.items.iter().map(|it| self.panel.line(it)).collect();
        let colors: Vec<Vec<Rgba>> = lines
            .iter()
            .zip(&self.items)
            .map(|(l, it)| l.spans.iter().map(|(_, r)| self.color(*r, *it == self.panel.sel)).collect())
            .collect();
        let (dim, accent, value) = (self.color(Role::Dim, false), self.color(Role::Accent, false), self.color(Role::Value, false));
        let cfg = &self.cfg;
        let Some(atlas) = self.atlas.as_mut() else { return };
        self.canvas.clear();
        self.canvas.resize(g.w * g.h, 0);
        let mut cv = Canvas { px: &mut self.canvas, w: g.w, h: g.h };
        let (cw, ch) = (atlas.cw, atlas.ch);
        let s = self.scale as usize;

        cv.fill(0, 0, g.w, g.h, cfg.background);
        if g.border > 0 {
            cv.fill(0, 0, g.w, g.border, cfg.border);
            cv.fill(0, g.h - g.border, g.w, g.border, cfg.border);
            cv.fill(0, 0, g.border, g.h, cfg.border);
            cv.fill(g.w - g.border, 0, g.border, g.h, cfg.border);
        }
        let text_y = |top: usize, h: usize| top + h.saturating_sub(ch) / 2;
        for (k, it) in self.items.iter().enumerate() {
            let (top, h) = g.rows[k];
            if *it == Item::Sep {
                cv.fill(g.pad_x, top + h / 2, g.w - 2 * g.pad_x, s, cfg.border);
                continue;
            }
            if *it == self.panel.sel {
                let inset = g.border + (cfg.pad_h * s) / 2;
                cv.fill(inset, top, g.w - 2 * inset, h, cfg.selection);
            }
            let mut x = g.pad_x;
            for ((text, _), col) in lines[k].spans.iter().zip(&colors[k]) {
                for c in text.chars() {
                    cv.glyph(atlas, x, text_y(top, h), c, *col);
                    x += cw;
                }
            }
        }
        for (k, (text, err)) in footer.iter().enumerate() {
            let (top, h) = g.rows[self.items.len() + k];
            let col = if *err { accent } else if k == 0 { value } else { dim };
            for (j, c) in text.chars().enumerate() {
                cv.glyph(atlas, g.pad_x + j * cw, text_y(top, h), c, col);
            }
        }

        let Some(layer) = &self.layer else { return };
        // Sized for the tallest panel likely (lists open), so growing rarely
        // needs a new pool.
        let need = g.w * g.h * 4 * 2;
        if self.pool.as_ref().is_some_and(|p| p.len() < need) {
            self.pool = None;
        }
        let pool = match &mut self.pool {
            Some(p) => p,
            None => match SlotPool::new(need, &self.shm) {
                Ok(p) => self.pool.insert(p),
                Err(e) => {
                    self.error = Some(format!("shm pool: {e}"));
                    return;
                }
            },
        };
        let Ok((buffer, bytes)) = pool.create_buffer(g.w as i32, g.h as i32, g.w as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        for (dst, px) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&self.canvas) {
            *dst = px.to_le_bytes();
        }
        let surface = layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, g.w as i32, g.h as i32);
            surface.commit();
        }
    }
}

impl LayerShellHandler for Ui {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.done = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        self.configured = true;
        self.paint();
    }
}

impl CompositorHandler for Ui {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, factor: i32) {
        let factor = factor.max(1);
        if factor == self.scale {
            return;
        }
        self.scale = factor;
        match Atlas::new(&self.cfg.font, self.cfg.font_size, self.scale) {
            Ok(a) => self.atlas = Some(a),
            Err(e) => {
                self.error = Some(e);
                return;
            }
        }
        self.pool = None;
        self.refresh();
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl KeyboardHandler for Ui {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        self.focused = true;
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        // Focus went to a window: the user is done here.
        if self.focused {
            self.done = true;
        }
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {
        // Driven by get_keyboard_with_repeat's callback (see lifemenu).
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Ui {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let Some(mine) = self.layer.as_ref().map(|l| l.wl_surface().clone()) else { return };
        for e in events {
            if e.surface != mine {
                continue;
            }
            let Some((item, col)) = self.hit(e.position.0, e.position.1) else { continue };
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if item.selectable() && item != self.panel.sel && self.panel.sel != Item::Password {
                        self.panel.sel = item;
                        self.paint();
                    }
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    let out = self.panel.click(&item, col);
                    self.start(out);
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = if vertical.discrete != 0 { vertical.discrete } else { (vertical.absolute / 15.0) as i32 };
                    if steps != 0 {
                        let out = self.panel.wheel(&item, steps);
                        self.start(out);
                    }
                }
                _ => {}
            }
        }
    }
}

impl SeatHandler for Ui {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|ui: &mut Ui, _kbd, event| ui.on_key(event)),
            ) {
                Ok(k) => self.keyboard = Some(k),
                Err(e) => self.error = Some(format!("no keyboard: {e}")),
            }
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl OutputHandler for Ui {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
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

smithay_client_toolkit::delegate_compositor!(Ui);
smithay_client_toolkit::delegate_output!(Ui);
smithay_client_toolkit::delegate_shm!(Ui);
smithay_client_toolkit::delegate_seat!(Ui);
smithay_client_toolkit::delegate_keyboard!(Ui);
smithay_client_toolkit::delegate_pointer!(Ui);
smithay_client_toolkit::delegate_layer!(Ui);
smithay_client_toolkit::delegate_registry!(Ui);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_are_half_rows_and_the_footer_follows() {
        let cfg = Cfg::default();
        let items = [Item::Wifi, Item::Sep, Item::Volume];
        let g = layout(&cfg, 8, 16, 1, &items, 2);
        let rh = 16 + 4;
        assert_eq!(g.rows.len(), 5);
        assert_eq!(g.rows[1], (g.pad_y + rh, rh / 2));
        assert_eq!(g.rows[2].0, g.pad_y + rh + rh / 2);
        assert_eq!(g.h, g.pad_y * 2 + rh * 4 + rh / 2);
        assert_eq!(g.w, COLS * 8 + 2 * g.pad_x);
        let two = layout(&cfg, 16, 33, 2, &items, 0);
        assert_eq!((two.w % 2, two.h % 2), (0, 0), "buffer divides by the scale");
    }

    #[test]
    fn footer_wraps_at_a_space_into_two_lines() {
        assert_eq!(wrap("short"), ["short"]);
        let long = "Connection activation failed: (7) Secrets were required, but not provided by the agent at all, ever";
        let w = wrap(long);
        assert_eq!(w.len(), 2);
        assert!(w.iter().all(|l| l.chars().count() <= COLS));
        assert!(w[0].ends_with("Secrets") || w[0].ends_with("were"), "{w:?}");
    }
}
