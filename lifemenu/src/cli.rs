// SPDX-License-Identifier: GPL-3.0-or-later
// Flags and config. The flags are the fuzzel subset LifeBranch's scripts use,
// with fuzzel's meanings, so a script switches by changing one word. The
// config reader takes fuzzel's ini too (section headers are skipped, keys are
// fuzzel's), which is what lifeconf generates and pinentry-fuzzel passes.

/// RGBA, straight (not premultiplied) alpha.
pub type Rgba = [u8; 4];

#[derive(Debug, Clone, PartialEq)]
pub struct Cfg {
    // Mode
    pub dmenu: bool,
    pub index: bool,
    pub password: bool,
    pub only_match: bool,
    pub prompt: String,
    /// --prompt-only: no list at all, just the input line.
    pub prompt_only: bool,
    pub mesg: Option<String>,
    // Look
    pub font: String,
    pub font_size: f32,
    pub width: usize, // characters
    pub lines: usize, // list rows
    pub pad_h: usize,
    pub pad_v: usize,
    pub inner: usize,
    pub border_width: usize,
    pub terminal: String,
    pub background: Rgba,
    pub text: Rgba,
    pub prompt_color: Rgba,
    pub input: Rgba,
    pub match_color: Rgba,
    pub selection: Rgba,
    pub selection_text: Rgba,
    pub selection_match: Rgba,
    pub border: Rgba,
}

impl Default for Cfg {
    fn default() -> Self {
        // The olive palette, matching what lifeconf generates for fuzzel.
        Cfg {
            dmenu: false,
            index: false,
            password: false,
            only_match: false,
            prompt: "> ".into(),
            prompt_only: false,
            mesg: None,
            font: "ShureTechMono Nerd Font".into(),
            font_size: 11.0,
            width: 40,
            lines: 12,
            pad_h: 16,
            pad_v: 12,
            inner: 6,
            border_width: 1,
            terminal: "kitty".into(),
            background: [0x12, 0x14, 0x12, 0xf2],
            text: [0x7b, 0x8c, 0x5a, 0xff],
            prompt_color: [0xa4, 0xc9, 0x4b, 0xff],
            input: [0x7b, 0x8c, 0x5a, 0xff],
            match_color: [0xa4, 0xc9, 0x4b, 0xff],
            selection: [0x17, 0x1a, 0x14, 0xff],
            selection_text: [0xa4, 0xc9, 0x4b, 0xff],
            selection_match: [0xa4, 0xc9, 0x4b, 0xff],
            border: [0x39, 0x41, 0x2b, 0xff],
        }
    }
}

/// `RRGGBB` or `RRGGBBAA`, `#` optional.
pub fn parse_rgba(s: &str) -> Option<Rgba> {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    match s.len() {
        6 => Some([(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xff]),
        8 => Some([(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8]),
        _ => None,
    }
}

impl Cfg {
    /// One config `key=value`. Unknown keys are ignored: a fuzzel ini carries
    /// plenty lifemenu has no use for (icons, layer, keybindings...).
    pub fn set(&mut self, key: &str, val: &str) {
        let val = val.trim();
        let col = |cur: Rgba| parse_rgba(val).unwrap_or(cur);
        let num = |cur: usize| val.parse().unwrap_or(cur);
        match key.trim() {
            // fuzzel: font=Family:size=11 (fontconfig pattern)
            "font" => {
                let mut parts = val.split(':');
                if let Some(f) = parts.next().filter(|f| !f.is_empty()) {
                    self.font = f.to_string();
                }
                for p in parts {
                    if let Some(sz) = p.strip_prefix("size=").and_then(|s| s.parse().ok()) {
                        self.font_size = sz;
                    }
                }
            }
            "font-size" => self.font_size = val.parse().unwrap_or(self.font_size),
            "prompt" => self.prompt = unquote(val),
            "width" => self.width = num(self.width),
            "lines" => self.lines = num(self.lines),
            "horizontal-pad" => self.pad_h = num(self.pad_h),
            "vertical-pad" => self.pad_v = num(self.pad_v),
            "inner-pad" => self.inner = num(self.inner),
            "terminal" => self.terminal = val.to_string(),
            "background" => self.background = col(self.background),
            "text" => self.text = col(self.text),
            "prompt-color" => self.prompt_color = col(self.prompt_color),
            "input" => self.input = col(self.input),
            "match" => self.match_color = col(self.match_color),
            "selection" => self.selection = col(self.selection),
            "selection-text" => self.selection_text = col(self.selection_text),
            "selection-match" => self.selection_match = col(self.selection_match),
            "border" => self.border = col(self.border),
            _ => {}
        }
    }

    /// Read a config file. In fuzzel's ini `prompt=` and the border colour
    /// share names with other sections' keys, so the section decides:
    /// [colors] prompt is a colour, [main] prompt is text, [border] width is
    /// the border's.
    pub fn load(&mut self, body: &str) {
        let mut section = String::from("main");
        for line in body.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
                continue;
            }
            if let Some(s) = l.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = s.trim().to_string();
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let k = k.trim();
            match (section.as_str(), k) {
                ("colors", "prompt") => self.set("prompt-color", v),
                ("border", "width") => self.border_width = v.trim().parse().unwrap_or(self.border_width),
                ("border", _) => {}
                _ => self.set(k, v),
            }
        }
    }

    pub fn clamp(&mut self) {
        self.font_size = self.font_size.clamp(4.0, 72.0);
        self.width = self.width.clamp(8, 400);
        self.lines = self.lines.min(100);
        self.border_width = self.border_width.min(20);
    }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v).to_string()
}

pub const USAGE: &str = "lifemenu — launcher and dmenu for LifeBranch (fuzzel-compatible flags)

  lifemenu                     launch an application
  ... | lifemenu --dmenu       pick a line from stdin, print it

  -d, --dmenu                  read choices from stdin, print the pick
      --index                  print the pick's 0-based index instead
      --password               mask the input; prints what was typed
      --only-match             Enter does nothing unless a choice matches
  -p, --prompt TEXT            prompt (default \"> \")
      --prompt-only TEXT       prompt with no list (an input box)
      --mesg TEXT              a line of text under the prompt
  -l, --lines N                list rows
  -w, --width N                width in characters
      --config PATH            config file (fuzzel ini or lifemenu config)
      --log-level LEVEL        accepted for fuzzel compatibility; ignored

Cancel (Esc) exits 1, as fuzzel does. Config: ~/.config/lifemenu/config,
else ~/.config/fuzzel/fuzzel.ini.
";

pub enum Parsed {
    Run(Cfg),
    Help,
    Error(String),
}

/// Parse argv (without argv[0]). The config file is applied first so flags
/// override it, whatever order they came in.
pub fn parse(args: &[String]) -> Parsed {
    // Split `--flag=value` into two words up front.
    let mut words: Vec<String> = Vec::new();
    for a in args {
        match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => {
                words.push(f.into());
                words.push(v.into());
            }
            _ => words.push(a.clone()),
        }
    }
    let mut cfg = Cfg::default();
    let config = words
        .iter()
        .position(|w| w == "--config")
        .and_then(|i| words.get(i + 1).cloned())
        .or_else(default_config);
    if let Some(body) = config.and_then(|p| std::fs::read_to_string(p).ok()) {
        cfg.load(&body);
    }

    let mut it = words.into_iter();
    while let Some(w) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        let r: Result<(), String> = (|| {
            match w.as_str() {
                "-d" | "--dmenu" => cfg.dmenu = true,
                "--index" => cfg.index = true,
                "--password" => cfg.password = true,
                "--only-match" => cfg.only_match = true,
                "-p" | "--prompt" => cfg.prompt = val("--prompt")?,
                "--prompt-only" => {
                    cfg.prompt = val("--prompt-only")?;
                    cfg.prompt_only = true;
                }
                "--mesg" => cfg.mesg = Some(val("--mesg")?),
                "-l" | "--lines" => {
                    cfg.lines = val("--lines")?.parse().map_err(|_| "--lines takes a number".to_string())?
                }
                "-w" | "--width" => {
                    cfg.width = val("--width")?.parse().map_err(|_| "--width takes a number".to_string())?
                }
                "--config" | "--log-level" => {
                    val(&w)?;
                }
                "-h" | "--help" => return Err("help".into()),
                other => return Err(format!("unknown flag {other}")),
            }
            Ok(())
        })();
        match r {
            Ok(()) => {}
            Err(e) if e == "help" => return Parsed::Help,
            Err(e) => return Parsed::Error(e),
        }
    }
    cfg.clamp();
    Parsed::Run(cfg)
}

fn default_config() -> Option<String> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))?;
    [format!("{base}/lifemenu/config"), format!("{base}/fuzzel/fuzzel.ini")]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(a: &[&str]) -> Cfg {
        // Point config lookup at nothing so a developer's own file can't leak in.
        let mut v: Vec<String> = vec!["--config".into(), "/nonexistent".into()];
        v.extend(a.iter().map(|s| s.to_string()));
        match parse(&v) {
            Parsed::Run(c) => c,
            _ => panic!("parse failed for {a:?}"),
        }
    }

    #[test]
    fn script_flags_parse_like_fuzzel() {
        let c = run(&["--dmenu", "--index", "--prompt", "wifi> ", "--width", "60"]);
        assert!(c.dmenu && c.index && !c.password);
        assert_eq!((c.prompt.as_str(), c.width), ("wifi> ", 60));
        let c = run(&["--dmenu", "--password", "--lines", "0", "--prompt=pw> "]);
        assert!(c.password && c.lines == 0 && c.prompt == "pw> ");
        let c = run(&["--log-level=none", "--dmenu", "--prompt-only", "PIN ", "--mesg", "card 1"]);
        assert!(c.prompt_only && c.mesg.as_deref() == Some("card 1"));
        assert!(matches!(parse(&["--bogus".into()]), Parsed::Error(_)));
        assert!(matches!(parse(&["--lines".into()]), Parsed::Error(_)));
    }

    #[test]
    fn reads_the_fuzzel_ini_lifeconf_generates() {
        let ini = "# generated\n[main]\nfont=ShureTechMono Nerd Font:size=13\nprompt=>\nicons-enabled=no\n\
                   terminal=kitty\nwidth=44\nlines=9\n\n[colors]\nbackground=121412f2\nprompt=a4c94bff\n\
                   selection=171a14ff\n\n[border]\nwidth=2\nradius=0\n";
        let mut c = Cfg::default();
        c.load(ini);
        assert_eq!((c.font.as_str(), c.font_size), ("ShureTechMono Nerd Font", 13.0));
        assert_eq!((c.width, c.lines, c.border_width), (44, 9, 2));
        assert_eq!(c.prompt, ">", "[main] prompt is text");
        assert_eq!(c.prompt_color, [0xa4, 0xc9, 0x4b, 0xff], "[colors] prompt is a colour");
        assert_eq!(c.background, [0x12, 0x14, 0x12, 0xf2]);
    }

    #[test]
    fn colours_with_and_without_alpha() {
        assert_eq!(parse_rgba("#a4c94b"), Some([0xa4, 0xc9, 0x4b, 0xff]));
        assert_eq!(parse_rgba("121412f2"), Some([0x12, 0x14, 0x12, 0xf2]));
        assert_eq!(parse_rgba("nope"), None);
    }
}
