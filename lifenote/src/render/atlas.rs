// SPDX-License-Identifier: GPL-3.0-or-later
// Char-cell glyph atlas. Forked from ../../lifelock/src/render/atlas.rs;
// where lifelock pre-renders four shade blocks and draws text as free runs,
// lifenote is a pure character grid: every glyph (box-drawing and text alike)
// is rasterized once into a cell-sized coverage bitmap and cached.
//
// Cell metrics come from the font: cell_h deliberately EXCLUDES line_gap so
// the vertical strokes of │┃║ on adjacent rows touch and frames read as
// continuous lines. Placement is metric-true (baseline at `ascent`), not the
// full-bleed stretch lifelock uses for ░▒▓█ — stretching `─` would turn a
// thin rule into a filled bar.

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
        let bytes = std::fs::read(font_path)
            .map_err(|e| format!("cannot read font {font_path}: {e}"))?;
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|e| format!("cannot parse font {font_path}: {e}"))?;
        let lm = font
            .horizontal_line_metrics(px)
            .ok_or_else(|| format!("font {font_path} has no horizontal metrics"))?;
        let cell_h = (lm.ascent - lm.descent).ceil().max(1.0) as usize;
        let ascent = lm.ascent.round() as i32;
        // Monospace: every advance is equal; '─' stands in for all of them.
        let mut cell_w = font.metrics('─', px).advance_width.round() as usize;
        if cell_w == 0 {
            cell_w = font.metrics('M', px).advance_width.round().max(1.0) as usize;
        }
        Ok(Atlas { font, px, cell_w, cell_h, ascent, cache: HashMap::new() })
    }

    pub fn cell_w(&self) -> usize {
        self.cell_w
    }
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    /// Cell-sized coverage bitmap (cell_w*cell_h bytes, row-major) for `ch`.
    pub fn cell(&mut self, ch: char) -> &[u8] {
        if !self.cache.contains_key(&ch) {
            let cov = self.rasterize_cell(ch);
            self.cache.insert(ch, cov);
        }
        &self.cache[&ch]
    }

    fn rasterize_cell(&self, ch: char) -> Vec<u8> {
        let (cw, chh) = (self.cell_w, self.cell_h);
        let mut out = vec![0u8; cw * chh];
        let (m, cov) = self.font.rasterize(ch, self.px);
        // Baseline math as in lifelock's text_run: fontdue y is measured up
        // from the baseline, ymin is the glyph's bottom edge.
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
        if is_box_drawing(ch) {
            connect_to_edges(&mut out, cw, chh);
        }
        out
    }
}

fn is_box_drawing(ch: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&ch)
}

/// Connection guard: box-drawing strokes are designed to meet the ones in
/// neighbouring cells, but metric rounding (and fonts that drew them for a
/// line box that includes line_gap) can leave a 1–2 px gap at a cell edge.
/// If coverage stops within GUARD px of an edge, replicate the edge-most
/// covered row/column out to the edge so strokes join seamlessly.
fn connect_to_edges(cov: &mut [u8], w: usize, h: usize) {
    const GUARD: usize = 2;
    let row_used = |cov: &[u8], y: usize| (0..w).any(|x| cov[y * w + x] != 0);
    let col_used = |cov: &[u8], x: usize| (0..h).any(|y| cov[y * w + x] != 0);

    if let Some(top) = (0..h).find(|&y| row_used(cov, y)) {
        if (1..=GUARD).contains(&top) {
            for y in 0..top {
                cov.copy_within(top * w..(top + 1) * w, y * w);
            }
        }
    }
    if let Some(bot) = (0..h).rev().find(|&y| row_used(cov, y)) {
        let gap = h - 1 - bot;
        if (1..=GUARD).contains(&gap) {
            for y in bot + 1..h {
                cov.copy_within(bot * w..(bot + 1) * w, y * w);
            }
        }
    }
    if let Some(left) = (0..w).find(|&x| col_used(cov, x)) {
        if (1..=GUARD).contains(&left) {
            for y in 0..h {
                let v = cov[y * w + left];
                for x in 0..left {
                    cov[y * w + x] = v;
                }
            }
        }
    }
    if let Some(right) = (0..w).rev().find(|&x| col_used(cov, x)) {
        let gap = w - 1 - right;
        if (1..=GUARD).contains(&gap) {
            for y in 0..h {
                let v = cov[y * w + right];
                for x in right + 1..w {
                    cov[y * w + x] = v;
                }
            }
        }
    }
}
