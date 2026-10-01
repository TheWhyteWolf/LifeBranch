# lifeportal

The file-chooser backend for xdg-desktop-portal. When an app opens an Open
or Save dialog through the portal (Brave, Element, Flatpaks, anything GTK
or Qt set to use portals), the dialog is lifefiles: the same browser as
`Mod+E`, in a floating kitty window, in `--pick` mode.

| dialog | in lifefiles |
|---|---|
| Open | Enter (or double-click) on a file chooses it |
| Open, several | Space marks; Ctrl+S takes the marked files |
| Choose folder | browse into it; Ctrl+S takes the folder you're in |
| Save | Ctrl+S asks the name (pre-filled), and confirms before replacing a file |
| Save several | choose a folder; each file is saved into it under its own name |
| Cancel | Esc (once nothing is marked or filtered) or q |

The app's suggested folder and file name are honoured. Folders that don't
exist fall back to home. Answers are `file://` URIs, percent-encoded.
SaveFiles' names stay names: `../../etc/passwd` becomes `passwd` in the
folder you picked.

## How it is wired

- `lifebranch.portal` names the backend. The installer links it into
  `~/.local/share/xdg-desktop-portal/portals/`, which xdg-desktop-portal 1.22
  searches first.
- A D-Bus service file in `~/.local/share/dbus-1/services/` starts lifeportal
  on the first dialog, so nothing runs until then.
- `xdg/portals.conf` routes FileChooser to `lifebranch;gtk`, so gtk's dialog
  is used if lifeportal isn't installed.
- When the app gives up on a dialog, the portal calls `Request.Close`, and
  the picker window closes.

## Testing

`cargo test` covers option parsing (folder, name, multiple, directory,
SaveFiles names), the picker's command line, and the output and URI
encoding. lifefiles' own tests cover each pick mode.

`LIFEPORTAL_PICKER=/path/to/script` swaps the kitty+lifefiles window for a
script that gets lifefiles' arguments. With it, the whole chain was checked
on a private session bus (`dbus-run-session`):
- a client's OpenFile and SaveFile go through the real xdg-desktop-portal
- `portals.conf` routes them to lifeportal, which D-Bus activation starts
- the script answers, and the client gets response 0 with the URIs
- Close, and a cancelled pick, give response 1
