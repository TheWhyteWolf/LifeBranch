# lifenote

A freedesktop notification daemon for niri, fourth of the life* family
(lifelock / lifegreet / lifewall). Popups are pure text: real box-drawing
frames rasterized as glyphs on a semitransparent wlr-layer-shell surface,
muted olive like the rest of the rice.

```
┌─ firefox ─────────────────────┐      ╔═ battery ═════════════════════╗
│ Download finished             │      ║ 5% remaining — plug in now    ║
│                               │      ╚═══════════════════════════════╝
│ rice-wallpaper.png saved to   │      (critical: double frame, rust red,
│ ~/Downloads                   │       stays until clicked)
└───────────────────────────────┘
```

## Why not mako

Mako (and every other daemon) draws pixel borders around wrapped text — it
cannot put `│` glyphs alongside a body it wraps itself. A frame that hugs the
text requires owning the renderer, so lifenote reuses the family's stack:
smithay-client-toolkit + calloop, fontdue char-cell atlas, software-composited
shm canvas — ARGB8888 with premultiplied alpha instead of the siblings' opaque
XRGB, so the panel is genuinely translucent.

## Behaviour

- `org.freedesktop.Notifications` on the session bus: Notify (with
  replaces_id), CloseNotification, GetCapabilities (`body` only — markup is
  stripped, icons and actions are ignored: pure-text rice), NotificationClosed
  reasons 1/2/3. Fails loudly at startup if the name is taken (a running mako).
- Frame style per urgency: `border-style` for low/normal,
  `critical-border-style` for critical (default: single olive / double rust
  red, critical persists until clicked).
- One top-right surface holds the stack (newest on top, `max-visible`, the
  rest queue). Created when a popup arrives, destroyed when the last one goes —
  zero idle footprint, no idle timers, no animation.
- Expiry follows visibility: a popup's timeout only runs while it is on
  screen, so a burst beyond `max-visible` (or a displaced popup) waits
  off-screen without burning its clock and gets its full time when it
  surfaces.
- Click a popup to dismiss it.
- `lifenote ctl dnd toggle|on|off|get` — do-not-disturb; swallowed
  notifications still land in history. `ctl history` (last 50, in memory,
  markup-stripped and flattened to one line per entry), `ctl unseen` (count
  of entries that arrived unseen — expired, DND-swallowed, or sender-closed —
  since history was last viewed; feeds the waybar `#` badge via RTMIN+9
  pings), `ctl dismiss-all`. Wired to Mod+N, the waybar DND label, and the
  waybar `#` history button (notif-menu.sh).

## Config

`~/.config/lifenote/config` (symlinked from this repo by install.sh),
mako-style `key=value`; every key is also a `--flag`. See `./config` for the
annotated olive defaults. Notables: `border-style single|rounded|heavy|double|
ascii`, `background-alpha 0.85`, `layer top|overlay` (overlay punches through
fullscreen apps), `anchor`, `max-width` (columns).

## Known limitations

- Column math is `chars().count()` — CJK/emoji double-width glyphs will
  misalign the right border of their line.
- No overflow indicator when more than `max-visible` popups queue.
- History is in-memory only (parity with `makoctl history` volatility).
- Notifications fired before the daemon owns the bus name are dropped (same
  as mako under spawn-at-startup; no DBus activation file on purpose — it
  would race a still-running mako during a fallback).

## Debug harnesses (debug builds only)

- `lifenote --render-ppm [out.ppm]` — rasterize a sample stack (all five
  styles + critical) over a checkerboard, no Wayland/DBus needed.
- `lifenote --selftest` — drive the live UI with staged fake notifications,
  no DBus (mako can keep running).
