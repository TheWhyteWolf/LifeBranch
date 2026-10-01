// SPDX-License-Identifier: GPL-3.0-or-later
// Every cursor, drawn on a 32×32 grid. Each is a stack of layers painted in
// order; an outlined layer gets the theme's fill inside, then a ring, then a
// thin edge, so a badge drawn after the arrow sits on top of it cleanly.

use crate::sdf::{rot, Shape, Shape::*, P};
use std::f32::consts::PI;

pub enum Layer {
    /// Filled and outlined.
    Solid(Shape),
    /// Painted in the ring colour with no outline: detail inside a solid.
    Detail(Shape),
}

pub struct Cursor {
    pub name: &'static str,
    /// Other names apps ask for that mean the same cursor (symlinks).
    pub aliases: &'static [&'static str],
    pub hot: P,
    /// One entry per animation frame (most cursors have one).
    pub frames: Vec<Vec<Layer>>,
    /// Milliseconds per frame.
    pub delay: u32,
}

fn arrow() -> Shape {
    Poly(vec![(3.5, 3.0), (3.5, 24.2), (8.8, 19.4), (12.4, 27.6), (16.4, 25.9), (12.8, 17.9), (19.8, 17.9)])
}

/// The arrow with `badge` layers on top, sharing its hotspot.
fn arrow_with(name: &'static str, aliases: &'static [&'static str], badge: Vec<Layer>) -> Cursor {
    let mut layers = vec![Layer::Solid(arrow())];
    layers.extend(badge);
    Cursor { name, aliases, hot: (3.5, 3.0), frames: vec![layers], delay: 0 }
}

/// A spinner: a three-quarter ring turning clockwise over 12 frames.
fn spinner(c: P, r: f32, hw: f32, under: impl Fn() -> Vec<Layer>) -> (Vec<Vec<Layer>>, u32) {
    let frames = (0..12)
        .map(|i| {
            let mut l = under();
            l.push(Layer::Solid(Arc(c, r, hw, i as f32 * PI / 6.0, 1.5 * PI)));
            l
        })
        .collect();
    (frames, 70)
}

/// A double-headed arrow along x through the centre, turned `deg`.
fn double_arrow(deg: f32) -> Shape {
    Union(vec![
        Capsule((9.0, 16.0), (23.0, 16.0), 1.5),
        Poly(vec![(3.5, 16.0), (10.5, 9.0), (10.5, 23.0)]),
        Poly(vec![(28.5, 16.0), (21.5, 23.0), (21.5, 9.0)]),
    ])
    .map(&rot(deg))
}

fn ibeam() -> Shape {
    Union(vec![
        Capsule((16.0, 7.0), (16.0, 25.0), 1.1),
        Capsule((12.0, 5.5), (20.0, 5.5), 1.1),
        Capsule((12.0, 26.5), (20.0, 26.5), 1.1),
    ])
}

fn single(name: &'static str, aliases: &'static [&'static str], hot: P, s: Shape) -> Cursor {
    Cursor { name, aliases, hot, frames: vec![vec![Layer::Solid(s)]], delay: 0 }
}

pub fn all() -> Vec<Cursor> {
    let c = (16.0, 16.0);
    let (wait, wait_delay) = spinner(c, 8.5, 2.3, Vec::new);
    let (progress, progress_delay) = spinner((24.0, 24.0), 4.0, 1.5, || vec![Layer::Solid(arrow())]);
    let mirror = |p: P| (32.0 - p.0, p.1);
    let magnifier = || {
        vec![Layer::Solid(Union(vec![
            Arc((13.0, 13.0), 7.5, 1.7, 0.0, 2.0 * PI),
            Capsule((18.6, 18.6), (26.5, 26.5), 2.4),
        ]))]
    };
    let mut zoom_in = magnifier();
    zoom_in.push(Layer::Solid(Union(vec![
        Capsule((13.0, 10.0), (13.0, 16.0), 1.0),
        Capsule((10.0, 13.0), (16.0, 13.0), 1.0),
    ])));
    let mut zoom_out = magnifier();
    zoom_out.push(Layer::Solid(Capsule((10.0, 13.0), (16.0, 13.0), 1.0)));

    vec![
        arrow_with("default", &["left_ptr", "arrow", "top_left_arrow", "left-arrow"], vec![]),
        single("right_ptr", &["right-arrow"], mirror((3.5, 3.0)), arrow().map(&mirror)),
        single(
            "pointer",
            &["hand", "hand1", "hand2", "pointing_hand", "9d800788f1b08800ae810202380a0822", "e29285e634086352946a0e7090d73106"],
            (12.0, 2.5),
            Union(vec![
                Capsule((12.0, 4.8), (12.0, 17.0), 2.3),
                Capsule((16.4, 12.0), (16.4, 19.0), 2.2),
                Capsule((20.6, 13.2), (20.6, 20.0), 2.1),
                Capsule((24.4, 15.2), (24.4, 21.0), 1.9),
                RoundBox((9.8, 16.0), (26.3, 27.8), 4.5),
                Capsule((6.4, 17.6), (10.6, 23.6), 2.2),
            ]),
        ),
        single(
            "grab",
            &["openhand", "fleur-hand", "5aca4d189052212118709018842178c0"],
            c,
            Union(vec![
                Capsule((11.9, 7.0), (12.5, 16.0), 2.1),
                Capsule((16.4, 5.0), (16.4, 16.0), 2.1),
                Capsule((20.6, 6.2), (20.3, 16.0), 2.0),
                Capsule((24.4, 9.4), (23.8, 17.0), 1.8),
                Capsule((6.0, 14.2), (10.2, 20.5), 2.1),
                RoundBox((9.8, 14.5), (26.0, 27.5), 4.5),
            ]),
        ),
        single(
            "grabbing",
            &["closedhand", "dnd-move", "dnd-none", "208530c400c041818281048008011002"],
            c,
            Union(vec![
                Circle((12.0, 11.8), 2.4),
                Circle((16.4, 10.8), 2.4),
                Circle((20.8, 11.3), 2.3),
                Circle((24.6, 13.0), 2.0),
                Capsule((7.4, 16.0), (11.0, 19.5), 2.2),
                RoundBox((9.6, 11.5), (26.4, 26.0), 4.8),
            ]),
        ),
        single("text", &["xterm", "ibeam"], c, ibeam()),
        single("vertical-text", &[], c, ibeam().map(&rot(90.0))),
        single(
            "crosshair",
            &["cross", "tcross", "cross_reverse", "diamond_cross"],
            c,
            Union(vec![
                Capsule((16.0, 4.0), (16.0, 12.5), 1.1),
                Capsule((16.0, 19.5), (16.0, 28.0), 1.1),
                Capsule((4.0, 16.0), (12.5, 16.0), 1.1),
                Capsule((19.5, 16.0), (28.0, 16.0), 1.1),
                Circle(c, 1.3),
            ]),
        ),
        single(
            "cell",
            &["plus"],
            c,
            Union(vec![RoundBox((12.5, 5.5), (19.5, 26.5), 1.2), RoundBox((5.5, 12.5), (26.5, 19.5), 1.2)]),
        ),
        single(
            "move",
            &["fleur", "all-scroll", "size_all", "4498f0e0c1937ffe01fd06f973665830", "9081237383d90e509aa00f00170e968f"],
            c,
            Union(vec![
                Capsule((8.0, 16.0), (24.0, 16.0), 1.4),
                Capsule((16.0, 8.0), (16.0, 24.0), 1.4),
                Poly(vec![(3.0, 16.0), (8.5, 12.3), (8.5, 19.7)]),
                Poly(vec![(29.0, 16.0), (23.5, 19.7), (23.5, 12.3)]),
                Poly(vec![(16.0, 3.0), (19.7, 8.5), (12.3, 8.5)]),
                Poly(vec![(16.0, 29.0), (12.3, 23.5), (19.7, 23.5)]),
            ]),
        ),
        single(
            "ew-resize",
            &["e-resize", "w-resize", "col-resize", "h_double_arrow", "sb_h_double_arrow", "size_hor", "left_side", "right_side", "split_h", "028006030e0e7ebffc7f7070c0600140", "14fef782d02440884392942c11205230"],
            c,
            double_arrow(0.0),
        ),
        single(
            "ns-resize",
            &["n-resize", "s-resize", "row-resize", "v_double_arrow", "sb_v_double_arrow", "size_ver", "top_side", "bottom_side", "split_v", "00008160000006810000408080010102", "2870a09082c103050810ffdffffe0204"],
            c,
            double_arrow(90.0),
        ),
        single(
            "nwse-resize",
            &["nw-resize", "se-resize", "size_fdiag", "bd_double_arrow", "top_left_corner", "bottom_right_corner", "c7088f0f3e6c8088236ef8e1e3e70000"],
            c,
            double_arrow(45.0),
        ),
        single(
            "nesw-resize",
            &["ne-resize", "sw-resize", "size_bdiag", "fd_double_arrow", "top_right_corner", "bottom_left_corner", "fcf1c3c7cd4491d801f1e1c78f100000"],
            c,
            double_arrow(-45.0),
        ),
        single(
            "not-allowed",
            &["no-drop", "dnd-no-drop", "forbidden", "crossed_circle", "circle", "X_cursor", "03b6e0fcb3499374a867c041f52298f0"],
            c,
            Union(vec![Arc(c, 9.0, 2.0, 0.0, 2.0 * PI), Capsule((9.6, 9.6), (22.4, 22.4), 2.0)]),
        ),
        Cursor { name: "wait", aliases: &["watch", "clock"], hot: c, frames: wait, delay: wait_delay },
        Cursor {
            name: "progress",
            aliases: &["left_ptr_watch", "half-busy", "00000000000000020006000e7e9ffc3f", "08e8e1c95fe2fc01f976f1e063a24ccd", "3ecb610c1bf2410f44200f48c40d3599"],
            hot: (3.5, 3.0),
            frames: progress,
            delay: progress_delay,
        },
        arrow_with(
            "help",
            &["question_arrow", "whats_this", "left_ptr_help", "5c6cd98b3f3ebcb1f9c7f1c204630408", "d9ce0ab605698f320427677b458ad60b"],
            vec![Layer::Solid(Union(vec![
                Arc((24.0, 19.0), 3.3, 1.25, PI, 4.0 * PI / 3.0),
                Capsule((25.65, 21.86), (24.0, 23.6), 1.25),
                Capsule((24.0, 23.6), (24.0, 24.8), 1.25),
                Circle((24.0, 28.4), 1.4),
            ]))],
        ),
        arrow_with(
            "copy",
            &["dnd-copy", "1081e37283d90000800003c07f3ef6bf", "6407b0e94181790501fd1e167b474872", "b66166c04f8c3109214a4fbd64a50fc8"],
            vec![Layer::Solid(Union(vec![
                Capsule((24.0, 19.0), (24.0, 28.5), 1.4),
                Capsule((19.25, 23.75), (28.75, 23.75), 1.4),
            ]))],
        ),
        arrow_with(
            "alias",
            &["link", "dnd-link", "3085a0e285430894940527032f8b26df", "640fb0e74195791501fd1ed57b41487f", "a2a266d0498c3104214a47bd64ab0fc8"],
            vec![Layer::Solid(Union(vec![
                Capsule((20.5, 28.0), (26.0, 22.5), 1.3),
                Poly(vec![(29.0, 19.5), (21.5, 20.3), (28.2, 27.0)]),
            ]))],
        ),
        arrow_with(
            "context-menu",
            &[],
            vec![
                Layer::Solid(RoundBox((19.0, 17.5), (29.0, 28.5), 1.2)),
                Layer::Detail(Union(vec![
                    Capsule((21.3, 20.5), (26.7, 20.5), 0.6),
                    Capsule((21.3, 23.0), (26.7, 23.0), 0.6),
                    Capsule((21.3, 25.5), (26.7, 25.5), 0.6),
                ])),
            ],
        ),
        Cursor { name: "zoom-in", aliases: &[], hot: (13.0, 13.0), frames: vec![zoom_in], delay: 0 },
        Cursor { name: "zoom-out", aliases: &[], hot: (13.0, 13.0), frames: vec![zoom_out], delay: 0 },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_core_names_exist() {
        let cs = all();
        let mut names: Vec<&str> = cs.iter().flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied())).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n, "a name is used twice");
        for want in ["default", "left_ptr", "pointer", "text", "wait", "progress", "not-allowed", "ew-resize", "ns-resize", "nwse-resize", "nesw-resize", "grab", "grabbing", "crosshair", "move", "help"] {
            assert!(names.contains(&want), "missing {want}");
        }
    }

    #[test]
    fn hotspots_sit_on_their_shapes() {
        for c in all() {
            assert!((0.0..32.0).contains(&c.hot.0) && (0.0..32.0).contains(&c.hot.1));
            if matches!(c.name, "wait" | "not-allowed") {
                continue; // aimed at the centre of a ring
            }
            let d = c.frames[0]
                .iter()
                .map(|l| match l {
                    Layer::Solid(s) | Layer::Detail(s) => s.dist(c.hot),
                })
                .fold(f32::INFINITY, f32::min);
            assert!(d < 1.6, "{}: hotspot {d} units from the shape", c.name);
        }
    }
}
