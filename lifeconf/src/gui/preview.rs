// SPDX-License-Identifier: GPL-3.0-or-later
// Live previews under the Cursor and Font pages: the chosen cursor theme's
// main cursors at the chosen size, and a pangram in the chosen font. Each is
// loaded once per setting and kept until the setting changes, so repaints
// cost a blit.

use super::render::blend;
use crate::cursors;
use lifefont::{Font, Metrics};
use std::collections::HashMap;

pub const PANGRAM: &str = "Amazingly few discotheques provide jukeboxes";

/// The cursors shown, in order.
const SHOWN: &[&str] = &["default", "pointer", "text", "progress", "wait", "grab", "move", "ew-resize", "not-allowed", "crosshair"];

#[derive(Default)]
pub struct Previews {
    cursor_key: Option<(String, u32)>,
    cursors: Vec<cursors::Image>,
    font_key: Option<(String, u32)>,
    font: Option<LoadedFont>,
}

pub struct LoadedFont {
    font: Font,
    px: f32,
    /// What fontconfig actually gave for the family asked for.
    pub family: String,
    glyphs: HashMap<char, (Metrics, Vec<u8>)>,
}

/// The font file fontconfig picks for `family`: (family it found, file, face index).
fn fc_match(family: &str) -> Option<(String, String, u32)> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{family[0]}\n%{file}\n%{index}", family])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut l = text.lines();
    Some((l.next()?.into(), l.next()?.into(), l.next().and_then(|i| i.parse().ok()).unwrap_or(0)))
}

impl Previews {
    pub fn cursors(&mut self, theme: &str, size: u32) -> &[cursors::Image] {
        let key = (theme.to_string(), size);
        if self.cursor_key.as_ref() != Some(&key) {
            let roots = cursors::roots();
            self.cursors = SHOWN.iter().filter_map(|n| cursors::load(&roots, theme, n, size)).collect();
            self.cursor_key = Some(key);
        }
        &self.cursors
    }

    pub fn font(&mut self, family: &str, pt: u32) -> Option<&mut LoadedFont> {
        let key = (family.to_string(), pt);
        if self.font_key.as_ref() != Some(&key) {
            self.font = fc_match(family).and_then(|(found, file, index)| {
                Some(LoadedFont {
                    font: Font::from_path_index(&file, index).ok()?,
                    px: pt as f32 * 96.0 / 72.0, // points at 96 dpi, as kitty and GTK size them
                    family: found,
                    glyphs: HashMap::new(),
                })
            });
            self.font_key = Some(key);
        }
        self.font.as_mut()
    }
}

/// Composite premultiplied ARGB `im` at (x, y) over the canvas.
pub fn blit_cursor(buf: &mut [u32], stride: usize, height: usize, x: usize, y: usize, im: &cursors::Image) {
    for ry in 0..im.h {
        for rx in 0..im.w {
            let (px, py) = (x + rx, y + ry);
            if px >= stride || py >= height {
                continue;
            }
            let s = im.px[ry * im.w + rx];
            let a = s >> 24;
            if a == 0 {
                continue;
            }
            let d = buf[py * stride + px];
            let ch = |sh: u32| (((s >> sh) & 255) + ((d >> sh) & 255) * (255 - a) / 255).min(255);
            buf[py * stride + px] = 0xff00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0);
        }
    }
}

impl LoadedFont {
    fn glyph(&mut self, c: char) -> &(Metrics, Vec<u8>) {
        let (font, px) = (&self.font, self.px);
        self.glyphs.entry(c).or_insert_with(|| font.rasterize(c, px))
    }

    pub fn line_h(&self) -> usize {
        self.font.horizontal_line_metrics(self.px).map_or(self.px * 1.3, |m| m.new_line_size).ceil() as usize
    }

    fn ascent(&self) -> f32 {
        self.font.horizontal_line_metrics(self.px).map_or(self.px, |m| m.ascent)
    }

    fn width(&mut self, s: &str) -> f32 {
        s.chars().map(|c| self.glyph(c).0.advance_width).sum()
    }

    /// `text` broken into lines no wider than `max_w` px, at spaces.
    pub fn wrap(&mut self, text: &str, max_w: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for word in text.split(' ') {
            match lines.last_mut() {
                Some(l) if self.width(&format!("{l} {word}")) <= max_w as f32 => {
                    l.push(' ');
                    l.push_str(word);
                }
                _ => lines.push(word.into()),
            }
        }
        lines
    }

    /// Draw `lines` from (x, y), clipped to `max_w` px wide.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lines(
        &mut self,
        buf: &mut [u32],
        stride: usize,
        height: usize,
        x: usize,
        y: usize,
        max_w: usize,
        lines: &[String],
        rgb: (u8, u8, u8),
    ) {
        let (lh, asc) = (self.line_h(), self.ascent());
        for (i, line) in lines.iter().enumerate() {
            let base = y as f32 + asc + (i * lh) as f32;
            let mut pen = x as f32;
            for c in line.chars() {
                let (m, cov) = self.glyph(c).clone();
                let gx = pen.round() as i32 + m.xmin;
                let gy = base.round() as i32 - (m.ymin + m.height as i32);
                for ry in 0..m.height {
                    for rx in 0..m.width {
                        let (px, py) = (gx + rx as i32, gy + ry as i32);
                        let a = cov[ry * m.width + rx];
                        if a == 0 || px < 0 || py < 0 || px as usize >= (x + max_w).min(stride) || py as usize >= height {
                            continue;
                        }
                        let idx = py as usize * stride + px as usize;
                        buf[idx] = blend(buf[idx], rgb, a);
                    }
                }
                pen += m.advance_width;
            }
        }
    }
}
