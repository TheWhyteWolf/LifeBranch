# lifewall

Conway's Game of Life as a smooth wallpaper. The simulation ticks at
a relaxed pace while rendering interpolates every cell's colour at 30 fps:
births fade in, the newborn flash melts into the mature tone, deaths dissolve
back into the background. Cells are drawn as random printable ASCII by
default; `--char` takes a whole string, and each cell picks one glyph from it
(stable until the cell dies and is reborn).

A single ~1 MB binary. `--layer` draws the board on the Wayland background
layer itself, on the GPU (EGL + GLES2); without it, the board goes to the
terminal as escape codes.

## Build

```sh
cargo build --release        # -> target/release/lifewall
```

## Run

As the wallpaper, on any compositor with wlr-layer-shell (niri, sway,
Hyprland, river, …):

```sh
lifewall --layer --font-family 'ShureTechMono Nerd Font' --font-size 8
```

Smaller `--font-size` = finer cells. One surface per output, each its own
board; outputs that come and go are followed. Fonts are found through
fontconfig, with a per-glyph fallback (kana needs a CJK font such as
noto-fonts-cjk), and are dropped once the glyphs are rasterized.

In any plain terminal it draws there instead, which is handy for previewing.
It also still runs inside `kitten panel --edge=background`, as it used to.

### Cost

Measured on a 3072x1920 panel at 30 fps, same flags, nothing covering it:

| | wallpaper CPU | niri CPU | anonymous memory |
|---|---|---|---|
| inside `kitten panel` | 25% (kitty) + 7% | 10% | 50 MB |
| `--layer` | 5% | 11% | 14 MB (mostly the GPU driver) |

Per frame the CPU only computes each cell's colour and glyph slot and uploads
that grid (4 bytes a cell); a shader draws the pixels. Frames follow the
compositor's frame callbacks, so a covered or powered-off output costs nothing,
and a frame where no cell changed is not drawn at all. CPU-drawn shm buffers
were tried first and cost more than kitty: niri holds an attached shm buffer
until another replaces it, and alternating buffers made it re-upload the whole
frame every time.

## Flags

```
--tick SECS     seconds per generation        (default 0.3)
--fps N         render frames per second      (default 30)
--fade GENS     fade length in generations    (default 3)
--density F     seed fill fraction 0..1       (default 0.14)
--char S        glyph(s) for live cells; 2+ chars picks randomly
                per cell        (default: printable ASCII)
--bg HEX        background colour             (default #121412)
--mature HEX    settled cell colour           (default #66744c)
--newborn HEX   birth flash colour            (default #87a540)
--glider-interval SECS  mean seconds between glider clusters;
                        0 disables                   (default 90)
--layer             draw on the Wayland background layer (GPU)
--font-family NAME  --layer: fontconfig family  (default ShureTechMono Nerd Font)
--font-size PT      --layer: cell size in points, like kitty's font_size (default 8)
```

In a terminal, pick `--char` glyphs that render at one column each, or they'll
smear into their neighbor — plain ASCII is safe, as are half-width katakana
(U+FF66-FF9D, e.g. `ｱｶﾀﾅ`); full-width kana/kanji are double-width in most
terminal fonts and will misalign the grid. `--layer` has no such limit: a
glyph wider than the cell is scaled down to fit it.

The board is a torus (gliders wrap). Every minute or two (randomized, see
`--glider-interval`) a small swarm of 1-3 gliders launches from a random edge
in a random diagonal heading, so the board keeps drifting even once the
ambient soup has settled into still lifes and oscillators — a gentler,
continuous alternative to a full reseed. If it still settles completely (e.g.
the gliders collide and die out) or nearly dies out, that's the backstop: it
crossfades into a fresh soup after ~20 s of no change.

## Sharing / binaries

Rust binaries are per-OS and per-architecture: build once per target
(`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, …)
and hand that file out, or just share this directory — anyone with rust runs
`cargo build --release`. For a maximally portable Linux binary build against
musl: `cargo build --release --target x86_64-unknown-linux-musl`.
