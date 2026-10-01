// SPDX-License-Identifier: GPL-3.0-or-later
// Paints a cursor's layers into premultiplied ARGB pixels at a pixel size.

use crate::cursors::Layer;

#[derive(Clone, Copy)]
pub struct Style {
    pub fill: [f32; 3],
    pub ring: [f32; 3],
    pub edge: [f32; 3],
}

pub const DARK: Style = Style { fill: [0.05, 0.05, 0.05], ring: [1.0, 1.0, 1.0], edge: [0.0, 0.0, 0.0] };
pub const LIGHT: Style = Style { fill: [0.96, 0.96, 0.96], ring: [0.04, 0.04, 0.04], edge: [1.0, 1.0, 1.0] };

/// Ring and edge widths in design units, with floors in pixels so small
/// cursors keep a visible outline.
const RING: f32 = 1.6;
const EDGE: f32 = 0.7;

fn cov(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Premultiplied ARGB, row-major, `size`×`size`.
pub fn paint(layers: &[Layer], size: u32, st: Style) -> Vec<u32> {
    let k = size as f32 / 32.0;
    let ring = (RING * k).max(1.2);
    let edge = (EDGE * k).max(0.6);
    let mut out = vec![0u32; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
            let mut acc = [0.0f32; 4]; // premultiplied r, g, b, a
            for l in layers {
                let (src, a) = match l {
                    Layer::Solid(s) => {
                        let d = s.dist(p) * k;
                        let (ae, ar, af) = (cov(d - ring - edge), cov(d - ring), cov(d));
                        let mut c = [0.0; 3];
                        for i in 0..3 {
                            c[i] = st.edge[i] * (ae - ar) + st.ring[i] * (ar - af) + st.fill[i] * af;
                        }
                        (c, ae)
                    }
                    Layer::Detail(s) => {
                        let a = cov(s.dist(p) * k);
                        (st.ring.map(|v| v * a), a)
                    }
                };
                for i in 0..3 {
                    acc[i] = src[i] + acc[i] * (1.0 - a);
                }
                acc[3] = a + acc[3] * (1.0 - a);
            }
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
            out[(y * size + x) as usize] = b(acc[3]) << 24 | b(acc[0]) << 16 | b(acc[1]) << 8 | b(acc[2]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdf::Shape;

    #[test]
    fn fill_ring_edge_from_inside_out_and_premultiplied() {
        let l = [Layer::Solid(Shape::Circle((16.0, 16.0), 8.0))];
        let px = paint(&l, 32, DARK);
        let at = |x: u32| px[(16 * 32 + x) as usize];
        assert_eq!(at(16), 0xff0d0d0d, "fill");
        assert_eq!(at(24) & 0xffffff, 0xffffff, "ring just outside the shape");
        assert_eq!(at(31), 0, "transparent beyond the edge");
        for p in px {
            let a = p >> 24;
            assert!((p >> 16 & 255) <= a && (p >> 8 & 255) <= a && (p & 255) <= a);
        }
    }
}
