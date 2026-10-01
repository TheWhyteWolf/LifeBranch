// SPDX-License-Identifier: GPL-3.0-or-later
// The whole interactive screenshot UI minus Wayland: selection (drag, move,
// eight resize handles), the toolbar, colour swatches, hotkeys, and painting.
// app.rs feeds it pointer/key events and shows the frame it paints; because
// nothing here touches a compositor, all of it is driven by tests and rendered
// to a file for visual checks.
//
// Coordinates are image pixels throughout (the capture's own resolution). `k`
// is image px per logical px (2.0 on a scale-2 output, 1.5 on a fractional
// one), used only to size the chrome so it looks the same on every display.

use crate::editor::{self, Editor, Tool};
use crate::image::{Image, Rect};
use crate::shapes::{self, Fonts, Pt};

pub const PALETTE_LEN: usize = 6;

#[derive(Clone, Copy)]
pub struct Palette {
    pub surface: u32,
    pub border: u32,
    pub text: u32,
    pub accent: u32,
    pub warn: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Palette { surface: 0x17_1a14, border: 0x39_412b, text: 0x7b_8c5a, accent: 0xa4_c94b, warn: 0xc7_d17a }
    }
}

impl Palette {
    pub fn parse(body: &str) -> Palette {
        let mut p = Palette::default();
        for line in body.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let Ok(c) = u32::from_str_radix(v.trim().trim_start_matches('#'), 16) else { continue };
            if v.trim().trim_start_matches('#').len() != 6 {
                continue;
            }
            match k.trim() {
                "surface" => p.surface = c,
                "border" => p.border = c,
                "text" => p.text = c,
                "accent" => p.accent = c,
                "warn" => p.warn = c,
                _ => {}
            }
        }
        p
    }

    pub fn load() -> Palette {
        let home = std::env::var("HOME").unwrap_or_default();
        let base = std::env::var("XDG_CONFIG_HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{home}/.config"));
        std::fs::read_to_string(format!("{base}/lifeshot/theme")).map(|b| Palette::parse(&b)).unwrap_or_default()
    }

    /// Annotation colours: a vivid red first (the default — it reads on anything),
    /// then the theme's accent and warn, then blue, white and black.
    pub fn swatches(&self) -> [u32; PALETTE_LEN] {
        [0xe5_484d, self.accent, self.warn, 0x4d_a3e5, 0xf0_f0f0, 0x10_1010]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    NewSel { anchor: Pt },
    Move { grab: Pt, orig: Rect },
    Resize { handle: Handle, orig: Rect },
    Tool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Tool(Tool),
    Undo,
    Redo,
    Copy,
    Save,
    Quit,
    Color(usize),
    ToggleFill,
}

/// What the event loop should do after an input event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Continue,
    Copy,
    Save,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Esc,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Char(char),
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

pub struct Session {
    base: Image,
    dim: Image,
    /// base + committed shapes
    annotated: Image,
    /// annotated + the shape being dragged / text being typed
    scratch: Image,
    rendered_n: usize,
    needs_full_render: bool,
    last_live: Option<Rect>,
    pub ed: Editor,
    pub tool: Option<Tool>,
    pub sel: Option<Rect>,
    drag: Option<Drag>,
    pub pointer: Pt,
    fonts: Option<Fonts>,
    pal: Palette,
    pub k: f32,
    swatches: [u32; PALETTE_LEN],
    color_ix: usize,
    hits: Vec<(Rect, Action)>,
    pub status: String,
}

fn tool_for_key(c: char) -> Option<Tool> {
    Some(match c {
        'p' => Tool::Pen,
        'l' => Tool::Line,
        'a' => Tool::Arrow,
        'r' => Tool::Rect,
        'o' | 'e' => Tool::Ellipse,
        'm' => Tool::Marker,
        't' => Tool::Text,
        'b' => Tool::Pixelate,
        'n' => Tool::Counter,
        _ => return None,
    })
}

const TOOLS: &[(&str, Tool)] = &[
    ("pen", Tool::Pen),
    ("line", Tool::Line),
    ("arrow", Tool::Arrow),
    ("box", Tool::Rect),
    ("oval", Tool::Ellipse),
    ("mark", Tool::Marker),
    ("text", Tool::Text),
    ("blur", Tool::Pixelate),
    ("num", Tool::Counter),
];

impl Session {
    pub fn new(base: Image, pal: Palette, fonts: Option<Fonts>, k: f32) -> Session {
        let swatches = pal.swatches();
        let mut ed = Editor::new(swatches[0]);
        ed.thick = (3.0 * k).round().max(2.0) as i32;
        ed.text_px = 20.0 * k;
        let dim = base.dimmed(0.45);
        Session {
            annotated: base.clone(),
            scratch: base.clone(),
            base,
            dim,
            rendered_n: 0,
            needs_full_render: false,
            last_live: None,
            ed,
            tool: None,
            sel: None,
            drag: None,
            pointer: (0, 0),
            fonts,
            pal,
            k,
            swatches,
            color_ix: 0,
            hits: Vec::new(),
            status: String::new(),
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.base.w, self.base.h)
    }

    fn handle_r(&self) -> i32 {
        (6.0 * self.k).round() as i32
    }

    // ---- selection geometry ----

    fn handle_at(&self, sel: Rect, p: Pt) -> Option<Handle> {
        let reach = (self.handle_r() * 2).max(8);
        let (l, t, r, b) = (sel.x, sel.y, sel.x + sel.w, sel.y + sel.h);
        let (cx, cy) = (l + sel.w / 2, t + sel.h / 2);
        let near = |a: i32, b2: i32| (a - b2).abs() <= reach;
        let on_x = |x: i32| near(p.0, x);
        let on_y = |y: i32| near(p.1, y);
        let within_x = p.0 >= l - reach && p.0 <= r + reach;
        let within_y = p.1 >= t - reach && p.1 <= b + reach;
        // Corners win over edges so a corner is always grabbable.
        Some(match () {
            _ if on_x(l) && on_y(t) => Handle::NW,
            _ if on_x(r) && on_y(t) => Handle::NE,
            _ if on_x(l) && on_y(b) => Handle::SW,
            _ if on_x(r) && on_y(b) => Handle::SE,
            _ if on_x(cx) && on_y(t) => Handle::N,
            _ if on_x(cx) && on_y(b) => Handle::S,
            _ if on_x(l) && on_y(cy) => Handle::W,
            _ if on_x(r) && on_y(cy) => Handle::E,
            // Anywhere along an edge (not just the midpoint handle) also resizes.
            _ if on_y(t) && within_x => Handle::N,
            _ if on_y(b) && within_x => Handle::S,
            _ if on_x(l) && within_y => Handle::W,
            _ if on_x(r) && within_y => Handle::E,
            _ => return None,
        })
    }

    fn resized(&self, orig: Rect, h: Handle, p: Pt) -> Rect {
        let (mut l, mut t, mut r, mut b) = (orig.x, orig.y, orig.x + orig.w, orig.y + orig.h);
        let (iw, ih) = (self.base.w as i32, self.base.h as i32);
        let (px, py) = (p.0.clamp(0, iw), p.1.clamp(0, ih));
        if matches!(h, Handle::W | Handle::NW | Handle::SW) {
            l = px;
        }
        if matches!(h, Handle::E | Handle::NE | Handle::SE) {
            r = px;
        }
        if matches!(h, Handle::N | Handle::NE | Handle::NW) {
            t = py;
        }
        if matches!(h, Handle::S | Handle::SE | Handle::SW) {
            b = py;
        }
        // Dragging an edge past its opposite flips the rect instead of inverting it.
        Rect { x: l.min(r), y: t.min(b), w: (r - l).abs(), h: (b - t).abs() }
    }

    // ---- pointer ----

    pub fn pointer_down(&mut self, p: Pt, mods: Mods) -> Outcome {
        self.pointer = p;
        if self.sel.is_some() {
            if let Some(&(_, a)) = self.hits.iter().find(|(r, _)| r.contains(p)) {
                return self.act(a);
            }
        }
        let Some(sel) = self.sel else {
            self.drag = Some(Drag::NewSel { anchor: p });
            return Outcome::Continue;
        };
        if let (Some(_), true) = (self.tool, sel.contains(p)) {
            self.ed.press(p);
            self.drag = Some(Drag::Tool);
            self.sync();
            return Outcome::Continue;
        }
        if let Some(h) = self.handle_at(sel, p) {
            self.drag = Some(Drag::Resize { handle: h, orig: sel });
        } else if sel.contains(p) {
            self.drag = Some(Drag::Move { grab: p, orig: sel });
        } else {
            self.drag = Some(Drag::NewSel { anchor: p });
        }
        let _ = mods;
        Outcome::Continue
    }

    pub fn pointer_move(&mut self, p: Pt, mods: Mods) {
        self.pointer = p;
        let (iw, ih) = (self.base.w as i32, self.base.h as i32);
        match self.drag {
            Some(Drag::NewSel { anchor }) => {
                let r = Rect::from_points(anchor, (p.0.clamp(0, iw), p.1.clamp(0, ih)));
                self.sel = Some(Rect { x: r.x.max(0), y: r.y.max(0), ..r });
            }
            Some(Drag::Move { grab, orig }) => {
                let x = (orig.x + p.0 - grab.0).clamp(0, (iw - orig.w).max(0));
                let y = (orig.y + p.1 - grab.1).clamp(0, (ih - orig.h).max(0));
                self.sel = Some(Rect { x, y, ..orig });
            }
            Some(Drag::Resize { handle, orig }) => self.sel = Some(self.resized(orig, handle, p)),
            Some(Drag::Tool) => self.ed.drag(p, mods.shift),
            None => {}
        }
    }

    pub fn pointer_up(&mut self, p: Pt, mods: Mods) {
        self.pointer = p;
        match self.drag.take() {
            Some(Drag::NewSel { .. }) => {
                // A click without a drag isn't a selection.
                if self.sel.is_some_and(|s| s.w < 4 || s.h < 4) {
                    self.sel = None;
                }
            }
            Some(Drag::Tool) => {
                self.ed.release(p, mods.shift);
                self.sync();
            }
            _ => {}
        }
    }

    /// Wheel: thicker/thinner strokes.
    pub fn scroll(&mut self, dir: i32) {
        self.set_thick(self.ed.thick + dir);
    }

    fn set_thick(&mut self, t: i32) {
        self.ed.thick = t.clamp(1, 40);
        self.status = format!("width {}", self.ed.thick);
    }

    // ---- actions / keys ----

    fn act(&mut self, a: Action) -> Outcome {
        match a {
            Action::Tool(t) => {
                self.ed.commit_text();
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                if let Some(t) = self.tool {
                    self.ed.tool = t;
                }
                self.sync();
            }
            Action::Undo => {
                self.ed.undo();
                self.needs_full_render = true;
                self.sync();
            }
            Action::Redo => {
                self.ed.redo();
                self.needs_full_render = true;
                self.sync();
            }
            Action::Color(i) => {
                self.color_ix = i.min(PALETTE_LEN - 1);
                self.ed.color = self.swatches[self.color_ix];
            }
            Action::ToggleFill => self.ed.filled = !self.ed.filled,
            Action::Copy => {
                self.ed.commit_text();
                self.sync();
                return Outcome::Copy;
            }
            Action::Save => {
                self.ed.commit_text();
                self.sync();
                return Outcome::Save;
            }
            Action::Quit => return Outcome::Quit,
        }
        Outcome::Continue
    }

    pub fn key(&mut self, key: Key, m: Mods) -> Outcome {
        // Typing text owns the keyboard: only Enter/Esc/Backspace are commands.
        if self.ed.typing.is_some() {
            match key {
                Key::Enter => self.ed.commit_text(),
                Key::Esc => self.ed.cancel_text(),
                Key::Backspace => self.ed.backspace(),
                Key::Char(c) if m.ctrl && c == 'z' => return self.act(Action::Undo),
                Key::Char(c) if !m.ctrl => self.ed.type_char(c),
                _ => {}
            }
            self.sync();
            return Outcome::Continue;
        }
        let ctrl_shift = m.ctrl && m.shift;
        match key {
            Key::Esc => {
                // First Esc backs out of a tool or selection; the last one quits.
                if self.tool.is_some() {
                    self.tool = None;
                } else if self.sel.is_some() {
                    self.sel = None;
                } else {
                    return Outcome::Quit;
                }
            }
            Key::Enter => return if self.sel.is_some() { self.act(Action::Copy) } else { Outcome::Continue },
            Key::Backspace | Key::Delete => return self.act(Action::Undo),
            Key::Left | Key::Right | Key::Up | Key::Down => self.nudge(key, if m.shift { 10 } else { 1 }),
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                if m.ctrl {
                    return match c {
                        'a' => {
                            self.sel = Some(Rect { x: 0, y: 0, w: self.base.w as i32, h: self.base.h as i32 });
                            Outcome::Continue
                        }
                        'c' => self.act(Action::Copy),
                        's' => self.act(Action::Save),
                        'z' if ctrl_shift => self.act(Action::Redo),
                        'z' => self.act(Action::Undo),
                        'y' => self.act(Action::Redo),
                        'q' => Outcome::Quit,
                        _ => Outcome::Continue,
                    };
                }
                if self.sel.is_none() {
                    return Outcome::Continue;
                }
                if let Some(t) = tool_for_key(c) {
                    return self.act(Action::Tool(t));
                }
                match c {
                    '1'..='6' => return self.act(Action::Color(c as usize - '1' as usize)),
                    '[' | '-' => self.set_thick(self.ed.thick - 1),
                    ']' | '=' | '+' => self.set_thick(self.ed.thick + 1),
                    'f' => return self.act(Action::ToggleFill),
                    _ => {}
                }
            }
        }
        Outcome::Continue
    }

    fn nudge(&mut self, key: Key, step: i32) {
        let Some(s) = self.sel else { return };
        let (dx, dy) = match key {
            Key::Left => (-step, 0),
            Key::Right => (step, 0),
            Key::Up => (0, -step),
            _ => (0, step),
        };
        let x = (s.x + dx).clamp(0, (self.base.w as i32 - s.w).max(0));
        let y = (s.y + dy).clamp(0, (self.base.h as i32 - s.h).max(0));
        self.sel = Some(Rect { x, y, ..s });
    }

    // ---- rendering the annotations ----

    /// Bring `annotated` (and `scratch`) in line with the editor's shape list.
    fn sync(&mut self) {
        let shapes = &self.ed.shapes;
        if self.needs_full_render || shapes.len() < self.rendered_n {
            self.annotated = self.base.clone();
            shapes::render_all(&mut self.annotated, shapes, self.fonts.as_ref());
            self.scratch = self.annotated.clone();
            self.rendered_n = shapes.len();
            self.needs_full_render = false;
            self.last_live = None;
        } else if shapes.len() > self.rendered_n {
            // New shapes only ever append, so draw just those, on both layers.
            let fonts = self.fonts.as_ref();
            for s in &shapes[self.rendered_n..] {
                shapes::render(&mut self.annotated, s, fonts);
                shapes::render(&mut self.scratch, s, fonts);
            }
            self.rendered_n = shapes.len();
        }
    }

    /// Redraw the in-progress shape onto `scratch`, undoing the last one first.
    fn refresh_live(&mut self) {
        if let Some(r) = self.last_live.take() {
            let r = r.clip(self.base.w, self.base.h);
            for y in r.y..r.y + r.h {
                let a = y as usize * self.base.w + r.x as usize;
                self.scratch.px[a..a + r.w as usize].copy_from_slice(&self.annotated.px[a..a + r.w as usize]);
            }
        }
        let live = self.ed.live();
        let mut bounds: Option<Rect> = None;
        for s in &live {
            let b = shapes::bbox(s, self.fonts.as_ref());
            bounds = Some(match bounds {
                None => b,
                Some(o) => {
                    let (x0, y0) = (o.x.min(b.x), o.y.min(b.y));
                    Rect { x: x0, y: y0, w: (o.x + o.w).max(b.x + b.w) - x0, h: (o.y + o.h).max(b.y + b.h) - y0 }
                }
            });
            shapes::render(&mut self.scratch, s, self.fonts.as_ref());
        }
        self.last_live = bounds;
    }

    /// The finished screenshot, or None without a selection.
    pub fn export(&mut self) -> Option<Image> {
        let sel = self.sel?;
        self.ed.commit_text();
        Some(editor::export(&self.base, sel, &self.ed.shapes, self.fonts.as_ref()))
    }

    // ---- painting ----

    fn fill(&self, f: &mut Image, r: Rect, rgb: u32) {
        let r = r.clip(f.w, f.h);
        for y in r.y..r.y + r.h {
            let a = y as usize * f.w + r.x as usize;
            f.px[a..a + r.w as usize].fill(0xff00_0000 | rgb);
        }
    }

    fn frame_rect(&self, f: &mut Image, r: Rect, t: i32, rgb: u32) {
        self.fill(f, Rect { h: t, ..r }, rgb);
        self.fill(f, Rect { y: r.y + r.h - t, h: t, ..r }, rgb);
        self.fill(f, Rect { w: t, ..r }, rgb);
        self.fill(f, Rect { x: r.x + r.w - t, w: t, ..r }, rgb);
    }

    fn text(&self, f: &mut Image, x: i32, y: i32, s: &str, px: f32, rgb: u32) -> i32 {
        match &self.fonts {
            Some(font) => font.draw(f, x, y, s, px, rgb),
            None => x + (px * 0.6 * s.chars().count() as f32) as i32,
        }
    }

    fn ui_px(&self) -> f32 {
        (14.0 * self.k).round().max(10.0)
    }

    pub fn paint(&mut self, frame: &mut Image) {
        debug_assert_eq!((frame.w, frame.h), (self.base.w, self.base.h));
        frame.px.copy_from_slice(&self.dim.px);
        self.hits.clear();
        let ui = self.ui_px();
        let (cw, ch) = match &self.fonts {
            Some(f) => (f.advance(ui), f.line_height(ui)),
            None => (ui * 0.6, ui * 1.3),
        };
        let (cw, ch) = (cw.round() as i32, ch.round() as i32);

        let Some(sel) = self.sel else {
            let msg = "drag to select an area  ·  Ctrl+A whole screen  ·  Esc cancel";
            let w = (msg.chars().count() as i32) * cw;
            let (x, y) = ((frame.w as i32 - w) / 2, frame.h as i32 / 2 - ch / 2);
            let pad = ch / 2;
            self.fill(frame, Rect { x: x - pad, y: y - pad / 2, w: w + 2 * pad, h: ch + pad }, self.pal.surface);
            self.text(frame, x, y, msg, ui, self.pal.text);
            return;
        };

        self.refresh_live();
        // The selection shows the real (annotated) pixels, undimmed.
        let c = sel.clip(frame.w, frame.h);
        for y in c.y..c.y + c.h {
            let a = y as usize * frame.w + c.x as usize;
            frame.px[a..a + c.w as usize].copy_from_slice(&self.scratch.px[a..a + c.w as usize]);
        }
        let bt = (self.k.round() as i32).max(1);
        self.frame_rect(frame, Rect { x: sel.x - bt, y: sel.y - bt, w: sel.w + 2 * bt, h: sel.h + 2 * bt }, bt, self.pal.accent);

        // Handles.
        let hr = self.handle_r();
        let (l, t, r, b) = (sel.x, sel.y, sel.x + sel.w, sel.y + sel.h);
        let (cx, cy) = (l + sel.w / 2, t + sel.h / 2);
        for (x, y) in [(l, t), (r, t), (l, b), (r, b), (cx, t), (cx, b), (l, cy), (r, cy)] {
            self.fill(frame, Rect { x: x - hr / 2 - bt, y: y - hr / 2 - bt, w: hr + 2 * bt, h: hr + 2 * bt }, self.pal.surface);
            self.fill(frame, Rect { x: x - hr / 2, y: y - hr / 2, w: hr, h: hr }, self.pal.accent);
        }

        // Size label, above the top-left corner (inside if there's no room).
        let label = format!("{} x {}", sel.w, sel.h);
        let lw = label.chars().count() as i32 * cw + ch;
        let ly = if sel.y > ch * 2 { sel.y - ch - ch / 2 - bt } else { sel.y + bt + 4 };
        let lx = sel.x.clamp(0, (frame.w as i32 - lw).max(0));
        self.fill(frame, Rect { x: lx, y: ly, w: lw, h: ch + 4 }, self.pal.surface);
        self.text(frame, lx + ch / 2, ly + 2, &label, ui, self.pal.accent);

        self.paint_toolbar(frame, sel, ui, cw, ch);
    }

    fn paint_toolbar(&mut self, frame: &mut Image, sel: Rect, ui: f32, cw: i32, ch: i32) {
        enum El {
            Button(String, Action, bool),
            Swatch(usize),
            Label(String),
        }
        let pad = cw;
        let mut els: Vec<El> = TOOLS
            .iter()
            .map(|(name, t)| El::Button(name.to_string(), Action::Tool(*t), self.tool == Some(*t)))
            .collect();
        els.push(El::Button(if self.ed.filled { "solid" } else { "hollow" }.into(), Action::ToggleFill, self.ed.filled));
        for (name, a) in [("undo", Action::Undo), ("redo", Action::Redo), ("copy", Action::Copy), ("save", Action::Save), ("x", Action::Quit)] {
            els.push(El::Button(name.into(), a, false));
        }
        els.extend((0..PALETTE_LEN).map(El::Swatch));
        els.push(El::Label(format!("w{}", self.ed.thick)));

        let sw = ch;
        let gap = ((4.0 * self.k) as i32).max(2);
        let width_of = |e: &El| match e {
            El::Button(s, _, _) => s.chars().count() as i32 * cw + 2 * pad,
            El::Swatch(_) => sw,
            El::Label(s) => (s.chars().count() as i32 + 1) * cw,
        };
        // Wrap onto more rows rather than run off a narrow screen, where the
        // clipped buttons could not be clicked.
        let avail = frame.w as i32 - 2 * gap;
        let mut rows: Vec<Vec<&El>> = vec![vec![]];
        let mut used = 0;
        for e in &els {
            let w = width_of(e) + gap;
            if used + w > avail && !rows.last().unwrap().is_empty() {
                rows.push(vec![]);
                used = 0;
            }
            rows.last_mut().unwrap().push(e);
            used += w;
        }
        let row_w = |r: &Vec<&El>| r.iter().map(|e| width_of(e) + gap).sum::<i32>() + gap;
        let total = rows.iter().map(row_w).max().unwrap_or(0).min(frame.w as i32);
        let row_h = ch + 10;
        let bar_h = row_h * rows.len() as i32;
        let margin = (8.0 * self.k) as i32;
        // Below the selection if it fits, else above, else inside at the bottom.
        let y = if sel.y + sel.h + margin + bar_h <= frame.h as i32 {
            sel.y + sel.h + margin
        } else if sel.y - margin - bar_h >= 0 {
            sel.y - margin - bar_h
        } else {
            (sel.y + sel.h - margin - bar_h).max(0)
        };
        let x = (sel.x + sel.w - total).clamp(0, (frame.w as i32 - total).max(0));
        self.fill(frame, Rect { x, y, w: total, h: bar_h }, self.pal.surface);
        self.frame_rect(frame, Rect { x, y, w: total, h: bar_h }, 1, self.pal.border);

        for (ri, row) in rows.iter().enumerate() {
            let ry = y + ri as i32 * row_h;
            let mut cx = x + gap;
            for e in row {
                let w = width_of(e);
                match e {
                    El::Button(label, action, active) => {
                        let r = Rect { x: cx, y: ry + 3, w, h: row_h - 6 };
                        if *active {
                            self.fill(frame, r, self.pal.accent);
                        }
                        let fg = if *active { self.pal.surface } else { self.pal.text };
                        self.text(frame, cx + pad, ry + 5, label, ui, fg);
                        self.hits.push((r, *action));
                    }
                    El::Swatch(i) => {
                        let r = Rect { x: cx, y: ry + (row_h - sw) / 2, w: sw, h: sw };
                        self.fill(frame, r, self.swatches[*i]);
                        if *i == self.color_ix {
                            self.frame_rect(frame, Rect { x: r.x - 2, y: r.y - 2, w: r.w + 4, h: r.h + 4 }, 2, 0xf0_f0f0);
                        }
                        self.hits.push((r, Action::Color(*i)));
                    }
                    El::Label(text) => {
                        self.text(frame, cx, ry + 5, text, ui, self.pal.text);
                    }
                }
                cx += w + gap;
            }
        }
    }

    // exposed for tests
    #[cfg(test)]
    fn hit_for(&self, a: Action) -> Option<Rect> {
        self.hits.iter().find(|(_, x)| *x == a).map(|(r, _)| *r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape_count(s: &Session) -> usize {
        s.ed.shapes.len()
    }

    const NO: Mods = Mods { ctrl: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, shift: false };

    fn session() -> Session {
        // 400x300 backdrop, k=1, no font (chrome falls back to estimated metrics).
        Session::new(Image::new(400, 300, 0xff30_3030), Palette::default(), None, 1.0)
    }

    fn select(s: &mut Session, a: Pt, b: Pt) {
        s.pointer_down(a, NO);
        s.pointer_move(b, NO);
        s.pointer_up(b, NO);
    }

    fn painted(s: &mut Session) -> Image {
        let mut f = Image::new(400, 300, 0);
        s.paint(&mut f);
        f
    }

    #[test]
    fn dragging_selects_a_region_and_a_click_does_not() {
        let mut s = session();
        select(&mut s, (50, 40), (200, 160));
        assert_eq!(s.sel, Some(Rect { x: 50, y: 40, w: 150, h: 120 }));
        let mut s = session();
        select(&mut s, (50, 40), (51, 41));
        assert_eq!(s.sel, None);
    }

    #[test]
    fn selection_is_clamped_to_the_screen_and_accepts_any_drag_direction() {
        let mut s = session();
        select(&mut s, (350, 250), (450, 400));
        assert_eq!(s.sel, Some(Rect { x: 350, y: 250, w: 50, h: 50 }));
        let mut s = session();
        select(&mut s, (200, 150), (100, 50));
        assert_eq!(s.sel, Some(Rect { x: 100, y: 50, w: 100, h: 100 }));
    }

    #[test]
    fn dragging_inside_moves_and_stays_on_screen() {
        let mut s = session();
        select(&mut s, (100, 100), (200, 180));
        s.pointer_down((150, 140), NO);
        s.pointer_move((170, 150), NO);
        s.pointer_up((170, 150), NO);
        assert_eq!(s.sel, Some(Rect { x: 120, y: 110, w: 100, h: 80 }));
        s.pointer_down((170, 150), NO);
        s.pointer_move((9999, 9999), NO);
        s.pointer_up((9999, 9999), NO);
        let r = s.sel.unwrap();
        assert_eq!((r.w, r.h), (100, 80), "size preserved");
        assert!(r.x + r.w <= 400 && r.y + r.h <= 300);
    }

    #[test]
    fn handles_resize_and_dragging_past_the_opposite_edge_flips() {
        let mut s = session();
        select(&mut s, (100, 100), (200, 200));
        // Bottom-right corner out to (260, 230).
        s.pointer_down((200, 200), NO);
        s.pointer_move((260, 230), NO);
        s.pointer_up((260, 230), NO);
        assert_eq!(s.sel, Some(Rect { x: 100, y: 100, w: 160, h: 130 }));
        // Right edge dragged left past the left edge flips instead of inverting.
        s.pointer_down((260, 165), NO);
        s.pointer_move((60, 165), NO);
        s.pointer_up((60, 165), NO);
        assert_eq!(s.sel, Some(Rect { x: 60, y: 100, w: 40, h: 130 }));
    }

    #[test]
    fn dragging_outside_starts_a_new_selection() {
        let mut s = session();
        select(&mut s, (100, 100), (150, 150));
        select(&mut s, (300, 40), (380, 90));
        assert_eq!(s.sel, Some(Rect { x: 300, y: 40, w: 80, h: 50 }));
    }

    #[test]
    fn ctrl_a_selects_everything_and_arrows_nudge_within_bounds() {
        let mut s = session();
        s.key(Key::Char('a'), CTRL);
        assert_eq!(s.sel, Some(Rect { x: 0, y: 0, w: 400, h: 300 }));
        select(&mut s, (100, 100), (150, 150)); // grabs inside? it's a move of the full sel
        let mut s = session();
        select(&mut s, (10, 10), (60, 60));
        s.key(Key::Left, NO);
        assert_eq!(s.sel.unwrap().x, 9);
        s.key(Key::Left, Mods { ctrl: false, shift: true });
        assert_eq!(s.sel.unwrap().x, 0, "clamped at the edge");
    }

    #[test]
    fn hotkeys_pick_tools_and_the_same_key_toggles_back_to_select_mode() {
        let mut s = session();
        s.key(Key::Char('a'), NO);
        assert_eq!(s.tool, None, "no tool hotkeys before there is a selection");
        select(&mut s, (50, 50), (300, 250));
        s.key(Key::Char('a'), NO);
        assert_eq!(s.tool, Some(Tool::Arrow));
        s.key(Key::Char('r'), NO);
        assert_eq!(s.tool, Some(Tool::Rect));
        s.key(Key::Char('r'), NO);
        assert_eq!(s.tool, None);
        s.key(Key::Char('3'), NO);
        assert_eq!(s.ed.color, Palette::default().warn);
        s.key(Key::Char(']'), NO);
        s.key(Key::Char('f'), NO);
        assert!(s.ed.filled);
    }

    #[test]
    fn drawing_with_a_tool_adds_shapes_and_undo_redo_restore_them() {
        let mut s = session();
        select(&mut s, (50, 50), (300, 250));
        s.key(Key::Char('r'), NO);
        s.pointer_down((80, 80), NO);
        s.pointer_move((150, 150), NO);
        s.pointer_up((150, 150), NO);
        assert_eq!(shape_count(&s), 1);
        let drawn = s.export().unwrap();
        s.key(Key::Char('z'), CTRL);
        assert_eq!(shape_count(&s), 0);
        let clean = s.export().unwrap();
        assert_ne!(drawn, clean, "the rectangle shows in the export, and undo removes it");
        s.key(Key::Char('z'), Mods { ctrl: true, shift: true });
        assert_eq!(shape_count(&s), 1);
        assert_eq!(s.export().unwrap(), drawn, "redo restores it exactly");
    }

    #[test]
    fn drawing_does_not_start_outside_the_selection() {
        let mut s = session();
        select(&mut s, (100, 100), (200, 200));
        s.key(Key::Char('l'), NO);
        select(&mut s, (300, 20), (380, 60)); // outside: starts a new selection instead
        assert_eq!(shape_count(&s), 0);
        assert_eq!(s.sel, Some(Rect { x: 300, y: 20, w: 80, h: 40 }));
    }

    #[test]
    fn export_is_the_selection_with_annotations() {
        let mut s = session();
        select(&mut s, (100, 100), (200, 180));
        s.key(Key::Char('r'), NO);
        s.key(Key::Char('f'), NO);
        s.pointer_down((120, 120), NO);
        s.pointer_move((140, 140), NO);
        s.pointer_up((140, 140), NO);
        let out = s.export().unwrap();
        assert_eq!((out.w, out.h), (100, 80));
        assert_eq!(out.get(30, 30).map(|p| p & 0xffffff), Some(0xe5_484d), "filled red box, selection-relative");
        assert_eq!(out.get(5, 5), Some(0xff30_3030), "untouched backdrop pixel is the real capture, not dimmed");
    }

    #[test]
    fn toolbar_buttons_and_swatches_are_clickable() {
        let mut s = session();
        select(&mut s, (50, 50), (300, 200));
        painted(&mut s); // lays out the toolbar
        let arrow = s.hit_for(Action::Tool(Tool::Arrow)).expect("arrow button");
        s.pointer_down((arrow.x + 2, arrow.y + 2), NO);
        s.pointer_up((arrow.x + 2, arrow.y + 2), NO);
        assert_eq!(s.tool, Some(Tool::Arrow));
        painted(&mut s);
        let blue = s.hit_for(Action::Color(3)).unwrap();
        s.pointer_down((blue.x + 1, blue.y + 1), NO);
        assert_eq!(s.ed.color, Palette::default().swatches()[3]);
        painted(&mut s);
        let copy = s.hit_for(Action::Copy).unwrap();
        assert_eq!(s.pointer_down((copy.x + 1, copy.y + 1), NO), Outcome::Copy);
        painted(&mut s);
        let x = s.hit_for(Action::Quit).unwrap();
        assert_eq!(s.pointer_down((x.x + 1, x.y + 1), NO), Outcome::Quit);
    }

    #[test]
    fn enter_copies_only_with_a_selection_and_escape_backs_out_in_steps() {
        let mut s = session();
        assert_eq!(s.key(Key::Enter, NO), Outcome::Continue);
        select(&mut s, (50, 50), (300, 200));
        s.key(Key::Char('t'), NO);
        assert_eq!(s.key(Key::Esc, NO), Outcome::Continue); // drops the tool
        assert!(s.tool.is_none() && s.sel.is_some());
        assert_eq!(s.key(Key::Esc, NO), Outcome::Continue); // drops the selection
        assert!(s.sel.is_none());
        assert_eq!(s.key(Key::Esc, NO), Outcome::Quit);
        select(&mut s, (50, 50), (300, 200));
        assert_eq!(s.key(Key::Enter, NO), Outcome::Copy);
        assert_eq!(s.key(Key::Char('s'), CTRL), Outcome::Save);
    }

    #[test]
    fn typing_text_does_not_fire_hotkeys() {
        let mut s = session();
        select(&mut s, (50, 50), (300, 200));
        s.key(Key::Char('t'), NO);
        s.pointer_down((80, 80), NO);
        s.pointer_up((80, 80), NO);
        for c in "quit all".chars() {
            assert_eq!(s.key(Key::Char(c), NO), Outcome::Continue);
        }
        assert_eq!(s.tool, Some(Tool::Text), "letters were text, not tool hotkeys");
        s.key(Key::Enter, NO);
        assert_eq!(shape_count(&s), 1);
    }

    #[test]
    fn toolbar_falls_back_above_or_inside_when_there_is_no_room_below() {
        let mut s = session();
        select(&mut s, (50, 80), (300, 296)); // flush with the bottom edge
        painted(&mut s);
        let bar = s.hit_for(Action::Quit).unwrap();
        assert!(bar.y < 80 + 216, "toolbar must stay on screen");
        assert!(bar.y + bar.h <= 300);
        let mut s = session();
        select(&mut s, (0, 0), (400, 300)); // whole screen: bar goes inside
        painted(&mut s);
        let bar = s.hit_for(Action::Quit).unwrap();
        assert!(bar.y + bar.h <= 300 && bar.x + bar.w <= 400);
    }

    #[test]
    fn toolbar_wraps_on_a_narrow_screen_so_every_button_stays_clickable() {
        let mut s = session(); // 400px wide: far narrower than one row of buttons
        select(&mut s, (50, 50), (300, 150));
        painted(&mut s);
        let all = [
            Action::Tool(Tool::Pen), Action::Tool(Tool::Counter), Action::ToggleFill, Action::Undo,
            Action::Redo, Action::Copy, Action::Save, Action::Quit, Action::Color(0), Action::Color(5),
        ];
        let mut ys = std::collections::BTreeSet::new();
        for a in all {
            let r = s.hit_for(a).unwrap_or_else(|| panic!("{a:?} missing"));
            assert!(r.x >= 0 && r.x + r.w <= 400, "{a:?} off screen: {r:?}");
            assert!(r.y >= 0 && r.y + r.h <= 300);
            ys.insert(r.y);
        }
        assert!(ys.len() >= 2, "should have wrapped onto several rows");
    }

    #[test]
    fn paint_dims_outside_and_shows_the_selection_undimmed() {
        let mut s = session();
        select(&mut s, (100, 100), (200, 200));
        let f = painted(&mut s);
        assert_eq!(f.get(150, 150), Some(0xff30_3030), "inside: real pixels");
        let outside = f.get(20, 20).unwrap();
        assert!(outside & 0xff < 0x30, "outside is dimmed: {outside:#x}");
    }

    #[test]
    fn live_shape_preview_appears_and_leaves_no_trace_after_cancel() {
        let mut s = session();
        select(&mut s, (50, 50), (300, 250));
        s.key(Key::Char('l'), NO);
        s.pointer_down((80, 100), NO);
        s.pointer_move((250, 100), NO);
        let during = painted(&mut s);
        assert_ne!(during.get(160, 100), Some(0xff30_3030), "line previewed while dragging");
        // Drag back to a point: the old preview must be fully erased on repaint.
        s.pointer_move((80, 100), NO);
        let back = painted(&mut s);
        assert_eq!(back.get(160, 100), Some(0xff30_3030), "stale preview pixels were cleaned up");
    }

    #[test]
    fn palette_parses_theme_file() {
        let p = Palette::parse("accent=#ff0000\nbogus=#00ff00\nwarn=nothex\n");
        assert_eq!(p.accent, 0xff0000);
        assert_eq!(p.warn, Palette::default().warn);
    }

    #[test]
    fn scroll_changes_width_within_limits() {
        let mut s = session();
        let t0 = s.ed.thick;
        s.scroll(2);
        assert_eq!(s.ed.thick, t0 + 2);
        s.scroll(-999);
        assert_eq!(s.ed.thick, 1);
    }
}
