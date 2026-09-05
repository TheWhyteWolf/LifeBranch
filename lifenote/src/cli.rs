// SPDX-License-Identifier: GPL-3.0-or-later
// Configuration: defaults -> ~/.config/lifenote/config -> CLI flags.
// The file is mako-style `key=value` with `#` comments; every key doubles as
// a --flag, so both paths feed the same `set()` below (lifelock/cli.rs
// pattern, extended with the file loader).

use crate::layout::Style;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub fn to_u8(self) -> [u8; 3] {
        self.0
    }
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorCorner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerChoice {
    /// Above windows, below fullscreen ones (mako's default).
    Top,
    /// Above everything, fullscreen apps included.
    Overlay,
}

#[derive(Clone)]
pub struct Cfg {
    pub border_style: Style,
    pub critical_border_style: Style,
    pub font: String,
    pub font_px: f32,
    pub background: Rgb,
    pub background_alpha: f64,
    pub text: Rgb,
    pub border: Rgb,
    pub title: Rgb,
    pub warn: Rgb,
    pub urgent: Rgb,
    pub default_timeout_ms: u64, // 0 = never
    pub critical_timeout_ms: u64, // 0 = never
    pub max_width: usize,  // content columns inside the frame
    pub max_visible: usize,
    pub max_lines: usize, // body lines per popup
    pub anchor: AnchorCorner,
    pub margin: i32,
    pub layer: LayerChoice,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            border_style: Style::Single,
            critical_border_style: Style::Double,
            font: "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf".into(),
            font_px: 15.0,
            background: Rgb([0x17, 0x1a, 0x14]),
            background_alpha: 0.85,
            text: Rgb([0x7b, 0x8c, 0x5a]),
            border: Rgb([0x39, 0x41, 0x2b]),
            title: Rgb([0xa4, 0xc9, 0x4b]),
            warn: Rgb([0xc7, 0xd1, 0x7a]),
            urgent: Rgb([0x8a, 0x3b, 0x2e]),
            default_timeout_ms: 3000,
            critical_timeout_ms: 0,
            max_width: 40,
            max_visible: 5,
            max_lines: 6,
            anchor: AnchorCorner::TopRight,
            margin: 8,
            layer: LayerChoice::Top,
        }
    }
}

const USAGE: &str = "lifenote — box-drawing-framed notification daemon for niri\n\
    Owns org.freedesktop.Notifications; popups are pure text in ASCII frames.\n\n\
    lifenote [flags]           run the daemon\n\
    lifenote ctl CMD...        talk to the running daemon:\n\
      ctl dnd toggle|on|off    switch do-not-disturb (prints new state)\n\
      ctl dnd get              print `on` or `off`\n\
      ctl dismiss-all          close every visible popup\n\
      ctl history              print swallowed/expired notifications, newest first\n\
      ctl unseen               count arrived-unseen since history was viewed\n\n\
    --config PATH              config file (default ~/.config/lifenote/config)\n\
    Every config key is a flag, e.g. --border-style double --margin 12:\n\
      border-style, critical-border-style (single|rounded|heavy|double|ascii)\n\
      font PATH, font-px N\n\
      background-color/text-color/border-color/title-color/warn-color/\n\
      urgent-color HEX, background-alpha 0..1\n\
      default-timeout MS, critical-timeout MS (0 = never)\n\
      max-width COLS, max-visible N, max-lines N\n\
      anchor top-right|top-left|bottom-right|bottom-left, margin PX\n\
      layer top|overlay (overlay shows above fullscreen apps)\n";

fn parse_style(s: &str) -> Option<Style> {
    match s {
        "single" => Some(Style::Single),
        "rounded" => Some(Style::Rounded),
        "heavy" => Some(Style::Heavy),
        "double" => Some(Style::Double),
        "ascii" => Some(Style::Ascii),
        _ => None,
    }
}

fn parse_anchor(s: &str) -> Option<AnchorCorner> {
    match s {
        "top-right" => Some(AnchorCorner::TopRight),
        "top-left" => Some(AnchorCorner::TopLeft),
        "bottom-right" => Some(AnchorCorner::BottomRight),
        "bottom-left" => Some(AnchorCorner::BottomLeft),
        _ => None,
    }
}

impl Cfg {
    /// Apply one `key=value`. Returns false for an unknown key; a bad value
    /// for a known key keeps the previous setting (lifelock convention).
    fn set(&mut self, key: &str, val: &str) -> bool {
        let hex = |cur: Rgb| parse_hex(val).unwrap_or(cur);
        match key {
            "border-style" => self.border_style = parse_style(val).unwrap_or(self.border_style),
            "critical-border-style" => {
                self.critical_border_style = parse_style(val).unwrap_or(self.critical_border_style)
            }
            "font" => self.font = val.into(),
            "font-px" => self.font_px = val.parse().unwrap_or(self.font_px),
            "background-color" => self.background = hex(self.background),
            "background-alpha" => {
                self.background_alpha = val.parse().unwrap_or(self.background_alpha)
            }
            "text-color" => self.text = hex(self.text),
            "border-color" => self.border = hex(self.border),
            "title-color" => self.title = hex(self.title),
            "warn-color" => self.warn = hex(self.warn),
            "urgent-color" => self.urgent = hex(self.urgent),
            "default-timeout" => {
                self.default_timeout_ms = val.parse().unwrap_or(self.default_timeout_ms)
            }
            "critical-timeout" => {
                self.critical_timeout_ms = val.parse().unwrap_or(self.critical_timeout_ms)
            }
            "max-width" => self.max_width = val.parse().unwrap_or(self.max_width),
            "max-visible" => self.max_visible = val.parse().unwrap_or(self.max_visible),
            "max-lines" => self.max_lines = val.parse().unwrap_or(self.max_lines),
            "anchor" => self.anchor = parse_anchor(val).unwrap_or(self.anchor),
            "margin" => self.margin = val.parse().unwrap_or(self.margin),
            "layer" => {
                self.layer = match val {
                    "top" => LayerChoice::Top,
                    "overlay" => LayerChoice::Overlay,
                    _ => self.layer,
                }
            }
            _ => return false,
        }
        true
    }

    fn clamp(&mut self) {
        self.font_px = self.font_px.clamp(8.0, 48.0);
        self.background_alpha = self.background_alpha.clamp(0.0, 1.0);
        self.max_width = self.max_width.clamp(10, 120);
        self.max_visible = self.max_visible.clamp(1, 20);
        self.max_lines = self.max_lines.clamp(1, 40);
        self.margin = self.margin.clamp(0, 200);
    }
}

/// Cut `raw` at the first `#` that starts the line or follows whitespace —
/// `key=value   # note` loses the note, `key=#a4c94b` keeps its colour.
fn strip_comment(raw: &str) -> &str {
    let cut = raw
        .char_indices()
        .find(|&(i, c)| c == '#' && (i == 0 || raw[..i].ends_with(char::is_whitespace)))
        .map_or(raw.len(), |(i, _)| i);
    &raw[..cut]
}

fn load_file(cfg: &mut Cfg, path: &str, explicit: bool) {
    let Ok(body) = std::fs::read_to_string(path) else {
        if explicit {
            eprintln!("lifenote: cannot read config {path}");
            std::process::exit(2);
        }
        return; // no config file is fine — defaults apply
    };
    for (n, raw) in body.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            eprintln!("lifenote: {path}:{}: expected key=value", n + 1);
            continue;
        };
        if !cfg.set(key.trim(), val.trim()) {
            eprintln!("lifenote: {path}:{}: unknown key {:?}", n + 1, key.trim());
        }
    }
}

pub fn parse() -> Cfg {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // --config first, so flags can override file values regardless of order.
    let mut cfg = Cfg::default();
    let explicit_config = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1).cloned());
    match &explicit_config {
        Some(p) => load_file(&mut cfg, p, true),
        None => {
            let home = std::env::var("HOME").unwrap_or_default();
            load_file(&mut cfg, &format!("{home}/.config/lifenote/config"), false);
        }
    }

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--config" => {
                it.next(); // already handled
            }
            flag if flag.starts_with("--") => {
                let key = &flag[2..];
                let Some(val) = it.next() else {
                    eprintln!("missing value for {flag}\n\n{USAGE}");
                    std::process::exit(2);
                };
                if !cfg.set(key, val) {
                    eprintln!("unknown flag {flag}\n\n{USAGE}");
                    std::process::exit(2);
                }
            }
            other => {
                eprintln!("unexpected argument {other:?}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    cfg.clamp();
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_strip_spares_hex_values() {
        assert_eq!(strip_comment("default-timeout=3000   # ms").trim(), "default-timeout=3000");
        assert_eq!(strip_comment("background-color=#171a14"), "background-color=#171a14");
        assert_eq!(strip_comment("# whole-line comment"), "");
        assert_eq!(strip_comment("key=v # a # b").trim(), "key=v");
    }

    #[test]
    fn hex_colours_parse_from_file_lines() {
        let mut cfg = Cfg::default();
        let line = strip_comment("urgent-color=#ff0000   # rust red").trim();
        let (k, v) = line.split_once('=').unwrap();
        assert!(cfg.set(k.trim(), v.trim()));
        assert_eq!(cfg.urgent, Rgb([0xff, 0x00, 0x00]));
    }

    #[test]
    fn file_colours_actually_load() {
        let path = std::env::temp_dir().join("lifenote-cli-test-config");
        std::fs::write(&path, "text-color=#dd1111\ntitle-color=#dd1a1a   # note\n").unwrap();
        let mut cfg = Cfg::default();
        load_file(&mut cfg, path.to_str().unwrap(), true);
        let _ = std::fs::remove_file(&path);
        assert_eq!(cfg.text, Rgb([0xdd, 0x11, 0x11]));
        assert_eq!(cfg.title, Rgb([0xdd, 0x1a, 0x1a]));
        assert_eq!(cfg.border, Rgb([0x39, 0x41, 0x2b])); // untouched default
    }
}
