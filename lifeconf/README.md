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
`smithay-client-toolkit` + `lifefont` (GUI — the same software-rendering stack as
lifelock/lifegreet/lifewall).

## Use

```sh
lifeconf                     # interactive UI: TUI in a terminal, GUI otherwise
lifeconf --gui               # force the GUI
lifeconf --gui --panel sound # open straight onto a category/panel (Mod+S opens the GUI)
lifeconf --apply             # regenerate every config from theme.toml (headless)
lifeconf --preset moss       # switch palette preset (olive, slate, moss, vivid-*, rainbow-*, ...), save + apply
lifeconf --print             # print the resolved theme.toml
lifeconf --no-restart        # with --apply: live-apply colours but don't respawn
                             #   the wallpaper/idle (colours only)
lifeconf --help
```

Both UIs share one editing model: pick a category (Presets, Palette, Lifewall,
Notifications, Lifelock, Lifegreet, Idle, Animations, Cursor, Font, Accessibility), edit a field,
and every change **previews live** — waybar/kitty/lifenote re-theme instantly
as you move. `s` saves + applies (respawning the wallpaper/idle if those
changed), `q` saves and quits, `Esc` cancels a field edit, `Ctrl+C` quits
without saving (reverting the live preview). The GUI adds mouse
click-to-select, a **Save** button (lit while there are unsaved changes), and
mirrors the TUI 1:1.

**Accessibility** scales the UI text the theme generates (the bar, lifemenu
and everything that reads its config, lifenote, and GTK apps through
`text-scaling-factor`; kitty keeps its own Ctrl+Shift+= zoom), and its reduce
motion turns niri's animations off and slows the wallpaper to 2 fps. Cursor
size is shown there too and edits the same setting as Cursor > size.

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

### System panels

Below the theme categories the sidebar has a **System** group. These panels
read and change the running system directly, so there is nothing to Save: a
change applies the moment you make it, and the values are re-read from the
system after every change and whenever you select the panel. A missing tool
shows "unavailable" rather than an error. Press `/` (or Ctrl+F) in the GUI to
search the sidebar — it matches category names *and* field labels, so "volume"
finds Sound.

| Panel | Backend | Notes |
|---|---|---|
| **Display** | `niri msg` | per output: mode, scale, transform, on/off. Every change asks **Keep this change?** (Enter/y keeps, Esc/n reverts, or click); with no answer in 60 s — or if you close lifeconf — it reverts, so a bad pick can't strand you. "Keep" holds it for the session: these are niri's *temporary* output changes, so a config reload still resets them. Refuses to turn off the last active output. |
| **Network** | `nmcli` | wifi on/off, pick a network, connect/disconnect. A new *secured* network needs a password, which is never put on a command line — use `net-menu.sh` (waybar `NET` click) for that. |
| **Mouse** | niri's config (`mouse` region) | on/off, natural scroll, left-handed, middle-click emulation, acceleration profile and speed, scroll speed. Applied live by niri; settings it doesn't know (scroll-button, ...) are kept. |
| **Night light** | `wlsunset` (via niri's config) | on/off, latitude/longitude (the installer guesses them from your timezone), night and day temperature, or fixed sunset/sunrise times instead of the sun. Rewrites the `nightlight` region (niri-validated) and restarts wlsunset, so changes apply at once; off parks the settings for next time. |
| **Updates** | `checkupdates`, `yay -Qua` | pending updates from the repos and the AUR, from the last check (cached; checking talks to the mirrors, so it runs only when asked). "update now" opens a terminal running `yay -Syu`, where its questions and output belong. Without pacman-contrib it falls back to `pacman -Qu` and says the list is from the last sync. |
| **Users** | `getent`, `id`, `pkexec usermod`/`chpasswd` | your account, groups and whether you're an administrator; set your full name; change your password (asked twice in lifemenu's password box, then to chpasswd on stdin, never argv; lifeauth asks your current password first). |
| **VPN** | `nmcli`, `tailscale`, `protonvpn`, `mullvad` | every tunnel in one list: connect/disconnect; NetworkManager profiles (WireGuard `.conf`, OpenVPN `.ovpn`) can be imported, set to autoconnect, or deleted (type the name to confirm); Tailscale exit node; Proton and Mullvad location. A provider that isn't installed isn't listed. Proton's status comes from NetworkManager; its CLI connects when it runs, else its app opens. |
| **Bluetooth** | `bluetoothctl` | power, pick a device, connect (pairing + trusting new ones first), disconnect, background scan. |
| **Sound** | `wpctl` | default output/input device, volume, mute (right-click waybar `VOL`). |
| **Keyboard** | niri config | layout (typed, checked against `localectl`), variant, options, numlock, key repeat. |
| **Touchpad** | niri config | enabled, tap, natural scroll, disable-while-typing, click/scroll method, accel profile/speed, button map. |
| **Power** | `powerprofilesctl`, `brightnessctl`, `upower` | power profile, brightness (never below 1%), battery state. The lid-close behaviour is shown read-only: it lives in hand-tuned logind drop-ins under `/etc`, which lifeconf deliberately doesn't rewrite. Idle lock/screen-off timeouts are in the Idle category. |
| **Date & Time** | `timedatectl` | timezone (type `Toronto` or `America/Toronto`), automatic time. Asks polkit. |
| **Region** | `localectl`, `locale-gen` | system language (LANG) and formats for dates, numbers, money, measurements and paper (the LC_* set, changed together), from the locales generated here; "add a language" generates a new one (validated against glibc's list, run as root through pkexec). Takes effect at the next login. |
| **Apps** | `xdg-mime` | default browser, file manager, text editor, image/video/audio/pdf/archive handlers, email. Each role switches all its MIME types together. |
| **Autostart** | XDG autostart | enable/disable login entries. Disabling a system entry writes a `Hidden=true` override in `~/.config/autostart`; nothing under `/etc` is touched. Takes effect at next login. |
| **About** | — | device, OS, kernel, CPU, memory, uptime, niri and lifeconf versions. |

Keyboard and Touchpad edit the installer's `// LIFEBRANCH:BEGIN keyboard|touchpad`
regions of `config.kdl` through the same stage → `niri validate` → rename path
as the theme regions, so a bad write leaves the config untouched and niri
hot-reloads a good one. Settings the panel doesn't know are kept; a region it
can't round-trip (properties, `;`-joined nodes) is refused with a message
rather than rewritten. Comments inside those two regions are replaced.

Each panel is a module in `src/sys/` behind a fake-able `Runner`, with its
parsing tested against real command output.

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
- **M6** (done) — the GUI (xdg-shell + software rendering).

### Deferred: `life-common`

The plan's M6 also proposed extracting a shared `life-common` crate (the
`Atlas` + hex helpers duplicated across lifenote/lifelock/lifewall/lifeconf) and
introducing a Cargo workspace. The font layer has since been extracted as
[`lifefont`](../lifefont/), because fontdue's eager parsing cost each process
37 MB; the atlases themselves are still per-crate copies.
