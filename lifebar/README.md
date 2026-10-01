# lifebar

The LifeBranch status bar. It replaces waybar with the same modules,
wording and clicks, drawn in pure text by the same renderer as the rest of the
desktop:

```
 1  2   kitty — ~/git        Thu 01 Oct  19:40     # 28  IDLE  NET 88%  BRT 9%  VOL 80%  BAT 80%  CPU 14%  MEM 52%  vesktop
```

Measured beside waybar on the same session: about 9 MB RSS, of which 2.2 MB
is private (the rest is the shared font and libraries). waybar was 74 MB.

## Modules

| | shows | left click | other |
|---|---|---|---|
| workspaces | niri's, per output; active underlined, urgent red | focus it | |
| title | the focused window on that output | | |
| clock | `Thu 01 Oct  19:40` | calendar (wheel or arrows change month) | |
| `#` | notifications you haven't seen (`# 3`) | notif-menu.sh | |
| `DND` | only while do-not-disturb is on | toggle it | |
| `IDLE`/`WAKE` | idle inhibitor | toggle: WAKE holds off lock and screen-off | |
| `NET` | wifi strength, `NET` wired, `NET --` offline | quick settings (lifepanel) | right: Network settings |
| `BRT` | panel backlight (laptops) | quick settings | wheel: brightness |
| `VOL` / `MUTE` | default output | mute | right: Sound settings; wheel: volume |
| `BAT` / `CHG` / `AC` | battery (laptops) | quick settings | |
| `CPU`, `MEM` | load, memory used | htop | |
| tray | each app's title as text | activate | right: its menu in lifemenu; middle: secondary action |

A reading that isn't available (no battery, no backlight, lifenote not
running) is left out, so one bar config fits the desktop and the laptop.

## Where things come from

- **Workspaces and titles.** niri's IPC socket, as an event stream. The bar
  keeps its own model from the events and never polls niri.
- **Battery and backlight.** /sys, re-read when the kernel announces a change
  (a uevent on netlink), plus every 30 s as a backstop. Battery percent is
  `charge_now / charge_full`, as waybar and upower count it. The kernel's
  `capacity` reads low on worn batteries, because some drivers report it
  against design capacity.
- **CPU, memory, network.** /proc and /sys every 5 s (waybar's interval).
  The network is whatever carries the default route. Wifi strength is
  `2 × (dBm + 100)`, as waybar and NetworkManager show it.
- **Volume.** `wpctl get-volume` every 5 s, and immediately when
  vol-osd.sh, lifepanel or a click changes it (they send `SIGRTMIN+10`).
- **Notifications and DND.** `lifenote ctl`, when lifenote or dnd-toggle.sh
  sends `SIGRTMIN+9` or `SIGRTMIN+8` (the signals waybar's modules used).
- **Tray.** lifebar is a StatusNotifierItem host. When no other watcher owns
  `org.kde.StatusNotifierWatcher` (once waybar is gone, nothing does),
  lifebar is the watcher too. Menus are read over com.canonical.dbusmenu and
  shown in lifemenu, with submenus flattened to `Parent › Child`.

The bar redraws an output only when what it shows changes.

## Config

`~/.config/lifebar/config` is written by lifeconf from theme.toml: the
palette roles, the font and its size (CSS pixels, as waybar's stylesheet
used). `SIGUSR2` reloads it in place, and lifeconf sends that on every
change.

## Running

`systemd/lifebar.service` is a supervised user unit, like waybar's was:
`Restart=always`, with logs in `journalctl --user -u lifebar`. The installers
enable it in place of waybar.service once lifebar builds. To switch a
running session:

```sh
systemctl --user stop waybar && systemctl --user start lifebar
```

## Testing

`cargo test` covers:
- the niri event model (per-output titles, activation, closing, urgency)
- every module's wording, colour and click, against the waybar config
- the layout (right group flush, clock centred, the title giving way)
- the /proc and /sys parsers, including this MacBook's battery
- tray labels and dbusmenu flattening
- the config reader
