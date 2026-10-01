// SPDX-License-Identifier: GPL-3.0-or-later
mod app;
mod capture;
mod editor;
mod image;
mod png;
mod session;
mod shapes;

use image::Image;
use shapes::{Fonts, Shape};

const USAGE: &str = "lifeshot — screenshot + annotation overlay\n\n\
    lifeshot                  freeze the focused output, drag to select, mark up\n\
    lifeshot --full           start with the whole screen selected\n\
    lifeshot --output NAME    capture a specific output (default: the focused one)\n\
    lifeshot --save-dir DIR   where Save puts files (default ~/Pictures/Screenshots)\n\n\
    Select: drag an area; drag inside to move, drag the handles to resize;\n\
            Ctrl+A whole screen; arrows nudge (Shift = 10px)\n\
    Tools:  p pen · l line · a arrow · r box · o oval · m marker · t text ·\n\
            b blur · n numbered marker   (press again to go back to select)\n\
    Style:  1-6 colour · [ ] or wheel width · f solid/hollow · Shift constrains\n\
    Finish: Enter / Ctrl+C copy to clipboard · Ctrl+S save to a file ·\n\
            Ctrl+Z undo · Ctrl+Shift+Z redo · Esc / right-click back out\n";

const FONT: &str = "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf";

/// Debug aid: draw one of every shape on a synthetic backdrop through the real
/// export path and write a PNG — a privacy-safe visual check (no screen capture).
fn demo(out: &str) -> i32 {
    let mut base = Image::new(720, 420, 0xff17_1a14);
    for y in 0..420usize {
        for x in 0..720usize {
            // A faint grid and gradient so pixelate has something to average.
            let g = ((x * 255 / 720) as u32) << 8 | ((y * 120 / 420) as u32) << 16;
            base.px[y * 720 + x] = 0xff00_0000 | (g & 0x00ff_ff00) | 0x20 | if x % 40 == 0 || y % 40 == 0 { 0x10_1010 } else { 0 };
        }
    }
    let fonts = Fonts::load(FONT);
    let (red, green, yellow, white) = (0xa4_c94b, 0xc7_d17a, 0xd8_a038, 0xe8_e8e8);
    let shapes = vec![
        Shape::Rect { a: (30, 30), b: (200, 120), color: red, thick: 4, filled: false },
        Shape::Rect { a: (230, 30), b: (330, 120), color: green, thick: 4, filled: true },
        Shape::Ellipse { a: (360, 30), b: (500, 120), color: yellow, thick: 4, filled: false },
        Shape::Ellipse { a: (530, 30), b: (620, 120), color: red, thick: 4, filled: true },
        Shape::Line { a: (30, 160), b: (200, 200), color: white, thick: 3 },
        Shape::Arrow { a: (230, 200), b: (360, 150), color: red, thick: 5 },
        Shape::Arrow { a: (400, 150), b: (400, 230), color: green, thick: 3 },
        Shape::Pen { pts: (0..60).map(|i| (440 + i * 4, 190 + ((i as f32 * 0.5).sin() * 22.0) as i32)).collect(), color: yellow, thick: 4 },
        Shape::Marker { pts: vec![(40, 280), (300, 285)], color: yellow, thick: 22 },
        Shape::Text { at: (46, 270), text: "Marker over text".into(), color: white, px: 22.0 },
        Shape::Pixelate { a: (340, 260), b: (520, 380), block: 14 },
        Shape::Counter { at: (570, 280), n: 1, color: red, r: 16 },
        Shape::Counter { at: (610, 320), n: 2, color: red, r: 16 },
        Shape::Counter { at: (650, 360), n: 3, color: red, r: 16 },
        Shape::Text { at: (30, 330), text: "lifeshot: text, arrows, boxes".into(), color: green, px: 24.0 },
    ];
    let sel = image::Rect { x: 10, y: 10, w: 700, h: 400 };
    let img = editor::export(&base, sel, &shapes, fonts.as_ref());
    match std::fs::write(out, png::encode(&img)) {
        Ok(()) => {
            eprintln!("wrote {out} ({}x{})", img.w, img.h);
            0
        }
        Err(e) => {
            eprintln!("lifeshot: write {out}: {e}");
            1
        }
    }
}

/// Debug aid: capture the focused output and print only metadata (size, how many
/// distinct colours, mean brightness) — proof the capture path works without
/// anyone having to look at, or store, what was on screen.
fn capture_test() -> i32 {
    let conn = match smithay_client_toolkit::reexports::client::Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifeshot: cannot connect to Wayland: {e}");
            return 1;
        }
    };
    let name = capture::focused_output_name();
    let t = std::time::Instant::now();
    match capture::grab(&conn, name.as_deref()) {
        Ok(g) => {
            let n = g.image.px.len() as u64;
            let mean = g.image.px.iter().map(|p| ((p >> 16) & 0xff) as u64 + ((p >> 8) & 0xff) as u64 + (p & 0xff) as u64).sum::<u64>() / (3 * n.max(1));
            let mut seen = std::collections::HashSet::new();
            for p in g.image.px.iter().step_by(97) {
                seen.insert(*p);
            }
            eprintln!(
                "captured {} {}x{} in {:?}: mean brightness {mean}/255, ~{} distinct colours sampled",
                g.output_name, g.image.w, g.image.h, t.elapsed(), seen.len()
            );
            0
        }
        Err(e) => {
            eprintln!("lifeshot: capture failed: {e}");
            1
        }
    }
}

/// Debug aid: paint the overlay for a synthetic desktop with a selection, some
/// annotations and a shape mid-drag, and write it as a PNG (no Wayland, no
/// real screen content).
fn render_ui(out: &str) -> i32 {
    use session::{Key, Mods, Palette, Session};
    let (w, h, k) = (1400usize, 800usize, 1.5f32);
    let mut base = Image::new(w, h, 0xff12_1412);
    for y in 0..h {
        for x in 0..w {
            let band = if (y / 60) % 2 == 0 { 0x0a } else { 0x14 };
            base.px[y * w + x] = 0xff00_0000 | (band << 16) | ((0x1c + (x * 40 / w) as u32) << 8) | 0x14;
        }
    }
    // Fake "window" content so the selection has something to cover.
    for y in 220..560usize {
        for x in 330..1010usize {
            base.px[y * w + x] = if y < 250 { 0xff39_412b } else { 0xff1c_2118 };
        }
    }
    let fonts = Fonts::load(FONT);
    let mut s = Session::new(base, Palette::default(), fonts, k);
    let none = Mods::default();
    let (a, b) = ((300, 190), (1040, 600));
    s.pointer_down(a, none);
    s.pointer_move(b, none);
    s.pointer_up(b, none);
    for (key, from, to) in [('r', (360, 280), (560, 380)), ('a', (900, 300), (700, 420))] {
        s.key(Key::Char(key), none);
        s.pointer_down(from, none);
        s.pointer_move(to, none);
        s.pointer_up(to, none);
    }
    s.key(Key::Char('n'), none);
    s.pointer_down((420, 470), none);
    s.pointer_up((420, 470), none);
    s.pointer_down((480, 470), none);
    s.pointer_up((480, 470), none);
    s.key(Key::Char('2'), none);
    s.key(Key::Char('l'), none);
    s.pointer_down((600, 520), none);
    s.pointer_move((900, 540), none); // still dragging: the live preview
    let mut frame = Image::new(w, h, 0);
    s.paint(&mut frame);
    match std::fs::write(out, png::encode(&frame)) {
        Ok(()) => {
            eprintln!("wrote {out} ({w}x{h})");
            0
        }
        Err(e) => {
            eprintln!("lifeshot: write {out}: {e}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--demo") {
        std::process::exit(demo(args.get(i + 1).map(String::as_str).unwrap_or("lifeshot-demo.png")));
    }
    if let Some(i) = args.iter().position(|a| a == "--render-ui") {
        std::process::exit(render_ui(args.get(i + 1).map(String::as_str).unwrap_or("lifeshot-ui.png")));
    }
    if args.iter().any(|a| a == "--capture-test") {
        std::process::exit(capture_test());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return;
    }
    let mut a = app::Args { output: None, save_dir: None, full: false, selftest: false, selftest_actions: false };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--output" => a.output = it.next().cloned(),
            "--save-dir" => a.save_dir = it.next().map(std::path::PathBuf::from),
            "--full" => a.full = true,
            "--selftest" => a.selftest = true,
            "--selftest-actions" => {
                a.selftest = true;
                a.selftest_actions = true;
            }
            other => {
                eprintln!("lifeshot: unexpected argument {other:?}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    std::process::exit(app::run(a));
}
