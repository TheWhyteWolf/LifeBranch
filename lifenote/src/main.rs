// SPDX-License-Identifier: GPL-3.0-or-later
// lifenote — a freedesktop notification daemon for niri.
//
// Fourth of the life* family (lifelock/lifegreet/lifewall). Popups are pure
// text: real box-drawing frames rasterized as glyphs on a semitransparent
// layer-shell surface, muted-olive like everything else in the rice. One
// binary, three modes: the daemon, `lifenote ctl …` (client), and a
// debug-only offline render harness.

mod app;
mod bus;
mod cli;
mod ctl;
mod layout;
mod render;
mod state;
mod text;

fn main() {
    // Client mode: `lifenote ctl …` — dispatched before anything else so the
    // control CLI never touches fonts or Wayland (lifelock's auth-child
    // dispatch pattern).
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("ctl") {
        std::process::exit(ctl::run(&argv[2..]));
    }
    // Debug-only: rasterize a sample stack (one popup per border style, one
    // critical) to a PPM over a checkerboard — frame joins and premultiplied
    // alpha are inspected offline, before any Wayland or DBus code runs.
    //   lifenote --render-ppm [OUT.ppm]
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--render-ppm") {
        render_ppm();
        return;
    }

    // Debug-only: drive the layer-shell UI with staged fake notifications,
    // no DBus — mako can keep running while this is tested live.
    //   lifenote --selftest [flags]
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--selftest") {
        std::process::exit(selftest());
    }

    // Daemon mode.
    let cfg = cli::parse();
    let (tx, rx) = smithay_client_toolkit::reexports::calloop::channel::channel();
    let dnd = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let history = state::new_history();
    let bus = match bus::start(tx, dnd, history.clone()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("lifenote: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("lifenote: serving org.freedesktop.Notifications");
    let code = app::run(
        cfg,
        rx,
        Box::new(move |id, reason| bus.emit_closed(id, reason)),
        history,
    );
    std::process::exit(code);
}

#[cfg(debug_assertions)]
fn selftest() -> i32 {
    use smithay_client_toolkit::reexports::calloop::channel;
    use state::{AppEvent, NoteData, Urgency};

    // Shift args so `--selftest --border-style rounded` works.
    let cfg = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // cli::parse reads std::env::args; simplest here is to re-exec-style
        // strip: selftest only honours the config file + defaults.
        let _ = args;
        cli::Cfg::default()
    };

    let (tx, rx) = channel::channel::<AppEvent>();
    std::thread::spawn(move || {
        let note = |id, app: &str, summary: &str, body: &str, urgency, timeout_ms| {
            AppEvent::Notify(NoteData {
                id,
                app: app.into(),
                summary: summary.into(),
                body: body.into(),
                urgency,
                timeout_ms,
            })
        };
        let sleep = |s| std::thread::sleep(std::time::Duration::from_millis(s));
        let _ = tx.send(note(1, "firefox", "Download finished",
            "rice-wallpaper.png saved to ~/Downloads", Urgency::Normal, -1));
        sleep(800);
        let _ = tx.send(note(2, "battery", "5% remaining — plug in now", "",
            Urgency::Critical, -1));
        sleep(800);
        let _ = tx.send(note(3, "mpd", "Now playing",
            "Boards of Canada — Olson", Urgency::Low, 10_000));
        sleep(1500);
        // Replace-by-id: popup 1 must update in place, not jump the stack.
        let _ = tx.send(note(1, "firefox", "Second download finished",
            "one more thing saved to ~/Downloads", Urgency::Normal, -1));
        sleep(2000);
        // Explicit close of the mpd popup.
        let _ = tx.send(AppEvent::Close(3));
        eprintln!("lifenote selftest: events sent — click the critical one to dismiss it");
        // Keep the channel open; the app exits when the user Ctrl-Cs.
        loop {
            sleep(60_000);
        }
    });

    app::run(
        cfg,
        rx,
        Box::new(|id, reason| eprintln!("lifenote selftest: closed id={id} reason={reason}")),
        state::new_history(),
    )
}

#[cfg(debug_assertions)]
fn render_ppm() {
    use crate::layout::Style;
    use crate::render::frame;
    use crate::state::Urgency;
    use std::io::Write;

    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/lifenote.ppm".into());
    let cfg = cli::Cfg::default();
    let mut atlas = match render::Atlas::new(&cfg.font, cfg.font_px) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifenote: {e}");
            std::process::exit(1);
        }
    };

    let samples: [(&str, &str, &str, Style, Urgency); 6] = [
        ("firefox", "Download finished", "rice-wallpaper.png saved to ~/Downloads", Style::Single, Urgency::Normal),
        ("signal", "Wolfram", "the greeter froze again after DPMS. kidding! it's fine.", Style::Rounded, Urgency::Normal),
        ("mpd", "Now playing", "Boards of Canada — Olson", Style::Heavy, Urgency::Normal),
        ("notify-send", "plain rule, no body", "", Style::Ascii, Urgency::Low),
        ("battery", "5% remaining — plug in now", "", Style::Double, Urgency::Critical),
        ("wrap-test", "A summary that is deliberately much too long to fit on a single line at forty columns",
         "And a body with an unbreakable token: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa end.\nSecond paragraph.", Style::Single, Urgency::Normal),
    ];

    const GAP: usize = 8;
    let mut grids = Vec::new();
    for (app, summary, body, style, urgency) in samples {
        let s = text::wrap(&text::strip_markup(summary), cfg.max_width);
        let b_src = text::strip_markup(body);
        let mut b = if body.is_empty() { Vec::new() } else { text::wrap(&b_src, cfg.max_width) };
        b.truncate(cfg.max_lines);
        grids.push((layout::compose(app, &s, &b, style, cfg.max_width), urgency));
    }

    let width = grids
        .iter()
        .map(|(g, _)| frame::popup_width(g, &atlas))
        .max()
        .unwrap_or(0)
        + GAP * 2;
    let height = grids
        .iter()
        .map(|(g, _)| frame::popup_height(g, &atlas) + GAP)
        .sum::<usize>()
        + GAP;

    let mut canvas = vec![0u8; width * height * 4];
    let bg = frame::premul_bg(cfg.background, cfg.background_alpha);
    let mut y = GAP;
    for (grid, urgency) in &grids {
        // Right-aligned like the live top-right anchor.
        let x = width - GAP - frame::popup_width(grid, &atlas);
        let pal = frame::palette(&cfg, *urgency);
        frame::draw_popup(&mut canvas, width, height, x, y, grid, &mut atlas, &pal, bg);
        y += frame::popup_height(grid, &atlas) + GAP;
    }

    // Composite over a checkerboard (premultiplied src-over) so translucency
    // and any premultiplication mistakes are visible in the PPM.
    let mut ppm = Vec::with_capacity(width * height * 3 + 32);
    ppm.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
    for (i, px) in canvas.chunks_exact(4).enumerate() {
        let (x, y) = (i % width, i / width);
        let check = if (x / 16 + y / 16) % 2 == 0 { 96u32 } else { 44u32 };
        let inv = 255 - px[3] as u32;
        ppm.push((px[2] as u32 + check * inv / 255) as u8); // R
        ppm.push((px[1] as u32 + check * inv / 255) as u8); // G
        ppm.push((px[0] as u32 + check * inv / 255) as u8); // B
    }
    std::fs::File::create(&out).unwrap().write_all(&ppm).unwrap();
    eprintln!("wrote {out} ({width}x{height})");
}
