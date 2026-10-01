// SPDX-License-Identifier: GPL-3.0-or-later
// The flash: a layer-shell surface on the overlay layer, bottom centre, that
// takes no keyboard and lets clicks through (empty input region). It exists
// only while showing; hiding destroys the surface and frees its buffers.

use crate::cli::{Cfg, Rgba};
use crate::msg::{self, Msg, Role, COLS};
use crate::render::{Atlas, Canvas};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            generic::Generic,
            timer::{TimeoutAction, Timer},
            EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

/// How long a flash stays after the last key press (wob's was 900 ms too).
const SHOW_FOR: Duration = Duration::from_millis(900);
/// Gap above the screen's bottom edge, logical pixels.
const MARGIN: i32 = 96;

struct Ui {
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    shm: Shm,
    layer_shell: LayerShell,
    qh: QueueHandle<Ui>,
    handle: LoopHandle<'static, Ui>,

    cfg: Cfg,
    atlas: Option<(i32, Atlas)>,
    scale: i32,
    layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    configured: bool,
    showing: Option<Msg>,
    hide: Option<RegistrationToken>,
    error: Option<String>,
}

pub fn run(cfg: Cfg, fifo: OwnedFd) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, event_queue) = registry_queue_init(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh: QueueHandle<Ui> = event_queue.handle();
    let mut event_loop: EventLoop<Ui> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let mut ui = Ui {
        compositor: CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?,
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        shm: Shm::bind(&globals, &qh).map_err(|e| e.to_string())?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|e| format!("wlr-layer-shell not supported: {e}"))?,
        qh,
        handle: event_loop.handle(),
        cfg,
        atlas: None,
        scale: 1,
        layer: None,
        pool: None,
        configured: false,
        showing: None,
        hide: None,
        error: None,
    };
    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(Generic::new(fifo, Interest::READ, Mode::Level), |_, fd, ui: &mut Ui| {
            let mut buf = [0u8; 4096];
            let mut chunk = String::new();
            loop {
                // SAFETY: read into our own buffer from the FIFO we own.
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                chunk.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            }
            if let Some(m) = msg::last(&chunk) {
                ui.show(m);
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.to_string())?;

    while ui.error.is_none() {
        event_loop.dispatch(None, &mut ui).map_err(|e| e.to_string())?;
    }
    Err(ui.error.unwrap())
}

impl Ui {
    fn show(&mut self, m: Msg) {
        self.showing = Some(m);
        if let Some(t) = self.hide.take() {
            self.handle.remove(t);
        }
        self.hide = self
            .handle
            .insert_source(Timer::from_duration(SHOW_FOR), |_, _, ui: &mut Ui| {
                ui.hide = None;
                ui.unmap();
                TimeoutAction::Drop
            })
            .ok();
        if self.layer.is_none() {
            self.map();
        } else {
            self.paint();
        }
    }

    /// Logical size at the current scale, building the font for it if needed.
    fn size(&mut self) -> Result<(u32, u32), String> {
        if self.atlas.as_ref().is_none_or(|(s, _)| *s != self.scale) {
            self.atlas = Some((self.scale, Atlas::new(&self.cfg.font, self.cfg.font_size, self.scale)?));
        }
        let (w, h) = self.buffer_size();
        Ok(((w / self.scale as usize) as u32, (h / self.scale as usize) as u32))
    }

    fn buffer_size(&self) -> (usize, usize) {
        let (_, a) = self.atlas.as_ref().expect("atlas");
        let s = self.scale as usize;
        let border = self.cfg.border_width * s;
        let up = |v: usize| v.div_ceil(s) * s;
        (up(COLS * a.cw + 2 * (self.cfg.pad_h * s + border)), up(a.ch + 2 * (self.cfg.pad_v * s + border)))
    }

    fn map(&mut self) {
        self.scale = self.output_state.outputs().filter_map(|o| self.output_state.info(&o)).map(|i| i.scale_factor).max().unwrap_or(1).max(1);
        let (w, h) = match self.size() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let surface = self.compositor.create_surface(&self.qh);
        // Clicks go through to whatever is underneath.
        if let Ok(r) = Region::new(&self.compositor) {
            surface.set_input_region(Some(r.wl_region()));
        }
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("lifeosd"), None);
        layer.set_anchor(Anchor::BOTTOM);
        layer.set_margin(0, 0, MARGIN, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.set_size(w, h);
        layer.commit();
        self.configured = false;
        self.layer = Some(layer);
    }

    fn unmap(&mut self) {
        self.layer = None;
        self.pool = None;
        self.showing = None;
        self.configured = false;
    }

    fn color(&self, r: Role) -> Rgba {
        let c = &self.cfg;
        let muted = self.showing.as_ref().is_some_and(|m| m.muted);
        match r {
            Role::Label => c.text,
            Role::Bar => c.prompt_color,
            Role::Track => [c.text[0], c.text[1], c.text[2], (c.text[3] as u32 * 4 / 10) as u8],
            Role::Value if muted => c.prompt_color,
            Role::Value => c.input,
        }
    }

    fn paint(&mut self) {
        if !self.configured {
            return;
        }
        let (Some(m), Some(layer)) = (self.showing.clone(), self.layer.as_ref()) else { return };
        let (w, h) = self.buffer_size();
        let runs: Vec<(String, Rgba)> = msg::line(&m).into_iter().map(|(t, r)| (t, self.color(r))).collect();
        let cfg = &self.cfg;
        let s = self.scale as usize;
        let border = cfg.border_width * s;
        let pool = match &mut self.pool {
            Some(p) => p,
            None => match SlotPool::new(w * h * 4 * 2, &self.shm) {
                Ok(p) => self.pool.insert(p),
                Err(e) => {
                    self.error = Some(format!("shm pool: {e}"));
                    return;
                }
            },
        };
        let Ok((buffer, bytes)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else { return };
        let mut px = vec![0u32; w * h];
        let (_, atlas) = self.atlas.as_mut().expect("atlas");
        let mut cv = Canvas { px: &mut px, w, h };
        cv.fill(0, 0, w, h, cfg.background);
        if border > 0 {
            cv.fill(0, 0, w, border, cfg.border);
            cv.fill(0, h - border, w, border, cfg.border);
            cv.fill(0, 0, border, h, cfg.border);
            cv.fill(w - border, 0, border, h, cfg.border);
        }
        let mut x = cfg.pad_h * s + border;
        let y = cfg.pad_v * s + border;
        for (text, col) in &runs {
            for c in text.chars() {
                cv.glyph(atlas, x, y, c, *col);
                x += atlas.cw;
            }
        }
        for (dst, p) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&px) {
            *dst = p.to_le_bytes();
        }
        let surface = layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, w as i32, h as i32);
            surface.commit();
        }
    }
}

impl LayerShellHandler for Ui {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.unmap();
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
        match self.size() {
            Ok((w, h)) => {
                self.pool = None;
                if let Some(l) = &self.layer {
                    l.set_size(w, h);
                    l.commit();
                }
                self.configured = false;
            }
            Err(e) => self.error = Some(e),
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
    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_compositor!(Ui);
smithay_client_toolkit::delegate_output!(Ui);
smithay_client_toolkit::delegate_shm!(Ui);
smithay_client_toolkit::delegate_layer!(Ui);
smithay_client_toolkit::delegate_registry!(Ui);
