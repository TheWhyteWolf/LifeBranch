// SPDX-License-Identifier: GPL-3.0-or-later
// The GUI front-end: a software-rendered xdg-shell toplevel that mirrors the
// TUI's layout (category sidebar + field list) in the rice's own olive glyphs.
// It drives the exact same model::Model as the TUI — only rendering and input
// mapping differ.
//
// Rendering reuses a self-contained copy of lifenote's fontdue Atlas (gui::
// render) rather than a shared crate: extracting a life-common crate would mean
// refactoring four already-working crates into a workspace for no user-facing
// gain, so that cleanup is deferred (see README milestones).

mod render;

use crate::model::{field_labels, kind, Focus, Kind, Model, CATS};
use crate::paths::Paths;
use crate::theme::{rgb, Theme};
use render::Atlas;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::xdg::{
        window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        XdgShell,
    },
    shell::WaylandSurface,
    shm::{slot::SlotPool, Shm, ShmHandler},
};

const FONT: &str = "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf";
const FONT_PX: f32 = 16.0;
const SIDEBAR_CELLS: usize = 16;

pub fn run(paths: Paths, theme: Theme) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifeconf: cannot connect to Wayland: {e} (no GUI here — try the terminal for the TUI)");
            return 1;
        }
    };
    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("lifeconf: registry init failed: {e}");
            return 1;
        }
    };
    let qh: QueueHandle<Gui> = event_queue.handle();
    let mut event_loop: EventLoop<Gui> = EventLoop::try_new().expect("event loop");

    let atlas = match Atlas::new(FONT, FONT_PX) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifeconf: {e}");
            return 1;
        }
    };
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg = match XdgShell::bind(&globals, &qh) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("lifeconf: xdg-shell not available: {e}");
            return 1;
        }
    };
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");

    let wl_surface = compositor.create_surface(&qh);
    let window = xdg.create_window(wl_surface, WindowDecorations::RequestServer, &qh);
    window.set_title("lifeconf — rice settings");
    window.set_app_id("lifeconf");
    window.set_min_size(Some((520, 360)));
    window.commit();

    let cell_w = atlas.cell_w();
    let cell_h = atlas.cell_h();
    let mut gui = Gui {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        shm,
        qh: qh.clone(),
        window,
        pool: None,
        width: 760,
        height: 520,
        configured: false,
        atlas,
        cell_w,
        cell_h,
        m: Model::new(paths, theme),
        keyboard: None,
        pointer: None,
        ctrl: false,
        pointer_pos: (0.0, 0.0),
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).expect("wayland source");

    while !gui.m.quit {
        if event_loop.dispatch(None, &mut gui).is_err() {
            eprintln!("lifeconf: event loop error");
            return 1;
        }
    }
    0
}

struct Gui {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    qh: QueueHandle<Self>,
    window: Window,
    pool: Option<SlotPool>,
    width: u32,
    height: u32,
    configured: bool,
    atlas: Atlas,
    cell_w: usize,
    cell_h: usize,
    m: Model,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    ctrl: bool,
    pointer_pos: (f64, f64),
}

// --- geometry (free fns so the offline renderer shares the exact layout) ---
fn margin(cell_h: usize) -> usize {
    cell_h
}
fn sidebar_w(cell_w: usize, cell_h: usize) -> usize {
    margin(cell_h) + SIDEBAR_CELLS * cell_w
}
fn row_y(cell_h: usize, i: usize) -> usize {
    margin(cell_h) + cell_h + i * (cell_h + 4)
}
/// The Save button's rect (x, y, w, h) — bottom-right corner.
fn save_btn_rect(w: usize, h: usize, cell_w: usize, cell_h: usize) -> (usize, usize, usize, usize) {
    let bw = 8 * cell_w;
    let bh = cell_h + 8;
    (w.saturating_sub(bw + cell_w), h.saturating_sub(bh + 4), bw, bh)
}

/// Paint the whole scene into an ARGB canvas. Free function so both the live
/// window and the offline PPM test render identically.
fn paint_scene(
    m: &Model,
    atlas: &mut Atlas,
    px: &mut [u32],
    stride: usize,
    h: usize,
    cell_w: usize,
    cell_h: usize,
) {
    let p = &m.theme.palette;
    let bg = rgb(&p.bg).unwrap_or((18, 20, 18));
    let surface = rgb(&p.surface).unwrap_or((23, 26, 20));
    let border = rgb(&p.border).unwrap_or((57, 65, 43));
    let text = rgb(&p.text).unwrap_or((123, 140, 90));
    let accent = rgb(&p.accent).unwrap_or((164, 201, 75));
    let warn = rgb(&p.warn).unwrap_or((199, 209, 122));

    render::fill_rect(px, stride, h, 0, 0, stride, h, bg);
    let side = sidebar_w(cell_w, cell_h);
    render::fill_rect(px, stride, h, 0, 0, side, h, surface);
    render::fill_rect(px, stride, h, side, 0, 1, h, border);

    let mgn = margin(cell_h);
    let cats_active = m.focus == Focus::Cats;
    atlas.draw_str(px, stride, h, mgn, mgn - cell_h / 2, "lifeconf", accent);

    for (i, cat) in CATS.iter().enumerate() {
        let y = row_y(cell_h, i);
        let selected = i == m.cat;
        if selected {
            let col = if cats_active { accent } else { border };
            render::fill_rect(px, stride, h, mgn - 4, y.saturating_sub(2), side - mgn - 2, cell_h + 3, col);
        }
        let fg = if selected { bg } else { text };
        atlas.draw_str(px, stride, h, mgn, y, cat, fg);
    }

    let fields_active = m.focus == Focus::Fields;
    let fx = side + cell_w * 2;
    atlas.draw_str(px, stride, h, fx, mgn - cell_h / 2, CATS[m.cat], accent);
    let labels = field_labels(m.cat);
    for (i, label) in labels.iter().enumerate() {
        let y = row_y(cell_h, i);
        let selected = fields_active && i == m.field;
        if selected {
            render::fill_rect(px, stride, h, fx - 4, y.saturating_sub(2), stride - fx, cell_h + 3, border);
        }
        atlas.draw_str(px, stride, h, fx, y, label, text);
        let vx = fx + 24 * cell_w;
        let editing_here = selected && m.editing.is_some();
        let val = if editing_here {
            format!("{}_", m.editing.as_deref().unwrap_or(""))
        } else {
            m.value(m.cat, i)
        };
        let vcol = if selected { accent } else { text };
        let after = atlas.draw_str(px, stride, h, vx, y, &val, vcol);
        if matches!(kind(m.cat, i), Kind::Hex) {
            if let Some(c) = rgb(&m.value(m.cat, i)) {
                render::fill_rect(px, stride, h, after + cell_w, y, cell_h, cell_h, c);
            }
        }
    }

    let sy = h.saturating_sub(cell_h + 4);
    let scol = if m.dirty { warn } else { text };
    let status = if m.dirty {
        format!("{}   ● unsaved", m.status)
    } else {
        m.status.clone()
    };
    atlas.draw_str(px, stride, h, mgn, sy, &status, scol);

    // Save button, bottom-right — lit accent while there are unsaved changes.
    // Clicking it commits (and pops the polkit password prompt if the greeter
    // needs updating).
    let (bx, by, bw, bh) = save_btn_rect(stride, h, cell_w, cell_h);
    let (btn_bg, btn_fg) = if m.dirty { (accent, bg) } else { (surface, text) };
    render::fill_rect(px, stride, h, bx, by, bw, bh, btn_bg);
    if !m.dirty {
        // Thin outline so the idle button still reads as a control.
        render::fill_rect(px, stride, h, bx, by, bw, 1, border);
        render::fill_rect(px, stride, h, bx, by + bh - 1, bw, 1, border);
        render::fill_rect(px, stride, h, bx, by, 1, bh, border);
        render::fill_rect(px, stride, h, bx + bw - 1, by, 1, bh, border);
    }
    let label = "Save";
    let lx = bx + (bw.saturating_sub(label.len() * cell_w)) / 2;
    let ly = by + (bh.saturating_sub(cell_h)) / 2;
    atlas.draw_str(px, stride, h, lx, ly, label, btn_fg);
}

/// Debug-only: render one frame of the default scene to a PPM, no Wayland —
/// the privacy-safe visual test for the GUI paint path.
#[cfg(debug_assertions)]
pub fn render_ppm(paths: Paths, theme: Theme, out: &str) -> i32 {
    let mut atlas = match Atlas::new(FONT, FONT_PX) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifeconf: {e}");
            return 1;
        }
    };
    let (cell_w, cell_h) = (atlas.cell_w(), atlas.cell_h());
    let (w, h) = (760usize, 520usize);
    let mut px = vec![0u32; w * h];
    let mut m = Model::new(paths, theme);
    // Optional: preview a specific category (LIFECONF_PPM_CAT=Palette) with the
    // fields pane focused, so the offline test can exercise swatches/selection.
    if let Ok(name) = std::env::var("LIFECONF_PPM_CAT") {
        if let Some(i) = crate::model::CATS.iter().position(|c| *c == name) {
            m.cat = i;
            m.focus = crate::model::Focus::Fields;
            m.field = 0;
        }
    }
    paint_scene(&m, &mut atlas, &mut px, w, h, cell_w, cell_h);

    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for pixel in &px {
        ppm.push((pixel >> 16) as u8);
        ppm.push((pixel >> 8) as u8);
        ppm.push(*pixel as u8);
    }
    if let Err(e) = std::fs::write(out, ppm) {
        eprintln!("lifeconf: write {out}: {e}");
        return 1;
    }
    eprintln!("wrote {out} ({w}x{h})");
    0
}

impl Gui {
    fn sidebar_w(&self) -> usize {
        sidebar_w(self.cell_w, self.cell_h)
    }
    fn row_y(&self, i: usize) -> usize {
        row_y(self.cell_h, i)
    }

    fn draw(&mut self) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = w;
        if self.pool.is_none() {
            match SlotPool::new(w * h * 4, &self.shm) {
                Ok(p) => self.pool = Some(p),
                Err(e) => {
                    eprintln!("lifeconf: shm pool: {e}");
                    return;
                }
            }
        }
        let mut pool = self.pool.take().unwrap();
        let (buffer, canvas) =
            match pool.create_buffer(w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("lifeconf: buffer: {e}");
                    self.pool = Some(pool);
                    return;
                }
            };
        // Reinterpret the &mut [u8] canvas as &mut [u32] for pixel writes.
        let px: &mut [u32] = bytemuck_cast(canvas);
        self.paint(px, stride, h);
        let surface = self.window.wl_surface();
        buffer.attach_to(surface).expect("attach");
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.commit();
        self.pool = Some(pool);
        let _ = buffer; // ownership held by pool
    }

    fn paint(&mut self, px: &mut [u32], stride: usize, h: usize) {
        paint_scene(&self.m, &mut self.atlas, px, stride, h, self.cell_w, self.cell_h);
    }

    fn redraw(&mut self) {
        self.window.wl_surface().frame(&self.qh, self.window.wl_surface().clone());
        self.draw();
    }

    /// Pointer click: the Save button, else the category/field row under (x,y).
    fn click(&mut self, x: f64, y: f64) {
        let (bx, by, bw, bh) =
            save_btn_rect(self.width as usize, self.height as usize, self.cell_w, self.cell_h);
        if x >= bx as f64 && x < (bx + bw) as f64 && y >= by as f64 && y < (by + bh) as f64 {
            self.m.commit();
            self.redraw();
            return;
        }

        let side = self.sidebar_w() as f64;
        // Which row was hit?
        let row_of = |gui: &Gui, py: f64, n: usize| -> Option<usize> {
            (0..n).find(|&i| {
                let ry = gui.row_y(i) as f64;
                py >= ry - 2.0 && py < ry + gui.cell_h as f64 + 1.0
            })
        };
        if x < side {
            if let Some(i) = row_of(self, y, CATS.len()) {
                self.m.cat = i;
                self.m.field = 0;
                self.m.focus = Focus::Cats;
            }
        } else if let Some(i) = row_of(self, y, self.m.n_fields()) {
            self.m.field = i;
            self.m.focus = Focus::Fields;
            // A click into the fields acts like Enter: edit or cycle.
            match kind(self.m.cat, i) {
                Kind::Hex | Kind::Text | Kind::Float(_) | Kind::Int(_) => self.m.begin_edit(),
                _ => self.m.nudge(1),
            }
        }
        self.redraw();
    }

    fn on_key(&mut self, ev: KeyEvent) {
        let sym = ev.keysym;
        // Editing mode.
        if self.m.editing.is_some() {
            match sym {
                Keysym::Return | Keysym::KP_Enter => {
                    let s = self.m.editing.take().unwrap();
                    self.m.set_text(&s);
                }
                Keysym::Escape => {
                    self.m.editing = None;
                    self.m.status = "edit cancelled".into();
                }
                Keysym::BackSpace => {
                    if let Some(b) = self.m.editing.as_mut() {
                        b.pop();
                    }
                }
                _ => {
                    if let Some(t) = ev.utf8 {
                        if let Some(b) = self.m.editing.as_mut() {
                            b.push_str(&t);
                        }
                    }
                }
            }
            self.redraw();
            return;
        }

        if self.ctrl && sym == Keysym::c {
            if self.m.dirty {
                self.m.revert();
            }
            self.m.quit = true;
            return;
        }

        match sym {
            Keysym::q => {
                if self.m.dirty {
                    self.m.commit();
                }
                self.m.quit = true;
                return;
            }
            Keysym::s => self.m.commit(),
            Keysym::Tab => self.m.toggle_focus(),
            Keysym::Down | Keysym::j => self.m.move_down(),
            Keysym::Up | Keysym::k => self.m.move_up(),
            Keysym::Right | Keysym::l => {
                if self.m.focus == Focus::Cats {
                    self.m.focus = Focus::Fields;
                } else {
                    self.m.nudge(1);
                }
            }
            Keysym::Left | Keysym::h => {
                if self.m.focus == Focus::Fields {
                    match self.m.kind_here() {
                        Kind::Hex | Kind::Text => self.m.focus = Focus::Cats,
                        _ => self.m.nudge(-1),
                    }
                }
            }
            Keysym::plus | Keysym::equal => {
                if self.m.focus == Focus::Fields {
                    self.m.nudge(1);
                }
            }
            Keysym::minus | Keysym::underscore => {
                if self.m.focus == Focus::Fields {
                    self.m.nudge(-1);
                }
            }
            Keysym::Return | Keysym::KP_Enter | Keysym::space => match self.m.focus {
                Focus::Cats => self.m.focus = Focus::Fields,
                Focus::Fields => match self.m.kind_here() {
                    Kind::Hex | Kind::Text | Kind::Float(_) | Kind::Int(_) => self.m.begin_edit(),
                    _ => self.m.nudge(1),
                },
            },
            _ => {}
        }
        self.redraw();
    }
}

/// Reinterpret a byte canvas as u32 pixels (ARGB8888, native endian). The shm
/// buffer is 4-byte aligned and sized w*h*4, so this is sound.
fn bytemuck_cast(bytes: &mut [u8]) -> &mut [u32] {
    let len = bytes.len() / 4;
    unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u32, len) }
}

impl WindowHandler for Gui {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.m.quit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let Some((w, h)) = configure.new_size.0.zip(configure.new_size.1) {
            self.width = w.get();
            self.height = h.get();
            self.pool = None;
        }
        self.configured = true;
        self.draw();
    }
}

impl CompositorHandler for Gui {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl KeyboardHandler for Gui {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        // Held keys repeat — useful for arrow navigation and +/- stepping.
        self.on_key(event);
    }
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.ctrl = modifiers.ctrl;
    }
}

impl PointerHandler for Gui {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            if e.surface != *self.window.wl_surface() {
                continue;
            }
            match e.kind {
                PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                    self.pointer_pos = e.position;
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    let (x, y) = e.position;
                    self.click(x, y);
                }
                _ => {}
            }
        }
    }
}

impl SeatHandler for Gui {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kb);
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

impl OutputHandler for Gui {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Gui {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Gui {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_compositor!(Gui);
smithay_client_toolkit::delegate_output!(Gui);
smithay_client_toolkit::delegate_shm!(Gui);
smithay_client_toolkit::delegate_seat!(Gui);
smithay_client_toolkit::delegate_keyboard!(Gui);
smithay_client_toolkit::delegate_pointer!(Gui);
smithay_client_toolkit::delegate_xdg_shell!(Gui);
smithay_client_toolkit::delegate_xdg_window!(Gui);
smithay_client_toolkit::delegate_registry!(Gui);
