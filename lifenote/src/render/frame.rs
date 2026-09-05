// SPDX-License-Identifier: GPL-3.0-or-later
// Grid -> pixels. Unlike the opaque XRGB8888 canvases in lifelock/lifegreet,
// lifenote's surface is ARGB8888 with PREMULTIPLIED alpha end-to-end: the
// canvas starts fully transparent, each popup fills its rect with the
// translucent panel colour, and glyphs composite as opaque ink over it.
// Getting premultiplication wrong shows up as bright fringes on glyph edges;
// the --render-ppm harness composites over a checkerboard to catch that.

use crate::cli::{Cfg, Rgb};
use crate::layout::{Grid, Role};
use crate::render::atlas::Atlas;
use crate::state::Urgency;

/// Per-role ink colours for one popup (resolved from cfg + urgency).
#[derive(Clone, Copy)]
pub struct Palette {
    pub border: [u8; 3],
    pub title: [u8; 3],
    pub summary: [u8; 3],
    pub body: [u8; 3],
}

pub fn palette(cfg: &Cfg, urgency: Urgency) -> Palette {
    match urgency {
        // Critical: the whole frame + title flip to rust red, summary warns.
        Urgency::Critical => Palette {
            border: cfg.urgent.to_u8(),
            title: cfg.urgent.to_u8(),
            summary: cfg.warn.to_u8(),
            body: cfg.text.to_u8(),
        },
        _ => Palette {
            border: cfg.border.to_u8(),
            title: cfg.title.to_u8(),
            summary: cfg.text.to_u8(),
            body: cfg.text.to_u8(),
        },
    }
}

impl Palette {
    fn for_role(&self, role: Role) -> [u8; 3] {
        match role {
            Role::Border => self.border,
            Role::Title => self.title,
            Role::Summary => self.summary,
            Role::Body => self.body,
        }
    }
}

/// Premultiplied ARGB8888 little-endian bytes [B, G, R, A] for the panel.
pub fn premul_bg(bg: Rgb, alpha: f64) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let [r, g, b] = bg.to_u8();
    [
        (b as u32 * a / 255) as u8,
        (g as u32 * a / 255) as u8,
        (r as u32 * a / 255) as u8,
        a as u8,
    ]
}

/// Draw one composed popup at pixel offset (x0, y0): translucent panel rect,
/// then the glyph grid on top. The canvas outside popup rects stays fully
/// transparent (it was cleared to zero).
#[allow(clippy::too_many_arguments)]
pub fn draw_popup(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    grid: &Grid,
    atlas: &mut Atlas,
    pal: &Palette,
    bg: [u8; 4],
) {
    let (cw, ch) = (atlas.cell_w(), atlas.cell_h());
    let cols = grid.first().map_or(0, |r| r.len());
    fill_rect(canvas, width, height, x0, y0, cols * cw, grid.len() * ch, bg);

    for (row, cells) in grid.iter().enumerate() {
        for (col, &(chr, role)) in cells.iter().enumerate() {
            if chr == ' ' {
                continue;
            }
            let ink = pal.for_role(role);
            let cov = atlas.cell(chr);
            blit_cell(canvas, width, height, x0 + col * cw, y0 + row * ch, cw, ch, cov, ink);
        }
    }
}

/// Pixel height of a composed popup.
pub fn popup_height(grid: &Grid, atlas: &Atlas) -> usize {
    grid.len() * atlas.cell_h()
}

/// Pixel width of a composed popup.
pub fn popup_width(grid: &Grid, atlas: &Atlas) -> usize {
    grid.first().map_or(0, |r| r.len()) * atlas.cell_w()
}

fn fill_rect(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    bg: [u8; 4],
) {
    for y in y0..(y0 + h).min(height) {
        let base = y * width;
        for x in x0..(x0 + w).min(width) {
            let o = (base + x) * 4;
            canvas[o..o + 4].copy_from_slice(&bg);
        }
    }
}

/// Composite a glyph as opaque ink: source alpha is the coverage value, so
/// premultiplied source = ink * cov. dst is already premultiplied.
#[allow(clippy::too_many_arguments)]
fn blit_cell(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    cw: usize,
    ch: usize,
    cov: &[u8],
    ink: [u8; 3],
) {
    for cy in 0..ch {
        let py = y0 + cy;
        if py >= height {
            break;
        }
        let rowbase = py * width;
        for cx in 0..cw {
            let px = x0 + cx;
            if px >= width {
                break;
            }
            let c = cov[cy * cw + cx] as u32;
            if c == 0 {
                continue;
            }
            let inv = 255 - c;
            let o = (rowbase + px) * 4;
            // ARGB8888 little-endian bytes: [B, G, R, A], premultiplied.
            canvas[o] = ((ink[2] as u32 * c + canvas[o] as u32 * inv) / 255) as u8;
            canvas[o + 1] = ((ink[1] as u32 * c + canvas[o + 1] as u32 * inv) / 255) as u8;
            canvas[o + 2] = ((ink[0] as u32 * c + canvas[o + 2] as u32 * inv) / 255) as u8;
            canvas[o + 3] = (c + canvas[o + 3] as u32 * inv / 255) as u8;
        }
    }
}
