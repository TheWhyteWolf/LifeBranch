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
pub mod mouse;
pub mod keyboard;
pub mod niri_input;
pub mod nightlight;
pub mod power;
pub mod region;
pub mod net;
pub mod sound;
pub mod touchpad;
pub mod vpn;

mod common;
pub use common::*;

pub const PANELS: &[&str] = &["Display", "Network", "VPN", "Bluetooth", "Sound", "Keyboard", "Touchpad", "Mouse", "Power", "Night light", "Date & Time", "Region", "Apps", "Autostart", "About"];

pub fn is_panel(name: &str) -> bool {
    PANELS.contains(&name)
}

pub fn labels(panel: &str) -> &'static [&'static str] {
    match panel {
        "Display" => display::LABELS,
        "Network" => net::LABELS,
        "VPN" => vpn::LABELS,
        "Bluetooth" => bluetooth::LABELS,
        "Sound" => sound::LABELS,
        "Keyboard" => keyboard::LABELS,
        "Touchpad" => touchpad::LABELS,
        "Mouse" => mouse::LABELS,
        "Date & Time" => datetime::LABELS,
        "Region" => region::LABELS,
        "About" => about::LABELS,
        "Power" => power::LABELS,
        "Night light" => nightlight::LABELS,
        "Apps" => apps::LABELS,
        "Autostart" => autostart::LABELS,
        _ => &[],
    }
}

pub fn row_kind(panel: &str, field: usize) -> RowKind {
    match panel {
        "Display" => display::kind(field),
        "Network" => net::kind(field),
        "VPN" => vpn::kind(field),
        "Bluetooth" => bluetooth::kind(field),
        "Sound" => sound::kind(field),
        "Keyboard" => keyboard::kind(field),
        "Touchpad" => touchpad::kind(field),
        "Mouse" => mouse::kind(field),
        "Date & Time" => datetime::kind(field),
        "Region" => region::kind(field),
        "About" => about::kind(field),
        "Power" => power::kind(field),
        "Night light" => nightlight::kind(field),
        "Apps" => apps::kind(field),
        "Autostart" => autostart::kind(field),
        _ => RowKind::Info,
    }
}

pub fn load(panel: &str, run: Runner) -> Vec<Row> {
    match panel {
        "Display" => display::load(run),
        "Network" => net::load(run),
        "VPN" => vpn::load(run),
        "Bluetooth" => bluetooth::load(run),
        "Sound" => sound::load(run),
        "Keyboard" => keyboard::load(run),
        "Touchpad" => touchpad::load(run),
        "Mouse" => mouse::load(run),
        "Date & Time" => datetime::load(run),
        "Region" => region::load(run),
        "About" => about::load(run),
        "Power" => power::load(run),
        "Night light" => nightlight::load(run),
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
        "VPN" => vpn::apply(field, rows, ch, run),
        "Bluetooth" => bluetooth::apply(field, rows, ch, run),
        "Sound" => sound::apply(field, rows, ch, run),
        "Keyboard" => keyboard::apply(field, rows, ch, run),
        "Touchpad" => touchpad::apply(field, rows, ch, run),
        "Mouse" => mouse::apply(field, rows, ch, run),
        "Date & Time" => datetime::apply(field, rows, ch, run),
        "Region" => region::apply(field, rows, ch, run),
        "About" => about::apply(field, rows, ch, run),
        "Power" => power::apply(field, rows, ch, run),
        "Night light" => nightlight::apply(field, rows, ch, run),
        "Apps" => apps::apply(field, rows, ch, run),
        "Autostart" => autostart::apply(field, rows, ch, run),
        _ => Err("read-only".into()),
    }
}
