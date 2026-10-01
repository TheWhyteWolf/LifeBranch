// SPDX-License-Identifier: GPL-3.0-or-later
// What every system panel shares: the command runner, the row model and the
// small helpers. Kept apart from mod.rs (the panel registry) so lifepanel can
// compile the network, bluetooth, sound and power panels as its backends
// without pulling in the rest of lifeconf.

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
