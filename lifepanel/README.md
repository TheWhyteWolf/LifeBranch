# lifepanel

Quick settings for LifeBranch, in one pure-text popup in the top-right corner:

```
wifi       [on]   BELL498 2.4                  >
bluetooth  [on]   Echo Pop-4WF                 >
──────────────────────────────────────────────
volume     ████░░░░░░░░░░░░░░░░░░░  20%   mute
  to       Echo Pop-4WF                        >
mic        ███████████████████████ 100%   mute
brightness █░░░░░░░░░░░░░░░░░░░░░░   9%
──────────────────────────────────────────────
power      performance  balanced  power-saver
battery    96% discharging, 1.9 hours left
do not disturb    [off]
──────────────────────────────────────────────
all settings…
```

It replaces three tray applets: nm-applet, blueman-applet and udiskie (about
225 MB between them on the author's machine). lifepanel is about 12 MB while
open and nothing when closed. `lifepanel --watch`, the drive automounter, is
a 3 MB process sleeping on a socket.

Open it with `Mod+A` or by clicking NET in the bar. Running it again closes it,
and so do Esc and clicking a window.

## Using it

| | keyboard | mouse |
|---|---|---|
| move | Up/Down, Tab | hover |
| wifi/bluetooth on/off | Left/Right | click `[on]` |
| list networks/devices | Enter | click the row |
| join/connect/disconnect | Enter on it | click it |
| volume, mic, brightness | Left/Right (5% steps) | click the bar, wheel |
| mute | Enter, or `m` | click `mute` |
| output device | Enter on "to", then on the device | click |
| power profile | Left/Right/Enter | click the name |
| drive | Enter mounts or opens; Delete or `e` ejects | click `mount`/`open`/`eject` |

A secured network you haven't joined before opens a password field under it.
The password goes to nmcli on stdin (never argv) and is wiped from memory
after.

A slow action (a wifi join, pairing) shows its progress at the bottom. If you
close the panel first, the action still finishes and the result arrives as a
notification.

Drives are listed only while something removable is plugged in. Eject unmounts
the drive's filesystems and powers it off, so it is safe to pull.

## Where things come from

- **Network, bluetooth, sound, power.** These are lifeconf's own system panels
  (`lifeconf/src/sys/{net,bluetooth,sound,power}.rs`), compiled in through
  `#[path]`, not copied. They run nmcli, bluetoothctl, wpctl, brightnessctl,
  powerprofilesctl and upower, the same commands the Settings app runs.
- **Drives.** lsblk lists them; udisksctl mounts, unmounts and powers off.
  Mounts land under `/run/media/$USER`, with the usual polkit rules (lifeauth
  asks when a rule wants a password).
- **Do not disturb.** `lifenote ctl dnd`.
- **Look and font.** lifemenu's config: `~/.config/lifemenu/config`, else the
  `fuzzel.ini` lifeconf generates. The renderer is lifemenu's.

Loads and actions run on worker threads, so the panel never freezes. Toggles
and sliders change on screen at once, and the command catches up. While the
panel is open it refreshes every 4 s: power and drives always, wifi and
bluetooth only while their lists are open.

## --watch

```sh
lifepanel --watch                 # mount what's plugged in, with a notification
lifepanel --watch --no-automount  # only say it arrived
```

It listens on udev's netlink broadcast, so it wakes only on device events and
never polls. Events arrive after udev has probed the filesystem. Anything
already plugged in at login is mounted too, as udiskie did. niri starts it
(`journalctl -t lifepanel`), and falls back to udiskie when lifepanel isn't
built.

## Not yet

- **Bluetooth pairing that needs a PIN or a confirmation.** No agent is
  registered yet, so devices that need one (keyboards, phones) fail with
  bluetoothctl's message. "Just works" devices (headphones, speakers, most
  mice) pair fine. blueman-manager is still installed for the others.
- **Enterprise (802.1X) and VPN secrets.** nm-applet used to answer these
  prompts. Use `nmcli --ask` or nm-connection-editor for now.
- **Encrypted (LUKS) drives.** These aren't listed, because unlocking one
  needs a passphrase.

## Testing

`cargo test` covers:
- each row's text and click targets at the panel's width
- every key, and what it runs
- selection surviving a reload
- the lsblk parser and the eject order
- the udev message parser

The backends' own parsers are tested in lifeconf.
