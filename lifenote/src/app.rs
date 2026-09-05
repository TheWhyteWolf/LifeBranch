// SPDX-License-Identifier: GPL-3.0-or-later
// Wayland layer-shell application: one top-anchored surface holding the
// whole popup stack. Forked from ../lifelock/src/app.rs (the sctk + calloop
// template all life* tools share), with the session-lock machinery replaced
// by wlr-layer-shell and the keyboard replaced by a pointer.
//
// Deliberate divergence from the animated siblings: there is NO fps timer
// and NO frame-callback watchdog. A notification daemon must idle at zero
// cost — the surface only exists while popups are visible, repaints happen
// only on state changes, and the event loop blocks until a Wayland event,
// an expiry timer, or a bus message (calloop channel) wakes it.

use crate::cli::{AnchorCorner, Cfg, LayerChoice};
use crate::render::{frame, Atlas};
use crate::state::{
    compose_note, effective_timeout, AppEvent, History, Queue, REASON_DISMISSED, REASON_EXPIRED,
    REASON_CLOSED,
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            channel::{Channel, Event as ChannelEvent},
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
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
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use std::time::Duration;

/// Vertical gap between stacked popups, px (mako's margin=8 look).
const INTER_GAP: usize = 8;

struct Popup {
    layer: LayerSurface,
    width: u32,
    height: u32,
    pool: Option<SlotPool>,
    configured: bool,
}

/// Screen-space rect of one visible popup, for click hit-testing.
struct Band {
    id: u32,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

pub struct App {
    loop_handle: LoopHandle<'static, Self>,
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    layer_shell: LayerShell,
    qh: QueueHandle<Self>,

    cfg: Cfg,
    atlas: Atlas,
    queue: Queue,
    dnd: bool,
    surface: Option<Popup>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_pos: (f64, f64),
    bands: Vec<Band>,
    timer_gen: u64,
    /// Called with (id, close reason); M3 points this at the bus's
    /// NotificationClosed signal. Harnesses pass a logger.
    on_closed: Box<dyn FnMut(u32, u32)>,
    exit: bool,
}

/// Run the daemon UI. `events` is fed by the bus thread (or a harness);
/// `on_closed` reports every popup close back to it; `history` is the ring
/// shared with `lifenote ctl history`.
pub fn run(
    cfg: Cfg,
    events: Channel<AppEvent>,
    on_closed: Box<dyn FnMut(u32, u32)>,
    history: History,
) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifenote: cannot connect to Wayland: {e}");
            return 1;
        }
    };
    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("lifenote: registry init failed: {e}");
            return 1;
        }
    };
    let qh: QueueHandle<App> = event_queue.handle();
    let mut event_loop: EventLoop<App> =
        EventLoop::try_new().expect("failed to create event loop");

    let atlas = match Atlas::new(&cfg.font, cfg.font_px) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifenote: {e}");
            return 1;
        }
    };
    let layer_shell = match LayerShell::bind(&globals, &qh) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("lifenote: wlr-layer-shell not supported: {e}");
            return 1;
        }
    };

    let mut app = App {
        loop_handle: event_loop.handle(),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        layer_shell,
        qh: qh.clone(),
        cfg,
        atlas,
        queue: Queue::new(history),
        dnd: false,
        surface: None,
        pointer: None,
        pointer_pos: (0.0, 0.0),
        bands: Vec::new(),
        timer_gen: 0,
        on_closed,
        exit: false,
    };

    app.loop_handle
        .insert_source(events, |ev, _, app: &mut App| match ev {
            ChannelEvent::Msg(e) => app.on_event(e),
            ChannelEvent::Closed => {
                // The bus thread is gone; a notification daemon that can't
                // receive notifications should die visibly, not linger.
                eprintln!("lifenote: event channel closed — exiting");
                app.exit = true;
            }
        })
        .expect("insert event channel");

    WaylandSource::new(conn, event_queue)
        .insert(app.loop_handle.clone())
        .expect("insert wayland source");

    while !app.exit {
        // Block until something happens — no idle wakeups at all.
        if event_loop.dispatch(None, &mut app).is_err() {
            eprintln!("lifenote: event loop error");
            return 1;
        }
    }
    0
}

impl App {
    fn on_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Notify(d) => {
                if self.dnd {
                    // Swallowed silently, but it still lands in history
                    // (mako's [mode=do-not-disturb] invisible=1 behaviour).
                    self.queue.remember(&d, false);
                    return;
                }
                let id = d.id;
                let timeout = effective_timeout(&self.cfg, &d);
                let grid = compose_note(&self.cfg, &d);
                let replaced = self.queue.upsert(d, grid);
                // A replaced on-screen popup restarts its clock (notify-send
                // -r updating a progress popup); everything else is armed by
                // sync() when — and only when — it actually shows.
                if replaced
                    && self
                        .queue
                        .visible(self.cfg.max_visible)
                        .iter()
                        .any(|n| n.data.id == id)
                {
                    self.arm_expiry(id, timeout);
                }
                self.sync();
            }
            AppEvent::Close(id) => {
                if self.queue.remove(id, false) {
                    (self.on_closed)(id, REASON_CLOSED);
                    self.sync();
                }
            }
            AppEvent::DndChanged(on) => self.dnd = on,
            AppEvent::Reload => self.reload_config(),
            AppEvent::DismissAll => {
                let ids: Vec<u32> = self.queue.notes.iter().map(|n| n.data.id).collect();
                for id in ids {
                    self.queue.remove(id, true);
                    (self.on_closed)(id, REASON_DISMISSED);
                }
                self.sync();
            }
        }
    }

    /// Re-read the config (file + the daemon's own flags) and restyle every
    /// queued popup live. The font atlas is rebuilt only when the font actually
    /// changed; the surface is dropped so a new anchor/layer/margin takes hold on
    /// the next sync().
    fn reload_config(&mut self) {
        let new = crate::cli::parse();
        let font_changed =
            new.font != self.cfg.font || (new.font_px - self.cfg.font_px).abs() > f32::EPSILON;
        if font_changed {
            match Atlas::new(&new.font, new.font_px) {
                Ok(a) => self.atlas = a,
                Err(e) => eprintln!("lifenote: reload: keeping current font ({e})"),
            }
        }
        self.cfg = new;
        for n in &mut self.queue.notes {
            n.grid = compose_note(&self.cfg, &n.data);
        }
        // Rebuild the surface so anchor/layer/margin changes take effect.
        self.surface = None;
        self.bands.clear();
        self.sync();
    }

    /// One-shot expiry timer, generation-keyed (lifelock's arm_password_clear
    /// pattern): a replace re-arms and the stale timer no-ops.
    fn arm_expiry(&mut self, id: u32, timeout: Option<u64>) {
        self.timer_gen += 1;
        let gen = self.timer_gen;
        if let Some(n) = self.queue.notes.iter_mut().find(|n| n.data.id == id) {
            n.deadline_gen = gen;
        }
        let Some(ms) = timeout else { return };
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            move |_, _, app: &mut App| {
                let live = app
                    .queue
                    .notes
                    .iter()
                    .any(|n| n.data.id == id && n.deadline_gen == gen);
                if live {
                    app.queue.remove(id, false);
                    (app.on_closed)(id, REASON_EXPIRED);
                    app.sync();
                }
                TimeoutAction::Drop
            },
        );
    }

    /// Expiry follows visibility: arm timers for notes surfacing on screen,
    /// stale-key (gen 0) any displaced off it. A hidden note must not burn
    /// its timeout — a burst beyond max-visible would otherwise expire
    /// popups that were never really shown.
    fn sync_expiries(&mut self) {
        let max = self.cfg.max_visible;
        let mut arm: Vec<(u32, Option<u64>)> = Vec::new();
        for (i, n) in self.queue.notes.iter_mut().enumerate() {
            if i < max && n.deadline_gen == 0 {
                arm.push((n.data.id, effective_timeout(&self.cfg, &n.data)));
            } else if i >= max && n.deadline_gen != 0 {
                n.deadline_gen = 0; // off-screen: the pending timer goes stale
            }
        }
        for (id, timeout) in arm {
            self.arm_expiry(id, timeout);
        }
    }

    /// Reconcile the surface with the queue: create/resize/destroy, then
    /// paint. The surface exists only while popups are visible.
    fn sync(&mut self) {
        self.sync_expiries();
        let visible = self.queue.visible(self.cfg.max_visible);
        if visible.is_empty() {
            // Dropping the LayerSurface destroys the role and the wl_surface.
            self.surface = None;
            self.bands.clear();
            return;
        }

        let width = visible
            .iter()
            .map(|n| frame::popup_width(&n.grid, &self.atlas))
            .max()
            .unwrap_or(0) as u32;
        let height = (visible
            .iter()
            .map(|n| frame::popup_height(&n.grid, &self.atlas))
            .sum::<usize>()
            + INTER_GAP * (visible.len() - 1)) as u32;

        match &mut self.surface {
            None => {
                let anchor = match self.cfg.anchor {
                    AnchorCorner::TopRight => Anchor::TOP | Anchor::RIGHT,
                    AnchorCorner::TopLeft => Anchor::TOP | Anchor::LEFT,
                    AnchorCorner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
                    AnchorCorner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
                };
                let wl_surface = self.compositor.create_surface(&self.qh);
                let layer = self.layer_shell.create_layer_surface(
                    &self.qh,
                    wl_surface,
                    match self.cfg.layer {
                        LayerChoice::Top => Layer::Top,
                        LayerChoice::Overlay => Layer::Overlay,
                    },
                    Some("lifenote"),
                    None, // compositor picks the output
                );
                layer.set_anchor(anchor);
                let m = self.cfg.margin;
                layer.set_margin(m, m, m, m);
                layer.set_exclusive_zone(0);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.set_size(width, height);
                layer.wl_surface().commit();
                // Paint happens on the first configure.
                self.surface = Some(Popup {
                    layer,
                    width,
                    height,
                    pool: None,
                    configured: false,
                });
            }
            Some(popup) => {
                if (popup.width, popup.height) != (width, height) {
                    popup.width = width;
                    popup.height = height;
                    popup.pool = None;
                    popup.configured = false; // repaint on the next configure
                    popup.layer.set_size(width, height);
                    popup.layer.wl_surface().commit();
                } else {
                    self.paint();
                }
            }
        }
    }

    fn paint(&mut self) {
        let Some(popup) = &mut self.surface else { return };
        if !popup.configured || popup.width == 0 || popup.height == 0 {
            return;
        }
        let (w, h) = (popup.width as usize, popup.height as usize);
        let stride = w as i32 * 4;
        if popup.pool.is_none() {
            match SlotPool::new(w * h * 4, &self.shm) {
                Ok(p) => popup.pool = Some(p),
                Err(e) => {
                    eprintln!("lifenote: shm pool failed: {e}");
                    return;
                }
            }
        }
        // Take the pool out so atlas (mut) + queue (shared) can be borrowed
        // alongside it (lifelock's paint() does the same dance).
        let mut pool = popup.pool.take().unwrap();
        let result = pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888);
        match result {
            Ok((buffer, canvas)) => {
                canvas.fill(0); // fully transparent between/around popups
                let bg = frame::premul_bg(self.cfg.background, self.cfg.background_alpha);
                let right_aligned = matches!(
                    self.cfg.anchor,
                    AnchorCorner::TopRight | AnchorCorner::BottomRight
                );
                self.bands.clear();
                let mut y = 0usize;
                for n in self.queue.visible(self.cfg.max_visible) {
                    let pw = frame::popup_width(&n.grid, &self.atlas);
                    let ph = frame::popup_height(&n.grid, &self.atlas);
                    let x = if right_aligned { w.saturating_sub(pw) } else { 0 };
                    let pal = frame::palette(&self.cfg, n.data.urgency);
                    frame::draw_popup(canvas, w, h, x, y, &n.grid, &mut self.atlas, &pal, bg);
                    self.bands.push(Band {
                        id: n.data.id,
                        x0: x as f64,
                        y0: y as f64,
                        x1: (x + pw) as f64,
                        y1: (y + ph) as f64,
                    });
                    y += ph + INTER_GAP;
                }
                let popup = self.surface.as_ref().unwrap();
                let surface = popup.layer.wl_surface();
                buffer.attach_to(surface).expect("attach");
                // Without damage the compositor never repaints — the lesson
                // both siblings learned the hard way.
                surface.damage_buffer(0, 0, w as i32, h as i32);
                surface.commit();
            }
            Err(e) => eprintln!("lifenote: buffer alloc failed: {e}"),
        }
        if let Some(popup) = &mut self.surface {
            popup.pool = Some(pool);
        }
    }

    fn click(&mut self, x: f64, y: f64) {
        let hit = self
            .bands
            .iter()
            .find(|b| x >= b.x0 && x < b.x1 && y >= b.y0 && y < b.y1)
            .map(|b| b.id);
        if let Some(id) = hit {
            self.queue.remove(id, true);
            (self.on_closed)(id, REASON_DISMISSED);
            self.sync();
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _layer: &LayerSurface) {
        // Compositor closed us (output gone, etc.) — recreate if needed.
        self.surface = None;
        self.bands.clear();
        self.sync();
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(popup) = &mut self.surface {
            let (w, h) = configure.new_size;
            // 0 means "pick your own size" — keep what we requested.
            if w > 0 && w != popup.width {
                popup.width = w;
                popup.pool = None;
            }
            if h > 0 && h != popup.height {
                popup.height = h;
                popup.pool = None;
            }
            popup.configured = true;
        }
        self.paint();
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            let ours = self
                .surface
                .as_ref()
                .is_some_and(|p| p.layer.wl_surface() == &e.surface);
            if !ours {
                continue;
            }
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_pos = e.position;
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    self.pointer_pos = e.position;
                    let (x, y) = self.pointer_pos;
                    self.click(x, y);
                }
                _ => {}
            }
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        // We never request frame callbacks — nothing animates.
    }
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

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(p) => self.pointer = Some(p),
                Err(e) => eprintln!("lifenote: no pointer: {e}"),
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
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

smithay_client_toolkit::delegate_compositor!(App);
smithay_client_toolkit::delegate_output!(App);
smithay_client_toolkit::delegate_seat!(App);
smithay_client_toolkit::delegate_pointer!(App);
smithay_client_toolkit::delegate_shm!(App);
smithay_client_toolkit::delegate_layer!(App);
smithay_client_toolkit::delegate_registry!(App);
