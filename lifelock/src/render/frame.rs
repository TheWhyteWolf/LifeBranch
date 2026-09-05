// SPDX-License-Identifier: GPL-3.0-or-later
// Per-frame compositor: GridMap + Scene -> XRGB8888 pixels.
// Background is opaque black; grid cells never overlap (each is one source),
// so cells are written directly (coverage as alpha over black). Text overlays
// blend on top.

use crate::geometry::GridMap;
use crate::render::atlas::Atlas;
use crate::scene::{clock_date, clock_hhmm, Scene};
use crate::sim::Rgb;

/// Fill `canvas` (width*height*4 XRGB8888) for scene time `t`.
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    map: &GridMap,
    atlas: &Atlas,
    scene: &Scene,
    t: f64,
    text_color: Rgb,
    urgent_color: Rgb,
    clock_px: f32,
    caps_px: f32,
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

    // Clock below the cube.
    if let Some(run) = atlas.text_run(&clock_hhmm(), clock_px) {
        let x = (map.center_x_px as usize).saturating_sub(run.width / 2);
        let y = (map.cube_bottom_px as usize + (clock_px as usize)).min(height.saturating_sub(run.height));
        blit_text(canvas, width, height, x, y, &run, text_color.to_u8());
    }

    // Date under the clock, slightly dimmer.
    if let Some(run) = atlas.text_run(&clock_date(), caps_px) {
        let x = (map.center_x_px as usize).saturating_sub(run.width / 2);
        let y = (map.cube_bottom_px as usize + (clock_px as usize) * 2 + caps_px as usize)
            .min(height.saturating_sub(run.height));
        blit_text(canvas, width, height, x, y, &run, text_color.scaled(0.8).to_u8());
    }

    // Failed attempts under the date — swaylock's show-failed-attempts parity.
    // (Same slot as lifegreet's PAM error line.)
    if scene.failed_attempts > 0 {
        let label = format!("{} FAILED", scene.failed_attempts);
        if let Some(run) = atlas.text_run(&label, caps_px) {
            let x = (map.center_x_px as usize).saturating_sub(run.width / 2);
            let y = (map.cube_bottom_px as usize + (clock_px as usize) * 2 + (caps_px as usize) * 3)
                .min(height.saturating_sub(run.height));
            blit_text(canvas, width, height, x, y, &run, urgent_color.to_u8());
        }
    }

    // PAM's explanation for the last failure (pam_faillock's lockout notice,
    // the dead-authenticator hint) one line below the FAILED count.
    if let Some(msg) = &scene.status_msg {
        if let Some(run) = atlas.text_run(msg, caps_px) {
            let x = (map.center_x_px as usize).saturating_sub(run.width / 2);
            let y = (map.cube_bottom_px as usize + (clock_px as usize) * 2 + (caps_px as usize) * 5)
                .min(height.saturating_sub(run.height));
            blit_text(canvas, width, height, x, y, &run, urgent_color.to_u8());
        }
    }

    // CAPS tag above the cube.
    if scene.caps_lock {
        if let Some(run) = atlas.text_run("CAPS", caps_px) {
            let x = (map.center_x_px as usize).saturating_sub(run.width / 2);
            let y = (map.cube_top_px as usize).saturating_sub((caps_px as usize) * 2);
            blit_text(canvas, width, height, x, y, &run, urgent_color.to_u8());
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
            // Blend over whatever's there (ember may be behind the clock).
            let inv = 255 - c;
            canvas[o] = (((color[2] as u32 * c) + canvas[o] as u32 * inv) / 255) as u8;
            canvas[o + 1] = (((color[1] as u32 * c) + canvas[o + 1] as u32 * inv) / 255) as u8;
            canvas[o + 2] = (((color[0] as u32 * c) + canvas[o + 2] as u32 * inv) / 255) as u8;
            canvas[o + 3] = 0xff;
        }
    }
}
