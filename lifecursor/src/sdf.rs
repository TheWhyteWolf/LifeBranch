// SPDX-License-Identifier: GPL-3.0-or-later
// Signed distance to the shapes cursors are drawn from, in design units (a
// 32×32 grid): negative inside, positive outside. One distance gives the fill,
// the ring and the edge, each anti-aliased, at any size.

use std::f32::consts::TAU;

pub type P = (f32, f32);

#[derive(Clone, Debug)]
pub enum Shape {
    Poly(Vec<P>),
    /// A line with round ends: from, to, radius.
    Capsule(P, P, f32),
    Circle(P, f32),
    /// Rounded rectangle: min corner, max corner, corner radius.
    RoundBox(P, P, f32),
    /// Part of a ring: centre, radius, half the stroke width, start angle,
    /// sweep (radians, y down, so +sweep turns clockwise on screen).
    Arc(P, f32, f32, f32, f32),
    Union(Vec<Shape>),
}

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn dot(a: P, b: P) -> f32 {
    a.0 * b.0 + a.1 * b.1
}
fn len(a: P) -> f32 {
    dot(a, a).sqrt()
}

fn seg(p: P, a: P, b: P) -> f32 {
    let (pa, ba) = (sub(p, a), sub(b, a));
    let h = (dot(pa, ba) / dot(ba, ba).max(1e-9)).clamp(0.0, 1.0);
    len(sub(pa, (ba.0 * h, ba.1 * h)))
}

fn poly(p: P, v: &[P]) -> f32 {
    let mut d = dot(sub(p, v[0]), sub(p, v[0]));
    let mut s = 1.0;
    let mut j = v.len() - 1;
    for i in 0..v.len() {
        let e = sub(v[j], v[i]);
        let w = sub(p, v[i]);
        let h = (dot(w, e) / dot(e, e).max(1e-9)).clamp(0.0, 1.0);
        let b = sub(w, (e.0 * h, e.1 * h));
        d = d.min(dot(b, b));
        let c = [p.1 >= v[i].1, p.1 < v[j].1, e.0 * w.1 > e.1 * w.0];
        if c.iter().all(|&x| x) || c.iter().all(|&x| !x) {
            s = -s;
        }
        j = i;
    }
    s * d.sqrt()
}

impl Shape {
    pub fn dist(&self, p: P) -> f32 {
        match self {
            Shape::Poly(v) => poly(p, v),
            Shape::Capsule(a, b, r) => seg(p, *a, *b) - r,
            Shape::Circle(c, r) => len(sub(p, *c)) - r,
            Shape::RoundBox(lo, hi, r) => {
                let c = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
                let half = ((hi.0 - lo.0) / 2.0, (hi.1 - lo.1) / 2.0);
                let q = ((p.0 - c.0).abs() - half.0 + r, (p.1 - c.1).abs() - half.1 + r);
                len((q.0.max(0.0), q.1.max(0.0))) + q.0.max(q.1).min(0.0) - r
            }
            Shape::Arc(c, r, hw, a0, sweep) => {
                let d = sub(p, *c);
                let rel = (d.1.atan2(d.0) - a0).rem_euclid(TAU);
                if rel <= *sweep {
                    (len(d) - r).abs() - hw
                } else {
                    let end = |a: f32| (c.0 + r * a.cos(), c.1 + r * a.sin());
                    len(sub(p, end(*a0))).min(len(sub(p, end(a0 + sweep)))) - hw
                }
            }
            Shape::Union(v) => v.iter().map(|s| s.dist(p)).fold(f32::INFINITY, f32::min),
        }
    }

    /// The shape moved by `f`, a rigid map (rotation, mirror, translation):
    /// radii are unchanged and only points move.
    pub fn map(&self, f: &dyn Fn(P) -> P) -> Shape {
        match self {
            Shape::Poly(v) => Shape::Poly(v.iter().map(|&p| f(p)).collect()),
            Shape::Capsule(a, b, r) => Shape::Capsule(f(*a), f(*b), *r),
            Shape::Circle(c, r) => Shape::Circle(f(*c), *r),
            // Only used axis-aligned; rotating one by 90° swaps its corners.
            Shape::RoundBox(lo, hi, r) => {
                let (a, b) = (f(*lo), f(*hi));
                Shape::RoundBox((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), *r)
            }
            Shape::Arc(c, r, hw, a0, sw) => {
                // Find where the start point went to recover the new angle.
                let p0 = f((c.0 + r * a0.cos(), c.1 + r * a0.sin()));
                let p1 = f((c.0 + r * (a0 + 0.5).cos(), c.1 + r * (a0 + 0.5).sin()));
                let nc = f(*c);
                let na0 = (p0.1 - nc.1).atan2(p0.0 - nc.0);
                let na1 = (p1.1 - nc.1).atan2(p1.0 - nc.0);
                let turns_same = (na1 - na0).rem_euclid(TAU) < std::f32::consts::PI;
                if turns_same {
                    Shape::Arc(nc, *r, *hw, na0, *sw)
                } else {
                    Shape::Arc(nc, *r, *hw, na0 - sw, *sw) // a mirror reverses direction
                }
            }
            Shape::Union(v) => Shape::Union(v.iter().map(|s| s.map(f)).collect()),
        }
    }
}

/// Rotation by `deg` about the grid centre.
pub fn rot(deg: f32) -> impl Fn(P) -> P {
    let (s, c) = deg.to_radians().sin_cos();
    move |(x, y)| {
        let (dx, dy) = (x - 16.0, y - 16.0);
        (16.0 + dx * c - dy * s, 16.0 + dx * s + dy * c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn signs_and_distances() {
        let sq = Shape::Poly(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        assert!(close(sq.dist((5.0, 5.0)), -5.0));
        assert!(close(sq.dist((13.0, 5.0)), 3.0));
        assert!(close(Shape::Circle((0.0, 0.0), 2.0).dist((5.0, 0.0)), 3.0));
        assert!(close(Shape::Capsule((0.0, 0.0), (10.0, 0.0), 1.0).dist((5.0, 3.0)), 2.0));
        let b = Shape::RoundBox((0.0, 0.0), (10.0, 4.0), 1.0);
        assert!(b.dist((5.0, 2.0)) < 0.0 && close(b.dist((5.0, 7.0)), 3.0));
    }

    #[test]
    fn arc_covers_its_sweep_only() {
        // Top half of a ring (y down: from 180° clockwise through 270° to 360°).
        let a = Shape::Arc((0.0, 0.0), 5.0, 0.5, std::f32::consts::PI, std::f32::consts::PI);
        assert!(a.dist((0.0, -5.0)) < 0.0, "top is drawn");
        assert!(a.dist((0.0, 5.0)) > 4.0, "bottom is not");
    }

    #[test]
    fn rotation_keeps_distances() {
        let s = Shape::Capsule((10.0, 16.0), (22.0, 16.0), 1.0);
        let r = s.map(&rot(90.0));
        assert!(close(r.dist((16.0, 10.0)), -1.0));
        assert!(close(s.dist((16.0, 10.0)), 5.0));
    }
}
