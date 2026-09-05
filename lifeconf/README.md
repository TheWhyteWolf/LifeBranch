# lifeconf

The rice's settings/theming front-end. One canonical file —
`~/.config/lifeconf/theme.toml` — is the single source of truth for the whole
olive look and the knobs previously buried across a dozen config files.
`lifeconf --apply` regenerates every consumer's *own* native config from it, so
nothing else has to learn a new format.

Fourth-and-a-bit member of the `life*` family (companion to
[lifenote](../lifenote), [lifelock](../lifelock), [lifegreet](../lifegreet),
[lifewall](../lifewall)) — it's the one that themes the other four.

## What it drives

| Consumer | File it regenerates |
|---|---|
| waybar | `~/.config/waybar/style.css` (`@define-color` roles + optional tray tint) |
| kitty | `~/.config/kitty/olive.conf` (core surfaces + 16 ANSI colours) |
| fuzzel | `~/.config/fuzzel/fuzzel.ini` (`[colors]`) |
| lifenote | `~/.config/lifenote/config` |
| swaylock | `~/.config/swaylock/config` |
| lifelock | `~/.config/lifelock/config` |
| lifegreet | staged for `/etc/lifegreet/config` (privileged — printed, not auto-applied) |
| niri | `~/.config/niri/config.kdl` — only the four `// LIFECONF:BEGIN … END` fenced regions (animations slowdown, cursor, idle timeouts, lifewall flags) |

On an installed rice those `~/.config` paths are symlinks back into the repo,
so regenerating updates the tracked source too.

## Build

```sh
cargo build --release        # -> target/release/lifeconf
```

Dependencies: `serde` + `toml` (model), `ratatui` (TUI), and
`smithay-client-toolkit` + `fontdue` (GUI — the same software-rendering stack as
lifelock/lifegreet/lifewall).

## Use

```sh
lifeconf                     # interactive UI: TUI in a terminal, GUI otherwise
lifeconf --gui               # force the GUI
lifeconf --apply             # regenerate every config from theme.toml (headless)
lifeconf --preset moss       # switch palette preset (olive, slate, moss, vivid-*, rainbow-*, ...), save + apply
lifeconf --print             # print the resolved theme.toml
lifeconf --no-restart        # with --apply: live-apply colours but don't respawn
                             #   the wallpaper/idle (colours only)
lifeconf --help
```

Both UIs share one editing model: pick a category (Presets, Palette, Lifewall,
Lifenote, Lifelock, Lifegreet, Idle, Animations, Cursor, Font), edit a field,
and every change **previews live** — waybar/kitty/lifenote re-theme instantly
as you move. `s` saves + applies (respawning the wallpaper/idle if those
changed), `q` saves and quits, `Esc` cancels a field edit, `Ctrl+C` quits
without saving (reverting the live preview). The GUI adds mouse
click-to-select, a **Save** button (lit while there are unsaved changes), and
mirrors the TUI 1:1.

Saving pops a **polkit password prompt** when (and only when) the login screen
needs root: it installs the staged palette to `/etc/lifegreet/config` and
refreshes `/usr/local/bin/lifegreet` if the repo build is newer than the
installed one. The lock screen (`lifelock`) needs no privilege — it reads
`~/.config/lifelock/config` fresh on every lock.

`theme.toml` is created from the **olive** preset on first run (so a fresh
checkout is a no-visual-diff refactor of the hand-tuned files). Edit it by hand
or switch presets; presets are compiled into the binary (`presets/*.toml`,
listed in `presets::NAMES`) so "reset to preset" can't be corrupted by an edit.
Beyond the original three rices (olive/slate/moss, each hand-tuned) there are
four fun groups to cycle through on the **Presets** category: `vivid-*`
(Super Saturated — pure R/G/B/CMY accents), `rainbow-*` (a spread of bright
hues), `pastel-*` (soft, low-saturation accents on a dark ground), and
`light-*` (inverted light-mode takes on olive/slate). The groups' 16-colour
terminal palettes are all machine-derived from their seven roles via
`ansi16::derive_ansi16`, same as `active_preset = "custom"` uses for hand-edited
palettes.

### Generated files & git

The plain-file outputs (`waybar/style.css`, `fuzzel/fuzzel.ini`,
`kitty/olive.conf`, `lifenote/config`, `swaylock/config`) are gitignored build
artifacts: each machine materialises them from its own `theme.toml`, so the
laptop can run `vivid-green` while the desktop stays `olive` without either
dirtying the repo. **After pulling the commit that untracked them, run
`lifeconf --apply` once** — git deletes the old tracked copies from the working
tree on that pull, and --apply regenerates them (through the `~/.config`
symlinks) from your local theme. install.sh already does this on fresh setups.
The niri configs are the exception: they stay tracked, and lifeconf edits only
their fenced `LIFECONF:BEGIN/END` regions.

### The greeter is special

`lifegreet` runs as the `greeter` system user with its own `$HOME`, so its
config lives at `/etc/lifegreet/config`. lifeconf never escalates: it stages the
file under `~/.cache/lifeconf/` and prints the one `sudo install …` line to run
by hand.

## Milestones

- **M1** (done) — canonical `theme.toml`, three presets, all generators, `--apply`.
- **M2** (done) — live-apply for waybar (SIGUSR2) + kitty (remote control).
- **M3** (done) — live-apply for lifenote (`ctl reload` over its DBus control
  interface).
- **M4** (done) — lifewall/swayidle respawn, cursor via gsettings.
- **M5** (done) — the keyboard-driven TUI (kitten-themes feel: preview on selection).
- **M6** (done) — the GUI (xdg-shell + fontdue software rendering).

### Deferred: `life-common`

The plan's M6 also proposed extracting a shared `life-common` crate (the fontdue
`Atlas` + hex helpers duplicated across lifenote/lifelock/lifewall/lifeconf) and
introducing a Cargo workspace. That's pure cleanup with no user-facing change and
real risk to four already-working crates, so lifeconf carries its own small copy
of the render primitives (`src/gui/render.rs`) for now. Extract the shared crate
when there's a reason to touch all of them at once.
