// SPDX-License-Identifier: GPL-3.0-or-later
// Argument parsing, lifewall-style (no dependencies). Adapted from
// ../lifelock/src/cli.rs: greeter has no daemonize/ready-fd/signal-unlock,
// and adds the session picker + username-box knobs.

use crate::sim::{parse_hex, Rgb};

pub struct Cfg {
    // Visual tunables.
    pub cell_w: u32,        // glyph grid cell size in px
    pub cell_h: u32,
    pub cube_height: f64,   // full cube silhouette height in px
    pub pitch: f64,         // target Life-cell pitch on the cube faces, px
    pub fps: f64,
    pub tick: f64,          // seconds per generation (cube faces)
    pub ember_tick: f64,    // seconds per generation (backdrop field)
    pub ember_dim: f64,     // backdrop brightness factor
    pub no_ember: bool,
    pub solid_cells: bool,  // procedural fills instead of ░▒▓█ glyphs
    pub font: String,
    pub clock_px: f32,
    pub caps_px: f32,
    pub user_px: f32,       // username box text size
    pub debug_timing: bool,
    // Session selection.
    pub sessions_dir: String,
    pub cmd: Vec<String>,   // fallback session command if the dir is empty
    // Palette (overridable like lifewall).
    pub mature: Rgb,
    pub newborn: Rgb,
    pub accent: Rgb, // flare / verifying
    pub urgent: Rgb, // backspace flare / wrong flash / CAPS
    pub text: Rgb,   // clock / box / username
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            cell_w: 10,
            cell_h: 20,
            cube_height: 360.0,
            pitch: 20.0,
            fps: 30.0,
            tick: 0.4,
            ember_tick: 0.8,
            ember_dim: 0.14,
            no_ember: false,
            solid_cells: false,
            font: "/usr/share/fonts/TTF/ShureTechMonoNerdFontMono-Regular.ttf".into(),
            clock_px: 64.0,
            caps_px: 26.0,
            user_px: 28.0,
            debug_timing: false,
            sessions_dir: "/usr/share/wayland-sessions".into(),
            cmd: vec!["niri-session".into()],
            mature: Rgb([102.0, 116.0, 76.0]),  // #66744c
            newborn: Rgb([135.0, 165.0, 64.0]), // #87a540
            accent: Rgb([164.0, 201.0, 75.0]),  // #a4c94b
            urgent: Rgb([138.0, 59.0, 46.0]),   // #8a3b2e
            text: Rgb([123.0, 140.0, 90.0]),    // #7b8c5a
        }
    }
}

const USAGE: &str = "lifegreet — greetd Wayland greeter (run under cage)\n\
    Type your username; the Game of Life cube grows out of the box and takes\n\
    your password. F3 cycles sessions, Ctrl+Alt+Del reboots.\n\n\
    --sessions DIR     wayland-sessions dir            (default /usr/share/wayland-sessions)\n\
    --cmd \"CMD ...\"    fallback session command        (default niri-session)\n\
    --cell-w PX        glyph grid cell width          (default 10)\n\
    --cell-h PX        glyph grid cell height         (default 20)\n\
    --cube-height PX   cube silhouette height         (default 360)\n\
    --pitch PX         Life cell pitch on cube faces  (default 20)\n\
    --fps N            render frames per second       (default 30)\n\
    --tick SECS        seconds per face generation    (default 0.4)\n\
    --ember-tick SECS  seconds per backdrop generation (default 0.8)\n\
    --ember-dim F      backdrop brightness 0..1       (default 0.14)\n\
    --no-ember         disable the backdrop field\n\
    --solid-cells      draw cells as fills, not ░▒▓█ glyphs\n\
    --font PATH        monospace TTF for glyphs and text\n\
    --clock-px PX      clock size                     (default 64)\n\
    --user-px PX       username box text size         (default 28)\n\
    --mature HEX       settled cell colour            (default #66744c)\n\
    --newborn HEX      birth flash colour             (default #87a540)\n\
    --accent HEX       keypress/verify colour         (default #a4c94b)\n\
    --urgent HEX       error colour                   (default #8a3b2e)\n\
    --text HEX         clock/box colour               (default #7b8c5a)\n\
    --debug-timing     print per-frame render times\n";

/// Strip a trailing `# comment` — but only a `#` that starts the line or
/// follows whitespace, so hex colour values (`mature=#990808`) survive.
fn strip_comment(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    for (i, _) in raw.match_indices('#') {
        if i == 0 || bytes[i - 1].is_ascii_whitespace() {
            return &raw[..i];
        }
    }
    raw
}

/// Overlay palette colours from /etc/lifegreet/config (written by lifeconf and
/// installed with root — the greeter runs as the `greeter` user with its own
/// $HOME, so the config is system-wide, not under a user home). mako-style
/// `key=value`; only the five colour keys are honoured. CLI flags still win.
fn load_file(cfg: &mut Cfg, path: &str) {
    let Ok(body) = std::fs::read_to_string(path) else {
        return;
    };
    for raw in body.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let set = |slot: &mut Rgb| {
            if let Some(c) = parse_hex(val) {
                *slot = c;
            }
        };
        match key {
            "mature" => set(&mut cfg.mature),
            "newborn" => set(&mut cfg.newborn),
            "accent" => set(&mut cfg.accent),
            "urgent" => set(&mut cfg.urgent),
            "text" => set(&mut cfg.text),
            _ => {}
        }
    }
}

pub fn parse() -> Cfg {
    let mut cfg = Cfg::default();
    // File first so flags override it (lifenote/cli.rs convention).
    load_file(&mut cfg, "/etc/lifegreet/config");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("missing value for {name}");
                std::process::exit(2);
            })
        };
        let mut hex = |name: &str, cur: Rgb| parse_hex(&val(name)).unwrap_or(cur);
        match a.as_str() {
            "--sessions" => cfg.sessions_dir = val("--sessions"),
            "--cmd" => {
                // Whitespace-split, no shell: same discipline as greetd itself.
                cfg.cmd = val("--cmd").split_whitespace().map(str::to_string).collect();
                if cfg.cmd.is_empty() {
                    eprintln!("--cmd needs a non-empty command");
                    std::process::exit(2);
                }
            }
            "--cell-w" => cfg.cell_w = val("--cell-w").parse().unwrap_or(cfg.cell_w),
            "--cell-h" => cfg.cell_h = val("--cell-h").parse().unwrap_or(cfg.cell_h),
            "--cube-height" => {
                cfg.cube_height = val("--cube-height").parse().unwrap_or(cfg.cube_height)
            }
            "--pitch" => cfg.pitch = val("--pitch").parse().unwrap_or(cfg.pitch),
            "--fps" => cfg.fps = val("--fps").parse().unwrap_or(cfg.fps),
            "--tick" => cfg.tick = val("--tick").parse().unwrap_or(cfg.tick),
            "--ember-tick" => cfg.ember_tick = val("--ember-tick").parse().unwrap_or(cfg.ember_tick),
            "--ember-dim" => cfg.ember_dim = val("--ember-dim").parse().unwrap_or(cfg.ember_dim),
            "--no-ember" => cfg.no_ember = true,
            "--solid-cells" => cfg.solid_cells = true,
            "--font" => cfg.font = val("--font"),
            "--clock-px" => cfg.clock_px = val("--clock-px").parse().unwrap_or(cfg.clock_px),
            "--user-px" => cfg.user_px = val("--user-px").parse().unwrap_or(cfg.user_px),
            "--mature" => cfg.mature = hex("--mature", cfg.mature),
            "--newborn" => cfg.newborn = hex("--newborn", cfg.newborn),
            "--accent" => cfg.accent = hex("--accent", cfg.accent),
            "--urgent" => cfg.urgent = hex("--urgent", cfg.urgent),
            "--text" => cfg.text = hex("--text", cfg.text),
            "--debug-timing" => cfg.debug_timing = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown flag {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    cfg.cell_w = cfg.cell_w.clamp(4, 64);
    cfg.cell_h = cfg.cell_h.clamp(8, 128);
    cfg.cube_height = cfg.cube_height.clamp(100.0, 1000.0);
    cfg.pitch = cfg.pitch.clamp(6.0, 100.0);
    cfg.fps = cfg.fps.clamp(1.0, 240.0);
    cfg.tick = cfg.tick.max(0.05);
    cfg.ember_tick = cfg.ember_tick.max(0.05);
    cfg.ember_dim = cfg.ember_dim.clamp(0.0, 1.0);
    cfg.user_px = cfg.user_px.clamp(10.0, 96.0);
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: a '#' that introduces a hex VALUE must not be stripped as a
    // comment — only line-leading or whitespace-preceded '#' starts one.
    #[test]
    fn hash_values_survive_comment_stripping() {
        assert_eq!(strip_comment("accent=#dd1a1a"), "accent=#dd1a1a");
        assert_eq!(strip_comment("key=value   # note"), "key=value   ");
        assert_eq!(strip_comment("# whole-line comment"), "");
    }

    #[test]
    fn file_colours_actually_load() {
        let path = std::env::temp_dir().join("lifegreet-cli-test-config");
        std::fs::write(&path, "accent=#dd1a1a\nurgent=#7e3b2c   # note\n").unwrap();
        let mut cfg = Cfg::default();
        load_file(&mut cfg, path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        assert_eq!(cfg.accent.0, [221.0, 26.0, 26.0]);
        assert_eq!(cfg.urgent.0, [126.0, 59.0, 44.0]);
        assert_eq!(cfg.text.0, [123.0, 140.0, 90.0]); // untouched default
    }
}
