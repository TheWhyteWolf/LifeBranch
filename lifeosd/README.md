# lifeosd

The volume and brightness on-screen display for LifeBranch. It replaces wob
with a labelled pure-text bar at the bottom centre of the screen:

```
volume     ████████████░░░░░░░░░  45%
volume     ░░░░░░░░░░░░░░░░░░░ muted
brightness ██░░░░░░░░░░░░░░░░░░░   9%
```

niri starts it at login (`journalctl -t lifeosd`), and starts wob instead when
lifeosd isn't built. The volume and brightness keys run
`scripts/vol-osd.sh` and `scripts/bright-osd.sh`, which change the level and
then write one line to `$XDG_RUNTIME_DIR/lifeosd.fifo`:

```sh
echo 'volume 45'       > $XDG_RUNTIME_DIR/lifeosd.fifo
echo 'volume 45 muted' > $XDG_RUNTIME_DIR/lifeosd.fifo
echo 'brightness 9'    > $XDG_RUNTIME_DIR/lifeosd.fifo
echo 72                > $XDG_RUNTIME_DIR/lifeosd.fifo   # bare number, as wob took
```

The bar stays for 0.9 s after the last line. A held key queues several lines,
and only the newest is drawn. Clicks pass through the bar, and it never takes
the keyboard.

## Cost

Between flashes it has no surface and no buffers, only a process asleep on the
FIFO. It owns the FIFO, so there is no `tail -f` pipe and no extra process.
RSS was 2.9 MB at start and 5.5 MB after the first flash (the font and a
cached glyph or two).

Colours and font come from lifemenu's config (`~/.config/lifemenu/config`,
else the `fuzzel.ini` lifeconf generates). The renderer is lifemenu's too.

## Testing

`cargo test` covers the message parser (labelled, bare, muted, garbage), the
newest-line rule, and the fixed line width. A live check:

```sh
~/.local/bin/lifeosd &
echo 'volume 60' > $XDG_RUNTIME_DIR/lifeosd.fifo
```
