# lifefiles

A terminal file browser in yazi's layout (places | parent | current | preview)
that you can drive with the mouse as well as the keyboard. Runs in kitty
(`Mod+E`), starts instantly, and takes its colours from the same palette as the
rest of the rice.

## Mouse

| | |
|---|---|
| click | select |
| double-click / middle-click | open |
| Ctrl+click | toggle mark |
| drag a row onto a folder, place or breadcrumb | move (hold Ctrl to copy) |
| right-click | context menu (open, cut, copy, paste, rename, trash, new folder) |
| wheel | scroll |
| click a breadcrumb / place / parent-column row | jump there |

## Keys

| | |
|---|---|
| ↑ ↓ / `j` `k`, PgUp/PgDn, Home/End | move |
| Enter / → / `l` | open (folders enter, files go to `xdg-open`) |
| Backspace / ← / `h` | parent folder |
| Alt+← / Alt+→ | back / forward |
| Space | mark and move down; Ctrl+A marks all; Esc clears |
| Ctrl+C / Ctrl+X / Ctrl+V (or `y` `x` `p`) | copy / cut / paste |
| F2 | rename |
| Del | move to Trash (recoverable) |
| Shift+Del | delete permanently (asks) |
| F7 or Ctrl+N | new folder |
| Ctrl+L | type a path |
| `/` or Ctrl+F | filter the current folder (live) |
| `.` or Ctrl+H | show hidden files |
| F5 or Ctrl+R | reload |
| `q` | quit |

`lifefiles [DIR|FILE]` — a file argument opens its folder with it highlighted.

## Notes

- Trash follows the freedesktop spec (`~/.local/share/Trash/{files,info}`), so
  Dolphin and friends can restore what lifefiles trashes.
- Directories load on a worker thread; a slow or huge folder never stalls a
  redraw.
- Colours are read once at startup from `~/.config/lifefiles/theme`, written by
  `lifeconf --apply`. Relaunch to pick up a new theme.
- install.sh makes `lifefiles.desktop` the `inode/directory` handler only if
  none is set or it is still Dolphin; a file manager you picked is left alone.
  Switch it any time in `lifeconf` > Apps > file manager.

## As a file chooser

`lifefiles --pick` is the Open/Save dialog for every app that uses the
portal ([lifeportal](../lifeportal/README.md) starts it). Browsing works as
usual; only choosing is new:

```
lifefiles --pick [--multiple | --directory | --save NAME] [--title T] --out FILE [DIR]
```

| mode | choose with |
|---|---|
| open | Enter or double-click on a file |
| `--multiple` | Space to mark, Ctrl+S |
| `--directory` | Ctrl+S takes the folder you're in |
| `--save NAME` | Ctrl+S asks the name (NAME pre-filled); replacing a file asks first |

The status line says which. Esc (once nothing is marked or filtered) or q
cancels with exit status 1. The chosen paths go to FILE, separated by NUL
bytes, because a file name can contain a newline.

## Known limitations

- Column math is `chars().count()`: wide (CJK/emoji) names misalign their row.
- No image previews yet; text files and folder listings only.
- Trash is the home trash only; files on other mounts are copied across and
  removed rather than placed in that mount's `.Trash-$uid`.
- Drag and drop works inside lifefiles. Dragging to or from another app is not
  possible in a terminal.
- Copying a large tree blocks the UI until it finishes.
