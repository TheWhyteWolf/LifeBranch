# niri — 2019 MacBook Pro (T2 / Arch)

The same minimal muted-olive niri setup as the root config, tweaked for the
2019 MacBook Pro running Arch on the T2 chip. It **reuses** the shared theme
files (fuzzel, lifenote, waybar `style.css`, swaylock, qt6ct, xdg portals,
kitty `rice.conf`, and the `clip-menu` / `power-menu` / `lifebg-toggle` /
`notif-menu` / `float-snap` / `scratch-term` scripts) and only overrides the
laptop-specific pieces. Notifications use **lifenote** (desktop parity), with
mako kept installed as the fallback (`pkill lifenote && mako`).

## What's different from the desktop config

| Area | Desktop | MacBook |
|---|---|---|
| Display | BenQ 1080p on `HDMI-A-1`, scale 1 | internal Retina on `eDP-1`, **scale 2** |
| Pointer | mouse | **trackpad**: tap, natural-scroll, clickfinger, dwt |
| Keyboard | full-size UK, numpad | internal **US** ANSI, **no numpad** |
| Volume | numpad `KP_*` | **XF86 media keys** (+ `Mod+F10/F11/F12` fallback) |
| Float zones | numpad `Mod+KP_*` direct-zone binds | arrows/HJKL smart snap only |
| Brightness | — | **XF86 brightness keys** → `bright-osd.sh` (+ `Mod+F1/F2`) |
| Waybar | net/audio/cpu/mem | adds **battery + backlight** |
| Sleep | **never** — sleep targets masked | **allowed** — lid close suspends (s2idle + `macbook/system/` fix), locking first; hibernate stays masked |
| Power menu | no Suspend (sleep masked) | **Suspend entry shows** (same script, auto-detects) |
| Packages | base stack | base stack **+ `brightnessctl`**, plus the same tray/laptop bits (nm-applet, blueman, udiskie, wlsunset, wf-recorder) |
| Kbd backlight | — | **XF86KbdBrightness keys** (+ `Mod+F5/F6`) → `:white:kbd_backlight` |

Key bindings, workspaces, columns, screenshots, lock/idle (`Mod+Alt+Escape`,
10 min lock via **lifelock** — the same Game of Life cube as the desktop, with
the `Mod+Shift+Alt+Escape` swaylock recovery bind — 15 min screens off), the
power menu (`Mod+Shift+E`), floating snap (`Mod+Alt+arrows/HJKL`, `Mod+Alt+C`
/ `Mod+Alt+R`, `Mod+Shift+Ctrl` nudges), the `Mod+Grave` dropdown terminal,
the wob volume/brightness OSD, do-not-disturb (`Mod+N`), the
wallpaper pause/reset binds (`Mod+Shift+G` / `Mod+Ctrl+G`) and the olive
palette are identical to the root config. T2 note: suspend is s2idle and
Wi-Fi/BT can need a reload on resume.

The login screen is shared too — greetd + [lifegreet](../lifegreet/README.md)
(the Game of Life cube greeter, tuigreet as fallback). Run
`bash ~/niri/greeter-install.sh`: it builds lifegreet, installs the greetd
config plus its service drop-in, and swaps whatever display manager is
current. Nothing in it is desktop-specific — the drop-in raises
`LimitMEMLOCK` for the mlockall'd greeter (frame buffers at Retina 2880×1800
need even more locked memory than the desktop's 1080p).

## Install

```sh
bash ~/niri/macbook/install.sh
```

Installs the shared stack (`kitty fuzzel waybar mako swaybg xwayland-satellite
wl-clipboard cliphist wev adw-gtk-theme phinger-cursors wob jq swaylock swayidle
ttf-sharetech-mono-nerd ttf-cousine-nerd xdg-desktop-portal-gnome qt6ct
qt6-wayland qt5-wayland`) plus `brightnessctl`, backs up
existing configs to `*.bak`, symlinks the laptop niri + waybar configs and the
shared theme files (swaylock, wob, qt6ct, portals, kitty `rice.conf` +
`olive.conf`), installs the `clip-menu` / `power-menu` / `lifebg-toggle` /
`vol-osd` / `bright-osd` / `dnd-toggle` / `float-snap` / `scratch-term`
scripts, builds **lifelock** (+ its
`/etc/pam.d/lifelock` service), sets the GTK dark theme + phinger cursor, then
runs `niri validate`. Unlike the desktop install it does **not** mask the
systemd sleep targets — the laptop is allowed to suspend.

It also symlinks `environment.d/50-niri-platform.conf` into
`~/.config/environment.d/` and `brave/brave-flags.conf` into `~/.config/`,
which pin Qt/Electron/Brave to native Wayland so drag-and-drop works — see
**Native Wayland** in the [root README](../README.md#rice). Two consequences
worth knowing on this machine: the change only takes effect after a re-login,
and file drags from Dolphin into **Bitwig Studio** (X11-only) no longer work,
in exchange for drags into Brave / vesktop / VS Code.

## System plumbing (`macbook/system/`)

`sudo bash macbook/system/apply-system.sh` (offered by install.sh, idempotent)
applies every T2 system-level fix in one shot; the files it installs live in
[`system/`](system/) so the whole setup is reproducible:

- **Suspend (fixed):** s2idle + `pm_async=off` (kernel cmdline), suspend
  unmasked, `t2-suspend-fix.service` cycling Wi-Fi + Touch Bar modules around
  sleep (`t2-sleep.sh` — never `apple_bce`, the sound card pins it), Touch Bar
  USB autosuspend disabled. **Hibernate stays masked on purpose**: the T2's BCE
  cuts Touch Bar power irrecoverably (deep/S3 has been broken since the macOS
  Sonoma firmware). Lid close = lock + suspend; critical battery = clean
  poweroff.
- **Audio:** `pipewire-alsa` (raw-ALSA apps route through PipeWire instead of
  fighting the 48 kHz-only T2 device) + `pipewire-jack` (replaces jack2), and a
  48 kHz graph pin (`pipewire-t2-rate.conf`).
- **Bluetooth:** `t2-bt-firmware.sh` clears the rfkill soft-block and powers
  the controller on. The UART BCM4364B3 runs ROM firmware — the kernel's
  "Patch file not found: brcm/BCM.hcd" line is expected and harmless (only the
  PCIe-BT models 15,4/16,3/9,1 have extractable firmware).
- **Network:** the T2's internal USB-NCM "ethernet" is renamed `t2_ncm` and
  excluded from NetworkManager auto-activation (it DHCP-fails forever).
- **Boot:** plymouth removed (it segfaulted on every boot and the splash was
  invisible at `loglevel=3` anyway). Nothing T2-specific goes in mkinitcpio's
  `MODULES=()`: the drivers were renamed `apple-bce` -> `t2bce_*` upstream and
  exist only for `linux-t2`, while `mkinitcpio -P` builds a preset for the
  stock `linux` kernel from the same config. There is no encrypt hook, so
  nothing needs them before the root filesystem mounts.
- **Lock screen:** faillock relaxed to `deny=10` / `unlock_time=60` so a few
  typos at the locker can't force a power-cycle.

## T2-specific notes (system-level, outside niri)

These are handled by the OS, not this config — listed so the laptop is fully usable:

- **Brightness keys** need `brightnessctl`'s udev rules, or add yourself to the
  `video` group: `sudo usermod -aG video "$USER"` then re-login.
- **Audio** on T2 needs the `apple-t2-audio-config` / `apple-bcm-firmware` bits
  (via the [t2linux](https://wiki.t2linux.org/) AUR packages) before `wpctl`
  volume control does anything.
- **Wi-Fi** needs T2 firmware extracted from macOS (`firmware.tar` per the
  t2linux wiki — `apple-bcm-firmware` packages it). **Bluetooth** needs no
  firmware on this model (UART BCM4364B3, ROM firmware); if it looks dead,
  it's rfkill — `system/t2-bt-firmware.sh` handles that.
- **Touch Bar** (13"/15" MBP 2019): run [`tiny-dfr`](https://github.com/AsahiLinux/tiny-dfr)
  to get a static F-key / Esc row. niri doesn't manage it.
- **Keyboard quirks** (`/etc/modprobe.d/hid_apple.conf`): to make the function
  row default to F-keys, add `options hid_apple fnmode=1`. To swap
  Command/Option, add `swap_opt_cmd=1`. Reboot or rebuild the initramfs after.
  (The `iso_layout` swap only affects ISO boards — N/A for this US ANSI keyboard.)
- **Function-row media keys**: with the stock `fnmode`, F1/F2 and F10–F12 emit
  XF86 brightness/volume events (what the binds expect). If you set `fnmode=1`,
  the `Mod+F1/F2/F10/F11/F12` fallbacks in the config cover you.

## Adjusting the display

`scale 2` suits every 2019 Retina panel (2560×1600 and 2880×1800). If you dock
an external monitor or auto-detect picks the wrong mode, uncomment/edit the
`mode` line in `niri/config.kdl`, or run `niri msg outputs` to see the exact
name and modes and add another `output "..." { … }` block.
