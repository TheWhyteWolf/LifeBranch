# lifewall

Conway's Game of Life as a smooth terminal wallpaper. The simulation ticks at
a relaxed pace while rendering interpolates every cell's colour at 30 fps:
births fade in, the newborn flash melts into the mature tone, deaths dissolve
back into the background. Cells are drawn as random printable ASCII by
default; `--char` takes a whole string, and each cell picks one glyph from it
(stable until the cell dies and is reborn).

A single ~400 KB binary; the only dependency is `libc`.

## Build

```sh
cargo build --release        # -> target/release/lifewall
```

## Run

As a wallpaper it rides inside [kitty](https://sw.kovidgoyal.net/kitty/)'s
panel kitten on the desktop background layer:

```sh
kitten panel --edge=background --config NONE -o font_size=8 \
  -o background='#121412' lifewall
```

Smaller `font_size` = finer cells. This needs kitty ≥ 0.42 and either a
Wayland compositor with layer-shell support (niri, sway, Hyprland, river, …)
or macOS. It also runs in any plain terminal — nice for previewing.

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
```

Pick `--char` glyphs that render at one terminal column each, or they'll
smear into their neighbor — plain ASCII is safe, as are half-width katakana
(U+FF66-FF9D, e.g. `ｱｶﾀﾅ`); full-width kana/kanji are double-width in most
terminal fonts and will misalign the grid.

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
