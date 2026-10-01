// SPDX-License-Identifier: GPL-3.0-or-later
// Browser state and behaviour: navigation, selection, clipboard, file
// operations, keyboard and mouse handling. Rendering is in ui.rs, which only
// reads this and records where things were drawn (Hits) so clicks can be
// mapped back to entries.

use crate::fs::{self, Entry};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub struct Loaded {
    gen: u64,
    dir: PathBuf,
    entries: std::io::Result<Vec<Entry>>,
    parent: Vec<Entry>,
}

pub enum Preview {
    None,
    Dir(Vec<String>),
    Text(Vec<String>),
    Note(String),
}

#[derive(Clone)]
pub enum InputKind {
    Rename(PathBuf),
    Path,
    Filter,
    NewDir,
}

pub struct Input {
    pub kind: InputKind,
    pub buf: Vec<char>,
    pub cur: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum MenuAction {
    Open,
    Cut,
    Copy,
    Paste,
    Rename,
    Trash,
    NewDir,
    ToggleHidden,
}

pub struct Menu {
    pub pos: (u16, u16),
    pub items: Vec<(&'static str, MenuAction)>,
    pub sel: usize,
}

pub enum Mode {
    Normal,
    Input(Input),
    /// Permanent delete awaiting y/n.
    Confirm(Vec<PathBuf>),
    Menu(Menu),
}

pub struct Drag {
    pub paths: Vec<PathBuf>,
    pub start: (u16, u16),
    pub active: bool,
    pub hover: Option<PathBuf>,
}

/// Where ui.rs drew things last frame.
#[derive(Default)]
pub struct Hits {
    pub places: Vec<(Rect, PathBuf)>,
    pub crumbs: Vec<(Rect, PathBuf)>,
    pub list: Rect,
    pub parent: Rect,
    pub parent_scroll: usize,
    pub menu: Vec<(Rect, usize)>,
    pub menu_rect: Rect,
    pub height: usize,
}

pub struct App {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub view: Vec<usize>,
    pub parent_entries: Vec<Entry>,
    pub parent_view: Vec<usize>,
    pub sel: usize,
    pub scroll: usize,
    /// Keyboard moves pin the cursor on screen; wheel scrolling must not.
    pub follow: bool,
    pub marks: BTreeSet<PathBuf>,
    pub clip: Option<(Vec<PathBuf>, bool)>,
    pub show_hidden: bool,
    pub filter: String,
    pub mode: Mode,
    pub msg: String,
    pub preview: Preview,
    pub loading: bool,
    pub places: Vec<(String, PathBuf)>,
    pub hits: Hits,
    pub drag: Option<Drag>,
    pub quit: bool,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    gen: u64,
    pending_select: Option<String>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    last_click: Option<(Instant, usize)>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

fn build_places() -> Vec<(String, PathBuf)> {
    let h = home();
    let mut v = vec![("Home".to_string(), h.clone())];
    for d in ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"] {
        let p = h.join(d);
        if p.is_dir() {
            v.push((d.to_string(), p));
        }
    }
    v.push(("Trash".into(), fs::home_trash().join("files")));
    v.push(("Root".into(), PathBuf::from("/")));
    if let Ok(user) = std::env::var("USER") {
        if let Ok(rd) = std::fs::read_dir(format!("/run/media/{user}")) {
            let mut media: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            media.sort();
            for p in media {
                let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                v.push((n, p));
            }
        }
    }
    v
}

impl App {
    pub fn new(start: PathBuf) -> App {
        let (tx, rx) = channel();
        let mut app = App {
            cwd: start.clone(),
            entries: Vec::new(),
            view: Vec::new(),
            parent_entries: Vec::new(),
            parent_view: Vec::new(),
            sel: 0,
            scroll: 0,
            follow: true,
            marks: BTreeSet::new(),
            clip: None,
            show_hidden: false,
            filter: String::new(),
            mode: Mode::Normal,
            msg: String::new(),
            preview: Preview::None,
            loading: false,
            places: build_places(),
            hits: Hits::default(),
            drag: None,
            quit: false,
            back: Vec::new(),
            fwd: Vec::new(),
            gen: 0,
            pending_select: None,
            tx,
            rx,
            last_click: None,
        };
        app.load(start, None);
        app
    }

    // ---- loading -------------------------------------------------------

    /// Read `dir` (and its parent, for the left column) off-thread so a huge or
    /// slow directory never blocks a redraw; stale results are dropped by gen.
    fn load(&mut self, dir: PathBuf, select: Option<String>) {
        self.gen += 1;
        self.cwd = dir.clone();
        self.loading = true;
        self.pending_select = select;
        let (gen, tx) = (self.gen, self.tx.clone());
        std::thread::spawn(move || {
            let entries = fs::read_dir(&dir);
            let parent = dir.parent().and_then(|p| fs::read_dir(p).ok()).unwrap_or_default();
            let _ = tx.send(Loaded { gen, dir, entries, parent });
        });
    }

    /// Returns true if something arrived (caller should redraw).
    pub fn poll_loader(&mut self) -> bool {
        let mut got = false;
        while let Ok(l) = self.rx.try_recv() {
            if l.gen != self.gen {
                continue;
            }
            got = true;
            self.loading = false;
            self.parent_entries = l.parent;
            match l.entries {
                Ok(e) => self.entries = e,
                Err(e) => {
                    self.entries.clear();
                    self.msg = format!("{}: {e}", l.dir.display());
                }
            }
            // view holds indices into the OLD entries; drop it before refilter
            // peeks at current().
            self.view.clear();
            let want = self.pending_select.take();
            self.refilter(want.as_deref());
            self.scroll = 0;
            self.follow = true;
        }
        got
    }

    pub fn reload(&mut self) {
        let keep = self.current().map(|e| e.name.clone());
        self.load(self.cwd.clone(), keep);
    }

    fn refilter(&mut self, select: Option<&str>) {
        let keep = select.map(str::to_owned).or_else(|| self.current().map(|e| e.name.clone()));
        let f = self.filter.to_lowercase();
        self.view = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                (self.show_hidden || !e.hidden()) && (f.is_empty() || e.name.to_lowercase().contains(&f))
            })
            .collect();
        self.parent_view = (0..self.parent_entries.len())
            .filter(|&i| self.show_hidden || !self.parent_entries[i].hidden())
            .collect();
        self.sel = keep
            .and_then(|n| self.view.iter().position(|&i| self.entries[i].name == n))
            .unwrap_or(0)
            .min(self.view.len().saturating_sub(1));
        self.follow = true;
        self.update_preview();
    }

    pub fn current(&self) -> Option<&Entry> {
        self.view.get(self.sel).map(|&i| &self.entries[i])
    }

    pub fn visible(&self) -> impl Iterator<Item = &Entry> {
        self.view.iter().map(|&i| &self.entries[i])
    }

    fn update_preview(&mut self) {
        let Some(e) = self.current() else {
            self.preview = Preview::None;
            return;
        };
        self.preview = if e.is_dir {
            match std::fs::read_dir(&e.path) {
                Ok(rd) => {
                    let mut names: Vec<(bool, String)> = rd
                        .flatten()
                        .take(300)
                        .map(|d| {
                            (d.path().is_dir(), d.file_name().to_string_lossy().into_owned())
                        })
                        .collect();
                    names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| fs::natural_cmp(&a.1, &b.1)));
                    if names.is_empty() {
                        Preview::Note("(empty)".into())
                    } else {
                        Preview::Dir(
                            names.into_iter().map(|(d, n)| if d { format!("{n}/") } else { n }).collect(),
                        )
                    }
                }
                Err(e) => Preview::Note(e.to_string()),
            }
        } else {
            match fs::text_preview(&e.path, 200) {
                Some(l) if l.is_empty() => Preview::Note("(empty file)".into()),
                Some(l) => Preview::Text(l),
                None => Preview::Note(format!("binary, {}", fs::human_size(e.size))),
            }
        };
    }

    // ---- navigation ----------------------------------------------------

    pub fn go(&mut self, dir: &Path, select: Option<String>) {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if !dir.is_dir() {
            self.msg = format!("not a folder: {}", dir.display());
            return;
        }
        if dir != self.cwd {
            self.back.push(self.cwd.clone());
            self.fwd.clear();
            self.marks.clear();
            self.filter.clear();
        }
        self.load(dir, select);
    }

    fn go_history(&mut self, backwards: bool) {
        let target = if backwards { self.back.pop() } else { self.fwd.pop() };
        let Some(t) = target else { return };
        let here = self.cwd.clone();
        if backwards { self.fwd.push(here) } else { self.back.push(here) }
        self.marks.clear();
        self.filter.clear();
        self.load(t, None);
    }

    pub fn go_up(&mut self) {
        if let Some(p) = self.cwd.parent().map(Path::to_path_buf) {
            let from = self.cwd.file_name().map(|n| n.to_string_lossy().into_owned());
            self.go(&p, from);
        }
    }

    pub fn open_current(&mut self) {
        let Some(e) = self.current().cloned() else { return };
        self.open(&e);
    }

    fn open(&mut self, e: &Entry) {
        if e.is_dir {
            self.go(&e.path, None);
        } else {
            match std::process::Command::new("xdg-open")
                .arg(&e.path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    // Reap it so a long-lived viewer doesn't leave a zombie
                    // once it exits.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    self.msg = format!("opened {}", e.name);
                }
                Err(err) => self.msg = format!("xdg-open: {err}"),
            }
        }
    }

    pub fn select(&mut self, i: usize) {
        if self.view.is_empty() {
            return;
        }
        let i = i.min(self.view.len() - 1);
        if i != self.sel {
            self.sel = i;
            self.update_preview();
        }
        self.follow = true;
    }

    fn move_sel(&mut self, delta: isize) {
        let n = self.view.len() as isize;
        if n > 0 {
            self.select((self.sel as isize + delta).clamp(0, n - 1) as usize);
        }
    }

    // ---- file operations ----------------------------------------------

    fn targets(&self) -> Vec<PathBuf> {
        if self.marks.is_empty() {
            self.current().map(|e| vec![e.path.clone()]).unwrap_or_default()
        } else {
            self.marks.iter().cloned().collect()
        }
    }

    fn set_clip(&mut self, cut: bool) {
        let t = self.targets();
        if t.is_empty() {
            return;
        }
        self.msg = format!("{} {} item(s)", if cut { "cut" } else { "copied" }, t.len());
        self.clip = Some((t, cut));
    }

    fn paste(&mut self) {
        let Some((paths, cut)) = self.clip.clone() else {
            self.msg = "clipboard is empty".into();
            return;
        };
        let dest = self.cwd.clone();
        let (ok, err) = self.transfer(&paths, &dest, !cut);
        if cut {
            self.clip = None;
        }
        self.finish_op(if cut { "moved" } else { "pasted" }, ok, err);
    }

    /// Move or copy every path into `dest`; returns (successes, first error).
    fn transfer(&mut self, paths: &[PathBuf], dest: &Path, copy: bool) -> (usize, Option<String>) {
        let (mut ok, mut err) = (0, None);
        // The Trash place is a drop target, but a bare rename into files/ would
        // leave no .trashinfo; go through the real trash path instead.
        let trash = fs::home_trash();
        if dest == trash.join("files") {
            if copy {
                return (0, Some("cannot copy into the trash".into()));
            }
            for p in paths {
                match fs::trash_to(p, &trash) {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        err.get_or_insert_with(|| format!("{}: {e}", p.display()));
                    }
                }
            }
            return (ok, err);
        }
        for p in paths {
            // Dropping a folder onto itself or a descendant would loop or vanish.
            if dest.starts_with(p) {
                err.get_or_insert_with(|| format!("cannot put {} inside itself", p.display()));
                continue;
            }
            let r = if copy { fs::copy_into(p, dest) } else { fs::move_into(p, dest) };
            match r {
                Ok(_) => ok += 1,
                Err(e) => {
                    err.get_or_insert_with(|| format!("{}: {e}", p.display()));
                }
            }
        }
        (ok, err)
    }

    fn finish_op(&mut self, verb: &str, ok: usize, err: Option<String>) {
        self.msg = match err {
            Some(e) => format!("{verb} {ok}, error: {e}"),
            None => format!("{verb} {ok} item(s)"),
        };
        self.marks.clear();
        self.reload();
    }

    fn trash(&mut self) {
        let t = self.targets();
        let trash = fs::home_trash();
        let (mut ok, mut err) = (0, None);
        for p in &t {
            match fs::trash_to(p, &trash) {
                Ok(()) => ok += 1,
                Err(e) => {
                    err.get_or_insert_with(|| format!("{}: {e}", p.display()));
                }
            }
        }
        self.finish_op("trashed", ok, err);
    }

    fn delete_forever(&mut self, paths: Vec<PathBuf>) {
        let (mut ok, mut err) = (0, None);
        for p in &paths {
            match fs::remove_recursive(p) {
                Ok(()) => ok += 1,
                Err(e) => {
                    err.get_or_insert_with(|| format!("{}: {e}", p.display()));
                }
            }
        }
        self.finish_op("deleted", ok, err);
    }

    fn start_input(&mut self, kind: InputKind, text: String) {
        let buf: Vec<char> = text.chars().collect();
        let cur = buf.len();
        self.mode = Mode::Input(Input { kind, buf, cur });
    }

    fn commit_input(&mut self, inp: Input) {
        let text: String = inp.buf.iter().collect();
        match inp.kind {
            InputKind::Filter => {} // applied live
            InputKind::Path => {
                let t = match text.strip_prefix('~') {
                    Some(rest) => format!("{}{rest}", home().display()),
                    None => text,
                };
                let p = PathBuf::from(t);
                if p.is_dir() {
                    self.go(&p, None);
                } else if let (Some(par), Some(name)) = (p.parent(), p.file_name()) {
                    self.go(par, Some(name.to_string_lossy().into_owned()));
                }
            }
            InputKind::NewDir => {
                if text.is_empty() || text.contains('/') {
                    self.msg = "invalid folder name".into();
                } else {
                    match std::fs::create_dir(self.cwd.join(&text)) {
                        Ok(()) => {
                            self.msg = format!("created {text}");
                            self.load(self.cwd.clone(), Some(text));
                        }
                        Err(e) => self.msg = format!("mkdir: {e}"),
                    }
                }
            }
            InputKind::Rename(path) => {
                let dest = path.with_file_name(&text);
                if text.is_empty() || text.contains('/') {
                    self.msg = "invalid name".into();
                } else if dest == path {
                    // unchanged
                } else if std::fs::symlink_metadata(&dest).is_ok() {
                    self.msg = format!("{text} already exists");
                } else {
                    match std::fs::rename(&path, &dest) {
                        Ok(()) => {
                            self.msg = format!("renamed to {text}");
                            self.marks.clear();
                            self.load(self.cwd.clone(), Some(text));
                        }
                        Err(e) => self.msg = format!("rename: {e}"),
                    }
                }
            }
        }
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refilter(None);
    }

    fn apply_menu(&mut self, a: MenuAction) {
        match a {
            MenuAction::Open => self.open_current(),
            MenuAction::Cut => self.set_clip(true),
            MenuAction::Copy => self.set_clip(false),
            MenuAction::Paste => self.paste(),
            MenuAction::Rename => {
                if let Some(e) = self.current().cloned() {
                    self.start_input(InputKind::Rename(e.path), e.name);
                }
            }
            MenuAction::Trash => self.trash(),
            MenuAction::NewDir => self.start_input(InputKind::NewDir, String::new()),
            MenuAction::ToggleHidden => self.toggle_hidden(),
        }
    }

    // ---- keyboard ------------------------------------------------------

    pub fn on_key(&mut self, k: KeyEvent) {
        self.msg.clear();
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.key_normal(k),
            Mode::Input(i) => self.key_input(k, i),
            Mode::Confirm(p) => match k.code {
                KeyCode::Char('y') => self.delete_forever(p),
                _ => self.msg = "cancelled".into(),
            },
            Mode::Menu(m) => self.key_menu(k, m),
        }
    }

    fn key_menu(&mut self, k: KeyEvent, mut m: Menu) {
        match k.code {
            KeyCode::Esc => {}
            KeyCode::Up | KeyCode::Char('k') => {
                m.sel = (m.sel + m.items.len() - 1) % m.items.len();
                self.mode = Mode::Menu(m);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                m.sel = (m.sel + 1) % m.items.len();
                self.mode = Mode::Menu(m);
            }
            KeyCode::Enter => {
                let a = m.items[m.sel].1;
                self.apply_menu(a);
            }
            _ => self.mode = Mode::Menu(m),
        }
    }

    fn key_input(&mut self, k: KeyEvent, mut i: Input) {
        let live_filter = matches!(i.kind, InputKind::Filter);
        match k.code {
            KeyCode::Esc => {
                if live_filter {
                    self.filter.clear();
                    self.refilter(None);
                }
                return;
            }
            KeyCode::Enter => return self.commit_input(i),
            KeyCode::Left => i.cur = i.cur.saturating_sub(1),
            KeyCode::Right => i.cur = (i.cur + 1).min(i.buf.len()),
            KeyCode::Home => i.cur = 0,
            KeyCode::End => i.cur = i.buf.len(),
            KeyCode::Backspace if i.cur > 0 => {
                i.cur -= 1;
                i.buf.remove(i.cur);
            }
            KeyCode::Delete if i.cur < i.buf.len() => {
                i.buf.remove(i.cur);
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                i.buf.drain(..i.cur);
                i.cur = 0;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                i.buf.insert(i.cur, c);
                i.cur += 1;
            }
            _ => {}
        }
        if live_filter {
            self.filter = i.buf.iter().collect();
            self.refilter(None);
        }
        self.mode = Mode::Input(i);
    }

    fn key_normal(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let page = self.hits.height.max(2) as isize - 1;
        match k.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') if !ctrl => self.move_sel(-1),
            KeyCode::Down | KeyCode::Char('j') if !ctrl => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-page),
            KeyCode::PageDown => self.move_sel(page),
            KeyCode::Home | KeyCode::Char('g') => self.select(0),
            KeyCode::End | KeyCode::Char('G') => self.select(usize::MAX),
            KeyCode::Left if alt => self.go_history(true),
            KeyCode::Right if alt => self.go_history(false),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace if !ctrl => self.go_up(),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter if !ctrl => self.open_current(),
            KeyCode::Char(' ') => {
                if let Some(e) = self.current() {
                    let p = e.path.clone();
                    if !self.marks.remove(&p) {
                        self.marks.insert(p);
                    }
                }
                self.move_sel(1);
            }
            KeyCode::Char('a') if ctrl => {
                self.marks = self.visible().map(|e| e.path.clone()).collect();
            }
            KeyCode::Esc => {
                self.marks.clear();
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.refilter(None);
                }
            }
            KeyCode::Char('c') if ctrl => self.set_clip(false),
            KeyCode::Char('x') if ctrl => self.set_clip(true),
            KeyCode::Char('v') if ctrl => self.paste(),
            KeyCode::Char('y') => self.set_clip(false),
            KeyCode::Char('x') => self.set_clip(true),
            KeyCode::Char('p') => self.paste(),
            KeyCode::F(2) => self.apply_menu(MenuAction::Rename),
            KeyCode::F(5) | KeyCode::Char('r') if ctrl || k.code == KeyCode::F(5) => self.reload(),
            KeyCode::F(7) | KeyCode::Char('n') if ctrl || k.code == KeyCode::F(7) => {
                self.apply_menu(MenuAction::NewDir)
            }
            KeyCode::Delete if shift => {
                let t = self.targets();
                if !t.is_empty() {
                    self.mode = Mode::Confirm(t);
                }
            }
            KeyCode::Delete => self.trash(),
            KeyCode::Char('l') | KeyCode::Char('L') if ctrl => {
                self.start_input(InputKind::Path, format!("{}/", self.cwd.display()).replace("//", "/"))
            }
            KeyCode::Char('f') if ctrl => self.start_input(InputKind::Filter, self.filter.clone()),
            KeyCode::Char('/') => self.start_input(InputKind::Filter, self.filter.clone()),
            KeyCode::Char('h') | KeyCode::Char('H') if ctrl => self.toggle_hidden(),
            KeyCode::Char('.') => self.toggle_hidden(),
            _ => {}
        }
    }

    // ---- mouse ---------------------------------------------------------

    fn in_rect(r: Rect, x: u16, y: u16) -> bool {
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    }

    fn list_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let r = self.hits.list;
        if !Self::in_rect(r, x, y) {
            return None;
        }
        let i = self.scroll + (y - r.y) as usize;
        (i < self.view.len()).then_some(i)
    }

    fn parent_row_at(&self, x: u16, y: u16) -> Option<&Entry> {
        let r = self.hits.parent;
        if !Self::in_rect(r, x, y) {
            return None;
        }
        self.parent_view.get(self.hits.parent_scroll + (y - r.y) as usize).map(|&i| &self.parent_entries[i])
    }

    /// The directory a drop at (x,y) would land in, if any.
    fn drop_target(&self, x: u16, y: u16, sources: &[PathBuf]) -> Option<PathBuf> {
        if let Some((_, p)) = self.hits.places.iter().find(|(r, _)| Self::in_rect(*r, x, y)) {
            return Some(p.clone());
        }
        if let Some((_, p)) = self.hits.crumbs.iter().find(|(r, _)| Self::in_rect(*r, x, y)) {
            return Some(p.clone());
        }
        if let Some(i) = self.list_row_at(x, y) {
            let e = &self.entries[self.view[i]];
            if e.is_dir && !sources.contains(&e.path) {
                return Some(e.path.clone());
            }
        }
        if let Some(e) = self.parent_row_at(x, y) {
            if e.is_dir {
                return Some(e.path.clone());
            }
        }
        None
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        if let Mode::Menu(menu) = &mut self.mode {
            let hit = self.hits.menu.iter().find(|(r, _)| Self::in_rect(*r, x, y)).map(|(_, i)| *i);
            match m.kind {
                MouseEventKind::Moved => {
                    if let Some(i) = hit {
                        menu.sel = i;
                    }
                }
                MouseEventKind::Down(_) => {
                    let act = hit.map(|i| menu.items[i].1);
                    self.mode = Mode::Normal;
                    if let Some(a) = act {
                        self.apply_menu(a);
                    }
                }
                _ => {}
            }
            return;
        }
        if !matches!(self.mode, Mode::Normal) {
            return; // a prompt owns the keyboard; ignore the mouse until it ends
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::Down(MouseButton::Left) => self.left_down(x, y, m.modifiers),
            MouseEventKind::Down(MouseButton::Right) => self.right_down(x, y),
            MouseEventKind::Down(MouseButton::Middle) => {
                if let Some(i) = self.list_row_at(x, y) {
                    self.select(i);
                    self.open_current();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(d) = &mut self.drag {
                    // Leaving the pressed row (or a real sideways pull) starts a
                    // drag; a wobble inside the row is still just a click.
                    let moved = d.start.1 != y || d.start.0.abs_diff(x) > 2;
                    if moved {
                        d.active = true;
                    }
                }
                if self.drag.as_ref().is_some_and(|d| d.active) {
                    let src = self.drag.as_ref().unwrap().paths.clone();
                    let h = self.drop_target(x, y, &src);
                    self.drag.as_mut().unwrap().hover = h;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(d) = self.drag.take() {
                    if d.active {
                        if let Some(dest) = d.hover {
                            let copy = m.modifiers.contains(KeyModifiers::CONTROL);
                            let (ok, err) = self.transfer(&d.paths, &dest, copy);
                            self.finish_op(if copy { "copied" } else { "moved" }, ok, err);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn scroll_by(&mut self, d: isize) {
        let max = self.view.len().saturating_sub(self.hits.height.max(1));
        self.scroll = (self.scroll as isize + d).clamp(0, max as isize) as usize;
        self.follow = false;
    }

    fn left_down(&mut self, x: u16, y: u16, mods: KeyModifiers) {
        self.msg.clear();
        if let Some((_, p)) = self.hits.places.iter().find(|(r, _)| Self::in_rect(*r, x, y)).cloned() {
            return self.go(&p, None);
        }
        if let Some((_, p)) = self.hits.crumbs.iter().find(|(r, _)| Self::in_rect(*r, x, y)).cloned() {
            return self.go(&p, None);
        }
        if let Some(e) = self.parent_row_at(x, y).cloned() {
            if e.is_dir {
                return self.go(&e.path, None);
            }
            // A file in the parent column: go there and land on it.
            if let Some(par) = e.path.parent() {
                return self.go(par, Some(e.name));
            }
        }
        let Some(i) = self.list_row_at(x, y) else {
            self.marks.clear();
            return;
        };
        let path = self.entries[self.view[i]].path.clone();
        if mods.contains(KeyModifiers::CONTROL) {
            if !self.marks.remove(&path) {
                self.marks.insert(path);
            }
            self.select(i);
            return;
        }
        let dbl = self.last_click.is_some_and(|(t, r)| r == i && t.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((Instant::now(), i));
        self.select(i);
        if dbl {
            self.last_click = None;
            return self.open_current();
        }
        // Dragging a marked row carries the whole selection; an unmarked row
        // carries just itself.
        let paths = if self.marks.contains(&path) { self.marks.iter().cloned().collect() } else { vec![path] };
        self.drag = Some(Drag { paths, start: (x, y), active: false, hover: None });
    }

    fn right_down(&mut self, x: u16, y: u16) {
        let on_row = self.list_row_at(x, y);
        if let Some(i) = on_row {
            let p = self.entries[self.view[i]].path.clone();
            if !self.marks.contains(&p) {
                self.marks.clear();
            }
            self.select(i);
        } else if !Self::in_rect(self.hits.list, x, y) {
            return;
        }
        let mut items = Vec::new();
        if on_row.is_some() {
            items.extend([
                ("Open", MenuAction::Open),
                ("Cut", MenuAction::Cut),
                ("Copy", MenuAction::Copy),
            ]);
        }
        if self.clip.is_some() {
            items.push(("Paste", MenuAction::Paste));
        }
        if on_row.is_some() {
            items.extend([("Rename", MenuAction::Rename), ("Move to Trash", MenuAction::Trash)]);
        }
        items.extend([("New Folder", MenuAction::NewDir), ("Toggle Hidden", MenuAction::ToggleHidden)]);
        self.mode = Mode::Menu(Menu { pos: (x, y), items, sel: 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventKind;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(c, KeyModifiers::NONE, KeyEventKind::Press)
    }

    fn settle(app: &mut App) {
        for _ in 0..200 {
            if app.poll_loader() && !app.loading {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("loader never finished");
    }

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lifefiles-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "hello").unwrap();
        std::fs::write(d.join(".hidden"), "").unwrap();
        d
    }

    #[test]
    fn hides_dotfiles_until_toggled() {
        let d = tree("hid");
        let mut app = App::new(d.clone());
        settle(&mut app);
        assert_eq!(app.view.len(), 2);
        app.on_key(key(KeyCode::Char('.')));
        assert_eq!(app.view.len(), 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn enter_and_backspace_navigate_and_restore_cursor() {
        let d = tree("nav");
        let mut app = App::new(d.clone());
        settle(&mut app);
        assert_eq!(app.current().unwrap().name, "sub"); // dirs first
        app.on_key(key(KeyCode::Enter));
        settle(&mut app);
        assert_eq!(app.cwd, d.canonicalize().unwrap().join("sub"));
        app.on_key(key(KeyCode::Backspace));
        settle(&mut app);
        assert_eq!(app.current().unwrap().name, "sub");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn filter_narrows_and_esc_clears() {
        let d = tree("flt");
        let mut app = App::new(d.clone());
        settle(&mut app);
        app.on_key(key(KeyCode::Char('/')));
        app.on_key(key(KeyCode::Char('a')));
        assert_eq!(app.view.len(), 1);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.view.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rename_and_cut_paste_move_files() {
        let d = tree("ops");
        let mut app = App::new(d.clone());
        settle(&mut app);
        app.select(1); // a.txt
        app.on_key(key(KeyCode::F(2)));
        for _ in 0..5 {
            app.on_key(key(KeyCode::Backspace));
        }
        for c in "b.md".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Enter));
        settle(&mut app);
        assert!(d.join("b.md").exists() && !d.join("a.txt").exists());
        assert_eq!(app.current().unwrap().name, "b.md");

        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Enter));
        settle(&mut app);
        app.on_key(key(KeyCode::Char('p')));
        settle(&mut app);
        assert!(d.join("sub/b.md").exists() && !d.join("b.md").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dropping_a_folder_on_itself_is_refused() {
        let d = tree("drop");
        let mut app = App::new(d.clone());
        settle(&mut app);
        let sub = d.join("sub");
        let (ok, err) = app.transfer(&[sub.clone()], &sub, false);
        assert_eq!(ok, 0);
        assert!(err.is_some());
        assert!(sub.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
