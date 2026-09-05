// SPDX-License-Identifier: GPL-3.0-or-later
// Game of Life engine — copied from ../lifewall/src/main.rs (same repo, MIT).
// Board, Rng, and the colour timeline are byte-compatible with the wallpaper so
// the lock screen's cube churns exactly like the desktop behind it.
//
// Timeline (continuous in fractional generations, `gen_f = elapsed / tick`):
//   birth:  bg -> newborn          (over 1 generation)
//   youth:  newborn -> mature      (over `fade` generations)
//   death:  colour-at-death -> bg  (over `fade` generations)

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub struct Rgb(pub [f64; 3]);

impl Rgb {
    pub fn to_u8(self) -> [u8; 3] {
        [
            self.0[0].clamp(0.0, 255.0) as u8,
            self.0[1].clamp(0.0, 255.0) as u8,
            self.0[2].clamp(0.0, 255.0) as u8,
        ]
    }

    pub fn scaled(self, k: f64) -> Rgb {
        Rgb([self.0[0] * k, self.0[1] * k, self.0[2] * k])
    }
}

pub fn blend(a: Rgb, b: Rgb, t: f64) -> Rgb {
    // Quantize so a fading cell changes colour ~16 times per phase, not every
    // frame (kept from lifewall; also keeps the two renderers in visual sync).
    let t = ((t.clamp(0.0, 1.0) * 16.0).round()) / 16.0;
    Rgb([
        a.0[0] + (b.0[0] - a.0[0]) * t,
        a.0[1] + (b.0[1] - a.0[1]) * t,
        a.0[2] + (b.0[2] - a.0[2]) * t,
    ])
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
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

/// Palette + dynamics for one simulation (the lock screen runs four: three
/// cube faces and the ember backdrop, each with its own LifeCfg).
pub struct LifeCfg {
    pub tick: f64,    // seconds per generation
    pub fade: f64,    // generations for newborn->mature and death->bg fades
    pub density: f64, // seed fill fraction
    pub bg: Rgb,
    pub mature: Rgb,
    pub newborn: Rgb,
    pub stale_hold: f64, // seconds a settled board may oscillate before reseed
    pub min_pop: f64,    // reseed below this alive fraction
}

impl Default for LifeCfg {
    fn default() -> Self {
        LifeCfg {
            tick: 0.4,
            fade: 3.0,
            density: 0.18,
            bg: Rgb([0.0, 0.0, 0.0]),           // lock screen: true black
            mature: Rgb([102.0, 116.0, 76.0]),  // #66744c
            newborn: Rgb([135.0, 165.0, 64.0]), // #87a540
            stale_hold: 12.0,
            min_pop: 0.02,
        }
    }
}

// xorshift64* — deterministic, dependency-free (from lifewall).
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15)
            | 1;
        Rng(seed)
    }

    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in [0, n). n must be > 0.
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_f64() * n as f64) as usize).min(n - 1)
    }
}

/// Where a cell is on its life timeline — drives glyph choice (░▒▓█) while
/// `color_at` drives its colour. Both read the same born/died stamps.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CellPhase {
    Dead,
    /// Alive; age in generations since (re)birth.
    Live(f64),
    /// Fading out; 0.0 = just died, 1.0 = fade complete.
    Dying(f64),
}

pub struct Board {
    pub w: usize,
    pub h: usize,
    pub alive: Vec<bool>,
    pub born: Vec<f64>, // generation the cell was (last) born; kept after death
    pub died: Vec<f64>, // generation the cell died; NAN when not fading out
    counts: Vec<u8>,
}

impl Board {
    pub fn new(w: usize, h: usize) -> Self {
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

    pub fn population(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }

    // Crossfade reseed: surviving cells stay put, others fade out while the
    // fresh soup fades in.
    pub fn reseed(&mut self, gen: f64, density: f64, rng: &mut Rng) {
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

    /// Kill every live cell at once (the wrong-password effect); each starts
    /// its normal fade-out from whatever colour it had.
    pub fn kill_all(&mut self, gen: f64) {
        for i in 0..self.alive.len() {
            if self.alive[i] {
                self.alive[i] = false;
                self.died[i] = gen;
            }
        }
    }

    // One B3/S23 generation on a torus. `gen` stamps births/deaths.
    pub fn step(&mut self, gen: f64, fade: f64) {
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

    pub fn hash(&self) -> u64 {
        let mut hsh = 0xcbf29ce484222325u64;
        for (i, &a) in self.alive.iter().enumerate() {
            if a {
                hsh = (hsh ^ i as u64).wrapping_mul(0x100000001b3);
            }
        }
        hsh
    }

    // Colour of the cell as a continuous function of the fractional generation.
    pub fn color_at(&self, i: usize, gen_f: f64, cfg: &LifeCfg) -> Rgb {
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
            live_color(gen_f - self.born[i])
        } else if !self.died[i].is_nan() {
            let dying = gen_f - self.died[i];
            if dying < cfg.fade {
                let at_death = live_color(self.died[i] - self.born[i]);
                blend(at_death, cfg.bg, dying / cfg.fade)
            } else {
                cfg.bg
            }
        } else {
            cfg.bg
        }
    }

    /// Timeline phase of the cell (glyph selection); mirrors color_at's cases.
    pub fn phase_at(&self, i: usize, gen_f: f64, cfg: &LifeCfg) -> CellPhase {
        if self.alive[i] {
            CellPhase::Live(gen_f - self.born[i])
        } else if !self.died[i].is_nan() {
            let dying = gen_f - self.died[i];
            if dying < cfg.fade {
                CellPhase::Dying(dying / cfg.fade)
            } else {
                CellPhase::Dead
            }
        } else {
            CellPhase::Dead
        }
    }
}

/// One running simulation: a Board plus the liveliness bookkeeping lifewall
/// keeps in its main loop (stale-cycle detection, minimum population).
pub struct Sim {
    pub board: Board,
    pub cfg: LifeCfg,
    gen: u64,
    phase: f64, // per-sim offset so the four sims don't tick in lockstep
    seen: std::collections::HashSet<u64>,
    order: std::collections::VecDeque<u64>,
    stale_since: Option<f64>, // in seconds of scene time
}

impl Sim {
    pub fn new(w: usize, h: usize, cfg: LifeCfg, phase: f64, rng: &mut Rng) -> Self {
        let mut board = Board::new(w, h);
        board.reseed(0.0, cfg.density, rng);
        Sim {
            board,
            cfg,
            gen: 0,
            phase,
            seen: std::collections::HashSet::new(),
            order: std::collections::VecDeque::new(),
            stale_since: None,
        }
    }

    pub fn gen_f(&self, t: f64) -> f64 {
        (t + self.phase) / self.cfg.tick
    }

    /// Crossfade into a fresh soup right now.
    pub fn reseed(&mut self, t: f64, rng: &mut Rng) {
        let gen_f = self.gen_f(t);
        self.board.reseed(gen_f, self.cfg.density, rng);
        self.seen.clear();
        self.order.clear();
        self.stale_since = None;
    }

    pub fn kill_all(&mut self, t: f64) {
        self.board.kill_all(self.gen_f(t));
        self.seen.clear();
        self.order.clear();
        self.stale_since = None;
    }

    /// Advance through any generation boundaries crossed by scene time `t`
    /// (seconds). Ported from lifewall's main loop, including the
    /// resync-after-pause guard and the auto-reseed rules.
    pub fn advance(&mut self, t: f64, rng: &mut Rng) {
        let gen_f = self.gen_f(t);

        // After a long stall (frame callbacks stop while monitors are off) the
        // wall clock has raced ahead — resync rather than burst-simulate.
        if gen_f - self.gen as f64 > 4.0 {
            self.gen = gen_f as u64;
        }

        while (self.gen + 1) as f64 <= gen_f {
            self.gen += 1;
            self.board.step(self.gen as f64, self.cfg.fade);

            let n = self.board.alive.len();
            let mut reseed = self.board.population() < (self.cfg.min_pop * n as f64) as usize;
            let key = self.board.hash();
            if self.seen.contains(&key) {
                let since = *self.stale_since.get_or_insert(t);
                if t - since >= self.cfg.stale_hold {
                    reseed = true;
                }
            } else {
                self.stale_since = None;
                self.seen.insert(key);
                self.order.push_back(key);
                if self.order.len() > 600 {
                    if let Some(old) = self.order.pop_front() {
                        self.seen.remove(&old);
                    }
                }
            }
            if reseed {
                self.board.reseed(self.gen as f64, self.cfg.density, rng);
                self.seen.clear();
                self.order.clear();
                self.stale_since = None;
            }
        }
    }
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
        let i = 5 + 2; // (2,1) died at gen 1
        assert!(!b.alive[i] && b.died[i] == 1.0);
        b.step(2.0, 3.0);
        assert!(b.alive[i] && b.died[i].is_nan()); // reborn -> fade cancelled
    }

    #[test]
    fn colour_timeline() {
        let cfg = LifeCfg::default();
        let mut b = board_with(3, 3, &[(1, 1)]);
        let i = 3 + 1;
        b.born[i] = 0.0;
        assert_eq!(b.color_at(i, 0.0, &cfg).to_u8(), cfg.bg.to_u8()); // birth starts at bg
        assert_eq!(b.color_at(i, 1.0, &cfg).to_u8(), cfg.newborn.to_u8()); // full flash
        assert_eq!(b.color_at(i, 1.0 + cfg.fade, &cfg).to_u8(), cfg.mature.to_u8());
        b.alive[i] = false;
        b.died[i] = 10.0; // died mature
        assert_eq!(b.color_at(i, 10.0, &cfg).to_u8(), cfg.mature.to_u8());
        assert_eq!(b.color_at(i, 10.0 + cfg.fade, &cfg).to_u8(), cfg.bg.to_u8());
    }

    #[test]
    fn phase_mirrors_colour_cases() {
        let cfg = LifeCfg::default();
        let mut b = board_with(3, 3, &[(1, 1)]);
        let i = 3 + 1;
        b.born[i] = 0.0;
        assert_eq!(b.phase_at(i, 2.0, &cfg), CellPhase::Live(2.0));
        b.alive[i] = false;
        b.died[i] = 10.0;
        assert_eq!(b.phase_at(i, 10.0, &cfg), CellPhase::Dying(0.0));
        assert_eq!(b.phase_at(i, 10.0 + cfg.fade + 0.1, &cfg), CellPhase::Dead);
        assert_eq!(b.phase_at(0, 0.0, &cfg), CellPhase::Dead);
    }

    #[test]
    fn kill_all_starts_fades() {
        let mut b = board_with(4, 4, &[(0, 0), (1, 1), (2, 2)]);
        b.kill_all(5.0);
        assert_eq!(b.population(), 0);
        assert!(b.died.iter().filter(|d| !d.is_nan()).count() == 3);
    }
}
