# Screenshots

Used by the top-level README.

| File | Shows |
|---|---|
| `desktop.png` | tiled kitty and the lifeconf TUI, wallpaper behind |
| `floating.png` | floating windows over the tiling layout |
| `yazi.png` | yazi beside lifeconf, tiled |
| `lifelock.png` | the lock screen |
| `lifegreet.png` | the login screen |

`Print` is region capture, `Ctrl+Print` whole screen, `Alt+Print` window; they
land in `~/Pictures/Screenshots`.

The lock and login screens cannot be captured that way: the lock surface blocks
screencopy and the greeter runs as another user on vt1. Both binaries render
themselves to a PPM instead, in a debug build (the mode is
`#[cfg(debug_assertions)]`, so a release build does not carry it):

```sh
cargo build --manifest-path lifelock/Cargo.toml
cargo build --manifest-path lifegreet/Cargo.toml

./lifelock/target/debug/lifelock --render-ppm /tmp/lifelock.ppm 6 3
./lifegreet/target/debug/lifegreet --render-ppm /tmp/lifegreet.ppm --phase user --username user

magick /tmp/lifelock.ppm docs/lifelock.png
magick /tmp/lifegreet.ppm docs/lifegreet.png
```

`6 3` is advance-seconds and flare-count; six seconds gives the board real
structure rather than initial soup. lifegreet takes
`--phase user|grow|password|validating|wrong`. Pass `--username`, or it renders
the author's.

These use each program's built-in defaults rather than `~/.config/lifeconf/theme.toml`,
so they always come out in the shipped olive palette.
