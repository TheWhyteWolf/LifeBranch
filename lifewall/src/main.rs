// SPDX-License-Identifier: GPL-3.0-or-later
// lifewall — Conway's Game of Life as a smooth terminal wallpaper.
//
// The simulation ticks at --tick seconds per generation; rendering runs at
// --fps, interpolating every cell's colour continuously along its timeline:
//   birth:  background -> newborn        (over 1 generation)
//   youth:  newborn    -> mature         (over --fade generations)
//   death:  colour-at-death -> background (over --fade generations)
// Only cells whose quantized colour changed since the last frame are redrawn,
// so steady-state output stays small even at 30 fps.

use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static QUIT: AtomicBool = AtomicBool::new(false);
static WINCH: AtomicBool = AtomicBool::new(false);
static RESEED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(sig: libc::c_int) {
    if sig == libc::SIGWINCH {
        WINCH.store(true, Ordering::Relaxed);
    } else if sig == libc::SIGUSR1 {
        RESEED.store(true, Ordering::Relaxed); // on-demand board reset
    } else {
        QUIT.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
struct Rgb([f64; 3]);

impl Rgb {
    fn to_u8(self) -> [u8; 3] {
        [self.0[0] as u8, self.0[1] as u8, self.0[2] as u8]
    }
}

fn blend(a: Rgb, b: Rgb, t: f64) -> Rgb {
    // Quantize so a fading cell changes colour ~16 times per phase, not every
    // frame — the diff renderer then skips it on most frames.
    let t = ((t.clamp(0.0, 1.0) * 16.0).round()) / 16.0;
    Rgb([
        a.0[0] + (b.0[0] - a.0[0]) * t,
        a.0[1] + (b.0[1] - a.0[1]) * t,
        a.0[2] + (b.0[2] - a.0[2]) * t,
    ])
}

struct Config {
    tick: f64,         // seconds per generation
    fps: f64,          // render frames per second
    fade: f64,         // generations for newborn->mature and death->bg fades
    density: f64,      // seed fill fraction
    glyphs: Vec<char>, // character(s) for live cells; 2+ = random per cell
    bg: Rgb,
    mature: Rgb,
    newborn: Rgb,
    stale_hold: f64,      // seconds a settled board may oscillate before reseed
    min_pop: f64,         // reseed below this alive fraction
    glider_interval: f64, // mean seconds between glider clusters; <=0 disables
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tick: 0.3,
            fps: 30.0,
            fade: 3.0,
            density: 0.14,
            // Printable ASCII, space excluded. A single repeated glyph gives
            // the board no texture; this needs no font beyond the terminal's
            // own and matches what lifeconf generates by default.
            glyphs: ('!'..='~').collect(),
            bg: Rgb([18.0, 20.0, 18.0]),        // #121412
            mature: Rgb([102.0, 116.0, 76.0]),  // #66744c
            newborn: Rgb([135.0, 165.0, 64.0]), // #87a540
            stale_hold: 20.0,
            min_pop: 0.005,
            glider_interval: 90.0,
        }
    }
}

// xorshift64* — deterministic, dependency-free.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15)
            | 1;
        Rng(seed)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

// The classic 5-cell glider, as (row, col) offsets in a 3x3 box:
//   .#.
//   ..#
//   ###
// It's the simplest still-life-avoiding pattern in Conway's Life: it
// reproduces itself shifted by (1,1) every 4 generations, forever (the CA
// rules are rotation-symmetric, so rotating it gives a glider heading in the
// other 3 diagonals — no separate mirrored shape is needed).
const GLIDER: [(i32, i32); 5] = [(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)];

/// Rotate a set of cells `turns` quarter-turns clockwise within their 3x3 box.
fn rotate(cells: &[(i32, i32); 5], turns: u8) -> [(i32, i32); 5] {
    let mut out = *cells;
    for _ in 0..(turns % 4) {
        for c in out.iter_mut() {
            *c = (c.1, 2 - c.0);
        }
    }
    out
}

struct Board {
    w: usize,
    h: usize,
    alive: Vec<bool>,
    born: Vec<f64>, // generation the cell was (last) born; kept after death
    died: Vec<f64>, // generation the cell died; NAN when not fading out
    counts: Vec<u8>,
}

impl Board {
    fn new(w: usize, h: usize) -> Self {
        let n = w * h;
        Board {
            w,
            h,
            alive: vec![false; n],
            born: vec![0.0; n],
            died: vec![f64::NAN; n],
            counts: vec![0; n],
        }
    }

    fn population(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }

    // Crossfade reseed: surviving cells stay put, others fade out while the
    // fresh soup fades in.
    fn reseed(&mut self, gen: f64, density: f64, rng: &mut Rng) {
        for i in 0..self.alive.len() {
            let keep = rng.next_f64() < density;
            if self.alive[i] && !keep {
                self.alive[i] = false;
                self.died[i] = gen;
            } else if !self.alive[i] && keep {
                self.alive[i] = true;
                self.born[i] = gen;
                self.died[i] = f64::NAN;
            }
        }
    }

    // One B3/S23 generation on a torus. `gen` stamps births/deaths.
    fn step(&mut self, gen: f64, fade: f64) {
        let (w, h) = (self.w, self.h);
        self.counts.fill(0);
        for y in 0..h {
            let ym1 = (y + h - 1) % h * w;
            let y0 = y * w;
            let yp1 = (y + 1) % h * w;
            for x in 0..w {
                if !self.alive[y0 + x] {
                    continue;
                }
                let xm1 = (x + w - 1) % w;
                let xp1 = (x + 1) % w;
                for row in [ym1, y0, yp1] {
                    self.counts[row + xm1] += 1;
                    self.counts[row + x] += 1;
                    self.counts[row + xp1] += 1;
                }
                self.counts[y0 + x] -= 1; // undo self-count
            }
        }
        for i in 0..self.alive.len() {
            let a = self.alive[i];
            let n = self.counts[i];
            if n == 3 || (a && n == 2) {
                if !a {
                    self.alive[i] = true;
                    self.born[i] = gen;
                    self.died[i] = f64::NAN; // rebirth cancels any fade-out
                }
            } else if a {
                self.alive[i] = false;
                self.died[i] = gen;
            } else if !self.died[i].is_nan() && gen - self.died[i] > fade + 1.0 {
                self.died[i] = f64::NAN; // fade finished; stop computing it
            }
        }
    }

    fn hash(&self) -> u64 {
        let mut hsh = 0xcbf29ce484222325u64;
        for (i, &a) in self.alive.iter().enumerate() {
            if a {
                hsh = (hsh ^ i as u64).wrapping_mul(0x100000001b3);
            }
        }
        hsh
    }

    // Colour of the cell as a continuous function of the fractional generation.
    fn color_at(&self, i: usize, gen_f: f64, cfg: &Config) -> [u8; 3] {
        let live_color = |age: f64| -> Rgb {
            if age < 1.0 {
                blend(cfg.bg, cfg.newborn, age)
            } else if age < 1.0 + cfg.fade {
                blend(cfg.newborn, cfg.mature, (age - 1.0) / cfg.fade)
            } else {
                cfg.mature
            }
        };
        if self.alive[i] {
            live_color(gen_f - self.born[i]).to_u8()
        } else if !self.died[i].is_nan() {
            let dying = gen_f - self.died[i];
            if dying < cfg.fade {
                let at_death = live_color(self.died[i] - self.born[i]);
                blend(at_death, cfg.bg, dying / cfg.fade).to_u8()
            } else {
                cfg.bg.to_u8()
            }
        } else {
            cfg.bg.to_u8()
        }
    }

    // Which glyph a cell draws, when there's more than one to choose from.
    // Keyed by (index, birth generation) so it's stable for the cell's whole
    // life — like colour, it only changes on rebirth — without needing extra
    // per-cell storage.
    fn glyph_at(&self, i: usize, cfg: &Config) -> char {
        if cfg.glyphs.len() <= 1 {
            return cfg.glyphs[0];
        }
        let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ self.born[i].to_bits();
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51afd7ed558ccd);
        h ^= h >> 33;
        cfg.glyphs[(h as usize) % cfg.glyphs.len()]
    }

    // Drop a small swarm (1-3) of gliders, each launched from near a random
    // edge in a random heading — a gentle, continuous nudge so the board
    // keeps moving instead of settling into still lifes/oscillators and
    // sitting there until the (much more disruptive) full reseed kicks in.
    // Doesn't touch anything already on the board.
    fn spawn_gliders(&mut self, gen: f64, rng: &mut Rng) {
        let count = 1 + (rng.next_f64() * 3.0) as i32; // 1..=3
        for _ in 0..count {
            let heading = (rng.next_f64() * 4.0) as u8;
            let cells = rotate(&GLIDER, heading);
            let w = self.w as i32;
            let h = self.h as i32;
            // Anchor in the outer margin of a random edge so it reads as
            // arriving from offscreen before the torus wraps it around with
            // everything else.
            let (ax, ay) = match (rng.next_f64() * 4.0) as u32 {
                0 => ((rng.next_f64() * w as f64) as i32, 0),
                1 => ((rng.next_f64() * w as f64) as i32, h - 3),
                2 => (0, (rng.next_f64() * h as f64) as i32),
                _ => (w - 3, (rng.next_f64() * h as f64) as i32),
            };
            for (dr, dc) in cells {
                let x = (ax + dc).rem_euclid(w) as usize;
                let y = (ay + dr).rem_euclid(h) as usize;
                let i = y * self.w + x;
                self.alive[i] = true;
                self.born[i] = gen;
                self.died[i] = f64::NAN;
            }
        }
    }
}

fn term_size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
    }
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    (env("COLUMNS", 80), env("LINES", 24))
}

fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Rgb([
        ((v >> 16) & 0xff) as f64,
        ((v >> 8) & 0xff) as f64,
        (v & 0xff) as f64,
    ]))
}

fn parse_args() -> Config {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1);
    let usage = "lifewall — Conway's Game of Life terminal wallpaper\n\
        Run inside `kitten panel --edge=background --config NONE lifewall`\n\
        (or any terminal / any layer-shell panel that runs a command).\n\n\
        --tick SECS     seconds per generation        (default 0.3)\n\
        --fps N         render frames per second      (default 30)\n\
        --fade GENS     fade length in generations    (default 3)\n\
        --density F     seed fill fraction 0..1       (default 0.14)\n\
        --char S        glyph(s) for live cells; 2+ chars picks randomly\n\
        \x20               per cell        (default: printable ASCII)\n\
        --bg HEX        background colour             (default #121412)\n\
        --mature HEX    settled cell colour           (default #66744c)\n\
        --newborn HEX   birth flash colour            (default #87a540)\n\
        --glider-interval SECS  mean seconds between glider clusters;\n\
        \x20                       0 disables                   (default 90)\n";
    while let Some(a) = args.next() {
        let mut val = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("missing value for {name}");
                std::process::exit(2);
            })
        };
        match a.as_str() {
            "--tick" => cfg.tick = val("--tick").parse().unwrap_or(cfg.tick),
            "--fps" => cfg.fps = val("--fps").parse().unwrap_or(cfg.fps),
            "--fade" => cfg.fade = val("--fade").parse().unwrap_or(cfg.fade),
            "--density" => cfg.density = val("--density").parse().unwrap_or(cfg.density),
            "--char" => {
                // Control characters (e.g. a stray ESC) would inject raw escape
                // sequences into our own output stream, so drop them.
                let v: Vec<char> = val("--char").chars().filter(|c| !c.is_control()).collect();
                if !v.is_empty() {
                    cfg.glyphs = v;
                }
            }
            "--glider-interval" => {
                cfg.glider_interval = val("--glider-interval")
                    .parse()
                    .unwrap_or(cfg.glider_interval)
            }
            "--bg" => cfg.bg = parse_hex(&val("--bg")).unwrap_or(cfg.bg),
            "--mature" => cfg.mature = parse_hex(&val("--mature")).unwrap_or(cfg.mature),
            "--newborn" => cfg.newborn = parse_hex(&val("--newborn")).unwrap_or(cfg.newborn),
            "--help" | "-h" => {
                print!("{usage}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown flag {other}\n\n{usage}");
                std::process::exit(2);
            }
        }
    }
    cfg.tick = cfg.tick.max(0.01);
    cfg.fps = cfg.fps.clamp(1.0, 240.0);
    cfg.fade = cfg.fade.max(0.25);
    cfg
}

// A randomized gap before the next glider cluster: roughly 0.67x-1.33x the
// configured mean, so spawns don't fall into an obvious metronomic beat.
fn next_glider_delay(rng: &mut Rng, interval: f64) -> Duration {
    Duration::from_secs_f64(interval * (0.67 + rng.next_f64() * 0.66))
}

// Append redraw commands for every cell whose colour changed. SGR state
// persists across cursor moves, so fg is tracked across the whole frame.
fn render(
    board: &Board,
    gen_f: f64,
    cfg: &Config,
    prev: &mut [[u8; 3]],
    cur_fg: &mut Option<[u8; 3]>,
    buf: &mut String,
) {
    use std::fmt::Write as _;
    let bg = cfg.bg.to_u8();
    for y in 0..board.h {
        let mut in_run = false;
        for x in 0..board.w {
            let i = y * board.w + x;
            let col = board.color_at(i, gen_f, cfg);
            if col == prev[i] {
                in_run = false;
                continue;
            }
            prev[i] = col;
            if !in_run {
                let _ = write!(buf, "\x1b[{};{}H", y + 1, x + 1);
                in_run = true;
            }
            if col == bg {
                buf.push(' '); // spaces paint the (already set) background
            } else {
                if *cur_fg != Some(col) {
                    let _ = write!(buf, "\x1b[38;2;{};{};{}m", col[0], col[1], col[2]);
                    *cur_fg = Some(col);
                }
                buf.push(board.glyph_at(i, cfg));
            }
        }
    }
}

fn main() {
    let cfg = parse_args();
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGWINCH, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGUSR1, on_signal as *const () as libc::sighandler_t);
    }

    let mut rng = Rng::new();
    let (mut w, mut h) = term_size();
    let mut board = Board::new(w, h);
    board.reseed(0.0, cfg.density, &mut rng);

    let bg = cfg.bg.to_u8();
    let mut prev = vec![bg; w * h];
    let mut seen: HashSet<u64> = HashSet::new();
    let mut order: VecDeque<u64> = VecDeque::new();
    let mut stale_since: Option<Instant> = None;
    let mut cur_fg: Option<[u8; 3]> = None;
    let mut buf = String::with_capacity(1 << 16);

    let mut out = std::io::stdout().lock();
    let bg_sgr = format!("\x1b[48;2;{};{};{}m", bg[0], bg[1], bg[2]);
    let _ = write!(out, "\x1b[?25l{bg_sgr}\x1b[2J");

    let start = Instant::now();
    let frame = Duration::from_secs_f64(1.0 / cfg.fps);
    let mut next_frame = start;
    let mut gen: u64 = 0;
    let mut next_glider = (cfg.glider_interval > 0.0)
        .then(|| start + next_glider_delay(&mut rng, cfg.glider_interval));

    'outer: while !QUIT.load(Ordering::Relaxed) {
        // Resize: rebuild the board and repaint from scratch.
        if WINCH.swap(false, Ordering::Relaxed) {
            let (nw, nh) = term_size();
            if (nw, nh) != (w, h) {
                (w, h) = (nw, nh);
                board = Board::new(w, h);
                board.reseed(gen as f64, cfg.density, &mut rng);
                prev = vec![bg; w * h];
                seen.clear();
                order.clear();
                stale_since = None;
                cur_fg = None;
                let _ = write!(out, "{bg_sgr}\x1b[2J");
            }
        }

        let gen_f = start.elapsed().as_secs_f64() / cfg.tick;

        // After a long pause (SIGSTOP from the wallpaper toggle) the wall clock
        // has raced ahead — resync rather than burst-simulate the missed time.
        if gen_f - gen as f64 > 4.0 {
            gen = gen_f as u64;
        }

        // SIGUSR1 (Mod+Ctrl+G): crossfade into a fresh soup right now.
        if RESEED.swap(false, Ordering::Relaxed) {
            board.reseed(gen_f, cfg.density, &mut rng);
            seen.clear();
            order.clear();
            stale_since = None;
        }

        // A glider cluster is additive (unlike reseed) and almost always
        // changes the board hash on its own, so it keeps `seen` from ever
        // matching again — a much gentler stagnation-buster than a full
        // reseed, which stays as the backstop if the gliders die out.
        if let Some(t) = next_glider {
            if Instant::now() >= t {
                board.spawn_gliders(gen_f, &mut rng);
                next_glider =
                    Some(Instant::now() + next_glider_delay(&mut rng, cfg.glider_interval));
            }
        }

        // Advance the simulation through any generation boundaries we crossed.
        while (gen + 1) as f64 <= gen_f {
            gen += 1;
            board.step(gen as f64, cfg.fade);

            let mut reseed = board.population() < (cfg.min_pop * (w * h) as f64) as usize;
            let key = board.hash();
            if seen.contains(&key) {
                let since = *stale_since.get_or_insert_with(Instant::now);
                if since.elapsed().as_secs_f64() >= cfg.stale_hold {
                    reseed = true;
                }
            } else {
                stale_since = None;
                seen.insert(key);
                order.push_back(key);
                if order.len() > 600 {
                    if let Some(old) = order.pop_front() {
                        seen.remove(&old);
                    }
                }
            }
            if reseed {
                board.reseed(gen as f64, cfg.density, &mut rng);
                seen.clear();
                order.clear();
                stale_since = None;
            }
        }

        buf.clear();
        render(&board, gen_f, &cfg, &mut prev, &mut cur_fg, &mut buf);
        if !buf.is_empty() {
            if write!(out, "{bg_sgr}{buf}").is_err() || out.flush().is_err() {
                break 'outer; // panel closed under us
            }
        }

        next_frame += frame;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now; // fell behind; don't try to catch up
        }
    }

    let _ = write!(out, "\x1b[0m\x1b[?25h\x1b[2J\x1b[H");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_with(w: usize, h: usize, cells: &[(usize, usize)]) -> Board {
        let mut b = Board::new(w, h);
        for &(x, y) in cells {
            b.alive[y * w + x] = true;
        }
        b
    }

    fn alive_set(b: &Board) -> Vec<(usize, usize)> {
        let mut v: Vec<_> = (0..b.alive.len())
            .filter(|&i| b.alive[i])
            .map(|i| (i % b.w, i / b.w))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn blinker_oscillates() {
        let mut b = board_with(5, 5, &[(2, 1), (2, 2), (2, 3)]);
        b.step(1.0, 3.0);
        assert_eq!(alive_set(&b), vec![(1, 2), (2, 2), (3, 2)]);
        b.step(2.0, 3.0);
        assert_eq!(alive_set(&b), vec![(2, 1), (2, 2), (2, 3)]);
    }

    #[test]
    fn block_is_still_and_lone_cell_dies() {
        let mut b = board_with(6, 6, &[(1, 1), (1, 2), (2, 1), (2, 2)]);
        b.step(1.0, 3.0);
        assert_eq!(alive_set(&b), vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
        let mut lone = board_with(5, 5, &[(2, 2)]);
        lone.step(1.0, 3.0);
        assert!(alive_set(&lone).is_empty());
    }

    #[test]
    fn torus_wraps_at_corner() {
        // Three cells around the corner birth the fourth across the seams.
        let mut b = board_with(9, 9, &[(0, 0), (8, 0), (0, 8)]);
        b.step(1.0, 3.0);
        assert!(alive_set(&b).contains(&(8, 8)));
    }

    #[test]
    fn rebirth_cancels_fade_and_death_starts_it() {
        let mut b = board_with(5, 5, &[(2, 1), (2, 2), (2, 3)]);
        b.step(1.0, 3.0);
        let i = 1 * 5 + 2; // (2,1) died at gen 1
        assert!(!b.alive[i] && b.died[i] == 1.0);
        b.step(2.0, 3.0);
        assert!(b.alive[i] && b.died[i].is_nan()); // reborn -> fade cancelled
    }

    #[test]
    fn glider_survives_and_translates() {
        // A real glider returns to 5 live cells, shifted by (1,1), every 4
        // generations. If the coordinates or rotation math were wrong, this
        // would decay instead of stabilizing at 5 — catches that silently.
        for heading in 0..4u8 {
            let mut b = Board::new(20, 20);
            for (dr, dc) in rotate(&GLIDER, heading) {
                let x = (5 + dc) as usize;
                let y = (5 + dr) as usize;
                b.alive[y * 20 + x] = true;
            }
            assert_eq!(b.population(), 5, "heading {heading}");
            for g in 1..=4 {
                b.step(g as f64, 3.0);
            }
            assert_eq!(b.population(), 5, "heading {heading} after one period");
        }
    }

    #[test]
    fn spawn_gliders_adds_without_disturbing_existing() {
        let mut b = board_with(30, 30, &[(1, 1), (1, 2), (2, 1), (2, 2)]); // a block
        let mut rng = Rng::new();
        b.spawn_gliders(0.0, &mut rng);
        // The pre-existing block survives untouched...
        assert!(b.alive[1 * 30 + 1] && b.alive[2 * 30 + 2]);
        // ...and at least one glider (5 cells) was actually added.
        assert!(b.population() >= 4 + 5);
    }

    #[test]
    fn colour_timeline() {
        let cfg = Config::default();
        let mut b = board_with(3, 3, &[(1, 1)]);
        let i = 1 * 3 + 1;
        b.born[i] = 0.0;
        assert_eq!(b.color_at(i, 0.0, &cfg), cfg.bg.to_u8()); // birth starts at bg
        assert_eq!(b.color_at(i, 1.0, &cfg), cfg.newborn.to_u8()); // full flash
        assert_eq!(b.color_at(i, 1.0 + cfg.fade, &cfg), cfg.mature.to_u8());
        b.alive[i] = false;
        b.died[i] = 10.0; // died mature
        assert_eq!(b.color_at(i, 10.0, &cfg), cfg.mature.to_u8());
        assert_eq!(b.color_at(i, 10.0 + cfg.fade, &cfg), cfg.bg.to_u8());
    }
}
