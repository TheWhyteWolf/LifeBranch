// SPDX-License-Identifier: GPL-3.0-or-later
// Wayland greeter application: one xdg-toplevel (cage fullscreens it), the
// event loop, keyboard routing by greeter phase, the grow/collapse map
// selection, and the greetd IPC channel. Forked from ../lifelock/src/app.rs —
// the calloop/SlotPool/keyboard-repeat/frame-pacing structure is identical;
// ext-session-lock and the PAM child are replaced by xdg-shell and greetd.

use crate::cli::Cfg;
use crate::geometry::{build_grid_map, build_grid_map_scaled, GridMap, MIN_DRAW_SCALE};
use crate::input::{key_action, Action, PASSWORD_CLEAR_SECS};
use crate::ipc::{Ipc, IpcCmd, IpcEvent};
use crate::render::{render_frame, Anchors, Atlas, Overlay};
use crate::scene::Scene;
use crate::secure_buf::SecureBuf;
use crate::sessions::Sessions;
use crate::state::{Greeter, Phase, IDLE_COLLAPSE_SECS, USERNAME_MAX};

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
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

/// Size to draw at before the first real configure (dev runs in a floating
/// window; under cage the configure always carries the output size).
const FALLBACK_SIZE: (u32, u32) = (1280, 800);

/// Transient PAM info/error lines stay up this long.
const INFO_SECS: f64 = 6.0;

/// How long to wait for a frame callback before assuming it was lost and
/// repainting anyway. Callbacks vanish when the output power-cycles under us
/// (monitors that drop the connector waking from deep standby); without this
/// watchdog one lost callback freezes the greeter until reboot while input
/// keeps working invisibly. Same fix as lifelock's; short enough that key
/// flares (0.8 s) stay visible even while running purely on the watchdog.
const FRAME_WATCHDOG: Duration = Duration::from_millis(250);

pub struct App {
    loop_handle: LoopHandle<'static, Self>,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    qh: QueueHandle<Self>,

    window: Window,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    width: u32,
    height: u32,
    pool: Option<SlotPool>,
    frame_pending: bool,
    frame_pending_since: Option<Instant>,
    watchdog_tripped: bool,
    configured: bool,

    // Maps for the three animation regimes; anim_map is rebuilt per frame
    // only while actually growing/collapsing (<1ms, see geometry.rs).
    final_map: Option<GridMap>,
    empty_map: Option<GridMap>,
    anim_map: Option<GridMap>,
    anchors: Option<Anchors>,

    cfg: Cfg,
    start: Instant,
    scene: Scene,
    atlas: Atlas,
    greeter: Greeter,
    sessions: Sessions,
    password: SecureBuf,
    ipc: Ipc,

    /// (message, shown_at) — PAM Info/Error line under the clock.
    info: Option<(String, f64)>,
    /// Unrecoverable IPC failure: shown permanently, greeter stays up so the
    /// VT-switch escape hatch remains reachable.
    fatal: Option<String>,

    mods: Modifiers,
    last_key_t: f64,
    exit: bool,
    exit_code: i32,
    password_clear_gen: u64,
}

/// Run the greeter. `ipc_events` is the calloop side of the worker channel.
/// Returns the process exit code (0 = session started; greetd takes over).
pub fn run(cfg: Cfg, ipc: Ipc, ipc_events: Channel<IpcEvent>) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifegreet: cannot connect to Wayland: {e}");
            return 1;
        }
    };
    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("lifegreet: registry init failed: {e}");
            return 1;
        }
    };
    let qh: QueueHandle<App> = event_queue.handle();
    let mut event_loop: EventLoop<App> =
        EventLoop::try_new().expect("failed to create event loop");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_wm_base");
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("lifegreet");
    window.set_app_id("lifegreet");
    window.commit();

    // A scene needs a grid map; build a provisional one now (rebuilt on
    // configure with the real size).
    let provisional = build_grid_map(
        FALLBACK_SIZE.0,
        FALLBACK_SIZE.1,
        cfg.cell_w,
        cfg.cell_h,
        cfg.cube_height,
        cfg.pitch,
    );
    let scene = Scene::new(&cfg, &provisional);
    let atlas = Atlas::new(&cfg.font, cfg.cell_w, cfg.cell_h, cfg.solid_cells);
    let sessions = Sessions::load(&cfg.sessions_dir, &cfg.cmd);

    let mut app = App {
        loop_handle: event_loop.handle(),
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        qh: qh.clone(),
        window,
        keyboard: None,
        width: 0,
        height: 0,
        pool: None,
        frame_pending: false,
        frame_pending_since: None,
        watchdog_tripped: false,
        configured: false,
        final_map: None,
        empty_map: None,
        anim_map: None,
        anchors: None,
        cfg,
        start: Instant::now(),
        scene,
        atlas,
        greeter: Greeter::new(),
        sessions,
        password: SecureBuf::new(),
        ipc,
        info: None,
        fatal: None,
        mods: Modifiers::default(),
        last_key_t: 0.0,
        exit: false,
        exit_code: 1,
        password_clear_gen: 0,
    };

    app.loop_handle
        .insert_source(ipc_events, |event, _, app: &mut App| {
            if let ChannelEvent::Msg(ev) = event {
                app.on_ipc_event(ev);
            }
        })
        .expect("insert ipc channel");

    // Animation timer.
    let frame = Duration::from_secs_f64(1.0 / app.cfg.fps);
    app.loop_handle
        .insert_source(Timer::from_duration(frame), move |_, _, app: &mut App| {
            app.tick();
            TimeoutAction::ToDuration(frame)
        })
        .expect("insert frame timer");

    WaylandSource::new(conn, event_queue)
        .insert(app.loop_handle.clone())
        .expect("insert wayland source");

    while !app.exit {
        if event_loop.dispatch(Duration::from_millis(100), &mut app).is_err() {
            break;
        }
    }
    app.exit_code
}

impl App {
    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn tick(&mut self) {
        if self.exit {
            return;
        }
        let t = self.t();
        self.greeter.advance(t);
        self.scene.advance(t);
        self.scene.validating = self.greeter.phase == Phase::Validating;

        // Idle at the password box with nothing typed: fall back to the
        // username box (it must be re-entered every attempt by design).
        if self.greeter.phase == Phase::EnterPassword
            && self.password.is_empty()
            && t - self.last_key_t > IDLE_COLLAPSE_SECS
        {
            self.greeter.cancel_to_user(t);
            self.ipc.send(IpcCmd::Cancel);
        }

        if let Some((_, since)) = &self.info {
            if t - since > INFO_SECS {
                self.info = None;
            }
        }

        if self.configured {
            if self.frame_pending {
                // Frame-callback watchdog: see FRAME_WATCHDOG.
                let stuck = self
                    .frame_pending_since
                    .is_some_and(|since| since.elapsed() >= FRAME_WATCHDOG);
                if !stuck {
                    return;
                }
                if !self.watchdog_tripped {
                    self.watchdog_tripped = true;
                    eprintln!("lifegreet: frame callback lost (DPMS/hotplug?) — repainting");
                }
                self.frame_pending = false;
            }
            self.paint();
        }
    }

    fn paint(&mut self) {
        let t = self.t();
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 || self.final_map.is_none() {
            return;
        }
        let stride = w as i32 * 4;
        if self.pool.is_none() {
            match SlotPool::new((stride * h as i32) as usize, &self.shm) {
                Ok(p) => self.pool = Some(p),
                Err(e) => {
                    eprintln!("lifegreet: shm pool failed: {e}");
                    return;
                }
            }
        }

        let t0 = self.cfg.debug_timing.then(Instant::now);

        // Pick the map for the current cube scale; rebuild the interpolated
        // one only while animating.
        let scale = self.greeter.cube_scale(t);
        let map: &GridMap = if scale >= 0.999 {
            self.final_map.as_ref().unwrap()
        } else if scale <= MIN_DRAW_SCALE {
            self.empty_map.as_ref().unwrap()
        } else {
            self.anim_map = Some(build_grid_map_scaled(
                w,
                h,
                self.cfg.cell_w,
                self.cfg.cell_h,
                self.cfg.cube_height,
                self.cfg.pitch,
                scale,
            ));
            self.anim_map.as_ref().unwrap()
        };
        let anchors = self.anchors.as_ref().unwrap();

        let session_line = (self.sessions.len() > 1)
            .then(|| format!("F3: {}", self.sessions.current().name));
        let ov = Overlay {
            username: &self.greeter.username,
            caret_on: t % 1.0 < 0.5,
            box_alpha: self.greeter.box_alpha(t),
            session_line: session_line.as_deref(),
            info: self
                .fatal
                .as_deref()
                .or(self.info.as_ref().map(|(m, _)| m.as_str())),
        };

        let pool = self.pool.as_mut().unwrap();
        match pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888) {
            Ok((buffer, canvas)) => {
                render_frame(
                    canvas,
                    w as usize,
                    h as usize,
                    map,
                    anchors,
                    &self.atlas,
                    &self.scene,
                    t,
                    &ov,
                    self.cfg.text,
                    self.cfg.urgent,
                    self.cfg.clock_px,
                    self.cfg.caps_px,
                    self.cfg.user_px,
                );
                let surface = self.window.wl_surface();
                surface.frame(&self.qh, surface.clone());
                buffer.attach_to(surface).expect("attach");
                surface.damage_buffer(0, 0, w as i32, h as i32);
                surface.commit();
                self.frame_pending = true;
                self.frame_pending_since = Some(Instant::now());
            }
            Err(e) => eprintln!("lifegreet: buffer alloc failed: {e}"),
        }

        if let Some(t0) = t0 {
            eprintln!("lifegreet: frame {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
        }
    }

    // ----- keyboard --------------------------------------------------------

    fn on_key(&mut self, event: KeyEvent) {
        if self.exit {
            return;
        }
        let t = self.t();
        self.last_key_t = t;
        let ctrl = self.mods.ctrl;

        // Global: Ctrl+Alt+Del reboots (the greeter user may command the
        // active seat via logind, same as tuigreet's power option did).
        if ctrl
            && self.mods.alt
            && matches!(event.keysym, Keysym::Delete | Keysym::KP_Delete)
        {
            let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            return;
        }

        match self.greeter.phase {
            Phase::EnterUser => self.on_key_username(event, ctrl, t),
            Phase::Growing { .. } | Phase::EnterPassword => {
                self.on_key_password(event, ctrl, t)
            }
            // Validating/Failed/Collapsing/Starting: input is ignored.
            _ => {}
        }
    }

    /// Username entry: visible text, plain editing. F3 cycles sessions.
    fn on_key_username(&mut self, event: KeyEvent, ctrl: bool, t: f64) {
        match event.keysym {
            Keysym::F3 => self.sessions.cycle(),
            Keysym::Return | Keysym::KP_Enter => {
                if self.greeter.user_submit(t) {
                    self.ipc.send(IpcCmd::CreateSession {
                        username: self.greeter.username.trim().to_string(),
                    });
                }
            }
            Keysym::BackSpace => {
                if ctrl {
                    self.greeter.username.clear();
                } else {
                    self.greeter.username.pop();
                }
            }
            Keysym::Escape => self.greeter.username.clear(),
            Keysym::u | Keysym::U if ctrl => self.greeter.username.clear(),
            _ => {
                if ctrl {
                    return;
                }
                let Some(s) = event.utf8.as_deref() else { return };
                for c in s.chars() {
                    if !c.is_control() && self.greeter.username.chars().count() < USERNAME_MAX {
                        self.greeter.username.push(c);
                    }
                }
            }
        }
    }

    /// Password entry: swaylock semantics on the SecureBuf, flares only.
    fn on_key_password(&mut self, event: KeyEvent, ctrl: bool, t: f64) {
        match key_action(event.keysym, event.utf8.as_deref(), ctrl) {
            Action::Append(mut s) => {
                self.password.push_str(&s);
                // Wipe this transient heap copy once it's in the mlock'd
                // buffer (the KeyEvent's own String is dropped by sctk).
                s.zeroize();
                self.scene.key_flare(t);
                self.arm_password_clear();
            }
            Action::PopChar => {
                if self.password.pop_char() {
                    if self.password.is_empty() {
                        self.scene.clear_dim(t);
                    } else {
                        self.scene.backspace_flare(t);
                        self.arm_password_clear();
                    }
                }
            }
            Action::ClearAll => {
                if event.keysym == Keysym::Escape && self.password.is_empty() {
                    // Username-typo recovery: back out to the username box.
                    self.greeter.cancel_to_user(t);
                    self.ipc.send(IpcCmd::Cancel);
                    self.scene.clear_dim(t);
                } else {
                    self.password.clear();
                    self.scene.clear_dim(t);
                }
            }
            Action::Submit => {
                if self.password.is_empty() {
                    return; // ignore-empty-password
                }
                if self.greeter.password_submit() {
                    // Transient copy for the wire; the worker zeroizes it
                    // after serialization (see ipc.rs header).
                    let pw = String::from_utf8_lossy(self.password.as_bytes()).into_owned();
                    self.password.clear();
                    self.ipc.send(IpcCmd::PostAuth { password: Some(pw) });
                }
            }
            Action::Neutral | Action::Ignore => {}
        }
    }

    // ----- greetd events ----------------------------------------------------

    fn on_ipc_event(&mut self, ev: IpcEvent) {
        let t = self.t();
        match ev {
            IpcEvent::Prompt { secret, message } => {
                // The first (password) prompt is expected — entry is already
                // open. A prompt arriving while Validating means greetd wants
                // a SECOND answer (e.g. 2FA), which this UI can't take.
                if self.greeter.phase == Phase::Validating {
                    eprintln!("lifegreet: unsupported extra prompt (secret={secret}): {message}");
                    self.password.clear();
                    self.scene.wrong(t);
                    self.greeter.on_auth_failed(t);
                    self.ipc.send(IpcCmd::Cancel);
                    self.info = Some(("unsupported extra auth prompt".into(), t));
                }
            }
            IpcEvent::Info(m) => self.info = Some((m, t)),
            IpcEvent::AuthOk => {
                if self.greeter.on_auth_ok() {
                    // systemd-cat: greetd gives the session vt1 as stdio, so
                    // its startup chatter would sit in the VT text buffer and
                    // flash as terminal text during the KMS handoff. Journal
                    // instead: journalctl -t greetd-session
                    let mut cmd = vec![
                        "systemd-cat".to_string(),
                        "-t".to_string(),
                        "greetd-session".to_string(),
                        "--".to_string(),
                    ];
                    cmd.extend(self.sessions.current().cmd.iter().cloned());
                    self.ipc.send(IpcCmd::StartSession { cmd });
                }
            }
            IpcEvent::AuthFailed(m) => {
                self.password.clear();
                self.scene.wrong(t);
                self.greeter.on_auth_failed(t);
                self.ipc.send(IpcCmd::Cancel);
                eprintln!("lifegreet: {m}");
            }
            IpcEvent::SessionStarted => {
                // greetd launches the session once we exit; cage follows.
                self.exit_code = 0;
                self.exit = true;
            }
            IpcEvent::Fatal(m) => {
                eprintln!("lifegreet: fatal: {m}");
                self.fatal = Some(m);
            }
        }
    }

    // ----- timers (calloop one-shots keyed by a generation counter) --------

    fn arm_password_clear(&mut self) {
        self.password_clear_gen += 1;
        let gen = self.password_clear_gen;
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs_f64(PASSWORD_CLEAR_SECS)),
            move |_, _, app: &mut App| {
                if app.password_clear_gen == gen && !app.password.is_empty() {
                    app.password.clear();
                    app.scene.clear_dim(app.t());
                }
                TimeoutAction::Drop
            },
        );
    }
}

impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        // Never happens under cage; honoring it in dev runs is convenient.
        self.exit_code = 1;
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let w = configure.new_size.0.map(|v| v.get()).unwrap_or(FALLBACK_SIZE.0);
        let h = configure.new_size.1.map(|v| v.get()).unwrap_or(FALLBACK_SIZE.1);
        if w != self.width || h != self.height {
            self.width = w;
            self.height = h;
            self.pool = None;
            self.anim_map = None;
            let final_map = build_grid_map(
                w,
                h,
                self.cfg.cell_w,
                self.cfg.cell_h,
                self.cfg.cube_height,
                self.cfg.pitch,
            );
            self.empty_map = Some(build_grid_map_scaled(
                w,
                h,
                self.cfg.cell_w,
                self.cfg.cell_h,
                self.cfg.cube_height,
                self.cfg.pitch,
                0.0,
            ));
            self.anchors = Some(Anchors::of(&final_map));
            // Board dims depend only on cube height/pitch, but the ember
            // field is cols×rows: rebuild the scene at the real size.
            self.scene = Scene::new(&self.cfg, &final_map);
            self.final_map = Some(final_map);
        }
        self.configured = true;
        self.frame_pending = false;
        self.frame_pending_since = None;
        self.watchdog_tripped = false;
        self.paint();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if surface == self.window.wl_surface() {
            if self.watchdog_tripped {
                self.watchdog_tripped = false;
                eprintln!("lifegreet: frame callbacks resumed");
            }
            self.frame_pending = false;
            self.frame_pending_since = None;
        }
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

    fn new_capability(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|app: &mut App, _kbd, event| app.on_key(event)),
            ) {
                Ok(kbd) => self.keyboard = Some(kbd),
                Err(e) => eprintln!("lifegreet: no keyboard: {e}"),
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(kbd) = self.keyboard.take() {
                kbd.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {
        // Compositor-driven (wl_keyboard v10) repeats — no-op. We drive repeat
        // ourselves through the get_keyboard_with_repeat calloop callback, so
        // handling this too would double-type on v10 compositors.
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.mods = modifiers;
        self.scene.caps_lock = modifiers.caps_lock;
    }
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
smithay_client_toolkit::delegate_keyboard!(App);
smithay_client_toolkit::delegate_shm!(App);
smithay_client_toolkit::delegate_xdg_shell!(App);
smithay_client_toolkit::delegate_xdg_window!(App);
smithay_client_toolkit::delegate_registry!(App);
