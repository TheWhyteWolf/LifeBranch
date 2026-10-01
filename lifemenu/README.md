# lifemenu

The launcher and the menu for LifeBranch, replacing fuzzel with the family's
own pure-text look.

```sh
lifemenu                          # pick an application and launch it
printf 'a\nb\n' | lifemenu --dmenu  # pick a line, print it
```

The flags are fuzzel's, in the subset the scripts use and with the same
meanings and exit codes (a cancel exits 1). Every script switched by changing
one word, and each still falls back to fuzzel when lifemenu isn't installed:

| flag | |
|---|---|
| `-d`, `--dmenu` | choices from stdin; print the pick |
| `--index` | print the pick's 0-based index instead |
| `--password` | hide the input entirely (only the cursor shows); prints what was typed |
| `--only-match` | Enter does nothing unless a choice matches |
| `-p`, `--prompt TEXT` | the prompt |
| `--prompt-only TEXT` | a bare input box, no list |
| `--mesg TEXT` | a line of text under the prompt |
| `-l`, `--lines N` / `-w`, `--width N` | list rows / width in characters |
| `--config PATH` | a config file |
| `--log-level` | accepted and ignored |

## Use

Type to filter (fuzzy: the letters in order, anywhere, with prefixes and word
starts ranked first). Up/Down or Ctrl+P/N (Tab/Shift+Tab too) move and wrap,
Page Up/Down page, Enter picks, Esc or Ctrl+C cancels. Ctrl+W or
Ctrl+Backspace deletes a word, Ctrl+U the line. The mouse works as well: hover
selects, a click picks, the wheel scrolls.

The launcher lists the Desktop Entry files in the XDG data dirs (minus hidden,
NoDisplay and OnlyShowIn/NotShowIn exclusions for `$XDG_CURRENT_DESKTOP`),
matching on the name, GenericName and Keywords. Apps you launch often move to
the top (counts in `~/.cache/lifemenu/history`). `Terminal=true` apps open in
the configured terminal (kitty).

## Config

`~/.config/lifemenu/config`, or, until lifeconf writes that, the
`~/.config/fuzzel/fuzzel.ini` it already generates. The reader takes fuzzel's
ini format: `font=Family:size=11`, `width`, `lines`, the pads, `terminal`, and
the `[colors]` keys (`background`, `text`, `prompt`, `input`, `match`,
`selection`, `selection-text`, `selection-match`, `border`, all `RRGGBBAA`),
plus `[border] width`. Anything else in the file is ignored, so the same file
can serve fuzzel too.

## Cost

About 10 MB resident while open, nothing when closed. It draws only when
something changes. Fonts come from fontconfig through
[`lifefont`](../lifefont/), which reads glyphs lazily.
