// SPDX-License-Identifier: GPL-3.0-or-later
// Copied from ../lifelock/src/geometry.rs with one addition:
// build_grid_map_scaled() for the greeter grow/collapse animation.
// Isometric cube geometry, all pure functions.
//
// The screen is one uniform character-cell grid (like a terminal). At
// configure time every cell center is classified once: inside one of the
// three cube faces (with its Life-board coordinate and 3x3 panel index) or
// part of the ember backdrop. Per frame the renderer just walks this table.
//
// Classic 2:1 axonometry. With silhouette height h (and width h):
//   a = h/2                          cube edge in projection
//   top vertex A = (cx, cy - h/2)
//   e_x = ( a, a/2)   right-down     e_y = (-a, a/2)   left-down
//   e_z = ( 0, a)     vertical edge
// Faces as parallelograms {O + u*e_u + v*e_v : u,v ∈ [0,1)}:
//   TOP    O = A            e_u = e_x    e_v = e_y
//   LEFT   O = A + e_y      e_u = e_x    e_v = e_z
//   RIGHT  O = A + e_x+e_y  e_u = -e_y   e_v = e_z

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn len(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

/// Per-face brightness for the 3D read, indexed by face (0 top, 1 left,
/// 2 right): top fully lit, left in deep shadow, right mid. Wide spread so
/// the three faces separate clearly even at the dim surface brightness.
pub const FACE_SHADE: [f64; 3] = [1.15, 0.42, 0.72];

#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub origin: Vec2,
    pub e_u: Vec2,
    pub e_v: Vec2,
    /// Life board dimensions for this face.
    pub board_w: usize,
    pub board_h: usize,
}

impl Face {
    /// Inverse affine: p -> (u, v) with the face covering [0,1)x[0,1).
    pub fn uv(&self, p: Vec2) -> (f64, f64) {
        let dx = p.x - self.origin.x;
        let dy = p.y - self.origin.y;
        let det = self.e_u.x * self.e_v.y - self.e_u.y * self.e_v.x;
        let u = (dx * self.e_v.y - dy * self.e_v.x) / det;
        let v = (self.e_u.x * dy - self.e_u.y * dx) / det;
        (u, v)
    }

    /// Forward map: board-cell center -> pixel position. (Used by the
    /// projection round-trip tests.)
    #[cfg(test)]
    pub fn cell_center(&self, bx: usize, by: usize) -> Vec2 {
        let u = (bx as f64 + 0.5) / self.board_w as f64;
        let v = (by as f64 + 0.5) / self.board_h as f64;
        Vec2 {
            x: self.origin.x + u * self.e_u.x + v * self.e_v.x,
            y: self.origin.y + u * self.e_u.y + v * self.e_v.y,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellSource {
    Face { face: u8, bx: u16, by: u16, panel: u8 },
    Ember,
}

pub struct GridMap {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<CellSource>,
    pub faces: [Face; 3],
    /// Pixel-space anchors for the overlays.
    pub cube_top_px: f64,
    pub cube_bottom_px: f64,
    pub center_x_px: f64,
}

/// Board dims from edge length: one cell per `pitch` px, at least 6, rounded
/// up to a multiple of 3 so the 3x3 panels cut on cell boundaries.
fn board_dim(edge_px: f64, pitch: f64) -> usize {
    let n = (edge_px / pitch).round().max(6.0) as usize;
    n.div_ceil(3) * 3
}

#[allow(dead_code)] // only tests call the unscaled form directly
pub fn make_faces(width: u32, height: u32, cube_height: f64, pitch: f64) -> ([Face; 3], f64, f64, f64) {
    make_faces_scaled(width, height, cube_height, pitch, 1.0)
}

/// Faces for a cube drawn at `cube_height * scale`, with Life-board dims
/// pinned to the FULL-size cube's so one set of Sims maps onto every frame
/// of the grow/collapse animation (the cube appears mid-life as it grows —
/// no reseeding). Growth is anchored at the cube center.
pub fn make_faces_scaled(width: u32, height: u32, cube_height: f64, pitch: f64, scale: f64) -> ([Face; 3], f64, f64, f64) {
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;
    let h_full = cube_height.min(height as f64 * 0.9);

    // Board dims from the unscaled cube (e_x.len() / e_z.len() at scale 1).
    let a_full = h_full / 2.0;
    let side_u = board_dim(Vec2 { x: a_full, y: a_full / 2.0 }.len(), pitch);
    let side_v = board_dim(a_full, pitch);

    let h = h_full * scale;
    let a = h / 2.0;

    let apex = Vec2 { x: cx, y: cy - h / 2.0 };
    let e_x = Vec2 { x: a, y: a / 2.0 };
    let e_y = Vec2 { x: -a, y: a / 2.0 };
    let e_z = Vec2 { x: 0.0, y: a };

    let top = Face {
        origin: apex,
        e_u: e_x,
        e_v: e_y,
        board_w: side_u,
        board_h: side_u, // both top axes are cube edges of equal length
    };
    let left = Face {
        origin: Vec2 { x: apex.x + e_y.x, y: apex.y + e_y.y },
        e_u: e_x,
        e_v: e_z,
        board_w: side_u,
        board_h: side_v,
    };
    let right = Face {
        origin: Vec2 { x: apex.x, y: apex.y + a }, // A + e_x + e_y
        e_u: Vec2 { x: a, y: -a / 2.0 },           // -e_y
        e_v: e_z,
        board_w: side_u,
        board_h: side_v,
    };

    ([top, left, right], cy - h / 2.0, cy + h / 2.0, cx)
}

fn panel_of(face: u8, bx: usize, by: usize, bw: usize, bh: usize) -> u8 {
    let pc = (bx * 3 / bw).min(2);
    let pr = (by * 3 / bh).min(2);
    face * 9 + (pr * 3 + pc) as u8
}

pub fn build_grid_map(width: u32, height: u32, cell_w: u32, cell_h: u32, cube_height: f64, pitch: f64) -> GridMap {
    build_grid_map_scaled(width, height, cell_w, cell_h, cube_height, pitch, 1.0)
}

/// Grow/collapse threshold: at or below this scale the cube is not drawn at
/// all (and the degenerate near-zero affine inverse is never evaluated).
pub const MIN_DRAW_SCALE: f64 = 0.02;

pub fn build_grid_map_scaled(width: u32, height: u32, cell_w: u32, cell_h: u32, cube_height: f64, pitch: f64, scale: f64) -> GridMap {
    let cols = (width as usize).div_ceil(cell_w as usize);
    let rows = (height as usize).div_ceil(cell_h as usize);
    let (faces, cube_top_px, cube_bottom_px, center_x_px) =
        make_faces_scaled(width, height, cube_height, pitch, scale);

    let mut cells = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let p = Vec2 {
                x: (col as f64 + 0.5) * cell_w as f64,
                y: (row as f64 + 0.5) * cell_h as f64,
            };
            let mut src = CellSource::Ember;
            if scale > MIN_DRAW_SCALE {
                for (fi, f) in faces.iter().enumerate() {
                    let (u, v) = f.uv(p);
                    if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                        let bx = ((u * f.board_w as f64) as usize).min(f.board_w - 1);
                        let by = ((v * f.board_h as f64) as usize).min(f.board_h - 1);
                        src = CellSource::Face {
                            face: fi as u8,
                            bx: bx as u16,
                            by: by as u16,
                            panel: panel_of(fi as u8, bx, by, f.board_w, f.board_h),
                        };
                        break; // faces only share edges; first hit wins
                    }
                }
            }
            cells.push(src);
        }
    }

    GridMap {
        cols,
        rows,
        cells,
        faces,
        cube_top_px,
        cube_bottom_px,
        center_x_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1920;
    const H: u32 = 1080;

    fn map() -> GridMap {
        build_grid_map(W, H, 10, 20, 360.0, 20.0)
    }

    #[test]
    fn forward_inverse_round_trip() {
        let (faces, ..) = make_faces(W, H, 360.0, 20.0);
        for f in &faces {
            for bx in 0..f.board_w {
                for by in 0..f.board_h {
                    let p = f.cell_center(bx, by);
                    let (u, v) = f.uv(p);
                    assert!((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v));
                    assert_eq!(((u * f.board_w as f64) as usize).min(f.board_w - 1), bx);
                    assert_eq!(((v * f.board_h as f64) as usize).min(f.board_h - 1), by);
                }
            }
        }
    }

    #[test]
    fn faces_do_not_overlap() {
        // Any pixel claimed by one face must not be strictly inside another.
        let (faces, ..) = make_faces(W, H, 360.0, 20.0);
        let eps = 1e-9;
        for (i, f) in faces.iter().enumerate() {
            for bx in 0..f.board_w {
                for by in 0..f.board_h {
                    let p = f.cell_center(bx, by);
                    for (j, g) in faces.iter().enumerate() {
                        if i == j {
                            continue;
                        }
                        let (u, v) = g.uv(p);
                        assert!(
                            !(u > eps && u < 1.0 - eps && v > eps && v < 1.0 - eps),
                            "face {i} cell ({bx},{by}) also strictly inside face {j}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cube_cells_within_bounds_and_plausible_area() {
        let m = map();
        let n_face = m.cells.iter().filter(|c| matches!(c, CellSource::Face { .. })).count();
        // Cube silhouette (hexagon) area = 3/4 * w * h = 0.75 * 360 * 360 px².
        let expect = (0.75 * 360.0 * 360.0) / (10.0 * 20.0);
        let ratio = n_face as f64 / expect;
        assert!(
            (0.85..1.15).contains(&ratio),
            "face cell count {n_face} vs expected ~{expect}"
        );
        // All face cells inside the cube's pixel bounding box.
        for (i, c) in m.cells.iter().enumerate() {
            if matches!(c, CellSource::Face { .. }) {
                let x = ((i % m.cols) as f64 + 0.5) * 10.0;
                let y = ((i / m.cols) as f64 + 0.5) * 20.0;
                assert!(y > m.cube_top_px && y < m.cube_bottom_px);
                assert!((x - m.center_x_px).abs() <= 180.0 + 1e-6);
            }
        }
    }

    #[test]
    fn all_27_panels_reachable() {
        let m = map();
        let mut seen = [false; 27];
        for c in &m.cells {
            if let CellSource::Face { panel, .. } = c {
                seen[*panel as usize] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "panels missing: {seen:?}");
    }

    #[test]
    fn board_dims_are_panel_divisible() {
        let (faces, ..) = make_faces(W, H, 360.0, 20.0);
        for f in &faces {
            assert_eq!(f.board_w % 3, 0);
            assert_eq!(f.board_h % 3, 0);
            assert!(f.board_w >= 6 && f.board_h >= 6);
        }
    }

    #[test]
    fn scale_one_matches_unscaled() {
        let a = build_grid_map(W, H, 10, 20, 360.0, 20.0);
        let b = build_grid_map_scaled(W, H, 10, 20, 360.0, 20.0, 1.0);
        assert_eq!(a.cells, b.cells);
        assert_eq!(a.cube_top_px, b.cube_top_px);
        assert_eq!(a.cube_bottom_px, b.cube_bottom_px);
    }

    #[test]
    fn board_dims_invariant_under_scale() {
        for &s in &[0.05, 0.3, 0.7, 1.0] {
            let (faces, ..) = make_faces_scaled(W, H, 360.0, 20.0, s);
            let (full, ..) = make_faces(W, H, 360.0, 20.0);
            for (f, g) in faces.iter().zip(full.iter()) {
                assert_eq!(f.board_w, g.board_w, "board_w drifted at scale {s}");
                assert_eq!(f.board_h, g.board_h, "board_h drifted at scale {s}");
            }
        }
    }

    #[test]
    fn tiny_scale_is_all_ember() {
        let m = build_grid_map_scaled(W, H, 10, 20, 360.0, 20.0, MIN_DRAW_SCALE);
        assert!(m.cells.iter().all(|c| *c == CellSource::Ember));
    }

    #[test]
    fn mid_scale_cube_is_proportional_and_centered() {
        let m = build_grid_map_scaled(W, H, 10, 20, 360.0, 20.0, 0.5);
        let n_face = m.cells.iter().filter(|c| matches!(c, CellSource::Face { .. })).count();
        // Hexagon area scales with s²: 3/4 * (360*0.5)² px².
        let expect = (0.75 * 180.0 * 180.0) / (10.0 * 20.0);
        let ratio = n_face as f64 / expect;
        assert!((0.8..1.2).contains(&ratio), "half-scale cell count {n_face} vs ~{expect}");
        // Anchored at the cube center: half-scale bounds are cy ± 90.
        for (i, c) in m.cells.iter().enumerate() {
            if matches!(c, CellSource::Face { .. }) {
                let x = ((i % m.cols) as f64 + 0.5) * 10.0;
                let y = ((i / m.cols) as f64 + 0.5) * 20.0;
                assert!(y > m.cube_top_px && y < m.cube_bottom_px);
                assert!((x - m.center_x_px).abs() <= 90.0 + 1e-6);
            }
        }
    }
}
