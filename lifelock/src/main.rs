// SPDX-License-Identifier: GPL-3.0-or-later
// lifelock — an ext-session-lock screen locker for niri.
//
// The isometric Game of Life cube is cosmetic. Everything security-relevant is
// modelled on swaylock 1.8.6 (see NOTICE): the auth child runs PAM in a
// separate process, the password lives in an mlock'd buffer that is zeroed at
// every clear point, core dumps and ptrace are disabled, and the session is
// only ever unlocked after PAM says yes.

mod app;
mod auth;
mod cli;
mod daemon;
mod geometry;
mod harden;
mod input;
mod render;
mod scene;
mod secure_buf;
mod sim;

fn main() {
    // The auth child is a distinct mode of this same binary. Dispatch it as
    // the very first thing — before CLI parsing, fonts, or Wayland — so the
    // child process never links any of that into its failure domain.
    if std::env::args().nth(1).as_deref() == Some(auth::ARG_AUTH_CHILD) {
        auth::child::run(); // never returns
    }

    // Debug-only harness for M3: spawn the auth child, read a password from
    // stdin, print the verdict. Compiled ONLY in debug builds so this
    // plaintext-stdin path can never exist in a released/installed binary.
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--selftest-auth") {
        selftest_auth();
        return;
    }

    // Debug-only: render one frame to a PPM (no Wayland, no display) so the
    // cube/ember/clock can be inspected offline while tuning constants.
    //   lifelock --render-ppm OUT.ppm [advance_secs] [flares]
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--render-ppm") {
        render_ppm();
        return;
    }

    // UI process.
    harden::harden_common();
    harden::refuse_setuid();

    let cfg = cli::parse();

    // A caller-supplied --ready-fd would otherwise be inherited by the auth
    // child across execve; mark it close-on-exec so only this process holds
    // it (the child never needs it).
    if let Some(fd) = cfg.ready_fd {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }

    // Refuse to lock a session we could provably never unlock. A debug-only
    // env bypass lets the Wayland/render path be smoke-tested in a nested
    // compositor before the PAM service file is installed; it can never exist
    // in a release build.
    #[cfg(debug_assertions)]
    let skip_pam_check = std::env::var_os("LIFELOCK_TEST_NO_PAM").is_some();
    #[cfg(not(debug_assertions))]
    let skip_pam_check = false;
    if !skip_pam_check {
        harden::require_pam_service();
    }

    // Daemonize BEFORE spawning the auth child or touching Wayland, so the
    // surviving (child) process owns the auth pipes and display connection.
    if cfg.daemonize {
        daemon::fork_early();
    }

    // Spawn the auth child before connecting to Wayland (swaylock ordering).
    let auth = auth::parent::AuthProc::spawn();

    let code = app::run(cfg, auth);
    std::process::exit(code);
}

#[cfg(debug_assertions)]
fn render_ppm() {
    use std::io::Write;
    let a: Vec<String> = std::env::args().collect();
    let out = a.get(2).cloned().unwrap_or_else(|| "/tmp/lifelock.ppm".into());
    let advance: f64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let flares: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let caps = a.iter().any(|s| s == "--caps");
    let validating = a.iter().any(|s| s == "--validating");
    let wrong = a.iter().any(|s| s == "--wrong");

    let (w, h) = (1920u32, 1080u32);
    let cfg = cli::Cfg::default();
    let map = geometry::build_grid_map(w, h, cfg.cell_w, cfg.cell_h, cfg.cube_height, cfg.pitch);
    let atlas = render::Atlas::new(&cfg.font, cfg.cell_w, cfg.cell_h, cfg.solid_cells);
    let mut scene = scene::Scene::new(&cfg, &map);

    // Step the sims forward so boards have real structure, not initial soup.
    let mut t = 0.0;
    while t < advance {
        t += 1.0 / cfg.fps;
        scene.advance(t);
    }
    for _ in 0..flares {
        scene.key_flare(t);
    }
    scene.caps_lock = caps;
    scene.validating = validating;
    if wrong {
        scene.wrong(t);
        t += 0.1;
        scene.advance(t);
    }

    let mut canvas = vec![0u8; (w * h * 4) as usize];
    render::render_frame(&mut canvas, w as usize, h as usize, &map, &atlas, &scene, t, cfg.text, cfg.urgent, cfg.clock_px, cfg.caps_px);

    // XRGB8888 little-endian [B,G,R,X] -> PPM (P6 RGB).
    let mut ppm = Vec::with_capacity((w * h * 3 + 32) as usize);
    ppm.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in canvas.chunks_exact(4) {
        ppm.push(px[2]); // R
        ppm.push(px[1]); // G
        ppm.push(px[0]); // B
    }
    std::fs::File::create(&out).unwrap().write_all(&ppm).unwrap();
    eprintln!("wrote {out} ({w}x{h})");
}

#[cfg(debug_assertions)]
fn selftest_auth() {
    use std::io::Read;
    harden::harden_common();
    let mut auth = auth::parent::AuthProc::spawn();
    let mut pw = secure_buf::SecureBuf::new();
    let mut line = String::new();
    std::io::stdin().read_to_string(&mut line).ok();
    pw.push_str(line.trim_end_matches('\n'));
    line.clear();
    eprintln!("selftest: submitting {} byte password", pw.len());
    if !auth.submit(&pw) {
        eprintln!("selftest: submit failed (channel broke)");
        std::process::exit(3);
    }
    pw.clear();
    // Block-read the verdict on the reply fd.
    let fd = auth.reply_fd();
    let mut b = [0u8; 1];
    let n = unsafe { libc::read(fd, b.as_mut_ptr() as *mut libc::c_void, 1) };
    if n == 1 {
        if b[0] == 1 {
            println!("AUTH OK");
        } else {
            println!("AUTH FAIL");
        }
    } else {
        println!("AUTH ERROR (no reply)");
    }
}
