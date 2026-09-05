// SPDX-License-Identifier: GPL-3.0-or-later
// Copied verbatim from ../lifelock/src/render/atlas.rs (same repo). Fix bugs in both.
// Glyph rasterization. Each shade block is pre-rendered once into a
// cell-sized 8-bit coverage bitmap; text (clock, CAPS) is rasterized on the
// fly. fontdue gives per-pixel coverage (0..=255), which is exactly the alpha
// we composite over the black background.

use crate::scene::Glyph;
use fontdue::{Font, FontSettings};
use std::collections::HashMap;

pub struct Atlas {
    font: Option<Font>,
    cell_w: usize,
    cell_h: usize,
    /// Per-glyph coverage, cell_w*cell_h bytes each (row-major).
    glyph_cov: HashMap<Glyph, Vec<u8>>,
}

const GLYPHS: [Glyph; 4] = [Glyph::Light, Glyph::Medium, Glyph::Dark, Glyph::Full];

impl Atlas {
    pub fn new(font_path: &str, cell_w: u32, cell_h: u32, solid: bool) -> Atlas {
        let cell_w = cell_w as usize;
        let cell_h = cell_h as usize;
        let font = std::fs::read(font_path)
            .ok()
            .and_then(|b| Font::from_bytes(b, FontSettings::default()).ok());
        if font.is_none() {
            eprintln!("lifelock: could not load font {font_path}; using solid cells");
        }

        let mut glyph_cov = HashMap::new();
        for g in GLYPHS {
            let cov = match (solid, font.as_ref()) {
                (false, Some(f)) => rasterize_block(f, g.ch(), cell_w, cell_h, g.coverage()),
                // Solid mode, or no font: uniform fill at the glyph's level.
                _ => vec![(g.coverage() * 255.0) as u8; cell_w * cell_h],
            };
            glyph_cov.insert(g, cov);
        }

        Atlas { font, cell_w, cell_h, glyph_cov }
    }

    pub fn cell(&self, g: Glyph) -> &[u8] {
        &self.glyph_cov[&g]
    }

    pub fn cell_w(&self) -> usize {
        self.cell_w
    }
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    /// Rasterize `text` at `px` and return (width, height, ascent, coverage
    /// rows) as a tight left-to-right run laid on a common baseline. Returns
    /// None if no font is available.
    pub fn text_run(&self, text: &str, px: f32) -> Option<TextRun> {
        let font = self.font.as_ref()?;
        // Two passes: measure the pen box, then composite.
        let mut pen = 0i32;
        let mut top = i32::MAX;
        let mut bot = i32::MIN;
        let mut glyphs = Vec::new();
        for ch in text.chars() {
            let (m, cov) = font.rasterize(ch, px);
            let x0 = pen + m.xmin;
            // fontdue y is measured up from baseline; ymin is the bottom edge.
            let y_top = -(m.ymin + m.height as i32); // rows above baseline (neg = above)
            top = top.min(y_top);
            bot = bot.max(y_top + m.height as i32);
            glyphs.push((x0, y_top, m.width, m.height, cov));
            pen += m.advance_width.round() as i32;
        }
        if glyphs.is_empty() {
            return None;
        }
        let x_off = glyphs.iter().map(|g| g.0).min().unwrap_or(0);
        let width = (pen - x_off).max(1) as usize;
        let height = (bot - top).max(1) as usize;
        let mut buf = vec![0u8; width * height];
        for (gx, gy, gw, gh, cov) in glyphs {
            for ry in 0..gh {
                for rx in 0..gw {
                    let c = cov[ry * gw + rx];
                    if c == 0 {
                        continue;
                    }
                    let px_x = (gx - x_off) as usize + rx;
                    let py_y = (gy - top) as usize + ry;
                    if px_x < width && py_y < height {
                        let d = &mut buf[py_y * width + px_x];
                        *d = (*d).max(c);
                    }
                }
            }
        }
        Some(TextRun { width, height, cov: buf })
    }
}

pub struct TextRun {
    pub width: usize,
    pub height: usize,
    pub cov: Vec<u8>,
}

/// Rasterize a full-bleed block glyph and resample it to exactly fill the
/// cell (nearest-neighbour), so blocks tile seamlessly regardless of the
/// font's metrics. `floor` guarantees a minimum coverage for the airy ░/▒
/// patterns so they still read at small cell sizes.
fn rasterize_block(font: &Font, ch: char, cell_w: usize, cell_h: usize, _level: f64) -> Vec<u8> {
    // Oversize so the glyph bitmap is at least cell-sized before downsampling.
    let px = (cell_h as f32) * 1.6;
    let (m, cov) = font.rasterize(ch, px);
    let mut out = vec![0u8; cell_w * cell_h];
    if m.width == 0 || m.height == 0 {
        return out;
    }
    for cy in 0..cell_h {
        for cx in 0..cell_w {
            let sx = (cx * m.width) / cell_w;
            let sy = (cy * m.height) / cell_h;
            out[cy * cell_w + cx] = cov[sy.min(m.height - 1) * m.width + sx.min(m.width - 1)];
        }
    }
    out
}
