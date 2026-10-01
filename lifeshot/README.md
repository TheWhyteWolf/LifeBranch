# lifeshot

A flameshot-style screenshot tool in the rice's own look: freeze the screen,
drag to select an area, mark it up, copy or save. No Qt — it is the same
sctk + fontdue software-rendered stack as lifelock and lifenote, with a
text-button toolbar instead of icons. `Shift+Print` (niri's own `Print`,
`Ctrl+Print` and `Alt+Print` are untouched).

## Use

1. **Select** — drag an area. Drag inside it to move, drag a handle or edge to
   resize. `Ctrl+A` selects the whole screen; arrow keys nudge (Shift = 10 px).
2. **Mark up** — pick a tool from the bar or press its key; press the same key
   again to go back to moving/resizing.

   | key | tool |
   |---|---|
   | `p` | pen (freehand) |
   | `l` | line |
   | `a` | arrow |
   | `r` | box (`f` toggles hollow/solid) |
   | `o` | oval (`f` toggles hollow/solid) |
   | `m` | marker (translucent highlighter) |
   | `t` | text (type, Enter to place) |
   | `b` | blur (pixelate an area — for hiding secrets) |
   | `n` | numbered marker, counting up (1, 2, 3…) |

   `1`–`6` pick the colour, `[` `]` or the mouse wheel change the width, and
   holding Shift while dragging snaps lines to 45° and boxes to squares.
3. **Finish** — `Enter` or `Ctrl+C` copies the PNG to the clipboard;
   `Ctrl+S` saves it to `~/Pictures/Screenshots/` (the same file name niri
   uses) and says where. `Ctrl+Z` / `Ctrl+Shift+Z` undo and redo. `Esc` or a
   right-click backs out one level (tool, then selection, then quit).

```
lifeshot                  the focused output
lifeshot --full           start with the whole screen selected
lifeshot --output NAME    a specific output
lifeshot --save-dir DIR   where Ctrl+S writes
```

## How it works

The screen is captured first (`wlr-screencopy`), *then* a full-output overlay
is created, so the overlay can never end up in the picture. The overlay shows
the frozen capture, dimmed, with the selection at full brightness. The capture
is in the output's physical pixels and is scaled to the output's logical size
by a viewport, so any scale (including fractional 1.5×) lines up exactly and
the saved PNG is at full resolution.

Annotations are drawn by one rasteriser (`shapes.rs`) used for both the live
overlay and the export, so what you see is what you save. The editor
(`editor.rs`) and the whole interactive session (`session.rs`) contain no
Wayland, so the behaviour is unit-tested and rendered to a file for visual
checks. Colours come from `~/.config/lifeshot/theme`, written by
`lifeconf --apply`.

Debug aids (no real screen content involved): `--demo OUT.png` renders one of
every shape, `--render-ui OUT.png` paints the overlay with a synthetic
desktop, `--capture-test` captures and prints only size/format/brightness, and
`--selftest` / `--selftest-actions` run the real overlay and (optionally) the
Save/Copy actions headlessly — meant for a nested compositor.

## Known limitations

- One output at a time (the focused one); a selection can't span monitors.
- Outputs with a rotated transform aren't handled (the capture would be
  sideways).
- Annotations can't be selected and edited after placing — undo and redo them.
- Text is single-line.
- The capture takes a moment on very large outputs; the screen freezes for it.
- It needs `wl-copy` (wl-clipboard) to copy, `notify-send` for the confirmation,
  and the compositor must offer `wlr-screencopy`, `wlr-layer-shell` and
  `wp-viewporter` (niri does).
