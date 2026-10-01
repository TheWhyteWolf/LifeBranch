// SPDX-License-Identifier: GPL-3.0-or-later
// System panels: settings that live in the running system (audio, later
// display/network/…) rather than in theme.toml. Unlike the theme categories
// they apply the instant they change — there is nothing to Save — and their
// current values are read from the system, not from a file.
//
// Each panel is a thin wrapper over an existing service (wpctl, nmcli, …) run
// through a `Runner`, so the parsing and the decisions about what to run are
// testable with a fake and a missing tool degrades to an "unavailable" row
// instead of an error.

pub mod about;
pub mod apps;
pub mod autostart;
pub mod bluetooth;
pub mod datetime;
pub mod desktop;
pub mod display;
pub mod kdl;
pub mod keyboard;
pub mod niri_input;
pub mod power;
pub mod net;
pub mod sound;
pub mod touchpad;

/// Run `program args…`, returning stdout on success or a one-line reason.
pub type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Result<String, String>;

pub fn run_real(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("failed");
        return Err(format!("{program}: {first}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// How one row of a panel behaves; the model maps these onto its `Kind`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RowKind {
    /// Integer with a step (percent, minutes…).
    Int(u32),
    Bool,
    /// One of `Row::choices`, cycled with +/-.
    Choice,
    /// Free text typed in (Enter to commit).
    Text,
    /// Read-only.
    Info,
    /// Does something when activated (Enter/click/+); `Row::value` says what.
    Action,
}

/// One current value, as read from the system.
#[derive(Clone, Default, Debug)]
pub struct Row {
    pub value: String,
    /// (label, id) pairs for a Choice row.
    pub choices: Vec<(String, String)>,
}

/// A request to change a row.
pub enum Change {
    Text(String),
    Toggle,
    Step(i32),
}

/// Resolve a Choice row change to the (label, id) it selects: +/- steps through
/// the list (wrapping) from the current value; text must name a choice.
pub fn pick<'a>(row: &'a Row, ch: &Change, what: &str) -> Result<&'a (String, String), String> {
    if row.choices.is_empty() {
        return Err(format!("no {what} choices"));
    }
    let n = row.choices.len() as i32;
    let cur = row.choices.iter().position(|(l, _)| *l == row.value).unwrap_or(0) as i32;
    let i = match ch {
        Change::Step(d) => (cur + d).rem_euclid(n),
        Change::Toggle => (cur + 1).rem_euclid(n),
        Change::Text(t) => row
            .choices
            .iter()
            .position(|(l, id)| l.eq_ignore_ascii_case(t.trim()) || id.eq_ignore_ascii_case(t.trim()))
            .ok_or_else(|| format!("no {what} named {:?}", t.trim()))? as i32,
    };
    Ok(&row.choices[i as usize])
}

/// Which item a panel's picker row is on. View state rather than system state,
/// but a reload after every change must not forget it.
pub struct Sel(std::sync::Mutex<String>);

impl Sel {
    pub const fn new() -> Sel {
        Sel(std::sync::Mutex::new(String::new()))
    }
    pub fn get(&self) -> String {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn set(&self, v: &str) {
        if let Ok(mut g) = self.0.lock() {
            *g = v.to_string();
        }
    }
}

/// Split one line of `nmcli -t` output on unescaped ':' (`\:` and `\\` are
/// literal), as nmcli's terse mode escapes them inside values.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for c in line.chars() {
        match (esc, c) {
            (true, c) => {
                out.last_mut().unwrap().push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, ':') => out.push(String::new()),
            (false, c) => out.last_mut().unwrap().push(c),
        }
    }
    out
}

pub const PANELS: &[&str] = &["Display", "Network", "Bluetooth", "Sound", "Keyboard", "Touchpad", "Power", "Date & Time", "Apps", "Autostart", "About"];

pub fn is_panel(name: &str) -> bool {
    PANELS.contains(&name)
}

pub fn labels(panel: &str) -> &'static [&'static str] {
    match panel {
        "Display" => display::LABELS,
        "Network" => net::LABELS,
        "Bluetooth" => bluetooth::LABELS,
        "Sound" => sound::LABELS,
        "Keyboard" => keyboard::LABELS,
        "Touchpad" => touchpad::LABELS,
        "Date & Time" => datetime::LABELS,
        "About" => about::LABELS,
        "Power" => power::LABELS,
        "Apps" => apps::LABELS,
        "Autostart" => autostart::LABELS,
        _ => &[],
    }
}

pub fn row_kind(panel: &str, field: usize) -> RowKind {
    match panel {
        "Display" => display::kind(field),
        "Network" => net::kind(field),
        "Bluetooth" => bluetooth::kind(field),
        "Sound" => sound::kind(field),
        "Keyboard" => keyboard::kind(field),
        "Touchpad" => touchpad::kind(field),
        "Date & Time" => datetime::kind(field),
        "About" => about::kind(field),
        "Power" => power::kind(field),
        "Apps" => apps::kind(field),
        "Autostart" => autostart::kind(field),
        _ => RowKind::Info,
    }
}

pub fn load(panel: &str, run: Runner) -> Vec<Row> {
    match panel {
        "Display" => display::load(run),
        "Network" => net::load(run),
        "Bluetooth" => bluetooth::load(run),
        "Sound" => sound::load(run),
        "Keyboard" => keyboard::load(run),
        "Touchpad" => touchpad::load(run),
        "Date & Time" => datetime::load(run),
        "About" => about::load(run),
        "Power" => power::load(run),
        "Apps" => apps::load(run),
        "Autostart" => autostart::load(run),
        _ => Vec::new(),
    }
}

/// Commands that would undo `ch` on `panel`, computed before it is applied; only
/// panels whose changes can lock you out (Display) provide them, and the front
/// ends then ask for confirmation and revert on silence.
pub fn undo_for(panel: &str, field: usize, rows: &[Row], ch: &Change) -> Option<Vec<Vec<String>>> {
    match panel {
        "Display" => display::undo(field, rows, ch),
        _ => None,
    }
}

/// Run undo commands (all `niri msg …`), newest first; the first error is
/// reported but the rest still run, since a half-reverted screen is worse.
pub fn run_undo(undo: &[Vec<String>], run: Runner) -> Result<(), String> {
    let mut first_err = None;
    for args in undo.iter().rev() {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(e) = run("niri", &a) {
            first_err.get_or_insert(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// Apply a change. Returns a status line on success; the caller reloads the rows.
pub fn apply(panel: &str, field: usize, rows: &[Row], ch: Change, run: Runner) -> Result<String, String> {
    match panel {
        "Display" => display::apply(field, rows, ch, run),
        "Network" => net::apply(field, rows, ch, run),
        "Bluetooth" => bluetooth::apply(field, rows, ch, run),
        "Sound" => sound::apply(field, rows, ch, run),
        "Keyboard" => keyboard::apply(field, rows, ch, run),
        "Touchpad" => touchpad::apply(field, rows, ch, run),
        "Date & Time" => datetime::apply(field, rows, ch, run),
        "About" => about::apply(field, rows, ch, run),
        "Power" => power::apply(field, rows, ch, run),
        "Apps" => apps::apply(field, rows, ch, run),
        "Autostart" => autostart::apply(field, rows, ch, run),
        _ => Err("read-only".into()),
    }
}
