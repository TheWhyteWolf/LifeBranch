// SPDX-License-Identifier: GPL-3.0-or-later
// The annotation editor, with no Wayland in it: pointer/key events in image
// coordinates go in, a list of Shapes comes out. The overlay (app.rs) only maps
// real input onto these calls and paints the result, which keeps all the
// behaviour that matters — tools, undo/redo, constrain, numbering, export —
// unit-testable.

use crate::image::{Image, Rect};
use crate::shapes::{self, Fonts, Pt, Shape};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Marker,
    Text,
    Pixelate,
    Counter,
}

pub struct Editor {
    pub shapes: Vec<Shape>,
    redo: Vec<Shape>,
    pub tool: Tool,
    pub color: u32,
    pub thick: i32,
    pub filled: bool,
    pub text_px: f32,
    /// The shape being dragged out right now.
    current: Option<Shape>,
    /// Text being typed: where it goes and what's been entered so far.
    pub typing: Option<(Pt, String)>,
    counter_next: u32,
    anchor: Pt,
}

impl Editor {
    pub fn new(color: u32) -> Editor {
        Editor {
            shapes: Vec::new(),
            redo: Vec::new(),
            tool: Tool::Arrow,
            color,
            thick: 4,
            filled: false,
            text_px: 22.0,
            current: None,
            typing: None,
            counter_next: 1,
            anchor: (0, 0),
        }
    }

    /// Snap `to` so the vector anchor->to lies on a 45° axis (Line/Arrow), or
    /// forms a square (Rect/Ellipse/Pixelate).
    fn constrained(&self, to: Pt) -> Pt {
        let (dx, dy) = (to.0 - self.anchor.0, to.1 - self.anchor.1);
        match self.tool {
            Tool::Line | Tool::Arrow => {
                let ang = (dy as f32).atan2(dx as f32);
                let snapped = (ang / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
                let len = ((dx * dx + dy * dy) as f32).sqrt();
                (self.anchor.0 + (len * snapped.cos()).round() as i32, self.anchor.1 + (len * snapped.sin()).round() as i32)
            }
            Tool::Rect | Tool::Ellipse | Tool::Pixelate => {
                // Sign of each axis (a zero delta counts as positive), so dragging
                // up-left keeps the square on the side the pointer is on.
                let side = dx.abs().max(dy.abs());
                let sign = |d: i32| if d < 0 { -1 } else { 1 };
                (self.anchor.0 + side * sign(dx), self.anchor.1 + side * sign(dy))
            }
            _ => to,
        }
    }

    fn make(&self, a: Pt, b: Pt) -> Option<Shape> {
        let (color, thick) = (self.color, self.thick);
        Some(match self.tool {
            Tool::Line => Shape::Line { a, b, color, thick },
            Tool::Arrow => Shape::Arrow { a, b, color, thick },
            Tool::Rect => Shape::Rect { a, b, color, thick, filled: self.filled },
            Tool::Ellipse => Shape::Ellipse { a, b, color, thick, filled: self.filled },
            Tool::Pixelate => Shape::Pixelate { a, b, block: (thick * 3).max(8) },
            Tool::Pen => Shape::Pen { pts: vec![a], color, thick },
            Tool::Marker => Shape::Marker { pts: vec![a], color, thick: thick * 4 },
            Tool::Text | Tool::Counter => return None,
        })
    }

    fn push(&mut self, s: Shape) {
        self.redo.clear();
        self.shapes.push(s);
    }

    pub fn press(&mut self, p: Pt) {
        self.commit_text();
        self.anchor = p;
        match self.tool {
            Tool::Text => self.typing = Some((p, String::new())),
            Tool::Counter => {
                let n = self.counter_next;
                self.counter_next += 1;
                let r = 8 + self.thick * 2;
                self.push(Shape::Counter { at: p, n, color: self.color, r });
            }
            _ => self.current = self.make(p, p),
        }
    }

    pub fn drag(&mut self, p: Pt, constrain: bool) {
        let to = if constrain { self.constrained(p) } else { p };
        match &mut self.current {
            Some(Shape::Pen { pts, .. }) | Some(Shape::Marker { pts, .. }) => {
                if pts.last() != Some(&p) {
                    pts.push(p);
                }
            }
            Some(Shape::Line { b, .. })
            | Some(Shape::Arrow { b, .. })
            | Some(Shape::Rect { b, .. })
            | Some(Shape::Ellipse { b, .. })
            | Some(Shape::Pixelate { b, .. }) => *b = to,
            _ => {}
        }
    }

    pub fn release(&mut self, p: Pt, constrain: bool) {
        self.drag(p, constrain);
        let Some(s) = self.current.take() else { return };
        // A click that never moved shouldn't leave an invisible sliver behind
        // (a pen/marker dot is the exception: that's a deliberate mark).
        let keep = match &s {
            Shape::Line { a, b, .. } | Shape::Arrow { a, b, .. } | Shape::Rect { a, b, .. } | Shape::Ellipse { a, b, .. } => {
                (a.0 - b.0).abs() + (a.1 - b.1).abs() >= 3
            }
            Shape::Pixelate { a, b, .. } => (a.0 - b.0).abs() >= 2 && (a.1 - b.1).abs() >= 2,
            _ => true,
        };
        if keep {
            self.push(s);
        }
    }

    // ---- text ----
    pub fn type_char(&mut self, c: char) {
        if let Some((_, t)) = &mut self.typing {
            if !c.is_control() {
                t.push(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some((_, t)) = &mut self.typing {
            t.pop();
        }
    }

    pub fn commit_text(&mut self) {
        if let Some((at, text)) = self.typing.take() {
            if !text.trim().is_empty() {
                let s = Shape::Text { at, text, color: self.color, px: self.text_px };
                self.push(s);
            }
        }
    }

    pub fn cancel_text(&mut self) {
        self.typing = None;
    }

    // ---- history ----
    pub fn undo(&mut self) {
        self.current = None;
        if self.typing.is_some() {
            self.typing = None; // undo the half-typed text first
            return;
        }
        if let Some(s) = self.shapes.pop() {
            if let Shape::Counter { .. } = s {
                self.counter_next = self.counter_next.saturating_sub(1).max(1);
            }
            self.redo.push(s);
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            if let Shape::Counter { n, .. } = &s {
                self.counter_next = self.counter_next.max(n + 1);
            }
            self.shapes.push(s);
        }
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty() || self.typing.is_some()
    }

    /// What to draw on top of the committed shapes while editing: the shape
    /// under the pointer, and the text being typed (with its caret).
    pub fn live(&self) -> Vec<Shape> {
        let mut v = Vec::new();
        if let Some(s) = &self.current {
            v.push(s.clone());
        }
        if let Some((at, t)) = &self.typing {
            v.push(Shape::Text { at: *at, text: format!("{t}\u{2502}"), color: self.color, px: self.text_px });
        }
        v
    }
}

/// The finished screenshot: the capture with every shape drawn on, cropped to
/// the selection.
pub fn export(base: &Image, sel: Rect, shapes: &[Shape], fonts: Option<&Fonts>) -> Image {
    let mut img = base.clone();
    shapes::render_all(&mut img, shapes, fonts);
    img.crop(sel)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xff_0000;

    fn ed(tool: Tool) -> Editor {
        let mut e = Editor::new(RED);
        e.tool = tool;
        e
    }

    fn drag(e: &mut Editor, a: Pt, b: Pt) {
        e.press(a);
        e.drag(b, false);
        e.release(b, false);
    }

    #[test]
    fn dragging_each_shape_tool_commits_one_shape() {
        for tool in [Tool::Line, Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Pixelate, Tool::Pen, Tool::Marker] {
            let mut e = ed(tool);
            drag(&mut e, (10, 10), (60, 40));
            assert_eq!(e.shapes.len(), 1, "{tool:?}");
        }
    }

    #[test]
    fn a_click_without_movement_leaves_nothing_except_pen_dots() {
        for tool in [Tool::Line, Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Pixelate] {
            let mut e = ed(tool);
            drag(&mut e, (10, 10), (10, 10));
            assert!(e.shapes.is_empty(), "{tool:?}");
        }
        let mut e = ed(Tool::Pen);
        drag(&mut e, (10, 10), (10, 10));
        assert_eq!(e.shapes.len(), 1);
    }

    #[test]
    fn live_shows_the_shape_being_dragged() {
        let mut e = ed(Tool::Rect);
        e.press((5, 5));
        e.drag((50, 30), false);
        assert_eq!(e.live().len(), 1);
        assert!(e.shapes.is_empty(), "not committed until release");
        e.release((50, 30), false);
        assert!(e.live().is_empty());
        assert_eq!(e.shapes.len(), 1);
    }

    #[test]
    fn undo_and_redo_walk_the_history_and_new_work_clears_redo() {
        let mut e = ed(Tool::Line);
        drag(&mut e, (0, 0), (20, 0));
        drag(&mut e, (0, 5), (20, 5));
        e.undo();
        assert_eq!(e.shapes.len(), 1);
        e.redo();
        assert_eq!(e.shapes.len(), 2);
        e.undo();
        e.undo();
        e.undo(); // nothing left: harmless
        assert!(e.shapes.is_empty() && !e.can_undo());
        e.redo();
        assert_eq!(e.shapes.len(), 1);
        drag(&mut e, (0, 9), (20, 9));
        e.redo(); // history was branched; the old redo is gone
        assert_eq!(e.shapes.len(), 2);
    }

    #[test]
    fn counters_number_up_and_undo_gives_the_number_back() {
        let mut e = ed(Tool::Counter);
        e.press((10, 10));
        e.press((30, 10));
        e.press((50, 10));
        let ns = |e: &Editor| -> Vec<u32> { e.shapes.iter().map(|s| if let Shape::Counter { n, .. } = s { *n } else { 0 }).collect() };
        assert_eq!(ns(&e), [1, 2, 3]);
        e.undo();
        e.press((70, 10));
        assert_eq!(ns(&e), [1, 2, 3], "the undone 3 is reused");
        e.undo();
        e.undo();
        e.redo();
        e.press((90, 10));
        assert_eq!(ns(&e), [1, 2, 3]);
    }

    #[test]
    fn text_is_typed_committed_and_blank_text_is_dropped() {
        let mut e = ed(Tool::Text);
        e.press((20, 20));
        for c in "hey".chars() {
            e.type_char(c);
        }
        e.backspace();
        e.type_char('\n'); // control chars are ignored
        e.type_char('!');
        assert_eq!(e.live().len(), 1, "caret preview");
        e.commit_text();
        assert!(matches!(&e.shapes[0], Shape::Text { text, .. } if text == "he!"));

        e.press((20, 60));
        e.type_char(' ');
        e.commit_text();
        assert_eq!(e.shapes.len(), 1, "whitespace-only text is discarded");

        // Clicking elsewhere commits the text being typed.
        e.press((20, 100));
        e.type_char('a');
        e.press((200, 100));
        assert_eq!(e.shapes.len(), 2);
    }

    #[test]
    fn undo_while_typing_cancels_the_text_first() {
        let mut e = ed(Tool::Text);
        drag(&mut e, (0, 0), (0, 0)); // press starts typing
        e.type_char('x');
        e.commit_text();
        e.press((5, 5));
        e.type_char('y');
        e.undo();
        assert!(e.typing.is_none());
        assert_eq!(e.shapes.len(), 1, "the committed text survives");
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees_and_boxes_to_squares() {
        let mut e = ed(Tool::Line);
        e.press((100, 100));
        e.drag((180, 108), true); // nearly horizontal
        e.release((180, 108), true);
        let Shape::Line { b, .. } = e.shapes[0] else { panic!() };
        assert_eq!(b.1, 100, "snapped flat");
        assert!(b.0 > 170);

        let mut e = ed(Tool::Rect);
        e.press((10, 10));
        e.drag((90, 40), true);
        e.release((90, 40), true);
        let Shape::Rect { a, b, .. } = e.shapes[0] else { panic!() };
        assert_eq!((b.0 - a.0).abs(), (b.1 - a.1).abs(), "square");
        // Dragging up-left keeps the square on the right side of the anchor.
        let mut e = ed(Tool::Ellipse);
        e.press((100, 100));
        e.release((60, 90), true);
        let Shape::Ellipse { a, b, .. } = e.shapes[0] else { panic!() };
        assert_eq!((b.0 - a.0, b.1 - a.1), (-40, -40));
    }

    #[test]
    fn export_draws_annotations_and_crops_to_the_selection() {
        let base = Image::new(100, 80, 0xff20_2020);
        let shapes = vec![
            Shape::Rect { a: (30, 20), b: (50, 40), color: 0xff0000, thick: 3, filled: true },
            Shape::Rect { a: (80, 60), b: (95, 75), color: 0x00ff00, thick: 3, filled: true }, // outside the crop
        ];
        let sel = Rect { x: 20, y: 10, w: 40, h: 40 };
        let out = export(&base, sel, &shapes, None);
        assert_eq!((out.w, out.h), (40, 40));
        assert_eq!(out.get(15, 15).map(|p| p & 0xffffff), Some(0xff0000), "selection-relative coordinates");
        assert!(!out.px.iter().any(|&p| p & 0xffffff == 0x00ff00), "shape outside the crop must not leak in");
        // The original capture is untouched.
        assert!(base.px.iter().all(|&p| p == 0xff20_2020));
    }
}
