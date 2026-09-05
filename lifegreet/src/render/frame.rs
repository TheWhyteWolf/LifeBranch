// SPDX-License-Identifier: GPL-3.0-or-later
// Per-frame compositor: GridMap + Scene + greeter overlays -> XRGB8888.
// Forked from ../lifelock/src/render/frame.rs (cell/text blitting identical;
// fix blit bugs in both). The greeter adds the username box, caret, session
// line, and info line, and anchors every overlay to the FINAL cube's
// geometry (`Anchors`) so nothing jumps while the cube itself grows and
// collapses through interpolated GridMaps.

use crate::geometry::GridMap;
use crate::render::atlas::Atlas;
use crate::scene::{clock_date, clock_hhmm, Scene};
use crate::sim::Rgb;

/// Session-toggle line text size (bottom-left corner, deliberately tiny).
const SESSION_PX: f32 = 15.0;

/// Pixel anchors from the FULL-scale map: overlays (box, clock, CAPS, info)
/// stay put from boot regardless of which animation map is being drawn.
pub struct Anchors {
    pub center_x: f64,
    pub cube_top: f64,
    pub cube_bottom: f64,
}

impl Anchors {
    pub fn of(map: &GridMap) -> Anchors {
        Anchors {
            center_x: map.center_x_px,
            cube_top: map.cube_top_px,
            cube_bottom: map.cube_bottom_px,
        }
    }
}

pub struct Overlay<'a> {
    pub username: &'a str,
    pub caret_on: bool,
    /// 0 hides the box entirely (password phases); 0..1 during grow/collapse.
    pub box_alpha: f64,
    /// Dim line under the box, e.g. "session: Niri  [F3]". Fades with the box.
    pub session_line: Option<&'a str>,
    /// Transient PAM Info/Error or fatal-IPC message, drawn in `urgent`.
    pub info: Option<&'a str>,
}

/// Fill `canvas` (width*height*4 XRGB8888) for scene time `t`.
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    map: &GridMap,
    anchors: &Anchors,
    atlas: &Atlas,
    scene: &Scene,
    t: f64,
    ov: &Overlay,
    text_color: Rgb,
    urgent_color: Rgb,
    clock_px: f32,
    caps_px: f32,
    user_px: f32,
) {
    canvas.fill(0); // opaque black (XRGB: X byte ignored)

    let cw = atlas.cell_w();
    let ch = atlas.cell_h();

    for row in 0..map.rows {
        let y0 = row * ch;
        if y0 >= height {
            break;
        }
        for col in 0..map.cols {
            let x0 = col * cw;
            if x0 >= width {
                break;
            }
            let src = map.cells[row * map.cols + col];
            let Some(paint) = scene.sample(src, col, row, t) else {
                continue;
            };
            blit_cell(canvas, width, height, x0, y0, cw, ch, atlas.cell(paint.glyph), paint.color);
        }
    }

    // Username box, centered where the cube grows from (the cube center).
    if ov.box_alpha > 0.0 {
        let a = ov.box_alpha;
        let box_w = (user_px as f64 * 14.0) as usize;
        let box_h = (user_px as f64 * 2.0) as usize;
        let mid_y = (anchors.cube_top + anchors.cube_bottom) / 2.0;
        let bx0 = (anchors.center_x as usize).saturating_sub(box_w / 2);
        let by0 = (mid_y as usize).saturating_sub(box_h / 2);
        let border = text_color.scaled(a);
        draw_box_border(canvas, width, height, bx0, by0, box_w, box_h, 2, border.to_u8());

        // Typed username + caret, tail-truncated to fit the box.
        let pad = (user_px * 0.6) as usize;
        let inner_w = box_w.saturating_sub(pad * 2);
        let mut shown = format!("{}{}", ov.username, if ov.caret_on { "_" } else { " " });
        let mut run = atlas.text_run(&shown, user_px);
        while let Some(r) = &run {
            if r.width <= inner_w || shown.len() <= 1 {
                break;
            }
            shown.remove(0);
            run = atlas.text_run(&shown, user_px);
        }
        if let Some(run) = run {
            let ty = by0 + (box_h.saturating_sub(run.height)) / 2;
            blit_text(canvas, width, height, bx0 + pad, ty, &run, text_color.scaled(a).to_u8());
        }

        // Session line: small and muted, tucked into the bottom-left corner
        // (F3 is a common enough bind not to need explaining). Fades with
        // the box — the choice only matters before the username is sent.
        if let Some(line) = ov.session_line {
            if let Some(run) = atlas.text_run(line, SESSION_PX) {
                let margin = (SESSION_PX * 1.2) as usize;
                let x = margin;
                let y = height.saturating_sub(run.height + margin);
                blit_text(canvas, width, height, x, y, &run, text_color.scaled(0.35 * a).to_u8());
            }
        }
    }

    // Clock below the cube — same spot from boot (anchors, not map).
    if let Some(run) = atlas.text_run(&clock_hhmm(), clock_px) {
        let x = (anchors.center_x as usize).saturating_sub(run.width / 2);
        let y = (anchors.cube_bottom as usize + (clock_px as usize)).min(height.saturating_sub(run.height));
        blit_text(canvas, width, height, x, y, &run, text_color.to_u8());
    }

    // Date under the clock, slightly dimmer (same slot as lifelock's date).
    if let Some(run) = atlas.text_run(&clock_date(), caps_px) {
        let x = (anchors.center_x as usize).saturating_sub(run.width / 2);
        let y = (anchors.cube_bottom as usize + (clock_px as usize) * 2 + caps_px as usize)
            .min(height.saturating_sub(run.height));
        blit_text(canvas, width, height, x, y, &run, text_color.scaled(0.8).to_u8());
    }

    // CAPS tag above the cube.
    if scene.caps_lock {
        if let Some(run) = atlas.text_run("CAPS", caps_px) {
            let x = (anchors.center_x as usize).saturating_sub(run.width / 2);
            let y = (anchors.cube_top as usize).saturating_sub((caps_px as usize) * 2);
            blit_text(canvas, width, height, x, y, &run, urgent_color.to_u8());
        }
    }

    // Info line (PAM message / fatal IPC), below the date (which now owns the
    // slot this used to occupy).
    if let Some(info) = ov.info {
        if let Some(run) = atlas.text_run(info, caps_px) {
            let x = (anchors.center_x as usize).saturating_sub(run.width / 2);
            let y = (anchors.cube_bottom as usize + (clock_px as usize) * 2 + (caps_px as usize) * 3)
                .min(height.saturating_sub(run.height));
            blit_text(canvas, width, height, x, y, &run, urgent_color.to_u8());
        }
    }
}

/// A hollow rectangle outline, `thick` px, blended over the backdrop.
#[allow(clippy::too_many_arguments)]
fn draw_box_border(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    thick: usize,
    color: [u8; 3],
) {
    let x1 = (x0 + w).min(width);
    let y1 = (y0 + h).min(height);
    for py in y0..y1 {
        for px in x0..x1 {
            let on_edge = py < y0 + thick
                || py >= y1.saturating_sub(thick)
                || px < x0 + thick
                || px >= x1.saturating_sub(thick);
            if !on_edge {
                continue;
            }
            let o = (py * width + px) * 4;
            // XRGB8888 little-endian bytes: [B, G, R, X]
            canvas[o] = color[2];
            canvas[o + 1] = color[1];
            canvas[o + 2] = color[0];
            canvas[o + 3] = 0xff;
        }
    }
}

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
    color: [u8; 3],
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
            let o = (rowbase + px) * 4;
            // XRGB8888 little-endian bytes: [B, G, R, X]
            canvas[o] = ((color[2] as u32 * c) / 255) as u8;
            canvas[o + 1] = ((color[1] as u32 * c) / 255) as u8;
            canvas[o + 2] = ((color[0] as u32 * c) / 255) as u8;
            canvas[o + 3] = 0xff;
        }
    }
}

fn blit_text(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    run: &crate::render::atlas::TextRun,
    color: [u8; 3],
) {
    for ry in 0..run.height {
        let py = y0 + ry;
        if py >= height {
            break;
        }
        for rx in 0..run.width {
            let px = x0 + rx;
            if px >= width {
                break;
            }
            let c = run.cov[ry * run.width + rx] as u32;
            if c == 0 {
                continue;
            }
            let o = (py * width + px) * 4;
            // Blend over whatever's there (ember may be behind the text).
            let inv = 255 - c;
            canvas[o] = (((color[2] as u32 * c) + canvas[o] as u32 * inv) / 255) as u8;
            canvas[o + 1] = (((color[1] as u32 * c) + canvas[o + 1] as u32 * inv) / 255) as u8;
            canvas[o + 2] = (((color[0] as u32 * c) + canvas[o + 2] as u32 * inv) / 255) as u8;
            canvas[o + 3] = 0xff;
        }
    }
}
