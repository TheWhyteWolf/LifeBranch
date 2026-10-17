// SPDX-License-Identifier: GPL-3.0-or-later
// The GUI front-end: a software-rendered xdg-shell toplevel that mirrors the
// TUI's layout (category sidebar + field list) in the rice's own olive glyphs.
// It drives the exact same model::Model as the TUI — only rendering and input
// mapping differ.
//
// Rendering reuses a self-contained copy of lifenote's glyph Atlas (gui::
// render) rather than a shared crate: extracting a life-common crate would mean
// refactoring four already-working crates into a workspace for no user-facing
// gain, so that cleanup is deferred (see README milestones).

mod preview;
mod render;

use crate::model::{field_labels, is_system, kind, Focus, Kind, Model, CATS};
use crate::paths::Paths;
use crate::theme::{rgb, Theme};
use preview::Previews;
use render::Atlas;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop, LoopHandle,
    },
    reexports::calloop_wayland_source::WaylandSource,
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::xdg::{
        window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        XdgShell,
    },
    shell::WaylandSurface,
    shm::{slot::SlotPool, Shm, ShmHandler},
};

const FONT: &str = "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf";
const FONT_PX: f32 = 16.0;
const SIDEBAR_CELLS: usize = 16;
const DROPDOWN_ROWS: usize = 6;
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

pub fn run(paths: Paths, theme: Theme, panel: Option<&str>) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lifeconf: cannot connect to Wayland: {e} (no GUI here — try the terminal for the TUI)");
            return 1;
        }
    };
    let (globals, event_queue) = match registry_queue_init(&conn) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("lifeconf: registry init failed: {e}");
            return 1;
        }
    };
    let qh: QueueHandle<Gui> = event_queue.handle();
    let mut event_loop: EventLoop<Gui> = EventLoop::try_new().expect("event loop");

    let atlas = match Atlas::new(FONT, FONT_PX) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifeconf: {e}");
            return 1;
        }
    };
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg = match XdgShell::bind(&globals, &qh) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("lifeconf: xdg-shell not available: {e}");
            return 1;
        }
    };
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");

    let wl_surface = compositor.create_surface(&qh);
    let window = xdg.create_window(wl_surface, WindowDecorations::RequestServer, &qh);
    window.set_title("lifeconf — rice settings");
    window.set_app_id("lifeconf");
    window.set_min_size(Some((520, 360)));
    window.commit();

    let cell_w = atlas.cell_w();
    let cell_h = atlas.cell_h();
    let mut gui = Gui {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        shm,
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        timer_armed: false,
        window,
        pool: None,
        width: 760,
        height: 640,
        configured: false,
        atlas,
        previews: Previews::default(),
        cell_w,
        cell_h,
        m: {
            let mut m = Model::new(paths, theme);
            if let Some(p) = panel {
                if !m.open_panel(p) {
                    eprintln!("lifeconf: no panel named {p:?}");
                }
            }
            m
        },
        keyboard: None,
        pointer: None,
        ctrl: false,
        pointer_pos: (0.0, 0.0),
        dropdown: None,
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).expect("wayland source");

    while !gui.m.quit {
        if event_loop.dispatch(None, &mut gui).is_err() {
            eprintln!("lifeconf: event loop error");
            gui.m.revert_pending("lifeconf failed");
            return 1;
        }
    }
    // Closing the window is not an answer: silence means no.
    gui.m.revert_pending("closed without answering");
    0
}

struct Gui {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    qh: QueueHandle<Self>,
    loop_handle: LoopHandle<'static, Gui>,
    /// A 1s countdown timer is already scheduled (see ensure_timer).
    timer_armed: bool,
    window: Window,
    pool: Option<SlotPool>,
    width: u32,
    height: u32,
    configured: bool,
    atlas: Atlas,
    previews: Previews,
    cell_w: usize,
    cell_h: usize,
    m: Model,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    ctrl: bool,
    pointer_pos: (f64, f64),
    dropdown: Option<Dropdown>,
}

struct Dropdown {
    options: Vec<String>,
    query: String,
    selected: usize, // index in the filtered list
    scroll: usize,
}

impl Dropdown {
    fn matches(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.options.iter().enumerate()
            .filter(|(_, option)| option.to_lowercase().contains(&q))
            .map(|(i, _)| i).collect()
    }

    fn move_by(&mut self, delta: isize) {
        let len = self.matches().len();
        if len > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
            if self.selected < self.scroll { self.scroll = self.selected; }
            if self.selected >= self.scroll + DROPDOWN_ROWS { self.scroll = self.selected + 1 - DROPDOWN_ROWS; }
        }
    }
}

// --- geometry (free fns so the offline renderer shares the exact layout) ---
fn margin(cell_h: usize) -> usize {
    cell_h
}
fn sidebar_w(cell_w: usize, cell_h: usize) -> usize {
    margin(cell_h) + SIDEBAR_CELLS * cell_w
}
fn row_y(cell_h: usize, i: usize) -> usize {
    margin(cell_h) + cell_h + i * (cell_h + 4)
}
/// One row of the sidebar: a section heading or a selectable category.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Header(&'static str),
    Cat(usize),
}

/// Headings only when browsing; a search shows a flat list of hits.
fn sidebar_rows(m: &Model) -> Vec<Side> {
    let vis = m.visible_cats();
    if m.search.as_deref().is_some_and(|q| !q.trim().is_empty()) {
        return vis.into_iter().map(Side::Cat).collect();
    }
    let mut out = Vec::new();
    let mut last = "";
    for c in vis {
        let group = if is_system(c) { "System" } else { "Appearance" };
        if group != last {
            out.push(Side::Header(group));
            last = group;
        }
        out.push(Side::Cat(c));
    }
    out
}

/// The search box under the category list: (y, height).
fn search_box(h: usize, cell_h: usize) -> (usize, usize) {
    (h.saturating_sub(2 * cell_h + 14), cell_h + 4)
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8)) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// The keep/revert banner shown while a Display change awaits an answer:
/// (banner rect, keep button, revert button), each (x, y, w, h).
type Rect = (usize, usize, usize, usize);
fn prompt_rects(w: usize, h: usize, cell_w: usize, cell_h: usize) -> (Rect, Rect, Rect) {
    let bh = 2 * cell_h + 16;
    let by = h.saturating_sub(cell_h + 4 + bh + 6);
    let x0 = sidebar_w(cell_w, cell_h) + 1;
    let (btn_w, btn_h) = (13 * cell_w, cell_h + 8);
    let btn_y = by + (bh - btn_h) / 2;
    let revert = (w.saturating_sub(btn_w + cell_w), btn_y, btn_w, btn_h);
    let keep = (w.saturating_sub(2 * btn_w + 2 * cell_w), btn_y, btn_w, btn_h);
    ((x0, by, w.saturating_sub(x0), bh), keep, revert)
}

fn hit(r: Rect, x: f64, y: f64) -> bool {
    x >= r.0 as f64 && x < (r.0 + r.2) as f64 && y >= r.1 as f64 && y < (r.1 + r.3) as f64
}

/// The list opens below its field when possible, otherwise above it.
fn dropdown_rect(w: usize, h: usize, cell_w: usize, cell_h: usize, field: usize, rows: usize) -> Rect {
    let x = sidebar_w(cell_w, cell_h) + cell_w * 2;
    let row_h = cell_h + 4;
    let height = (rows.min(DROPDOWN_ROWS) + 1) * row_h + 8;
    let below = row_y(cell_h, field) + cell_h + 5;
    let y = if below + height + cell_h < h { below } else { row_y(cell_h, field).saturating_sub(height + 5) };
    (x, y, w.saturating_sub(x + cell_w), height)
}

/// The Save button's rect (x, y, w, h) — bottom-right corner.
fn save_btn_rect(w: usize, h: usize, cell_w: usize, cell_h: usize) -> (usize, usize, usize, usize) {
    let bw = 8 * cell_w;
    let bh = cell_h + 8;
    (w.saturating_sub(bw + cell_w), h.saturating_sub(bh + 4), bw, bh)
}

/// Paint the whole scene into an ARGB canvas. Free function so both the live
/// window and the offline PPM test render identically.
fn paint_scene(
    m: &Model,
    dropdown: Option<&Dropdown>,
    atlas: &mut Atlas,
    pv: &mut Previews,
    px: &mut [u32],
    stride: usize,
    h: usize,
    cell_w: usize,
    cell_h: usize,
) {
    let p = &m.theme.palette;
    let bg = rgb(&p.bg).unwrap_or((18, 20, 18));
    let surface = rgb(&p.surface).unwrap_or((23, 26, 20));
    let border = rgb(&p.border).unwrap_or((57, 65, 43));
    let text = rgb(&p.text).unwrap_or((123, 140, 90));
    let accent = rgb(&p.accent).unwrap_or((164, 201, 75));
    let warn = rgb(&p.warn).unwrap_or((199, 209, 122));

    render::fill_rect(px, stride, h, 0, 0, stride, h, bg);
    let side = sidebar_w(cell_w, cell_h);
    render::fill_rect(px, stride, h, 0, 0, side, h, surface);
    render::fill_rect(px, stride, h, side, 0, 1, h, border);

    let mgn = margin(cell_h);
    let cats_active = m.focus == Focus::Cats;
    atlas.draw_str(px, stride, h, mgn, mgn - cell_h / 2, "lifeconf", accent);

    let dim = mix(text, surface);
    for (i, row) in sidebar_rows(m).iter().enumerate() {
        let y = row_y(cell_h, i);
        match *row {
            Side::Header(name) => {
                atlas.draw_str(px, stride, h, mgn, y, &name.to_uppercase(), dim);
            }
            Side::Cat(c) => {
                let selected = c == m.cat;
                if selected {
                    let col = if cats_active { accent } else { border };
                    render::fill_rect(px, stride, h, mgn - 4, y.saturating_sub(2), side - mgn - 2, cell_h + 3, col);
                }
                let fg = if selected { bg } else { text };
                atlas.draw_str(px, stride, h, mgn + cell_w, y, CATS[c], fg);
            }
        }
    }
    // Search box: "/ query_" while typing, the retained filter otherwise, else a hint.
    let (sy0, sh) = search_box(h, cell_h);
    render::fill_rect(px, stride, h, mgn - 4, sy0, side - mgn - 2, sh, bg);
    let q = m.search.as_deref().unwrap_or("");
    let (shown, scol) = match (m.searching, q.is_empty()) {
        (true, _) => (format!("/{q}_"), accent),
        (false, false) => (format!("/{q}"), warn),
        (false, true) => ("/ search".to_string(), dim),
    };
    let max = SIDEBAR_CELLS.saturating_sub(1);
    let shown: String = shown.chars().rev().take(max).collect::<Vec<_>>().into_iter().rev().collect();
    atlas.draw_str(px, stride, h, mgn, sy0 + 2, &shown, scol);

    let fields_active = m.focus == Focus::Fields;
    let fx = side + cell_w * 2;
    atlas.draw_str(px, stride, h, fx, mgn - cell_h / 2, CATS[m.cat], accent);
    let labels = field_labels(m.cat);
    for (i, label) in labels.iter().enumerate() {
        let y = row_y(cell_h, i);
        let selected = fields_active && i == m.field;
        if selected {
            render::fill_rect(px, stride, h, fx - 4, y.saturating_sub(2), stride - fx, cell_h + 3, border);
        }
        atlas.draw_str(px, stride, h, fx, y, label, text);
        let vx = fx + 24 * cell_w;
        let editing_here = selected && m.editing.is_some();
        let val = if editing_here {
            format!("{}_{}", m.editing.as_deref().unwrap_or(""), m.search_hint().unwrap_or_default())
        } else {
            m.value(m.cat, i)
        };
        let vcol = if selected { accent } else { text };
        let selectable = matches!(kind(m.cat, i), Kind::Preset | Kind::Enum(_) | Kind::Choice);
        let val = if selectable {
            let cells = stride.saturating_sub(vx + 4 * cell_w) / cell_w;
            val.chars().take(cells).collect::<String>()
        } else { val };
        let after = atlas.draw_str(px, stride, h, vx, y, &val, vcol);
        if selectable {
            atlas.draw_str(px, stride, h, stride.saturating_sub(3 * cell_w), y, "▾", vcol);
        }
        if matches!(kind(m.cat, i), Kind::Hex) {
            if let Some(c) = rgb(&m.value(m.cat, i)) {
                render::fill_rect(px, stride, h, after + cell_w, y, cell_h, cell_h, c);
            }
        }
    }

    // Previews under the fields, down to just above the status line.
    let top = row_y(cell_h, labels.len()) + cell_h;
    let bottom = save_btn_rect(stride, h, cell_w, cell_h).1.saturating_sub(cell_h);
    let pw = stride.saturating_sub(fx + 2 * cell_w);
    match CATS[m.cat] {
        "Cursor" if bottom > top + 2 * cell_h => {
            let t = &m.theme.cursor;
            atlas.draw_str(px, stride, h, fx, top, "preview", dim);
            let cursors = pv.cursors(&t.theme, t.size);
            if cursors.is_empty() {
                atlas.draw_str(px, stride, h, fx, top + cell_h + 4, "nothing to show: this theme has none of the usual cursors", dim);
            }
            let size = cursors.iter().map(|c| c.h).max().unwrap_or(0);
            let strip = size + 16;
            // Once on light and once on dark: an outline has to read on both.
            for (k, back) in [(0, (230, 230, 230)), (1, (24, 24, 24))] {
                let y0 = top + cell_h + 4 + k * strip;
                if y0 + strip > bottom {
                    break;
                }
                render::fill_rect(px, stride, h, fx, y0, pw, strip, back);
                let mut x = fx + 8;
                for c in cursors {
                    if x + c.w > fx + pw {
                        break;
                    }
                    preview::blit_cursor(px, stride, h, x, y0 + 8, c);
                    x += c.w + 12;
                }
            }
        }
        "Font" if bottom > top + 2 * cell_h => {
            let f = &m.theme.font;
            atlas.draw_str(px, stride, h, fx, top, "preview", dim);
            let y0 = top + cell_h + 4;
            match pv.font(&f.family, f.size) {
                Some(lf) => {
                    let room = (bottom.saturating_sub(y0 + 2 * cell_h + 16) / lf.line_h().max(1)).max(1);
                    let mut lines = lf.wrap(preview::PANGRAM, pw.saturating_sub(16));
                    lines.truncate(room);
                    let used = lines.len() * lf.line_h();
                    render::fill_rect(px, stride, h, fx, y0, pw, used + 16, surface);
                    lf.draw_lines(px, stride, h, fx + 8, y0 + 8, pw.saturating_sub(16), &lines, text);
                    render::fill_rect(px, stride, h, fx, y0, pw, 1, border);
                    render::fill_rect(px, stride, h, fx, y0 + used + 16, pw, 1, border);
                    if !lf.family.eq_ignore_ascii_case(f.family.trim()) {
                        let note = format!("\"{}\" isn't installed: this is {}", f.family.trim(), lf.family);
                        atlas.draw_str(px, stride, h, fx, y0 + used + 24, &note, warn);
                    }
                }
                None => {
                    atlas.draw_str(px, stride, h, fx, y0, "fontconfig found no font to show (is fc-match installed?)", warn);
                }
            }
        }
        _ => {}
    }

    let sy = h.saturating_sub(cell_h + 4);
    let scol = if m.dirty { warn } else { text };
    let status = if m.dirty {
        format!("{}   ● unsaved", m.status)
    } else {
        m.status.clone()
    };
    atlas.draw_str(px, stride, h, mgn, sy, &status, scol);

    // Save button, bottom-right — lit accent while there are unsaved changes.
    // Clicking it commits (and pops the polkit password prompt if the greeter
    // needs updating).
    let (bx, by, bw, bh) = save_btn_rect(stride, h, cell_w, cell_h);
    let (btn_bg, btn_fg) = if m.dirty { (accent, bg) } else { (surface, text) };
    render::fill_rect(px, stride, h, bx, by, bw, bh, btn_bg);
    if !m.dirty {
        // Thin outline so the idle button still reads as a control.
        render::fill_rect(px, stride, h, bx, by, bw, 1, border);
        render::fill_rect(px, stride, h, bx, by + bh - 1, bw, 1, border);
        render::fill_rect(px, stride, h, bx, by, 1, bh, border);
        render::fill_rect(px, stride, h, bx + bw - 1, by, 1, bh, border);
    }
    let label = "Save";
    let lx = bx + (bw.saturating_sub(label.len() * cell_w)) / 2;
    let ly = by + (bh.saturating_sub(cell_h)) / 2;
    atlas.draw_str(px, stride, h, lx, ly, label, btn_fg);

    if let Some(dd) = dropdown {
        let matches = dd.matches();
        let rect = dropdown_rect(stride, h, cell_w, cell_h, m.field, matches.len().max(1));
        let row_h = cell_h + 4;
        render::fill_rect(px, stride, h, rect.0, rect.1, rect.2, rect.3, surface);
        render::fill_rect(px, stride, h, rect.0, rect.1, rect.2, 1, accent);
        render::fill_rect(px, stride, h, rect.0, rect.1 + rect.3 - 1, rect.2, 1, accent);
        let cells = rect.2.saturating_sub(2 * cell_w) / cell_w;
        let prompt = format!("filter: {}_", dd.query);
        atlas.draw_str(px, stride, h, rect.0 + cell_w, rect.1 + 4,
            &prompt.chars().take(cells).collect::<String>(), accent);
        if matches.is_empty() {
            atlas.draw_str(px, stride, h, rect.0 + cell_w, rect.1 + row_h + 4, "no matches", warn);
        }
        for (visible, &option) in matches.iter().skip(dd.scroll).take(DROPDOWN_ROWS).enumerate() {
            let y = rect.1 + (visible + 1) * row_h + 4;
            let selected = dd.scroll + visible == dd.selected;
            if selected {
                render::fill_rect(px, stride, h, rect.0 + 2, y - 2, rect.2 - 4, row_h, border);
            }
            let label = &dd.options[option];
            atlas.draw_str(px, stride, h, rect.0 + cell_w, y,
                &label.chars().take(cells).collect::<String>(), if selected { accent } else { text });
        }
        if dd.scroll > 0 {
            atlas.draw_str(px, stride, h, rect.0 + rect.2 - cell_w, rect.1 + row_h + 4, "↑", accent);
        }
        if dd.scroll + DROPDOWN_ROWS < matches.len() {
            atlas.draw_str(px, stride, h, rect.0 + rect.2 - cell_w, rect.1 + rect.3 - row_h, "↓", accent);
        }
    }

    // Pending keep/revert prompt, drawn last so it sits over everything.
    if let (Some(p), Some(secs)) = (&m.pending, m.pending_secs(std::time::Instant::now())) {
        let (banner, keep, revert) = prompt_rects(stride, h, cell_w, cell_h);
        render::fill_rect(px, stride, h, banner.0, banner.1, banner.2, banner.3, surface);
        render::fill_rect(px, stride, h, banner.0, banner.1, banner.2, 1, warn);
        render::fill_rect(px, stride, h, banner.0, banner.1 + banner.3 - 1, banner.2, 1, warn);
        let tx = banner.0 + cell_w;
        let max_cells = (keep.0.saturating_sub(tx + cell_w)) / cell_w;
        let clip = |s: &str| -> String { s.chars().take(max_cells).collect() };
        atlas.draw_str(px, stride, h, tx, banner.1 + 6, &clip("Keep this change?"), warn);
        atlas.draw_str(
            px, stride, h, tx, banner.1 + 6 + cell_h,
            &clip(&format!("{}  (reverts in {secs}s)", p.what)),
            text,
        );
        for (r, label, active) in [(keep, "Keep (Enter)", true), (revert, "Revert (Esc)", false)] {
            let (fill, fg) = if active { (accent, bg) } else { (border, text) };
            render::fill_rect(px, stride, h, r.0, r.1, r.2, r.3, fill);
            let lx = r.0 + r.2.saturating_sub(label.len() * cell_w) / 2;
            atlas.draw_str(px, stride, h, lx, r.1 + (r.3.saturating_sub(cell_h)) / 2, label, fg);
        }
    }
}

/// Debug-only: render one frame of the default scene to a PPM, no Wayland —
/// the privacy-safe visual test for the GUI paint path.
#[cfg(debug_assertions)]
pub fn render_ppm(paths: Paths, theme: Theme, out: &str) -> i32 {
    let mut atlas = match Atlas::new(FONT, FONT_PX) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lifeconf: {e}");
            return 1;
        }
    };
    let (cell_w, cell_h) = (atlas.cell_w(), atlas.cell_h());
    let (w, h) = (760usize, 640usize);
    let mut px = vec![0u32; w * h];
    let mut m = Model::new(paths, theme);
    // Optional: preview a specific category (LIFECONF_PPM_CAT=Palette) with the
    // fields pane focused, so the offline test can exercise swatches/selection.
    if let Ok(name) = std::env::var("LIFECONF_PPM_CAT") {
        if let Some(i) = crate::model::CATS.iter().position(|c| *c == name) {
            m.enter_cat(i);
            m.focus = crate::model::Focus::Fields;
        }
    }
    // Preview the keep/revert banner without touching a real display.
    if std::env::var_os("LIFECONF_PPM_PENDING").is_some() {
        m.pending = Some(crate::model::Pending {
            undo: vec![],
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(42),
            what: "eDP-1 scale 1.5".into(),
        });
    }
    // A field mid-edit, e.g. a choice search (LIFECONF_PPM_EDIT=ubu).
    if let Ok(t) = std::env::var("LIFECONF_PPM_EDIT") {
        m.focus = crate::model::Focus::Fields;
        m.editing = Some(t);
    }
    if let Ok(q) = std::env::var("LIFECONF_PPM_SEARCH") {
        m.search = Some(q);
        m.searching = true;
        m.clamp_to_search();
    }
    paint_scene(&m, None, &mut atlas, &mut Previews::default(), &mut px, w, h, cell_w, cell_h);

    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for pixel in &px {
        ppm.push((pixel >> 16) as u8);
        ppm.push((pixel >> 8) as u8);
        ppm.push(*pixel as u8);
    }
    if let Err(e) = std::fs::write(out, ppm) {
        eprintln!("lifeconf: write {out}: {e}");
        return 1;
    }
    eprintln!("wrote {out} ({w}x{h})");
    0
}

impl Gui {
    fn open_dropdown(&mut self) {
        let options = self.m.options_here();
        if options.is_empty() {
            self.m.status = "no choices available".into();
            return;
        }
        let current = self.m.value(self.m.cat, self.m.field);
        let selected = options.iter().position(|v| *v == current).unwrap_or(0);
        self.dropdown = Some(Dropdown { options, query: String::new(), selected, scroll: selected.saturating_sub(DROPDOWN_ROWS - 1) });
    }

    fn sidebar_w(&self) -> usize {
        sidebar_w(self.cell_w, self.cell_h)
    }
    fn row_y(&self, i: usize) -> usize {
        row_y(self.cell_h, i)
    }

    fn draw(&mut self) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = w;
        if self.pool.is_none() {
            match SlotPool::new(w * h * 4, &self.shm) {
                Ok(p) => self.pool = Some(p),
                Err(e) => {
                    eprintln!("lifeconf: shm pool: {e}");
                    return;
                }
            }
        }
        let mut pool = self.pool.take().unwrap();
        let (buffer, canvas) =
            match pool.create_buffer(w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("lifeconf: buffer: {e}");
                    self.pool = Some(pool);
                    return;
                }
            };
        // Reinterpret the &mut [u8] canvas as &mut [u32] for pixel writes.
        let px: &mut [u32] = bytemuck_cast(canvas);
        self.paint(px, stride, h);
        let surface = self.window.wl_surface();
        buffer.attach_to(surface).expect("attach");
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.commit();
        self.pool = Some(pool);
        let _ = buffer; // ownership held by pool
    }

    fn paint(&mut self, px: &mut [u32], stride: usize, h: usize) {
        paint_scene(&self.m, self.dropdown.as_ref(), &mut self.atlas, &mut self.previews, px, stride, h, self.cell_w, self.cell_h);
    }

    fn redraw(&mut self) {
        self.window.wl_surface().frame(&self.qh, self.window.wl_surface().clone());
        self.draw();
        self.ensure_timer();
    }

    /// While a keep/revert prompt is open, wake once a second to repaint the
    /// countdown and fire the revert at the deadline. The timer drops itself the
    /// moment nothing is pending, so an idle settings window costs nothing.
    fn ensure_timer(&mut self) {
        if self.m.pending.is_none() || self.timer_armed {
            return;
        }
        self.timer_armed = true;
        let _ = self.loop_handle.insert_source(Timer::from_duration(TICK), |_, _, gui: &mut Gui| {
            gui.m.tick(std::time::Instant::now());
            if gui.m.pending.is_some() {
                gui.window.wl_surface().frame(&gui.qh, gui.window.wl_surface().clone());
                gui.draw();
                TimeoutAction::ToDuration(TICK)
            } else {
                gui.timer_armed = false;
                gui.draw();
                TimeoutAction::Drop
            }
        });
    }

    /// Which of the first `n` rows (sidebar or fields) a click at height `y` hit.
    fn row_at(&self, y: f64, n: usize) -> Option<usize> {
        (0..n).find(|&i| {
            let ry = self.row_y(i) as f64;
            y >= ry - 2.0 && y < ry + self.cell_h as f64 + 1.0
        })
    }

    /// Pointer click: the Save button, else the category/field row under (x,y).
    fn click(&mut self, x: f64, y: f64) {
        // A pending keep/revert prompt is modal: only its two buttons respond.
        if self.m.pending.is_some() {
            let (_, keep, revert) =
                prompt_rects(self.width as usize, self.height as usize, self.cell_w, self.cell_h);
            if hit(keep, x, y) {
                self.m.keep_pending();
            } else if hit(revert, x, y) {
                self.m.revert_pending("reverted");
            }
            self.redraw();
            return;
        }
        if let Some(dd) = self.dropdown.take() {
            let matches = dd.matches();
            let rect = dropdown_rect(self.width as usize, self.height as usize,
                self.cell_w, self.cell_h, self.m.field, matches.len().max(1));
            if hit(rect, x, y) {
                let row_h = self.cell_h + 4;
                let offset = (y as usize - rect.1).saturating_sub(4) / row_h;
                if offset > 0 {
                    if let Some(&index) = matches.get(dd.scroll + offset - 1) {
                        if offset <= DROPDOWN_ROWS {
                            self.m.pick_option(&dd.options[index]);
                            self.redraw();
                            return;
                        }
                    }
                }
                self.dropdown = Some(dd);
                self.redraw();
                return;
            }
        }
        // A click anywhere but the field being typed into ends that edit first,
        // keeping what was typed. Left open, the buffer would follow the
        // selection and Enter would write it into whichever field was next.
        if self.m.editing.is_some() {
            if x >= self.sidebar_w() as f64 && self.row_at(y, self.m.n_fields()) == Some(self.m.field) {
                return;
            }
            let s = self.m.editing.take().unwrap();
            // A half-typed choice search is dropped, not resolved to a guess.
            if !matches!(self.m.kind_here(), Kind::Choice) {
                self.m.set_text(&s);
            }
        }
        let (bx, by, bw, bh) =
            save_btn_rect(self.width as usize, self.height as usize, self.cell_w, self.cell_h);
        if x >= bx as f64 && x < (bx + bw) as f64 && y >= by as f64 && y < (by + bh) as f64 {
            self.m.commit();
            self.redraw();
            return;
        }

        let side = self.sidebar_w() as f64;
        let (sy0, sh) = search_box(self.height as usize, self.cell_h);
        if x < side && y >= sy0 as f64 && y < (sy0 + sh) as f64 {
            self.m.searching = true;
            self.m.search.get_or_insert_with(String::new);
            self.m.focus = Focus::Cats;
        } else if x < side {
            let rows = sidebar_rows(&self.m);
            if let Some(Side::Cat(c)) = self.row_at(y, rows.len()).map(|i| rows[i]) {
                self.m.searching = false;
                self.m.enter_cat(c);
                self.m.focus = Focus::Cats;
            }
        } else if let Some(i) = self.row_at(y, self.m.n_fields()) {
            self.m.field = i;
            self.m.focus = Focus::Fields;
            // Selectable rows open a list; other controls retain their action.
            match kind(self.m.cat, i) {
                Kind::Hex | Kind::Text | Kind::Float(_) | Kind::Int(_) => self.m.begin_edit(),
                Kind::Preset | Kind::Enum(_) | Kind::Choice => self.open_dropdown(),
                _ => self.m.nudge(1),
            }
        }
        self.redraw();
    }

    fn on_key(&mut self, ev: KeyEvent) {
        let sym = ev.keysym;
        // Modal prompt: Enter/y keeps, Esc/n reverts, everything else waits.
        if self.m.pending.is_some() {
            match sym {
                Keysym::Return | Keysym::KP_Enter | Keysym::y => self.m.keep_pending(),
                Keysym::Escape | Keysym::n => self.m.revert_pending("reverted"),
                _ => {}
            }
            self.redraw();
            return;
        }
        if let Some(dd) = self.dropdown.as_mut() {
            match sym {
                Keysym::Escape => self.dropdown = None,
                Keysym::Return | Keysym::KP_Enter => {
                    let picked = dd.matches().get(dd.selected).map(|&i| dd.options[i].clone());
                    self.dropdown = None;
                    if let Some(value) = picked { self.m.pick_option(&value); }
                }
                Keysym::Up => dd.move_by(-1),
                Keysym::Down => dd.move_by(1),
                Keysym::Page_Up => dd.move_by(-(DROPDOWN_ROWS as isize)),
                Keysym::Page_Down => dd.move_by(DROPDOWN_ROWS as isize),
                Keysym::BackSpace => {
                    dd.query.pop();
                    dd.selected = 0;
                    dd.scroll = 0;
                }
                _ => {
                    if !self.ctrl {
                        if let Some(t) = ev.utf8.as_deref().filter(|t| !t.chars().any(char::is_control)) {
                            dd.query.push_str(t);
                            dd.selected = 0;
                            dd.scroll = 0;
                        }
                    }
                }
            }
            self.redraw();
            return;
        }
        // Editing mode.
        if self.m.editing.is_some() {
            match sym {
                Keysym::Return | Keysym::KP_Enter => {
                    let s = self.m.editing.take().unwrap();
                    self.m.set_text(&s);
                }
                Keysym::Escape => {
                    self.m.editing = None;
                    self.m.status = "edit cancelled".into();
                }
                Keysym::BackSpace => {
                    if let Some(b) = self.m.editing.as_mut() {
                        b.pop();
                    }
                }
                _ => {
                    if let Some(t) = ev.utf8 {
                        if let Some(b) = self.m.editing.as_mut() {
                            b.push_str(&t);
                        }
                    }
                }
            }
            self.redraw();
            return;
        }

        if self.m.searching {
            match sym {
                Keysym::Escape => {
                    self.m.search = None;
                    self.m.searching = false;
                }
                Keysym::Return | Keysym::KP_Enter => {
                    self.m.searching = false;
                    if !self.m.visible_cats().is_empty() {
                        self.m.focus = Focus::Fields;
                    }
                }
                Keysym::Down => self.m.move_down(),
                Keysym::Up => self.m.move_up(),
                Keysym::BackSpace => {
                    if let Some(q) = self.m.search.as_mut() {
                        q.pop();
                    }
                    self.m.clamp_to_search();
                }
                _ => {
                    // Tab, newline etc. arrive as utf8 too; they'd poison the filter.
                    let text = ev.utf8.as_deref().filter(|t| !t.chars().any(char::is_control));
                    if let (Some(t), false) = (text, self.ctrl) {
                        if let Some(q) = self.m.search.as_mut() {
                            q.push_str(t);
                        }
                        self.m.clamp_to_search();
                    }
                }
            }
            self.redraw();
            return;
        }

        if (self.ctrl && sym == Keysym::f) || sym == Keysym::slash {
            self.m.searching = true;
            self.m.search.get_or_insert_with(String::new);
            self.m.focus = Focus::Cats;
            self.redraw();
            return;
        }
        if sym == Keysym::Escape && self.m.search.is_some() {
            self.m.search = None;
            self.redraw();
            return;
        }

        if self.ctrl && sym == Keysym::c {
            if self.m.dirty {
                self.m.revert();
            }
            self.m.quit = true;
            return;
        }

        match sym {
            Keysym::q => {
                if self.m.dirty {
                    self.m.commit();
                }
                self.m.quit = true;
                return;
            }
            Keysym::s => self.m.commit(),
            Keysym::Tab => self.m.toggle_focus(),
            Keysym::Down | Keysym::j => self.m.move_down(),
            Keysym::Up | Keysym::k => self.m.move_up(),
            Keysym::Right | Keysym::l => {
                if self.m.focus == Focus::Cats {
                    self.m.focus = Focus::Fields;
                } else {
                    self.m.nudge(1);
                }
            }
            Keysym::Left | Keysym::h => {
                if self.m.focus == Focus::Fields {
                    match self.m.kind_here() {
                        Kind::Hex | Kind::Text => self.m.focus = Focus::Cats,
                        _ => self.m.nudge(-1),
                    }
                }
            }
            Keysym::plus | Keysym::equal => {
                if self.m.focus == Focus::Fields {
                    self.m.nudge(1);
                }
            }
            Keysym::minus | Keysym::underscore => {
                if self.m.focus == Focus::Fields {
                    self.m.nudge(-1);
                }
            }
            Keysym::Return | Keysym::KP_Enter | Keysym::space => match self.m.focus {
                Focus::Cats => self.m.focus = Focus::Fields,
                // Space steps a choice; Enter searches it.
                Focus::Fields if sym == Keysym::space && matches!(self.m.kind_here(), Kind::Choice) => self.m.nudge(1),
                Focus::Fields => self.m.activate(),
            },
            _ => {}
        }
        self.redraw();
    }
}

/// Reinterpret a byte canvas as u32 pixels (ARGB8888, native endian). The shm
/// buffer is 4-byte aligned and sized w*h*4, so this is sound.
fn bytemuck_cast(bytes: &mut [u8]) -> &mut [u32] {
    let len = bytes.len() / 4;
    unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u32, len) }
}

impl WindowHandler for Gui {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.m.quit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let Some((w, h)) = configure.new_size.0.zip(configure.new_size.1) {
            self.width = w.get();
            self.height = h.get();
            self.pool = None;
        }
        self.configured = true;
        self.draw();
    }
}

impl CompositorHandler for Gui {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl KeyboardHandler for Gui {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.on_key(event);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        // Held keys repeat — useful for arrow navigation and +/- stepping.
        self.on_key(event);
    }
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.ctrl = modifiers.ctrl;
    }
}

impl PointerHandler for Gui {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            if e.surface != *self.window.wl_surface() {
                continue;
            }
            match e.kind {
                PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                    self.pointer_pos = e.position;
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    let (x, y) = e.position;
                    self.click(x, y);
                }
                PointerEventKind::Axis { vertical, .. } => {
                    if let Some(dd) = self.dropdown.as_mut() {
                        let rect = dropdown_rect(self.width as usize, self.height as usize,
                            self.cell_w, self.cell_h, self.m.field, dd.matches().len().max(1));
                        if hit(rect, e.position.0, e.position.1) {
                            let steps = if vertical.discrete != 0 { vertical.discrete as isize }
                                else { (vertical.absolute / 15.0) as isize };
                            dd.move_by(steps);
                            self.redraw();
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl SeatHandler for Gui {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kb);
            }
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl OutputHandler for Gui {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Gui {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Gui {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_compositor!(Gui);
smithay_client_toolkit::delegate_output!(Gui);
smithay_client_toolkit::delegate_shm!(Gui);
smithay_client_toolkit::delegate_seat!(Gui);
smithay_client_toolkit::delegate_keyboard!(Gui);
smithay_client_toolkit::delegate_pointer!(Gui);

#[cfg(test)]
mod dropdown_tests {
    use super::*;

    #[test]
    fn filtering_and_navigation_keep_the_selected_item_visible() {
        let mut dd = Dropdown {
            options: (0..20).map(|i| format!("item {i:02}")).collect(),
            query: String::new(), selected: 0, scroll: 0,
        };
        dd.move_by(12);
        assert_eq!((dd.selected, dd.scroll), (12, 7));
        dd.query = "1".into();
        dd.selected = 0;
        dd.scroll = 0;
        assert_eq!(dd.matches().len(), 11);
        dd.move_by(20);
        assert_eq!((dd.selected, dd.scroll), (10, 5));
    }

    #[test]
    fn dropdown_uses_space_above_low_fields() {
        let rect = dropdown_rect(760, 360, 10, 18, 12, 20);
        assert!(rect.1 + rect.3 < row_y(18, 12));
        let rect = dropdown_rect(760, 360, 10, 18, 0, 3);
        assert!(rect.1 > row_y(18, 0));
        let rect = dropdown_rect(760, 360, 10, 18, 5, 20);
        assert!(rect.1 > row_y(18, 5));
        assert!(rect.1 + rect.3 < 360);
    }
}
smithay_client_toolkit::delegate_xdg_shell!(Gui);
smithay_client_toolkit::delegate_xdg_window!(Gui);
smithay_client_toolkit::delegate_registry!(Gui);
