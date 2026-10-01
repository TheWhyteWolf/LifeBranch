// SPDX-License-Identifier: GPL-3.0-or-later
// Monospace text into a premultiplied ARGB8888 canvas (the panel is slightly
// translucent, like fuzzel's). Adapted from lifeconf/src/gui/render.rs: each
// char is rasterized once into a cell-sized coverage bitmap and cached.

use crate::cli::Rgba;
use lifefont::Font;
use std::collections::HashMap;
use std::process::Command;

pub struct Atlas {
    font: Font,
    px: f32,
    pub cw: usize,
    pub ch: usize,
    ascent: i32,
    cache: HashMap<char, Vec<u8>>,
}

/// `fc-match` a family to its font file and face index.
pub fn fc_match(family: &str) -> Option<(String, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{file}\n%{index}", family]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let mut it = s.lines();
    let file = it.next().filter(|f| !f.is_empty())?.to_string();
    Some((file, it.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0)))
}

impl Atlas {
    /// `pt` in points at 96 dpi, times the output's buffer scale.
    pub fn new(family: &str, pt: f32, scale: i32) -> Result<Atlas, String> {
        let (file, index) = fc_match(family).ok_or_else(|| format!("no font for '{family}' (is fontconfig installed?)"))?;
        let font = Font::from_path_index(&file, index)?;
        let px = pt * 96.0 / 72.0 * scale as f32;
        let lm = font.horizontal_line_metrics(px).ok_or("font has no line metrics")?;
        let cw = font.metrics('M', px).advance_width.round().max(1.0) as usize;
        let ch = (lm.ascent - lm.descent).ceil().max(1.0) as usize;
        Ok(Atlas { font, px, cw, ch, ascent: lm.ascent.round() as i32, cache: HashMap::new() })
    }

    fn cell(&mut self, c: char) -> &[u8] {
        if !self.cache.contains_key(&c) {
            let cov = self.rasterize(c);
            self.cache.insert(c, cov);
        }
        &self.cache[&c]
    }

    fn rasterize(&self, c: char) -> Vec<u8> {
        let (cw, ch) = (self.cw, self.ch);
        let mut out = vec![0u8; cw * ch];
        let (m, cov) = self.font.rasterize(c, self.px);
        let y_top = self.ascent - (m.ymin + m.height as i32);
        for ry in 0..m.height {
            let y = y_top + ry as i32;
            if y < 0 || y as usize >= ch {
                continue;
            }
            for rx in 0..m.width {
                let x = m.xmin + rx as i32;
                if x < 0 || x as usize >= cw {
                    continue;
                }
                out[y as usize * cw + x as usize] = cov[ry * m.width + rx];
            }
        }
        out
    }
}

pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub w: usize,
    pub h: usize,
}

/// Premultiply a straight-alpha colour into an ARGB8888 pixel.
pub fn premul(c: Rgba) -> u32 {
    let a = c[3] as u32;
    let m = |v: u8| v as u32 * a / 255;
    (a << 24) | (m(c[0]) << 16) | (m(c[1]) << 8) | m(c[2])
}

/// Src-over of colour `c` at coverage `cov` onto a premultiplied pixel.
fn over(dst: u32, c: Rgba, cov: u8) -> u32 {
    let a = c[3] as u32 * cov as u32 / 255;
    let inv = 255 - a;
    let ch = |shift: u32, src: u32| ((src * a + ((dst >> shift) & 0xff) * inv) / 255) << shift;
    ch(24, 255) | ch(16, c[0] as u32) | ch(8, c[1] as u32) | ch(0, c[2] as u32)
}

impl Canvas<'_> {
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, c: Rgba) {
        let p = premul(c);
        for ry in y..(y + h).min(self.h) {
            let row = &mut self.px[ry * self.w..][..self.w];
            for v in &mut row[x.min(self.w)..(x + w).min(self.w)] {
                *v = p;
            }
        }
    }

    /// One char cell at (x, y) in colour `c`.
    pub fn glyph(&mut self, atlas: &mut Atlas, x: usize, y: usize, ch: char, c: Rgba) {
        if ch == ' ' {
            return;
        }
        let (cw, chh) = (atlas.cw, atlas.ch);
        let cov = atlas.cell(ch).to_vec();
        for ry in 0..chh.min(self.h.saturating_sub(y)) {
            for rx in 0..cw.min(self.w.saturating_sub(x)) {
                let a = cov[ry * cw + rx];
                if a != 0 {
                    let i = (y + ry) * self.w + x + rx;
                    self.px[i] = over(self.px[i], c, a);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplied_blending() {
        assert_eq!(premul([255, 0, 0, 128]), 0x8080_0000);
        let bg = premul([0, 0, 0, 0xf2]);
        assert_eq!(over(bg, [0xa4, 0xc9, 0x4b, 0xff], 255), 0xffa4_c94b, "full coverage: the colour");
        assert_eq!(over(bg, [0xa4, 0xc9, 0x4b, 0xff], 0), bg, "no coverage: untouched");
    }
}
