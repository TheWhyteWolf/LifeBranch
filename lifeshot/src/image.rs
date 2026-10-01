// SPDX-License-Identifier: GPL-3.0-or-later
// A plain pixel buffer: 0xAARRGGBB per pixel (the layout wl_shm calls
// Argb8888/Xrgb8888 on little-endian), row-major, no padding. Screenshots are
// opaque, so alpha is carried but only the colour matters downstream.

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Integer rectangle, x/y may be negative before clipping; w/h non-negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Normalised rect spanning two corners in any order.
    pub fn from_points(a: (i32, i32), b: (i32, i32)) -> Rect {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    pub fn contains(&self, p: (i32, i32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The part of `self` inside a `w` x `h` canvas.
    pub fn clip(&self, w: usize, h: usize) -> Rect {
        let x0 = self.x.max(0);
        let y0 = self.y.max(0);
        let x1 = (self.x + self.w).min(w as i32);
        let y1 = (self.y + self.h).min(h as i32);
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }
}

impl Image {
    pub fn new(w: usize, h: usize, fill: u32) -> Image {
        Image { w, h, px: vec![fill; w * h] }
    }

    #[cfg(test)]
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| self.px[y as usize * self.w + x as usize])
    }

    /// Blend `rgb` over the pixel at `alpha` (0..=1); out-of-bounds is ignored.
    pub fn blend(&mut self, x: i32, y: i32, rgb: u32, alpha: f32) {
        if alpha <= 0.0 || x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        let d = self.px[i];
        let a = alpha.min(1.0);
        let mix = |s: u32, d: u32| (s as f32 * a + d as f32 * (1.0 - a)).round() as u32;
        let r = mix((rgb >> 16) & 0xff, (d >> 16) & 0xff);
        let g = mix((rgb >> 8) & 0xff, (d >> 8) & 0xff);
        let b = mix(rgb & 0xff, d & 0xff);
        self.px[i] = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }

    /// A copy of the `r` region (clipped to the image).
    pub fn crop(&self, r: Rect) -> Image {
        let r = r.clip(self.w, self.h);
        let mut out = Image::new(r.w as usize, r.h as usize, 0xff00_0000);
        for y in 0..r.h as usize {
            let src = (r.y as usize + y) * self.w + r.x as usize;
            out.px[y * out.w..(y + 1) * out.w].copy_from_slice(&self.px[src..src + r.w as usize]);
        }
        out
    }

    /// Multiply every pixel's colour by `f` (0..=1): the dimmed backdrop.
    pub fn dimmed(&self, f: f32) -> Image {
        let mut out = self.clone();
        for p in &mut out.px {
            let s = |c: u32| ((c as f32) * f) as u32;
            *p = 0xff00_0000 | (s((*p >> 16) & 0xff) << 16) | (s((*p >> 8) & 0xff) << 8) | s(*p & 0xff);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_any_corner_order() {
        assert_eq!(Rect::from_points((10, 20), (4, 2)), Rect { x: 4, y: 2, w: 6, h: 18 });
    }

    #[test]
    fn clip_and_contains() {
        let r = Rect { x: -5, y: 8, w: 20, h: 20 };
        assert_eq!(r.clip(10, 10), Rect { x: 0, y: 8, w: 10, h: 2 });
        assert!(Rect { x: 0, y: 0, w: 2, h: 2 }.contains((1, 1)));
        assert!(!Rect { x: 0, y: 0, w: 2, h: 2 }.contains((2, 1)));
        assert!(Rect { x: 50, y: 50, w: 1, h: 1 }.clip(10, 10).is_empty());
    }

    #[test]
    fn blend_mixes_and_ignores_out_of_bounds() {
        let mut i = Image::new(2, 2, 0xff00_0000);
        i.blend(0, 0, 0xffffff, 0.5);
        assert_eq!(i.get(0, 0), Some(0xff80_8080));
        i.blend(-1, 0, 0xffffff, 1.0);
        i.blend(5, 5, 0xffffff, 1.0);
        assert_eq!(i.get(1, 1), Some(0xff00_0000));
    }

    #[test]
    fn crop_copies_the_region() {
        let mut i = Image::new(4, 4, 0xff00_0000);
        i.px[1 * 4 + 2] = 0xffff_0000;
        let c = i.crop(Rect { x: 2, y: 1, w: 2, h: 2 });
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.px[0], 0xffff_0000);
    }
}
