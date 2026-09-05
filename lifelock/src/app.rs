// SPDX-License-Identifier: GPL-3.0-or-later
// Wayland session-lock application: the UI process's event loop, per-output
// lock surfaces, keyboard handling, animation timer, and the auth channel.
//
// Structure follows sctk's session_lock example, extended with the things a
// real locker needs and the example omits: keyboard input, output hotplug
// while locked, the locked->ready-fd->daemonize ordering, and the SIGUSR1
// graceful-unlock escape hatch.

use crate::auth::parent::AuthProc;
use crate::cli::Cfg;
use crate::geometry::{build_grid_map, GridMap};
use crate::input::{key_action, Action, AuthState, Machine};
use crate::render::{render_frame, Atlas};
use crate::scene::Scene;
use crate::secure_buf::SecureBuf;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
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
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
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

/// How long to wait for a frame callback before assuming it was lost and
/// repainting anyway. Callbacks vanish when the output power-cycles under us
/// (BenQ monitors drop the HDMI connector when waking from deep standby, so
/// niri destroys/recreates the output several times in a row); without this
/// watchdog one lost callback freezes the UI until reboot while input keeps
/// working invisibly. Short enough that key flares (0.8 s) stay visible even
/// while running purely on the watchdog.
const FRAME_WATCHDOG: Duration = Duration::from_millis(250);

struct LockOutput {
    output: wl_output::WlOutput,
    surface: SessionLockSurface,
    width: u32,
    height: u32,
    map: Option<GridMap>,
    pool: Option<SlotPool>,
    frame_pending: bool,
    frame_pending_since: Option<Instant>,
    watchdog_tripped: bool,
    configured: bool,
}

pub struct App {
    loop_handle: LoopHandle<'static, Self>,
    conn: Connection,
    compositor: CompositorState,
    output_state: OutputState,
    registry_state: RegistryState,
    seat_state: SeatState,
    shm: Shm,
    qh: QueueHandle<Self>,

    session_lock_state: SessionLockState,
    session_lock: Option<SessionLock>,
    outputs: Vec<LockOutput>,
    keyboard: Option<wl_keyboard::WlKeyboard>,

    cfg: Cfg,
    start: Instant,
    scene: Scene,
    atlas: Atlas,
    machine: Machine,
    password: SecureBuf,
    auth: AuthProc,

    mods: Modifiers,
    locked: bool,
    saw_first_key: bool,
    exit: bool,
    unlocked_ok: bool,
    final_code: i32,
    auth_dead: bool,
    // Deferred timer tokens (armed as calloop timers keyed by generation).
    input_idle_gen: u64,
    password_clear_gen: u64,
    auth_invalid_gen: u64,
}

/// Run the UI process. `auth` is the already-spawned auth child.
/// Returns the process exit code.
pub fn run(cfg: Cfg, auth: AuthProc) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifelock: cannot connect to Wayland: {e}");
            return 1;
        }
    };
    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("lifelock: registry init failed: {e}");
            return 1;
        }
    };
    let qh: QueueHandle<App> = event_queue.handle();
    let mut event_loop: EventLoop<App> =
        EventLoop::try_new().expect("failed to create event loop");

    // A scene needs a grid map; build a provisional one now (rebuilt per
    // output on configure with the real size).
    let provisional = build_grid_map(1920, 1080, cfg.cell_w, cfg.cell_h, cfg.cube_height, cfg.pitch);
    let scene = Scene::new(&cfg, &provisional);
    let atlas = Atlas::new(&cfg.font, cfg.cell_w, cfg.cell_h, cfg.solid_cells);

    let mut app = App {
        loop_handle: event_loop.handle(),
        conn: conn.clone(),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        qh: qh.clone(),
        session_lock_state: SessionLockState::new(&globals, &qh),
        session_lock: None,
        outputs: Vec::new(),
        keyboard: None,
        cfg,
        start: Instant::now(),
        scene,
        atlas,
        machine: Machine::new(),
        password: SecureBuf::new(),
        auth,
        mods: Modifiers::default(),
        locked: false,
        saw_first_key: false,
        exit: false,
        unlocked_ok: false,
        final_code: 1,
        auth_dead: false,
        input_idle_gen: 0,
        password_clear_gen: 0,
        auth_invalid_gen: 0,
    };

    let lock = match app.session_lock_state.lock(&qh) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("lifelock: ext-session-lock not supported: {e}");
            return 1;
        }
    };
    // Create a lock surface per existing output.
    for output in app.output_state.outputs() {
        let surface = app.compositor.create_surface(&qh);
        let ls = lock.create_lock_surface(surface, &output, &qh);
        app.outputs.push(LockOutput {
            output,
            surface: ls,
            width: 0,
            height: 0,
            map: None,
            pool: None,
            frame_pending: false,
            frame_pending_since: None,
            watchdog_tripped: false,
            configured: false,
        });
    }
    app.session_lock = Some(lock);

    // Poll the auth reply fd.
    let reply_fd = app.auth.reply_fd();
    let source = smithay_client_toolkit::reexports::calloop::generic::Generic::new(
        unsafe { std::os::unix::io::BorrowedFd::borrow_raw(reply_fd) }.try_clone_to_owned().unwrap(),
        smithay_client_toolkit::reexports::calloop::Interest::READ,
        smithay_client_toolkit::reexports::calloop::Mode::Level,
    );
    app.loop_handle
        .insert_source(source, |_, _, app: &mut App| {
            use smithay_client_toolkit::reexports::calloop::PostAction;
            // If the auth child died, its reply pipe is at permanent EOF —
            // a level-triggered source would then spin at 100% CPU. Remove it.
            let alive = app.on_auth_reply();
            Ok(if alive { PostAction::Continue } else { PostAction::Remove })
        })
        .expect("insert auth reply source");

    // SIGUSR1: an unauthenticated unlock is only wired up when explicitly
    // opted in. By default we ignore the signal so PAM is the sole unlock
    // path and no same-UID process can raise the lock.
    if app.cfg.allow_signal_unlock {
        install_sigusr1(&app.loop_handle);
    } else {
        unsafe {
            libc::signal(libc::SIGUSR1, libc::SIG_IGN);
        }
    }

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

    if app.unlocked_ok {
        0
    } else {
        app.final_code
    }
}

// SIGUSR1 self-pipe -----------------------------------------------------------

static mut SIGUSR1_PIPE: [i32; 2] = [-1, -1];

extern "C" fn on_sigusr1(_sig: libc::c_int) {
    unsafe {
        let _ = libc::write(SIGUSR1_PIPE[1], b"1".as_ptr() as *const libc::c_void, 1);
    }
}

fn install_sigusr1(handle: &LoopHandle<'static, App>) {
    let read_fd = unsafe {
        if libc::pipe2(std::ptr::addr_of_mut!(SIGUSR1_PIPE) as *mut i32, libc::O_CLOEXEC | libc::O_NONBLOCK) != 0 {
            eprintln!("lifelock: warning: SIGUSR1 pipe failed; graceful unlock disabled");
            return;
        }
        libc::signal(libc::SIGUSR1, on_sigusr1 as *const () as libc::sighandler_t);
        SIGUSR1_PIPE[0]
    };
    let owned = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(read_fd) }
        .try_clone_to_owned()
        .expect("dup sigusr1 read fd");
    let source = smithay_client_toolkit::reexports::calloop::generic::Generic::new(
        owned,
        smithay_client_toolkit::reexports::calloop::Interest::READ,
        smithay_client_toolkit::reexports::calloop::Mode::Level,
    );
    handle
        .insert_source(source, move |_, _, app: &mut App| {
            // Drain the self-pipe so the level-triggered source quiesces.
            let mut buf = [0u8; 8];
            while unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) } > 0 {}
            app.begin_unlock(0); // scripted graceful-unlock escape hatch
            Ok(smithay_client_toolkit::reexports::calloop::PostAction::Continue)
        })
        .expect("insert sigusr1 source");
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
        self.scene.advance(t);
        self.scene.validating = self.machine.auth == AuthState::Validating;
        self.scene.failed_attempts = self.machine.failed_attempts;
        for i in 0..self.outputs.len() {
            if !self.outputs[i].configured {
                continue;
            }
            if self.outputs[i].frame_pending {
                // Frame-callback watchdog: see FRAME_WATCHDOG.
                let stuck = self.outputs[i]
                    .frame_pending_since
                    .is_some_and(|since| since.elapsed() >= FRAME_WATCHDOG);
                if !stuck {
                    continue;
                }
                if !self.outputs[i].watchdog_tripped {
                    self.outputs[i].watchdog_tripped = true;
                    eprintln!("lifelock: frame callback lost on output {i} (DPMS/hotplug?) — repainting");
                }
                self.outputs[i].frame_pending = false;
            }
            self.paint(i);
        }
    }

    fn paint(&mut self, idx: usize) {
        let t = self.t();
        let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
        if w == 0 || h == 0 {
            return;
        }
        // Ensure a grid map + pool sized to this output.
        if self.outputs[idx].map.is_none() {
            let map = build_grid_map(w, h, self.cfg.cell_w, self.cfg.cell_h, self.cfg.cube_height, self.cfg.pitch);
            self.outputs[idx].map = Some(map);
        }
        let stride = w as i32 * 4;
        if self.outputs[idx].pool.is_none() {
            match SlotPool::new((stride * h as i32) as usize, &self.shm) {
                Ok(p) => self.outputs[idx].pool = Some(p),
                Err(e) => {
                    eprintln!("lifelock: shm pool failed: {e}");
                    return;
                }
            }
        }

        let t0 = self.cfg.debug_timing.then(Instant::now);

        let cfg_text = self.cfg.text;
        let cfg_urgent = self.cfg.urgent;
        let clock_px = self.cfg.clock_px;
        let caps_px = self.cfg.caps_px;

        // Take map + pool out so we can borrow self.atlas/self.scene at the
        // same time (disjoint from self.outputs); restore before returning.
        let map = self.outputs[idx].map.take().unwrap();
        let mut pool = self.outputs[idx].pool.take().unwrap();

        let result = pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888);
        match result {
            Ok((buffer, canvas)) => {
                render_frame(
                    canvas,
                    w as usize,
                    h as usize,
                    &map,
                    &self.atlas,
                    &self.scene,
                    t,
                    cfg_text,
                    cfg_urgent,
                    clock_px,
                    caps_px,
                );
                let surface = self.outputs[idx].surface.wl_surface();
                surface.frame(&self.qh, surface.clone());
                buffer.attach_to(surface).expect("attach");
                // Without damage the compositor never repaints the output —
                // the monitor freezes on the first frame while screencopy
                // (grim) still looks live. lifegreet always had this line.
                surface.damage_buffer(0, 0, w as i32, h as i32);
                surface.commit();
                self.outputs[idx].frame_pending = true;
                self.outputs[idx].frame_pending_since = Some(Instant::now());
            }
            Err(e) => eprintln!("lifelock: buffer alloc failed: {e}"),
        }

        self.outputs[idx].map = Some(map);
        self.outputs[idx].pool = Some(pool);

        if let Some(t0) = t0 {
            eprintln!("lifelock: frame {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
        }
    }

    // ----- keyboard --------------------------------------------------------

    fn on_key(&mut self, event: KeyEvent) {
        if self.exit {
            return;
        }
        let ctrl = self.mods.ctrl;
        let t = self.t();
        match key_action(event.keysym, event.utf8.as_deref(), ctrl) {
            Action::Append(mut s) => {
                self.password.push_str(&s);
                // Wipe this transient heap copy once it's in the mlock'd buffer
                // (the KeyEvent's own String is dropped by sctk right after).
                s.zeroize();
                self.machine.on_letter();
                self.scene.key_flare(t);
                self.arm_password_clear();
                self.arm_input_idle();
            }
            Action::PopChar => {
                let had = !self.password.is_empty();
                let removed = self.password.pop_char();
                let now_empty = self.password.is_empty();
                if removed {
                    self.machine.on_backspace(now_empty);
                    if now_empty {
                        self.scene.clear_dim(t);
                    } else {
                        self.scene.backspace_flare(t);
                        self.arm_password_clear();
                    }
                    self.arm_input_idle();
                }
                let _ = had;
            }
            Action::ClearAll => {
                self.password.clear();
                self.machine.on_clear();
                self.scene.clear_dim(t);
                self.arm_input_idle();
            }
            Action::Submit => {
                if self.password.is_empty() {
                    return; // ignore-empty-password
                }
                if self.auth_dead {
                    // No authenticator left; can't succeed. Show the failure
                    // feedback without touching the dead pipe.
                    self.password.clear();
                    self.machine.on_submit();
                    self.machine.on_reply(false);
                    self.scene.wrong(t);
                    self.scene.status_msg =
                        Some("auth broken — use the recovery bind".into());
                    self.arm_auth_invalid();
                    return;
                }
                let (go, _) = self.machine.on_submit();
                if go {
                    self.scene.status_msg = None; // stale PAM text
                    let ok = self.auth.submit(&self.password);
                    self.password.clear(); // clear whether or not the write took
                    if !ok {
                        // Auth child gone: don't leave the state machine stuck
                        // in Validating (which would block every future retry).
                        // Treat it as a failed attempt so the user can try
                        // again; the session stays locked either way.
                        eprintln!("lifelock: auth channel broke");
                        self.machine.on_reply(false);
                        self.scene.wrong(t);
                        self.arm_auth_invalid();
                    }
                }
            }
            Action::Neutral => {
                self.machine.on_neutral();
                self.arm_password_clear();
                self.arm_input_idle();
            }
            Action::Ignore => {}
        }
    }

    /// Handle an auth verdict. Returns false if the auth child is gone (so the
    /// caller removes the reply source to avoid a busy-loop on EOF).
    fn on_auth_reply(&mut self) -> bool {
        match self.auth.read_reply() {
            Some((true, _)) => {
                self.machine.on_reply(true);
                self.begin_unlock(0);
                true
            }
            Some((false, msg)) => {
                self.machine.on_reply(false);
                self.scene.wrong(self.t());
                // PAM's own words (faillock lockout etc.) beat a bare flash;
                // stays up until the next submit.
                self.scene.status_msg = msg;
                self.arm_auth_invalid();
                true
            }
            None => {
                // Auth child died. Session stays locked (fail-secure); further
                // password attempts can't succeed, so recover via the
                // Mod+Shift+Alt+L swaylock-takeover bind or a TTY. Report once
                // and let the reply source be removed so it doesn't spin.
                self.machine.auth = AuthState::Idle;
                self.auth_dead = true;
                eprintln!("lifelock: auth child died — session stays locked; use the recovery bind");
                false
            }
        }
    }

    // ----- timers (calloop one-shots keyed by a generation counter) --------

    fn arm_input_idle(&mut self) {
        self.input_idle_gen += 1;
        let gen = self.input_idle_gen;
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs_f64(crate::input::INPUT_IDLE_SECS)),
            move |_, _, app: &mut App| {
                if app.input_idle_gen == gen {
                    app.machine.on_input_idle();
                }
                TimeoutAction::Drop
            },
        );
    }

    fn arm_password_clear(&mut self) {
        self.password_clear_gen += 1;
        let gen = self.password_clear_gen;
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs_f64(crate::input::PASSWORD_CLEAR_SECS)),
            move |_, _, app: &mut App| {
                if app.password_clear_gen == gen && !app.password.is_empty() {
                    app.password.clear();
                    app.machine.on_clear();
                    app.scene.clear_dim(app.t());
                }
                TimeoutAction::Drop
            },
        );
    }

    fn arm_auth_invalid(&mut self) {
        self.auth_invalid_gen += 1;
        let gen = self.auth_invalid_gen;
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs_f64(crate::input::AUTH_INVALID_SECS)),
            move |_, _, app: &mut App| {
                if app.auth_invalid_gen == gen {
                    app.machine.on_auth_idle();
                }
                TimeoutAction::Drop
            },
        );
    }

    // ----- unlock ----------------------------------------------------------

    fn begin_unlock(&mut self, code: i32) {
        if self.exit {
            return;
        }
        self.password.clear();
        if let Some(lock) = self.session_lock.take() {
            lock.unlock();
            // Roundtrip so the compositor processes unlock before we exit;
            // otherwise the connection could tear down first and the session
            // would stay locked.
            let _ = self.conn.roundtrip();
        }
        self.unlocked_ok = code == 0;
        self.final_code = code;
        self.exit = true;
    }
}

impl SessionLockHandler for App {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        self.locked = true;
        eprintln!("lifelock: session locked (compositor confirmed)");
        // Ordering matters: signal readiness, THEN daemonize — both only after
        // the compositor confirms the session is locked, so
        // `lifelock -f && systemctl suspend` cannot race.
        if let Some(fd) = self.cfg.ready_fd.take() {
            unsafe {
                let _ = libc::write(fd, b"\n".as_ptr() as *const libc::c_void, 1);
                libc::close(fd);
            }
        }
        if self.cfg.daemonize {
            crate::daemon::finish_daemonize();
        }
    }

    fn finished(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        eprintln!("lifelock: failed to lock session — is another locker already running?");
        self.session_lock = None;
        self.unlocked_ok = false;
        self.final_code = 2;
        self.exit = true;
        // If we were about to daemonize, the parent is still blocked on the
        // status pipe; tell it we failed so `&&` chains halt.
        if self.cfg.daemonize {
            crate::daemon::abort_daemonize();
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        if let Some(o) = self.outputs.iter_mut().find(|o| o.surface.wl_surface() == surface.wl_surface()) {
            let resized = o.width != w || o.height != h;
            eprintln!("lifelock: lock surface configured {w}x{h}{}", if resized { " (resized)" } else { "" });
            o.width = w;
            o.height = h;
            o.configured = true;
            if resized {
                o.map = None;
                o.pool = None;
            }
            o.frame_pending = false;
            o.frame_pending_since = None;
            o.watchdog_tripped = false;
        }
        // Paint immediately (ack is handled internally by sctk).
        if let Some(idx) = self.outputs.iter().position(|o| o.surface.wl_surface() == surface.wl_surface()) {
            self.paint(idx);
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.surface.wl_surface() == surface) {
            if o.watchdog_tripped {
                o.watchdog_tripped = false;
                eprintln!("lifelock: frame callbacks resumed");
            }
            o.frame_pending = false;
            o.frame_pending_since = None;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // A monitor was plugged in while locked — it MUST get a lock surface,
        // or it would show unlocked content. (sctk's example omits this.)
        if let Some(lock) = &self.session_lock {
            if !self.outputs.iter().any(|o| o.output == output) {
                let name = self
                    .output_state
                    .info(&output)
                    .and_then(|i| i.name)
                    .unwrap_or_default();
                eprintln!("lifelock: output {name:?} appeared — creating lock surface");
                let surface = self.compositor.create_surface(qh);
                let ls = lock.create_lock_surface(surface, &output, qh);
                self.outputs.push(LockOutput {
                    output,
                    surface: ls,
                    width: 0,
                    height: 0,
                    map: None,
                    pool: None,
                    frame_pending: false,
                    frame_pending_since: None,
                    watchdog_tripped: false,
                    configured: false,
                });
            }
        }
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let before = self.outputs.len();
        self.outputs.retain(|o| o.output != output);
        if self.outputs.len() != before {
            eprintln!("lifelock: output gone — dropped its lock surface ({} left)", self.outputs.len());
        }
    }
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
                Ok(kbd) => {
                    eprintln!("lifelock: keyboard acquired");
                    self.keyboard = Some(kbd);
                }
                Err(e) => eprintln!("lifelock: no keyboard: {e}"),
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(kbd) = self.keyboard.take() {
                eprintln!("lifelock: keyboard capability removed");
                kbd.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        // Input-path diagnostic: if this never fires, the compositor never
        // gave the lock surface keyboard focus (the "typing does nothing"
        // wedge) — the bug is upstream of us.
        eprintln!("lifelock: keyboard focus entered lock surface");
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        eprintln!("lifelock: keyboard focus left lock surface");
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if !self.saw_first_key {
            self.saw_first_key = true;
            // Deliberately content-free: proves delivery without logging keys.
            eprintln!("lifelock: first key event received");
        }
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
smithay_client_toolkit::delegate_session_lock!(App);
smithay_client_toolkit::delegate_registry!(App);
