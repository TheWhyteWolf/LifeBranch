// SPDX-License-Identifier: GPL-3.0-or-later
// Copied verbatim from ../lifelock/src/scene.rs (same repo). Fix bugs in both.
// The living scene: three face simulations + the ember backdrop, plus the
// transient visual effects driven by typing and authentication state.
// Pure except for wall-clock time passed in as `t` (seconds since start).

use crate::cli::Cfg;
use crate::geometry::{CellSource, GridMap, FACE_SHADE};
use crate::sim::{blend, CellPhase, LifeCfg, Rgb, Rng, Sim};

pub const FLARE_SECS: f64 = 0.8;
pub const DIM_SECS: f64 = 0.3;
pub const WRONG_SECS: f64 = 0.4;

/// Shade-block density steps (░ ▒ ▓ █), also the --solid-cells fill levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Glyph {
    Light,
    Medium,
    Dark,
    Full,
}

impl Glyph {
    pub fn ch(self) -> char {
        match self {
            Glyph::Light => '\u{2591}',  // ░
            Glyph::Medium => '\u{2592}', // ▒
            Glyph::Dark => '\u{2593}',   // ▓
            Glyph::Full => '\u{2588}',   // █
        }
    }

    pub fn coverage(self) -> f64 {
        match self {
            Glyph::Light => 0.25,
            Glyph::Medium => 0.5,
            Glyph::Dark => 0.75,
            Glyph::Full => 1.0,
        }
    }

    fn bump(self) -> Glyph {
        match self {
            Glyph::Light => Glyph::Medium,
            Glyph::Medium => Glyph::Dark,
            _ => Glyph::Full,
        }
    }
}

/// What one grid cell renders as this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellPaint {
    pub glyph: Glyph,
    pub color: [u8; 3],
}

struct Flare {
    panel: u8,
    color: Rgb,
    start: f64,
}

enum Effect {
    Dim { start: f64 },
    Wrong { start: f64 },
}

pub struct Scene {
    faces: [Sim; 3],
    ember: Option<Sim>,
    flares: Vec<Flare>,
    effect: Option<Effect>,
    rng: Rng,
    /// Verify wave runs while a submission is in flight.
    pub validating: bool,
    pub caps_lock: bool,
    accent: Rgb,
    urgent: Rgb,
    surface: Rgb,
}

impl Scene {
    pub fn new(cfg: &Cfg, map: &GridMap) -> Scene {
        let mut rng = Rng::new();

        let face_cfg = || LifeCfg {
            tick: cfg.tick,
            mature: cfg.mature,
            newborn: cfg.newborn,
            ..LifeCfg::default()
        };
        let faces = [0, 1, 2].map(|i| {
            let f = &map.faces[i];
            // Phase offsets desync the three faces' generation boundaries.
            Sim::new(f.board_w, f.board_h, face_cfg(), i as f64 * 0.137, &mut rng)
        });

        let ember = (!cfg.no_ember).then(|| {
            let ecfg = LifeCfg {
                tick: cfg.ember_tick,
                density: 0.10,
                mature: cfg.mature.scaled(cfg.ember_dim),
                newborn: cfg.newborn.scaled(cfg.ember_dim),
                stale_hold: 30.0,
                min_pop: 0.01,
                ..LifeCfg::default()
            };
            Sim::new(map.cols, map.rows, ecfg, 0.41, &mut rng)
        });

        Scene {
            faces,
            ember,
            flares: Vec::new(),
            effect: None,
            rng,
            validating: false,
            caps_lock: false,
            accent: cfg.accent,
            urgent: cfg.urgent,
            // Dim olive face "material": brighter than the ember field so the
            // cube silhouette always reads as a solid object above it.
            surface: cfg.mature.scaled(0.5),
        }
    }

    /// Advance simulations and expire transient effects.
    pub fn advance(&mut self, t: f64) {
        // Wrong-flash finished: crossfade all faces into fresh soup.
        if let Some(Effect::Wrong { start }) = self.effect {
            if t - start >= WRONG_SECS {
                self.effect = None;
                for f in &mut self.faces {
                    f.reseed(t, &mut self.rng);
                }
            }
        }
        if let Some(Effect::Dim { start }) = self.effect {
            if t - start >= DIM_SECS {
                self.effect = None;
            }
        }
        self.flares.retain(|f| t - f.start < FLARE_SECS);

        for f in &mut self.faces {
            f.advance(t, &mut self.rng);
        }
        if let Some(e) = &mut self.ember {
            e.advance(t, &mut self.rng);
        }
    }

    // ----- typing / auth events -> visuals ---------------------------------

    /// Keystroke: one random panel of the 27 flares up. The position comes
    /// from the scene RNG only — nothing about the password influences it.
    pub fn key_flare(&mut self, t: f64) {
        let panel = self.rng.below(27) as u8;
        self.flares.push(Flare { panel, color: self.accent, start: t });
    }

    /// Backspace: a rust flare.
    pub fn backspace_flare(&mut self, t: f64) {
        let panel = self.rng.below(27) as u8;
        self.flares.push(Flare { panel, color: self.urgent, start: t });
    }

    /// Buffer cleared: brief global dim.
    pub fn clear_dim(&mut self, t: f64) {
        self.effect = Some(Effect::Dim { start: t });
    }

    /// Wrong password: every cell dies at once under a rust flash; faces
    /// reseed when the flash expires (in advance()).
    pub fn wrong(&mut self, t: f64) {
        for f in &mut self.faces {
            f.kill_all(t);
        }
        self.flares.clear();
        self.effect = Some(Effect::Wrong { start: t });
    }

    // ----- per-cell sampling ------------------------------------------------

    pub fn sample(&self, src: CellSource, col: usize, row: usize, t: f64) -> Option<CellPaint> {
        match src {
            CellSource::Face { face, bx, by, panel } => self.sample_face(face, bx, by, panel, t),
            CellSource::Ember => self.sample_ember(col, row, t),
        }
    }

    fn sample_face(&self, face: u8, bx: u16, by: u16, panel: u8, t: f64) -> Option<CellPaint> {
        let sim = &self.faces[face as usize];
        let (bw, bh) = (sim.board.w, sim.board.h);
        let i = by as usize * bw + bx as usize;
        let gen_f = sim.gen_f(t);

        // Every face cell paints, so the cube always reads as a solid 3D
        // object: dead cells show a dim "material" surface, live cells glow
        // on top of it. Face shading (top/left/right) then gives the 3D form.
        let (mut color, mut glyph) = match sim.board.phase_at(i, gen_f, &sim.cfg) {
            CellPhase::Live(age) => {
                let g = if age < 0.5 {
                    Glyph::Medium
                } else if age < 1.0 {
                    Glyph::Dark
                } else {
                    Glyph::Full
                };
                (sim.board.color_at(i, gen_f, &sim.cfg), g)
            }
            CellPhase::Dying(x) => {
                let g = if x < 0.4 {
                    Glyph::Dark
                } else if x < 0.75 {
                    Glyph::Medium
                } else {
                    Glyph::Light
                };
                (sim.board.color_at(i, gen_f, &sim.cfg), g)
            }
            CellPhase::Dead => (self.surface, Glyph::Light), // dim face material
        };

        // Panel flares: brighten the whole 3x3 panel, dead material included.
        for fl in &self.flares {
            if fl.panel != panel {
                continue;
            }
            let x = ((t - fl.start) / FLARE_SECS).clamp(0.0, 1.0);
            let k = (1.0 - x) * (1.0 - x); // fast attack, eased decay
            color = blend(color, fl.color, 0.7 * k);
            if k > 0.5 {
                glyph = glyph.bump().bump();
            } else if k > 0.2 {
                glyph = glyph.bump();
            }
        }

        // Verify wave: a lime band sweeping diagonally while validating.
        if self.validating {
            let u = (bx as f64 + 0.5) / bw as f64;
            let v = (by as f64 + 0.5) / bh as f64;
            let w = 0.5 + 0.5 * (std::f64::consts::TAU * (t * 0.9 - (u + v) * 0.55 - face as f64 * 0.21)).sin();
            let k = 0.55 * w * w;
            color = blend(color, self.accent, k);
            if k > 0.35 {
                glyph = glyph.bump();
            }
        }

        // Global effects.
        match self.effect {
            Some(Effect::Wrong { start }) => {
                let pulse = 1.0 - ((t - start) / WRONG_SECS).clamp(0.0, 1.0);
                color = blend(color, self.urgent, 0.8 * pulse);
                if pulse > 0.5 {
                    glyph = glyph.bump();
                }
            }
            Some(Effect::Dim { start }) => {
                let x = ((t - start) / DIM_SECS).clamp(0.0, 1.0);
                color = color.scaled(0.5 + 0.5 * x);
            }
            None => {}
        }

        let shade = FACE_SHADE[face as usize];
        Some(CellPaint { glyph, color: color.scaled(shade).to_u8() })
    }

    fn sample_ember(&self, col: usize, row: usize, t: f64) -> Option<CellPaint> {
        let sim = self.ember.as_ref()?;
        let i = row * sim.board.w + col;
        if i >= sim.board.alive.len() {
            return None;
        }
        let gen_f = sim.gen_f(t);
        let glyph = match sim.board.phase_at(i, gen_f, &sim.cfg) {
            CellPhase::Live(age) => {
                if age < 1.0 {
                    Glyph::Light
                } else {
                    Glyph::Medium // embers stay airy: never ▓/█
                }
            }
            CellPhase::Dying(_) => Glyph::Light,
            CellPhase::Dead => return None,
        };
        let color = sim.board.color_at(i, gen_f, &sim.cfg).to_u8();
        if color == [0, 0, 0] {
            return None;
        }
        Some(CellPaint { glyph, color })
    }
}

/// Local wall-clock HH:MM via localtime_r (no chrono dependency).
pub fn clock_hhmm() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return "--:--".into();
        }
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

/// Local date "FRI 11 JUL", same localtime_r path (no chrono dependency).
pub fn clock_date() -> String {
    const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN",
        "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        let day = DAYS.get(tm.tm_wday as usize).copied().unwrap_or("");
        let mon = MONTHS.get(tm.tm_mon as usize).copied().unwrap_or("");
        format!("{day} {:02} {mon}", tm.tm_mday)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::build_grid_map;

    fn scene_and_map() -> (Scene, GridMap) {
        let cfg = Cfg::default();
        let map = build_grid_map(1920, 1080, 10, 20, 360.0, 20.0);
        (Scene::new(&cfg, &map), map)
    }

    #[test]
    fn glyph_monotone_in_age() {
        // Older live cells never render lighter than younger ones.
        let (mut scene, _) = scene_and_map();
        scene.faces[0].board.alive[0] = true;
        scene.faces[0].board.born[0] = 0.0;
        let ages = [0.1, 0.7, 1.5, 5.0];
        let mut last = Glyph::Light;
        for &age in &ages {
            let t = age * scene.faces[0].cfg.tick - 0.0;
            let paint = scene
                .sample_face(0, 0, 0, 0, t)
                .expect("live cell must paint");
            assert!(paint.glyph >= last, "glyph regressed at age {age}");
            last = paint.glyph;
        }
    }

    #[test]
    fn flare_brightens_dead_panel_cells() {
        let (mut scene, _) = scene_and_map();
        // Cell (0,0) of face 0 is dead: it paints the dim surface material, so
        // the cube always reads as a solid object.
        scene.faces[0].board.alive[0] = false;
        scene.faces[0].board.died[0] = f64::NAN;
        let base = scene.sample_face(0, 0, 0, 0, 1.0).expect("dead cell shows surface");
        // A fresh flare on panel 0 must brighten that same cell above its base.
        scene.flares.push(Flare { panel: 0, color: Rgb([164.0, 201.0, 75.0]), start: 1.0 });
        let lit = scene.sample_face(0, 0, 0, 0, 1.02).expect("flare paints panel");
        let sum = |p: CellPaint| p.color.iter().map(|&c| c as u32).sum::<u32>();
        assert!(sum(lit) > sum(base), "flare must brighten the panel");
    }

    #[test]
    fn wrong_kills_and_reseeds_after_flash() {
        let (mut scene, _) = scene_and_map();
        scene.advance(0.5);
        scene.wrong(1.0);
        assert_eq!(scene.faces.iter().map(|f| f.board.population()).sum::<usize>(), 0);
        // After the flash expires, advance() reseeds.
        scene.advance(1.0 + WRONG_SECS + 0.01);
        let pop: usize = scene.faces.iter().map(|f| f.board.population()).sum();
        assert!(pop > 0, "faces must reseed after the wrong flash");
    }

    #[test]
    fn flares_expire() {
        let (mut scene, _) = scene_and_map();
        scene.key_flare(0.0);
        scene.backspace_flare(0.1);
        assert_eq!(scene.flares.len(), 2);
        scene.advance(FLARE_SECS + 0.2);
        assert!(scene.flares.is_empty());
    }

    #[test]
    fn clock_shape() {
        let c = clock_hhmm();
        assert_eq!(c.len(), 5);
        assert_eq!(c.as_bytes()[2], b':');
    }
}
