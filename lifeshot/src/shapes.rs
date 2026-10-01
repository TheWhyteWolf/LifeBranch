// SPDX-License-Identifier: GPL-3.0-or-later
// Annotation shapes and their rasteriser. One renderer serves both the live
// overlay and the exported PNG, so what you see is exactly what you save.
//
// Strokes are drawn by coverage: for each pixel near a segment, coverage falls
// off with distance (anti-aliased, any thickness), and is accumulated into a
// per-shape mask with `max` before being blended once. That is what keeps a
// translucent highlighter stroke from darkening where it overlaps itself.

use crate::image::{Image, Rect};

pub type Pt = (i32, i32);

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect { a: Pt, b: Pt, color: u32, thick: i32, filled: bool },
    Ellipse { a: Pt, b: Pt, color: u32, thick: i32, filled: bool },
    Line { a: Pt, b: Pt, color: u32, thick: i32 },
    Arrow { a: Pt, b: Pt, color: u32, thick: i32 },
    Pen { pts: Vec<Pt>, color: u32, thick: i32 },
    /// Wide translucent freehand stroke (a highlighter).
    Marker { pts: Vec<Pt>, color: u32, thick: i32 },
    Text { at: Pt, text: String, color: u32, px: f32 },
    Pixelate { a: Pt, b: Pt, block: i32 },
    Counter { at: Pt, n: u32, color: u32, r: i32 },
}

/// Highlighter opacity: strong enough to read as a marker, light enough that
/// the text underneath stays legible.
const MARKER_ALPHA: f32 = 0.38;

// ---- text ------------------------------------------------------------------

pub struct Fonts {
    font: fontdue::Font,
}

impl Fonts {
    pub fn load(path: &str) -> Option<Fonts> {
        let bytes = std::fs::read(path).ok()?;
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok().map(|font| Fonts { font })
    }

    pub fn advance(&self, px: f32) -> f32 {
        self.font.metrics('M', px).advance_width
    }

    pub fn line_height(&self, px: f32) -> f32 {
        self.font.horizontal_line_metrics(px).map(|m| m.new_line_size).unwrap_or(px * 1.2)
    }

    pub fn width(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.font.metrics(c, px).advance_width).sum()
    }

    /// Draw `text` with its top-left at (x, y). Returns the x after the last glyph.
    pub fn draw(&self, img: &mut Image, x: i32, y: i32, text: &str, px: f32, rgb: u32) -> i32 {
        let ascent = self.font.horizontal_line_metrics(px).map(|m| m.ascent).unwrap_or(px * 0.8);
        let base = y as f32 + ascent;
        let mut pen = x as f32;
        for ch in text.chars() {
            let (m, cov) = self.font.rasterize(ch, px);
            let gx = pen.round() as i32 + m.xmin;
            let gy = (base - m.height as f32 - m.ymin as f32).round() as i32;
            for ry in 0..m.height {
                for rx in 0..m.width {
                    let c = cov[ry * m.width + rx];
                    if c != 0 {
                        img.blend(gx + rx as i32, gy + ry as i32, rgb, c as f32 / 255.0);
                    }
                }
            }
            pen += m.advance_width;
        }
        pen.round() as i32
    }
}

/// Black or near-white, whichever reads on `rgb`.
pub fn contrast(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
    if (r * 299 + g * 587 + b * 114) / 1000 > 140 { 0x10_1410 } else { 0xf0_f0f0 }
}

// ---- coverage mask ---------------------------------------------------------

struct Mask {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    a: Vec<u8>,
}

impl Mask {
    /// A mask over `bbox` clipped to the image; None if nothing is visible.
    fn over(bbox: Rect, img: &Image) -> Option<Mask> {
        let r = bbox.clip(img.w, img.h);
        (!r.is_empty()).then(|| Mask { x0: r.x, y0: r.y, w: r.w as usize, h: r.h as usize, a: vec![0; r.w as usize * r.h as usize] })
    }

    fn put(&mut self, x: i32, y: i32, cov: f32) {
        let (lx, ly) = (x - self.x0, y - self.y0);
        if lx < 0 || ly < 0 || lx as usize >= self.w || ly as usize >= self.h || cov <= 0.0 {
            return;
        }
        let i = ly as usize * self.w + lx as usize;
        self.a[i] = self.a[i].max((cov.min(1.0) * 255.0) as u8);
    }

    /// A round-capped segment `r` px either side of the line a-b.
    fn segment(&mut self, a: (f32, f32), b: (f32, f32), r: f32) {
        let pad = r + 1.5;
        let (x0, x1) = ((a.0.min(b.0) - pad).floor() as i32, (a.0.max(b.0) + pad).ceil() as i32);
        let (y0, y1) = ((a.1.min(b.1) - pad).floor() as i32, (a.1.max(b.1) + pad).ceil() as i32);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in y0.max(self.y0)..y1.min(self.y0 + self.h as i32) {
            for x in x0.max(self.x0)..x1.min(self.x0 + self.w as i32) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 == 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) };
                let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                self.put(x, y, r + 0.5 - d);
            }
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], r: f32) {
        match pts {
            [] => {}
            [p] => self.segment(*p, *p, r),
            _ => pts.windows(2).for_each(|w| self.segment(w[0], w[1], r)),
        }
    }

    fn fill_rect(&mut self, r: Rect) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.put(x, y, 1.0);
            }
        }
    }

    fn fill_ellipse(&mut self, r: Rect) {
        let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
        let (rx, ry) = ((r.w as f32 / 2.0).max(0.5), (r.h as f32 / 2.0).max(0.5));
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let (nx, ny) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
                // Distance past the edge, in px, along the shorter axis: a cheap
                // anti-aliasing ramp that is exact for circles.
                let d = ((nx * nx + ny * ny).sqrt() - 1.0) * rx.min(ry);
                self.put(x, y, 0.5 - d);
            }
        }
    }

    /// Solid triangle, 3x3 supersampled for soft edges.
    fn fill_triangle(&mut self, t: [(f32, f32); 3]) {
        let min_x = t.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32;
        let max_x = t.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32;
        let min_y = t.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32;
        let max_y = t.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32;
        let side = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let mut hit = 0;
                for sy in 0..3 {
                    for sx in 0..3 {
                        let p = (x as f32 + (sx as f32 + 0.5) / 3.0, y as f32 + (sy as f32 + 0.5) / 3.0);
                        let (d1, d2, d3) = (side(p, t[0], t[1]), side(p, t[1], t[2]), side(p, t[2], t[0]));
                        if (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0) {
                            hit += 1;
                        }
                    }
                }
                self.put(x, y, hit as f32 / 9.0);
            }
        }
    }

    fn apply(&self, img: &mut Image, rgb: u32, alpha: f32) {
        for ly in 0..self.h {
            for lx in 0..self.w {
                let a = self.a[ly * self.w + lx];
                if a > 0 {
                    img.blend(self.x0 + lx as i32, self.y0 + ly as i32, rgb, a as f32 / 255.0 * alpha);
                }
            }
        }
    }
}

fn fp(p: Pt) -> (f32, f32) {
    (p.0 as f32, p.1 as f32)
}

// ---- geometry --------------------------------------------------------------

pub fn arrow_head_len(thick: i32) -> f32 {
    (thick as f32 * 4.0).max(16.0)
}

/// Tight-ish bounds of a shape in image coordinates (for incremental redraw).
pub fn bbox(s: &Shape, fonts: Option<&Fonts>) -> Rect {
    let around = |pts: &[Pt], pad: i32| {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        Rect { x: x0 - pad, y: y0 - pad, w: x1 - x0 + 2 * pad + 1, h: y1 - y0 + 2 * pad + 1 }
    };
    match s {
        Shape::Rect { a, b, thick, .. } | Shape::Ellipse { a, b, thick, .. } => around(&[*a, *b], thick + 2),
        Shape::Line { a, b, thick, .. } => around(&[*a, *b], thick + 2),
        Shape::Arrow { a, b, thick, .. } => around(&[*a, *b], (arrow_head_len(*thick) as i32) + thick + 2),
        Shape::Pen { pts, thick, .. } | Shape::Marker { pts, thick, .. } => around(pts, thick + 2),
        Shape::Pixelate { a, b, .. } => around(&[*a, *b], 0),
        Shape::Counter { at, r, .. } => around(&[*at], r + 2),
        Shape::Text { at, text, px, .. } => {
            let w = fonts.map(|f| f.width(text, *px)).unwrap_or(px * 0.6 * text.chars().count() as f32);
            let h = fonts.map(|f| f.line_height(*px)).unwrap_or(px * 1.3);
            Rect { x: at.0 - 2, y: at.1 - 2, w: w.ceil() as i32 + 4, h: h.ceil() as i32 + 4 }
        }
    }
}

// ---- rendering -------------------------------------------------------------

pub fn render(img: &mut Image, s: &Shape, fonts: Option<&Fonts>) {
    match s {
        Shape::Line { a, b, color, thick } => stroke(img, &[fp(*a), fp(*b)], *thick, *color, 1.0, s, fonts),
        Shape::Pen { pts, color, thick } => {
            let p: Vec<_> = pts.iter().map(|p| fp(*p)).collect();
            stroke(img, &p, *thick, *color, 1.0, s, fonts)
        }
        Shape::Marker { pts, color, thick } => {
            let p: Vec<_> = pts.iter().map(|p| fp(*p)).collect();
            stroke(img, &p, *thick, *color, MARKER_ALPHA, s, fonts)
        }
        Shape::Rect { a, b, color, thick, filled } => {
            let r = Rect::from_points(*a, *b);
            let Some(mut m) = Mask::over(bbox(s, fonts), img) else { return };
            if *filled {
                m.fill_rect(Rect { w: r.w + 1, h: r.h + 1, ..r });
            } else {
                let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
                let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)];
                m.polyline(&corners, *thick as f32 / 2.0);
            }
            m.apply(img, *color, 1.0);
        }
        Shape::Ellipse { a, b, color, thick, filled } => {
            let r = Rect::from_points(*a, *b);
            let Some(mut m) = Mask::over(bbox(s, fonts), img) else { return };
            if *filled {
                m.fill_ellipse(Rect { w: r.w + 1, h: r.h + 1, ..r });
            } else {
                let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
                let (rx, ry) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
                // Enough vertices that the polygon error stays well under a pixel.
                let n = (((rx + ry) * 1.5) as usize).clamp(24, 720);
                let pts: Vec<_> = (0..=n)
                    .map(|i| {
                        let t = i as f32 / n as f32 * std::f32::consts::TAU;
                        (cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect();
                m.polyline(&pts, *thick as f32 / 2.0);
            }
            m.apply(img, *color, 1.0);
        }
        Shape::Arrow { a, b, color, thick } => {
            let (pa, pb) = (fp(*a), fp(*b));
            let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
            let len = (dx * dx + dy * dy).sqrt();
            let Some(mut m) = Mask::over(bbox(s, fonts), img) else { return };
            if len >= 1.0 {
                let (ux, uy) = (dx / len, dy / len);
                let head = arrow_head_len(*thick).min(len.max(4.0));
                // The shaft stops inside the head so its round cap can't poke
                // past the tip.
                let neck = (pb.0 - ux * head * 0.6, pb.1 - uy * head * 0.6);
                m.segment(pa, neck, *thick as f32 / 2.0);
                let base = (pb.0 - ux * head, pb.1 - uy * head);
                let half = head * 0.45;
                m.fill_triangle([pb, (base.0 - uy * half, base.1 + ux * half), (base.0 + uy * half, base.1 - ux * half)]);
            } else {
                m.segment(pa, pb, *thick as f32 / 2.0);
            }
            m.apply(img, *color, 1.0);
        }
        Shape::Pixelate { a, b, block } => {
            // Inclusive of both corner pixels, like a filled rect.
            let r = Rect::from_points(*a, *b);
            pixelate(img, Rect { w: r.w + 1, h: r.h + 1, ..r }, (*block).max(2))
        }
        Shape::Counter { at, n, color, r } => {
            let Some(mut m) = Mask::over(bbox(s, fonts), img) else { return };
            m.fill_ellipse(Rect { x: at.0 - r, y: at.1 - r, w: 2 * r + 1, h: 2 * r + 1 });
            m.apply(img, *color, 1.0);
            if let Some(f) = fonts {
                let px = *r as f32 * 1.35;
                let text = n.to_string();
                let w = f.width(&text, px).round() as i32;
                let h = f.line_height(px).round() as i32;
                f.draw(img, at.0 - w / 2, at.1 - h / 2, &text, px, contrast(*color));
            }
        }
        Shape::Text { at, text, color, px } => {
            if let Some(f) = fonts {
                f.draw(img, at.0, at.1, text, *px, *color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke(img: &mut Image, pts: &[(f32, f32)], thick: i32, color: u32, alpha: f32, s: &Shape, fonts: Option<&Fonts>) {
    let Some(mut m) = Mask::over(bbox(s, fonts), img) else { return };
    m.polyline(pts, thick as f32 / 2.0);
    m.apply(img, color, alpha);
}

/// Replace `region` with `block`-sized flat cells of its average colour.
fn pixelate(img: &mut Image, region: Rect, block: i32) {
    let r = region.clip(img.w, img.h);
    if r.is_empty() {
        return;
    }
    let mut by = r.y;
    while by < r.y + r.h {
        let bh = block.min(r.y + r.h - by);
        let mut bx = r.x;
        while bx < r.x + r.w {
            let bw = block.min(r.x + r.w - bx);
            let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let p = img.px[y as usize * img.w + x as usize];
                    sr += ((p >> 16) & 0xff) as u64;
                    sg += ((p >> 8) & 0xff) as u64;
                    sb += (p & 0xff) as u64;
                }
            }
            let n = (bw * bh) as u64;
            let avg = 0xff00_0000 | (((sr / n) as u32) << 16) | (((sg / n) as u32) << 8) | (sb / n) as u32;
            for y in by..by + bh {
                for x in bx..bx + bw {
                    img.px[y as usize * img.w + x as usize] = avg;
                }
            }
            bx += block;
        }
        by += block;
    }
}

pub fn render_all(img: &mut Image, shapes: &[Shape], fonts: Option<&Fonts>) {
    for s in shapes {
        render(img, s, fonts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: u32 = 0xff20_2020;
    const RED: u32 = 0xffff_0000;

    fn canvas(w: usize, h: usize) -> Image {
        Image::new(w, h, BG)
    }

    fn is_red(i: &Image, x: i32, y: i32) -> bool {
        i.get(x, y).is_some_and(|p| (p >> 16) & 0xff > 0xc0 && (p >> 8) & 0xff < 0x50)
    }

    fn painted(i: &Image) -> usize {
        i.px.iter().filter(|&&p| p != BG).count()
    }

    #[test]
    fn line_has_the_requested_thickness() {
        let mut i = canvas(40, 20);
        render(&mut i, &Shape::Line { a: (5, 10), b: (35, 10), color: RED, thick: 4 }, None);
        // Centre rows solid, a couple of rows away untouched.
        assert!(is_red(&i, 20, 9) && is_red(&i, 20, 10));
        assert_eq!(i.get(20, 6), Some(BG));
        assert_eq!(i.get(20, 14), Some(BG));
        // Rounded end cap reaches a little past the endpoint, not far.
        assert!(is_red(&i, 4, 10));
        assert_eq!(i.get(1, 10), Some(BG));
    }

    #[test]
    fn rect_outline_is_hollow_and_filled_is_solid() {
        let mut hollow = canvas(40, 30);
        render(&mut hollow, &Shape::Rect { a: (5, 5), b: (30, 20), color: RED, thick: 2, filled: false }, None);
        assert!(is_red(&hollow, 5, 12) && is_red(&hollow, 17, 20));
        assert_eq!(hollow.get(17, 12), Some(BG), "centre untouched");

        let mut solid = canvas(40, 30);
        render(&mut solid, &Shape::Rect { a: (5, 5), b: (30, 20), color: RED, thick: 2, filled: true }, None);
        assert!(is_red(&solid, 17, 12) && is_red(&solid, 5, 5) && is_red(&solid, 30, 20));
        assert_eq!(solid.get(31, 21), Some(BG));
    }

    #[test]
    fn reversed_corners_draw_the_same_rect() {
        let (mut a, mut b) = (canvas(30, 30), canvas(30, 30));
        render(&mut a, &Shape::Rect { a: (4, 4), b: (20, 18), color: RED, thick: 3, filled: false }, None);
        render(&mut b, &Shape::Rect { a: (20, 18), b: (4, 4), color: RED, thick: 3, filled: false }, None);
        assert_eq!(a, b);
    }

    #[test]
    fn ellipse_outline_passes_through_the_axes() {
        let mut i = canvas(60, 40);
        render(&mut i, &Shape::Ellipse { a: (10, 5), b: (50, 35), color: RED, thick: 2, filled: false }, None);
        assert!(is_red(&i, 10, 20) && is_red(&i, 50, 20) && is_red(&i, 30, 5) && is_red(&i, 30, 35));
        assert_eq!(i.get(30, 20), Some(BG), "hollow");
        assert_eq!(i.get(11, 6), Some(BG), "bounding-box corner is outside the curve");
        let mut f = canvas(60, 40);
        render(&mut f, &Shape::Ellipse { a: (10, 5), b: (50, 35), color: RED, thick: 2, filled: true }, None);
        assert!(is_red(&f, 30, 20));
        assert_eq!(f.get(11, 6), Some(BG));
    }

    #[test]
    fn arrow_has_a_head_wider_than_its_shaft() {
        let mut i = canvas(80, 40);
        render(&mut i, &Shape::Arrow { a: (5, 20), b: (70, 20), color: RED, thick: 3 }, None);
        assert!(is_red(&i, 30, 20), "shaft");
        assert!(is_red(&i, 67, 20), "tip");
        // Near the tip the head spans more rows than the 3px shaft does.
        let col = |x: i32| (0..40).filter(|&y| is_red(&i, x, y)).count();
        assert!(col(60) > col(30) + 4, "head {} vs shaft {}", col(60), col(30));
        assert_eq!(i.get(75, 20), Some(BG), "nothing past the tip");
    }

    #[test]
    fn degenerate_arrow_does_not_panic() {
        let mut i = canvas(20, 20);
        render(&mut i, &Shape::Arrow { a: (10, 10), b: (10, 10), color: RED, thick: 3 }, None);
        render(&mut i, &Shape::Line { a: (10, 10), b: (10, 10), color: RED, thick: 3 }, None);
    }

    #[test]
    fn pen_follows_its_points() {
        let mut i = canvas(40, 40);
        render(&mut i, &Shape::Pen { pts: vec![(5, 5), (20, 20), (35, 5)], color: RED, thick: 3 }, None);
        assert!(is_red(&i, 20, 20) && is_red(&i, 12, 12) && is_red(&i, 28, 12));
        assert_eq!(i.get(20, 5), Some(BG));
    }

    #[test]
    fn marker_is_translucent_and_does_not_double_darken_on_overlap() {
        let mut once = canvas(40, 20);
        render(&mut once, &Shape::Marker { pts: vec![(5, 10), (35, 10)], color: RED, thick: 8 }, None);
        let mut twice = canvas(40, 20);
        // Retraces the same path: with per-shape max coverage it must look identical.
        render(&mut twice, &Shape::Marker { pts: vec![(5, 10), (35, 10), (5, 10), (35, 10)], color: RED, thick: 8 }, None);
        assert_eq!(once, twice);
        let p = once.get(20, 10).unwrap();
        assert!((p >> 16) & 0xff > 0x20 && (p >> 16) & 0xff < 0xff, "blended, not opaque: {p:#x}");
    }

    #[test]
    fn pixelate_flattens_blocks_to_their_average() {
        let mut i = canvas(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                i.px[y * 16 + x] = 0xff00_0000 | ((x as u32 * 16) << 16) | ((y as u32 * 16) << 8);
            }
        }
        let before = i.clone();
        render(&mut i, &Shape::Pixelate { a: (0, 0), b: (15, 15), block: 8 }, None);
        // Every pixel inside one 8x8 block is now identical.
        assert_eq!(i.get(0, 0), i.get(7, 7));
        assert_eq!(i.get(8, 8), i.get(15, 15));
        assert_ne!(i.get(0, 0), i.get(8, 0));
        assert_ne!(i, before);
        // Outside the region nothing changes.
        let mut j = before.clone();
        render(&mut j, &Shape::Pixelate { a: (0, 0), b: (7, 7), block: 8 }, None);
        assert_eq!(j.get(12, 12), before.get(12, 12));
        assert_eq!(j.get(8, 8), before.get(8, 8), "second corner is inclusive, not beyond");
    }

    #[test]
    fn shapes_clip_at_the_image_edge() {
        let mut i = canvas(20, 20);
        render(&mut i, &Shape::Rect { a: (-30, -30), b: (50, 50), color: RED, thick: 3, filled: false }, None);
        render(&mut i, &Shape::Line { a: (-10, 5), b: (100, 5), color: RED, thick: 3 }, None);
        render(&mut i, &Shape::Pixelate { a: (-5, -5), b: (999, 999), block: 6 }, None);
        render(&mut i, &Shape::Counter { at: (-50, -50), n: 1, color: RED, r: 8 }, None);
    }

    #[test]
    fn counter_is_a_filled_disc() {
        let mut i = canvas(40, 40);
        render(&mut i, &Shape::Counter { at: (20, 20), n: 3, color: RED, r: 10 }, None);
        assert!(is_red(&i, 20, 12) && is_red(&i, 12, 20));
        assert_eq!(i.get(20, 5), Some(BG));
        assert!(painted(&i) > 250 && painted(&i) < 400, "area of r=10 disc, got {}", painted(&i));
    }

    const FONT: &str = "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf";

    #[test]
    fn text_and_counter_digit_draw_when_a_font_is_present() {
        let Some(f) = Fonts::load(FONT) else { return }; // CI image may lack the font
        let mut i = canvas(120, 40);
        render(&mut i, &Shape::Text { at: (4, 4), text: "Hi 42".into(), color: RED, px: 20.0 }, Some(&f));
        assert!(painted(&i) > 30);
        assert!((0..120).any(|x| (0..40).any(|y| is_red(&i, x, y))));
        let mut c = canvas(40, 40);
        render(&mut c, &Shape::Counter { at: (20, 20), n: 7, color: RED, r: 12 }, Some(&f));
        let digit = contrast(RED);
        assert!(c.px.iter().any(|&p| p & 0xffffff == digit & 0xffffff || ((p >> 16) & 0xff > 0xd0 && (p >> 8) & 0xff > 0xd0)));
        assert!(f.width("abc", 20.0) > 0.0);
    }

    #[test]
    fn bbox_covers_what_is_drawn() {
        let shapes = [
            Shape::Line { a: (10, 10), b: (50, 30), color: RED, thick: 5 },
            Shape::Arrow { a: (10, 10), b: (60, 25), color: RED, thick: 4 },
            Shape::Rect { a: (8, 9), b: (40, 44), color: RED, thick: 6, filled: false },
            Shape::Ellipse { a: (8, 9), b: (40, 44), color: RED, thick: 6, filled: false },
            Shape::Pen { pts: vec![(10, 10), (30, 50), (55, 12)], color: RED, thick: 7 },
            Shape::Counter { at: (30, 30), n: 1, color: RED, r: 9 },
        ];
        for s in &shapes {
            let mut i = canvas(80, 70);
            render(&mut i, s, None);
            let bb = bbox(s, None);
            for y in 0..70 {
                for x in 0..80 {
                    if i.get(x, y) != Some(BG) {
                        assert!(bb.contains((x, y)), "{s:?}: pixel ({x},{y}) outside bbox {bb:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn long_freehand_stroke_is_fast_enough() {
        let pts: Vec<Pt> = (0..3000).map(|i| (20 + (i % 900), 20 + ((i * 7) % 500))).collect();
        let mut i = canvas(1000, 600);
        let t = std::time::Instant::now();
        render(&mut i, &Shape::Pen { pts, color: RED, thick: 4 }, None);
        assert!(t.elapsed().as_secs() < 8, "pen too slow: {:?}", t.elapsed());
    }
}
