// SPDX-License-Identifier: GPL-3.0-or-later
// A minimal monospace glyph renderer for the GUI, adapted from
// lifenote/src/render/atlas.rs (kept as a self-contained copy rather than a
// shared crate — see the note in gui/mod.rs). Rasterizes each char once into a
// cell-sized coverage bitmap, then blits strings into an ARGB8888 canvas.

use fontdue::{Font, FontSettings};
use std::collections::HashMap;

pub struct Atlas {
    font: Font,
    px: f32,
    cell_w: usize,
    cell_h: usize,
    ascent: i32,
    cache: HashMap<char, Vec<u8>>,
}

impl Atlas {
    pub fn new(font_path: &str, px: f32) -> Result<Atlas, String> {
        let bytes =
            std::fs::read(font_path).map_err(|e| format!("cannot read font {font_path}: {e}"))?;
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|e| format!("cannot parse font {font_path}: {e}"))?;
        let lm = font
            .horizontal_line_metrics(px)
            .ok_or_else(|| format!("font {font_path} has no horizontal metrics"))?;
        let cell_h = (lm.ascent - lm.descent).ceil().max(1.0) as usize;
        let ascent = lm.ascent.round() as i32;
        let mut cell_w = font.metrics('M', px).advance_width.round() as usize;
        if cell_w == 0 {
            cell_w = (px * 0.6).round().max(1.0) as usize;
        }
        Ok(Atlas { font, px, cell_w, cell_h, ascent, cache: HashMap::new() })
    }

    pub fn cell_w(&self) -> usize {
        self.cell_w
    }
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    fn cell(&mut self, ch: char) -> &[u8] {
        if !self.cache.contains_key(&ch) {
            let cov = self.rasterize(ch);
            self.cache.insert(ch, cov);
        }
        &self.cache[&ch]
    }

    fn rasterize(&self, ch: char) -> Vec<u8> {
        let (cw, chh) = (self.cell_w, self.cell_h);
        let mut out = vec![0u8; cw * chh];
        let (m, cov) = self.font.rasterize(ch, self.px);
        let y_top = self.ascent - (m.ymin + m.height as i32);
        for ry in 0..m.height {
            let py = y_top + ry as i32;
            if py < 0 || py as usize >= chh {
                continue;
            }
            for rx in 0..m.width {
                let px_x = m.xmin + rx as i32;
                if px_x < 0 || px_x as usize >= cw {
                    continue;
                }
                let c = cov[ry * m.width + rx];
                if c != 0 {
                    out[py as usize * cw + px_x as usize] = c;
                }
            }
        }
        out
    }

    /// Blit `s` starting at pixel (x0,y0) in colour `rgb`, over the ARGB canvas
    /// `buf` of width `stride` px. Alpha-composites glyph coverage onto whatever
    /// is there. Returns the x pixel after the last glyph.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_str(
        &mut self,
        buf: &mut [u32],
        stride: usize,
        height: usize,
        x0: usize,
        y0: usize,
        s: &str,
        rgb: (u8, u8, u8),
    ) -> usize {
        let (cw, chh) = (self.cell_w, self.cell_h);
        let mut x = x0;
        for ch in s.chars() {
            if ch == ' ' {
                x += cw;
                continue;
            }
            let cov = self.cell(ch).to_vec();
            for ry in 0..chh {
                let py = y0 + ry;
                if py >= height {
                    break;
                }
                for rx in 0..cw {
                    let px_x = x + rx;
                    if px_x >= stride {
                        break;
                    }
                    let a = cov[ry * cw + rx];
                    if a == 0 {
                        continue;
                    }
                    let idx = py * stride + px_x;
                    buf[idx] = blend(buf[idx], rgb, a);
                }
            }
            x += cw;
        }
        x
    }
}

/// Src-over composite of `rgb` at coverage `a` onto an existing ARGB pixel.
fn blend(dst: u32, rgb: (u8, u8, u8), a: u8) -> u32 {
    let a = a as u32;
    let inv = 255 - a;
    let dr = (dst >> 16) & 0xff;
    let dg = (dst >> 8) & 0xff;
    let db = dst & 0xff;
    let r = (rgb.0 as u32 * a + dr * inv) / 255;
    let g = (rgb.1 as u32 * a + dg * inv) / 255;
    let b = (rgb.2 as u32 * a + db * inv) / 255;
    0xff00_0000 | (r << 16) | (g << 8) | b
}

/// Fill a rectangle with an opaque ARGB colour.
#[allow(clippy::too_many_arguments)]
pub fn fill_rect(
    buf: &mut [u32],
    stride: usize,
    height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    rgb: (u8, u8, u8),
) {
    let color = 0xff00_0000 | ((rgb.0 as u32) << 16) | ((rgb.1 as u32) << 8) | rgb.2 as u32;
    for ry in y..(y + h).min(height) {
        for rx in x..(x + w).min(stride) {
            buf[ry * stride + rx] = color;
        }
    }
}
