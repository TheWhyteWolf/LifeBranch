// SPDX-License-Identifier: GPL-3.0-or-later
// The window: one centred wlr-layer-shell surface on the overlay layer with
// the keyboard exclusively, like fuzzel. It repaints only when something
// changes and idles in between. Keyboard and mouse both work: hover selects,
// a click picks, the wheel scrolls.

use crate::cli::Cfg;
use crate::matcher;
use crate::model::{Menu, Outcome};
use crate::render::{Atlas, Canvas};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
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

#[derive(Debug, PartialEq)]
pub enum Done {
    Picked(usize),
    Text(String),
    Cancel,
}

/// Pixel geometry at the current scale (buffer pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geo {
    pub w: usize,
    pub h: usize,
    pub pad_x: usize,
    pub pad_y: usize,
    pub row_h: usize,
    pub list_y: usize,
    pub border: usize,
}

/// Size the window for `cols` characters, an optional message line and
/// `rows` list rows; rounded up so the buffer divides by the scale.
pub fn layout(cfg: &Cfg, cw: usize, ch: usize, scale: usize, rows: usize, mesg: bool) -> Geo {
    let border = cfg.border_width * scale;
    let (pad_x, pad_y) = (cfg.pad_h * scale + border, cfg.pad_v * scale + border);
    let row_h = ch + 2 * scale;
    let header = row_h * (1 + usize::from(mesg));
    let list_y = pad_y + header + if rows > 0 { cfg.inner * scale } else { 0 };
    let up = |v: usize| v.div_ceil(scale) * scale;
    Geo {
        w: up(cfg.width * cw + 2 * pad_x),
        h: up(list_y + rows * row_h + pad_y),
        pad_x,
        pad_y,
        row_h,
        list_y,
        border,
    }
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

    cfg: Cfg,
    labels: Vec<String>,
    menu: Menu,
    atlas: Option<Atlas>,
    scale: i32,
    geo: Option<Geo>,
    layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    canvas: Vec<u32>,
    configured: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    mods: Modifiers,
    done: Option<Done>,
    error: Option<String>,
}

/// Show the menu and block until something is picked or it is cancelled.
/// `labels` are what each row shows; `hay` what the matcher searches.
pub fn run(cfg: Cfg, labels: Vec<String>, hay: Vec<String>) -> Result<Done, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh: QueueHandle<Ui> = event_queue.handle();
    let mut event_loop: EventLoop<Ui> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("wlr-layer-shell not supported: {e}"))?;

    let rows = if cfg.prompt_only { 0 } else { cfg.lines.min(labels.len()) };
    let mut ui = Ui {
        compositor: CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?,
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).map_err(|e| e.to_string())?,
        layer_shell,
        loop_handle: event_loop.handle(),
        qh: qh.clone(),
        menu: Menu::new(hay, rows, cfg.only_match),
        cfg,
        labels,
        atlas: None,
        scale: 1,
        geo: None,
        layer: None,
        pool: None,
        canvas: Vec::new(),
        configured: false,
        keyboard: None,
        pointer: None,
        mods: Modifiers::default(),
        done: None,
        error: None,
    };
    // Learn the outputs first: the first frame should already be sharp on a
    // HiDPI panel, not drawn at 1x and redrawn when the scale arrives.
    event_queue.roundtrip(&mut ui).map_err(|e| e.to_string())?;
    ui.scale = ui.output_state.outputs().filter_map(|o| ui.output_state.info(&o)).map(|i| i.scale_factor).max().unwrap_or(1).max(1);
    ui.rebuild()?;
    ui.create_surface();

    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    while ui.done.is_none() && ui.error.is_none() {
        event_loop.dispatch(None, &mut ui).map_err(|e| e.to_string())?;
    }
    match ui.error {
        Some(e) => Err(e),
        None => Ok(ui.done.unwrap()),
    }
}

impl Ui {
    /// (Re)build the atlas and geometry for the current scale.
    fn rebuild(&mut self) -> Result<(), String> {
        let atlas = Atlas::new(&self.cfg.font, self.cfg.font_size, self.scale)?;
        self.geo = Some(layout(&self.cfg, atlas.cw, atlas.ch, self.scale as usize, self.menu.rows, self.cfg.mesg.is_some()));
        self.atlas = Some(atlas);
        Ok(())
    }

    fn logical_size(&self) -> (u32, u32) {
        let g = self.geo.unwrap();
        ((g.w / self.scale as usize) as u32, (g.h / self.scale as usize) as u32)
    }

    fn create_surface(&mut self) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("lifemenu"), None);
        // No anchor: the compositor centres it on the focused output.
        layer.set_anchor(Anchor::empty());
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        let (w, h) = self.logical_size();
        layer.set_size(w, h);
        layer.commit();
        self.layer = Some(layer);
    }

    fn finish(&mut self, d: Done) {
        self.done = Some(d);
    }

    fn act(&mut self, o: Outcome) {
        match o {
            Outcome::Stay => {}
            Outcome::Pick(i) => self.finish(Done::Picked(i)),
            Outcome::Text(t) => self.finish(Done::Text(t)),
        }
    }

    fn on_key(&mut self, ev: KeyEvent) {
        let ctrl = self.mods.ctrl;
        let m = &mut self.menu;
        match ev.keysym {
            Keysym::Escape => return self.finish(Done::Cancel),
            Keysym::c | Keysym::g if ctrl => return self.finish(Done::Cancel),
            Keysym::Return | Keysym::KP_Enter => {
                let o = m.enter();
                return self.act(o);
            }
            Keysym::Up | Keysym::ISO_Left_Tab => m.up(),
            Keysym::p | Keysym::k if ctrl => m.up(),
            Keysym::Down | Keysym::Tab => m.down(),
            Keysym::n | Keysym::j if ctrl => m.down(),
            Keysym::Page_Up => m.page(false),
            Keysym::Page_Down => m.page(true),
            Keysym::Home if ctrl => m.select(0),
            Keysym::End if ctrl => m.select(usize::MAX),
            Keysym::BackSpace if ctrl => m.delete_word(),
            Keysym::w if ctrl => m.delete_word(),
            Keysym::u if ctrl => m.clear(),
            Keysym::BackSpace => m.backspace(),
            _ => {
                if ctrl || self.mods.alt || self.mods.logo {
                    return; // a shortcut we don't have: don't type its letter
                }
                match ev.utf8 {
                    Some(t) => m.type_str(&t),
                    None => return,
                }
            }
        }
        self.paint();
    }

    /// The list row under buffer-pixel y, as a position in `shown`.
    fn row_at(&self, y: f64) -> Option<usize> {
        let g = self.geo?;
        let y = (y * self.scale as f64) as usize;
        let k = y.checked_sub(g.list_y)? / g.row_h;
        (k < self.menu.rows).then_some(self.menu.scroll + k).filter(|&p| p < self.menu.shown.len())
    }

    fn paint(&mut self) {
        if !self.configured {
            return;
        }
        let (Some(g), Some(atlas)) = (self.geo, self.atlas.as_mut()) else { return };
        let cfg = &self.cfg;
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
        let text_y = |row_top: usize| row_top + s; // centre the cell in its row
        let cols = (g.w - 2 * g.pad_x) / cw;

        // Prompt and input. A long query shows its tail, so the cursor is
        // always in view.
        let mut x = g.pad_x;
        let y = text_y(g.pad_y);
        let prompt: Vec<char> = cfg.prompt.chars().take(cols / 2).collect();
        for &c in &prompt {
            cv.glyph(atlas, x, y, c, cfg.prompt_color);
            x += cw;
        }
        // A password shows nothing, not even its length: only the cursor.
        let shown: Vec<char> = if cfg.password {
            Vec::new()
        } else {
            self.menu.query.chars().collect()
        };
        let room = cols.saturating_sub(prompt.len() + 1);
        for &c in &shown[shown.len().saturating_sub(room)..] {
            cv.glyph(atlas, x, y, c, cfg.input);
            x += cw;
        }
        cv.fill(x, y, (2 * s).max(1), ch, cfg.input); // cursor bar

        if let Some(msg) = &cfg.mesg {
            let y = text_y(g.pad_y + g.row_h);
            for (k, c) in msg.chars().take(cols).enumerate() {
                cv.glyph(atlas, g.pad_x + k * cw, y, c, cfg.text);
            }
        }

        for (k, (pos, idx)) in self.menu.visible().enumerate() {
            let top = g.list_y + k * g.row_h;
            let selected = pos == self.menu.sel;
            if selected {
                let inset = g.border + (cfg.pad_h * s) / 2;
                cv.fill(inset, top, g.w - 2 * inset, g.row_h, cfg.selection);
            }
            let (fg, hl) = if selected { (cfg.selection_text, cfg.selection_match) } else { (cfg.text, cfg.match_color) };
            let label = &self.labels[idx];
            let hits = matcher::score(&self.menu.query, label).map(|(_, p)| p).unwrap_or_default();
            let n = label.chars().count();
            for (j, c) in label.chars().enumerate().take(cols) {
                let c = if n > cols && j == cols - 1 { '…' } else { c };
                let col = if hits.contains(&j) { hl } else { fg };
                cv.glyph(atlas, g.pad_x + j * cw, text_y(top), c, col);
            }
        }

        let Some(layer) = &self.layer else { return };
        let pool = match &mut self.pool {
            Some(p) => p,
            None => match SlotPool::new(g.w * g.h * 4 * 2, &self.shm) {
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
        self.finish(Done::Cancel);
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        // We asked for an exact size and draw exactly that.
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
        if let Err(e) = self.rebuild() {
            self.error = Some(e);
            return;
        }
        self.pool = None;
        let (w, h) = self.logical_size();
        if let Some(l) = &self.layer {
            l.set_size(w, h);
        }
        self.paint();
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl KeyboardHandler for Ui {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {
        // Repeat is driven by the calloop callback given to
        // get_keyboard_with_repeat; handling this too would double-type on
        // compositors that send wl_keyboard v10 repeats.
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Ui {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let ours = |s: &wl_surface::WlSurface| self.layer.as_ref().is_some_and(|l| l.wl_surface() == s);
        let mut dirty = false;
        for e in events {
            if !ours(&e.surface) {
                continue;
            }
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let Some(p) = self.row_at(e.position.1) {
                        if p != self.menu.sel {
                            self.menu.select(p);
                            dirty = true;
                        }
                    }
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    if let Some(p) = self.row_at(e.position.1) {
                        return self.finish(Done::Picked(self.menu.shown[p]));
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = if vertical.discrete != 0 { vertical.discrete as isize } else { (vertical.absolute / 15.0) as isize };
                    if steps != 0 {
                        self.menu.scroll_by(steps);
                        dirty = true;
                    }
                }
                _ => {}
            }
        }
        if dirty {
            self.paint();
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
    fn layout_grows_with_rows_and_scale() {
        let cfg = Cfg::default();
        let one = layout(&cfg, 8, 16, 1, 3, false);
        assert_eq!(one.w, 40 * 8 + 2 * (16 + 1));
        assert_eq!(one.list_y, 12 + 1 + 18 + 6);
        assert_eq!(one.h, one.list_y + 3 * 18 + 13);
        let bare = layout(&cfg, 8, 16, 1, 0, false);
        assert!(bare.h < one.h && bare.list_y == 12 + 1 + 18, "no list: no gap");
        let two = layout(&cfg, 16, 32, 2, 3, true);
        assert_eq!((two.w % 2, two.h % 2), (0, 0), "buffer divides by the scale");
        assert_eq!(two.row_h, 36);
    }
}
