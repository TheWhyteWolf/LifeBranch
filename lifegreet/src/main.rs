// SPDX-License-Identifier: GPL-3.0-or-later
// lifegreet — a greetd Wayland greeter (run under cage) that looks like the
// lifelock lock screen: an olive username box that the isometric Game of
// Life cube grows out of. Password entry shows nothing but panel flares.
//
// Security posture follows lifelock where it applies: the password lives in
// an mlock'd buffer zeroed at every clear point, core dumps and ptrace are
// disabled, and PAM is greetd's job on the far side of $GREETD_SOCK.

mod app;
mod cli;
mod geometry;
mod harden;
mod input;
mod ipc;
mod render;
mod scene;
mod secure_buf;
mod sessions;
mod sim;
mod state;

fn main() {
    // Debug-only: render one frame of any phase to a PPM (no Wayland, no
    // greetd) so the box/grow/cube states can be inspected offline.
    //   lifegreet --render-ppm OUT.ppm [--phase user|grow|password|validating|wrong]
    //             [--grow-frac F] [--flares N] [--caps] [--username NAME]
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--render-ppm") {
        render_ppm();
        return;
    }

    harden::harden_common();
    harden::refuse_setuid();
    harden::lock_all_memory();

    let cfg = cli::parse();

    let (events_tx, events_rx) =
        smithay_client_toolkit::reexports::calloop::channel::channel();

    // Debug-only stub authenticator so the UI loop can run as a normal niri
    // window without greetd (mirrors lifelock's LIFELOCK_TEST_NO_PAM).
    #[cfg(debug_assertions)]
    let ipc = if std::env::var_os("LIFEGREET_TEST_NO_IPC").is_some() {
        eprintln!("lifegreet: STUB AUTH (LIFEGREET_TEST_NO_IPC) — password \"ok\" succeeds");
        ipc::spawn_stub(events_tx)
    } else {
        ipc::spawn(events_tx)
    };
    #[cfg(not(debug_assertions))]
    let ipc = ipc::spawn(events_tx);

    let code = app::run(cfg, ipc, events_rx);
    std::process::exit(code);
}

#[cfg(debug_assertions)]
fn render_ppm() {
    use crate::render::{Anchors, Overlay};
    use std::io::Write;

    let a: Vec<String> = std::env::args().collect();
    let out = a.get(2).cloned().unwrap_or_else(|| "/tmp/lifegreet.ppm".into());
    let get = |flag: &str| a.iter().position(|s| s == flag).and_then(|i| a.get(i + 1));
    let phase = get("--phase").cloned().unwrap_or_else(|| "user".into());
    let grow_frac: f64 = get("--grow-frac").and_then(|s| s.parse().ok()).unwrap_or(0.5);
    let flares: usize = get("--flares").and_then(|s| s.parse().ok()).unwrap_or(0);
    let username = get("--username").cloned().unwrap_or_else(|| "voyd".into());
    let caps = a.iter().any(|s| s == "--caps");

    let (w, h) = (1920u32, 1080u32);
    let mut cfg = cli::Cfg::default();
    if let Some(f) = get("--font") {
        cfg.font = f.clone();
    }
    let final_map = geometry::build_grid_map(w, h, cfg.cell_w, cfg.cell_h, cfg.cube_height, cfg.pitch);
    let anchors = Anchors::of(&final_map);
    let atlas = render::Atlas::new(&cfg.font, cfg.cell_w, cfg.cell_h, cfg.solid_cells);
    let mut scene = scene::Scene::new(&cfg, &final_map);

    // Step the sims forward so boards have real structure, not initial soup.
    let mut t = 0.0;
    while t < 6.0 {
        t += 1.0 / cfg.fps;
        scene.advance(t);
    }

    // Drive the greeter to the requested phase.
    let mut g = state::Greeter::new();
    g.username = username;
    match phase.as_str() {
        "user" => {}
        "grow" => {
            g.user_submit(t);
            t += state::GROW_SECS * grow_frac.clamp(0.0, 1.0);
        }
        "password" | "validating" | "wrong" => {
            g.user_submit(t);
            t += state::GROW_SECS;
            g.advance(t);
            for _ in 0..flares.max(if phase == "password" { 3 } else { 0 }) {
                scene.key_flare(t);
            }
            if phase == "validating" {
                g.password_submit();
                scene.validating = true;
            }
            if phase == "wrong" {
                g.password_submit();
                g.on_auth_failed(t);
                scene.wrong(t);
                t += 0.1;
            }
        }
        other => {
            eprintln!("unknown --phase {other}");
            std::process::exit(2);
        }
    }
    scene.caps_lock = caps;
    scene.advance(t);

    let scale = g.cube_scale(t);
    let map = if scale >= 0.999 {
        final_map
    } else {
        geometry::build_grid_map_scaled(w, h, cfg.cell_w, cfg.cell_h, cfg.cube_height, cfg.pitch, scale)
    };

    let ov = Overlay {
        username: &g.username,
        caret_on: true,
        box_alpha: g.box_alpha(t),
        session_line: Some("F3: Niri"),
        info: None,
    };

    let mut canvas = vec![0u8; (w * h * 4) as usize];
    render::render_frame(
        &mut canvas, w as usize, h as usize, &map, &anchors, &atlas, &scene, t, &ov,
        cfg.text, cfg.urgent, cfg.clock_px, cfg.caps_px, cfg.user_px,
    );

    // XRGB8888 little-endian [B,G,R,X] -> PPM (P6 RGB).
    let mut ppm = Vec::with_capacity((w * h * 3 + 32) as usize);
    ppm.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in canvas.chunks_exact(4) {
        ppm.push(px[2]); // R
        ppm.push(px[1]); // G
        ppm.push(px[0]); // B
    }
    std::fs::File::create(&out).unwrap().write_all(&ppm).unwrap();
    eprintln!("wrote {out} ({w}x{h}) phase={phase} scale={scale:.2}");
}
