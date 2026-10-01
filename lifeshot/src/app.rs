// SPDX-License-Identifier: GPL-3.0-or-later
// The Wayland side: capture first, then a full-output overlay (wlr-layer-shell,
// Overlay layer, exclusive keyboard) that shows the frozen capture through a
// viewport and forwards pointer and key events to the Session. Everything
// interesting lives in session.rs; this file is plumbing — the same sctk +
// calloop shape as lifenote's app.rs.
//
// The overlay surface is created only after the capture, so it can never end up
// in the screenshot, and the buffer is the capture's own resolution with the
// viewport scaling it to the output's logical size (exact at any scale).

use crate::capture;
use crate::image::Image;
use crate::png;
use crate::session::{Key, Mods, Outcome, Palette, Session};
use crate::shapes::Fonts;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
            Connection, Dispatch, QueueHandle,
        },
        protocols::wp::{
            cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::io::Write;
use std::path::PathBuf;

const FONT: &str = "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf";

pub struct Args {
    pub output: Option<String>,
    pub save_dir: Option<PathBuf>,
    pub full: bool,
    /// Draw one frame, report, exit: lets a nested compositor verify the whole
    /// capture -> overlay -> paint path headlessly.
    pub selftest: bool,
    /// With `selftest`: also draw a box and run the real Save and Copy actions.
    pub selftest_actions: bool,
}

struct App {
    loop_handle: LoopHandle<'static, App>,
    qh: QueueHandle<App>,
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    layer_shell: LayerShell,
    viewporter: Option<WpViewporter>,
    cursor_manager: Option<CursorShapeManager>,
    cursor_device: Option<WpCursorShapeDeviceV1>,

    output_name: String,
    layer: Option<LayerSurface>,
    viewport: Option<WpViewport>,
    pool: Option<SlotPool>,
    /// The capture, held until the first configure tells us the logical size.
    capture: Option<Image>,
    session: Option<Session>,
    frame: Option<Image>,
    palette: Palette,
    full: bool,
    selftest: bool,
    selftest_actions: bool,
    save_dir: PathBuf,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    mods: Mods,
    logical: (f64, f64),
    done: Option<i32>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

pub fn default_save_dir() -> PathBuf {
    home().join("Pictures/Screenshots")
}

pub fn run(args: Args) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => return fail(&format!("cannot connect to Wayland: {e}")),
    };
    let name = args.output.clone().or_else(capture::focused_output_name);
    let grab = match capture::grab(&conn, name.as_deref()) {
        Ok(g) => g,
        Err(e) => return fail(&format!("capture failed: {e}")),
    };

    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => return fail(&format!("registry init failed: {e}")),
    };
    let qh: QueueHandle<App> = event_queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    let layer_shell = match LayerShell::bind(&globals, &qh) {
        Ok(l) => l,
        Err(e) => return fail(&format!("wlr-layer-shell not supported: {e}")),
    };
    let mut app = App {
        loop_handle: event_loop.handle(),
        qh: qh.clone(),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        layer_shell,
        viewporter: globals.bind(&qh, 1..=1, ()).ok(),
        cursor_manager: CursorShapeManager::bind(&globals, &qh).ok(),
        cursor_device: None,
        output_name: grab.output_name,
        layer: None,
        viewport: None,
        pool: None,
        capture: Some(grab.image),
        session: None,
        frame: None,
        palette: Palette::load(),
        full: args.full,
        selftest: args.selftest,
        selftest_actions: args.selftest_actions,
        save_dir: args.save_dir.unwrap_or_else(default_save_dir),
        keyboard: None,
        pointer: None,
        mods: Mods::default(),
        logical: (0.0, 0.0),
        done: None,
    };
    if app.viewporter.is_none() {
        return fail("wp_viewporter not supported (needed to scale the capture to the output)");
    }

    WaylandSource::new(conn, event_queue).insert(app.loop_handle.clone()).expect("wayland source");

    // Outputs need a roundtrip's worth of events before we can pick ours.
    for _ in 0..2 {
        if event_loop.dispatch(std::time::Duration::from_millis(50), &mut app).is_err() {
            return fail("event loop error");
        }
    }
    if !app.make_surface() {
        return fail(&format!("output {:?} is gone", app.output_name));
    }

    while app.done.is_none() {
        if event_loop.dispatch(None, &mut app).is_err() {
            return fail("event loop error");
        }
    }
    app.done.unwrap_or(0)
}

fn fail(msg: &str) -> i32 {
    eprintln!("lifeshot: {msg}");
    // Nothing is on screen to explain a failure, so also say it as a notification.
    let _ = std::process::Command::new("notify-send").args(["-a", "lifeshot", "lifeshot failed", msg]).status();
    1
}

impl App {
    fn make_surface(&mut self) -> bool {
        let output = self
            .output_state
            .outputs()
            .find(|o| self.output_state.info(o).and_then(|i| i.name).as_deref() == Some(self.output_name.as_str()));
        let Some(output) = output else { return false };
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("lifeshot"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1); // cover panels too
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_size(0, 0);
        layer.wl_surface().commit();
        self.layer = Some(layer);
        true
    }

    fn redraw(&mut self) {
        let (Some(layer), Some(session)) = (&self.layer, &mut self.session) else { return };
        let (w, h) = session.size();
        let frame = self.frame.get_or_insert_with(|| Image::new(w, h, 0xff00_0000));
        session.paint(frame);

        let pool = match &mut self.pool {
            Some(p) => p,
            None => match SlotPool::new(w * h * 4 * 2, &self.shm) {
                Ok(p) => self.pool.insert(p),
                Err(e) => {
                    eprintln!("lifeshot: shm pool failed: {e}");
                    return;
                }
            },
        };
        let (buffer, canvas) = match pool.create_buffer(w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("lifeshot: buffer alloc failed: {e}");
                return;
            }
        };
        // Image px are u32 ARGB (native endian), which is exactly Argb8888.
        for (dst, px) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(&frame.px) {
            *dst = px.to_ne_bytes();
        }
        let surface = layer.wl_surface();
        buffer.attach_to(surface).expect("attach");
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.commit();
    }

    /// Image-space position of a pointer event (logical px -> capture px).
    fn to_image(&self, pos: (f64, f64)) -> (i32, i32) {
        let Some(s) = &self.session else { return (0, 0) };
        map_pos(pos, self.logical, s.size())
    }

    fn handle(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Continue => {}
            Outcome::Quit => self.done = Some(0),
            Outcome::Copy | Outcome::Save => {
                let Some(img) = self.session.as_mut().and_then(|s| s.export()) else { return };
                let png = png::encode(&img);
                let code = if outcome == Outcome::Copy {
                    copy_to_clipboard(&png)
                } else {
                    save_file(&self.save_dir, &png)
                };
                self.done = Some(code);
            }
        }
    }

    fn on_key(&mut self, ev: KeyEvent) {
        let Some(key) = map_key(ev.keysym, ev.utf8.as_deref(), self.mods.ctrl) else { return };
        if let Some(s) = &mut self.session {
            let out = s.key(key, self.mods);
            self.handle(out);
        }
        self.redraw();
    }
}

/// Logical surface position -> capture pixel, clamped to the capture.
pub fn map_pos(pos: (f64, f64), logical: (f64, f64), image: (usize, usize)) -> (i32, i32) {
    let (lw, lh) = (logical.0.max(1.0), logical.1.max(1.0));
    let x = (pos.0 * image.0 as f64 / lw).round() as i32;
    let y = (pos.1 * image.1 as f64 / lh).round() as i32;
    (x.clamp(0, image.0 as i32), y.clamp(0, image.1 as i32))
}

/// Keysym + typed text -> our Key. With Ctrl held the text is a control
/// character, so the key is identified by its symbol; otherwise by what the
/// layout actually typed (so non-US layouts and shifted symbols just work).
pub fn map_key(sym: Keysym, utf8: Option<&str>, ctrl: bool) -> Option<Key> {
    Some(match sym {
        Keysym::Escape => Key::Esc,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::BackSpace => Key::Backspace,
        Keysym::Delete => Key::Delete,
        Keysym::Left => Key::Left,
        Keysym::Right => Key::Right,
        Keysym::Up => Key::Up,
        Keysym::Down => Key::Down,
        sym => {
            let c = if ctrl { sym.key_char() } else { utf8.and_then(|t| t.chars().next()).or_else(|| sym.key_char()) };
            match c {
                Some(c) if !c.is_control() => Key::Char(c),
                _ => return None,
            }
        }
    })
}

/// `wl-copy --type image/png`, fed the PNG on stdin. wl-copy forks a background
/// server to keep owning the clipboard; that server must not inherit our
/// stdout/stderr, or anything reading lifeshot's output through a pipe would
/// block until the clipboard changed. The wait on the foreground process is
/// bounded so a wedged wl-copy can't hang the screenshot either.
fn copy_to_clipboard(png: &[u8]) -> i32 {
    use std::process::Stdio;
    let child = std::process::Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return fail(&format!("wl-copy: {e} (install wl-clipboard)")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(png).is_err() {
            return fail("could not hand the image to wl-copy");
        }
    } // stdin closes here: that is wl-copy's end-of-data
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => break,
            Ok(Some(st)) => return fail(&format!("wl-copy failed ({st})")),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                return fail("wl-copy did not finish; clipboard not set");
            }
            Err(e) => return fail(&format!("wl-copy: {e}")),
        }
    }
    notify("Screenshot copied", &format!("{} KiB PNG on the clipboard", png.len() / 1024));
    0
}

/// `Screenshot from YYYY-MM-DD HH-MM-SS.png`, the same name niri's own
/// screenshots get, with `-1`, `-2`… if it exists.
pub fn file_name(now: &str, taken: impl Fn(&str) -> bool) -> String {
    let first = format!("Screenshot from {now}.png");
    if !taken(&first) {
        return first;
    }
    (1..).map(|n| format!("Screenshot from {now}-{n}.png")).find(|c| !taken(c)).unwrap()
}

fn local_stamp() -> String {
    // SAFETY: localtime_r fills our zeroed tm; time(NULL) is always valid.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn save_file(dir: &std::path::Path, png: &[u8]) -> i32 {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(&format!("cannot create {}: {e}", dir.display()));
    }
    let name = file_name(&local_stamp(), |n| dir.join(n).exists());
    let path = dir.join(name);
    match std::fs::write(&path, png) {
        Ok(()) => {
            println!("{}", path.display());
            notify("Screenshot saved", &path.display().to_string());
            0
        }
        Err(e) => fail(&format!("cannot write {}: {e}", path.display())),
    }
}

fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send").args(["-a", "lifeshot", title, body]).status();
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // The compositor took the output away: nothing sensible left to do.
        self.done = Some(1);
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        let (lw, lh) = c.new_size;
        if lw == 0 || lh == 0 {
            return;
        }
        self.logical = (lw as f64, lh as f64);
        if self.viewport.is_none() {
            if let Some(vp) = &self.viewporter {
                self.viewport = Some(vp.get_viewport(layer.wl_surface(), qh, ()));
            }
        }
        if let Some(vp) = &self.viewport {
            vp.set_destination(lw as i32, lh as i32);
        }
        if self.session.is_none() {
            if let Some(img) = self.capture.take() {
                let k = img.w as f32 / lw as f32;
                let mut s = Session::new(img, self.palette, Fonts::load(FONT), k);
                if self.full {
                    s.key(Key::Char('a'), Mods { ctrl: true, shift: false });
                }
                self.session = Some(s);
            }
        }
        self.redraw();
        if self.selftest && self.selftest_actions {
            if let Some(s) = &mut self.session {
                let none = Mods::default();
                s.key(Key::Char('a'), Mods { ctrl: true, shift: false });
                s.key(Key::Char('r'), none);
                s.pointer_down((100, 100), none);
                s.pointer_move((300, 220), none);
                s.pointer_up((300, 220), none);
            }
            let saved = self.session.as_mut().and_then(|s| s.export()).map(|i| (i.w, i.h));
            self.handle(Outcome::Save);
            let save_code = self.done.take();
            self.handle(Outcome::Copy);
            eprintln!("selftest actions: export {saved:?}, save exit {save_code:?}, copy exit {:?}", self.done);
            return;
        }
        if self.selftest {
            let (iw, ih) = self.session.as_ref().map(|s| s.size()).unwrap_or((0, 0));
            eprintln!("selftest ok: overlay configured at {lw}x{lh} logical, capture {iw}x{ih}, frame drawn");
            self.done = Some(0);
        }
    }
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let mut dirty = false;
        for e in events {
            if self.layer.as_ref().map(|l| l.wl_surface()) != Some(&e.surface) {
                continue;
            }
            let p = self.to_image(e.position);
            let mods = self.mods;
            match e.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(d) = &self.cursor_device {
                        d.set_shape(serial, Shape::Crosshair);
                    }
                }
                PointerEventKind::Motion { .. } => {
                    if let Some(s) = &mut self.session {
                        s.pointer_move(p, mods);
                        dirty = true;
                    }
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    if let Some(s) = &mut self.session {
                        let out = s.pointer_down(p, mods);
                        dirty = true;
                        self.handle(out);
                    }
                }
                PointerEventKind::Release { button: BTN_LEFT, .. } => {
                    if let Some(s) = &mut self.session {
                        s.pointer_up(p, mods);
                        dirty = true;
                    }
                }
                // Right click backs out one level, like Esc.
                PointerEventKind::Press { button: BTN_RIGHT, .. } => {
                    if let Some(s) = &mut self.session {
                        let out = s.key(Key::Esc, mods);
                        dirty = true;
                        self.handle(out);
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let dir = if vertical.discrete != 0 { -vertical.discrete.signum() } else { -(vertical.absolute.signum() as i32) };
                    if let (Some(s), true) = (&mut self.session, dir != 0) {
                        s.scroll(dir);
                        dirty = true;
                    }
                }
                _ => {}
            }
        }
        if dirty {
            self.redraw();
        }
    }
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, m: Modifiers, _: smithay_client_toolkit::seat::keyboard::RawModifiers, _: u32) {
        self.mods = Mods { ctrl: m.ctrl, shift: m.shift };
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            let repeat = Box::new(|app: &mut App, _: &wl_keyboard::WlKeyboard, ev: KeyEvent| app.on_key(ev));
            match self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), repeat) {
                Ok(k) => self.keyboard = Some(k),
                Err(e) => eprintln!("lifeshot: no keyboard: {e}"),
            }
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.cursor_device = self.cursor_manager.as_ref().map(|m| m.get_shape_device(&p, qh));
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

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// Protocol objects with no events of interest to us.
impl Dispatch<WpViewporter, ()> for App {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WpViewport, ()> for App {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_layer!(App);
delegate_registry!(App);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_follow_niris_pattern_and_never_overwrite() {
        let none = |_: &str| false;
        assert_eq!(file_name("2026-10-01 12-00-00", none), "Screenshot from 2026-10-01 12-00-00.png");
        let taken = |n: &str| n == "Screenshot from 2026-10-01 12-00-00.png" || n.ends_with("-1.png");
        assert_eq!(file_name("2026-10-01 12-00-00", taken), "Screenshot from 2026-10-01 12-00-00-2.png");
    }

    #[test]
    fn pointer_positions_scale_to_capture_pixels_and_clamp() {
        // Scale-2 output: 1536x960 logical, 3072x1920 capture.
        assert_eq!(map_pos((100.0, 50.0), (1536.0, 960.0), (3072, 1920)), (200, 100));
        // Fractional 1.5x: 2048x1280 logical, 3072x1920 capture.
        assert_eq!(map_pos((1024.0, 640.0), (2048.0, 1280.0), (3072, 1920)), (1536, 960));
        assert_eq!(map_pos((-5.0, 99999.0), (1536.0, 960.0), (3072, 1920)), (0, 1920));
        // A zero logical size is treated as 1x1: no division by zero, result still clamped.
        assert_eq!(map_pos((1.0, 1.0), (0.0, 0.0), (10, 10)), (10, 10));
    }

    #[test]
    fn keys_map_by_symbol_with_ctrl_and_by_typed_text_without() {
        assert_eq!(map_key(Keysym::Escape, None, false), Some(Key::Esc));
        assert_eq!(map_key(Keysym::KP_Enter, None, false), Some(Key::Enter));
        // Ctrl+Z arrives with a control character as its text; the symbol wins.
        assert_eq!(map_key(Keysym::z, Some("\u{1a}"), true), Some(Key::Char('z')));
        // Without Ctrl, what the layout typed wins (shifted symbol, other layout).
        assert_eq!(map_key(Keysym::bracketright, Some("]"), false), Some(Key::Char(']')));
        assert_eq!(map_key(Keysym::a, Some("ä"), false), Some(Key::Char('ä')));
        // Keys with no text and no symbol char (modifiers, F-keys) are ignored.
        assert_eq!(map_key(Keysym::Shift_L, None, false), None);
        assert_eq!(map_key(Keysym::F5, None, false), None);
    }

    #[test]
    fn stamp_has_the_expected_shape() {
        let s = local_stamp();
        assert_eq!(s.len(), 19, "{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
    }
}
