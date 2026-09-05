#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shortcuts-window.sh: the keyboard cheat sheet LifeBranch opens at login.
#
# The list is PARSED OUT OF THE LIVE niri CONFIG rather than kept in a table
# here, so it cannot drift: rebind something, and the window says so next login.
#
# Turn it off by editing one line in the config (the window itself tells you
# which, and how):
#     // shortcuts-at-startup off
#
# Usage:
#   shortcuts-window.sh              open the window now
#   shortcuts-window.sh --at-startup open it unless the config says off
#   shortcuts-window.sh --print      write the sheet to stdout and exit
#   shortcuts-window.sh --show       render + wait for a key (runs inside kitty)
set -uo pipefail

CONFIG=${NIRI_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/niri/config.kdl}
APP_ID=lifebranch-shortcuts

# The config is normally a symlink into the LifeBranch checkout. Editing through
# the link is fine (nano and every other editor follow it), but say where the
# file really lives so nobody is surprised to find their edit in a git repo.
REAL_CONFIG=$(readlink -f "$CONFIG" 2>/dev/null || printf '%s' "$CONFIG")

if [[ ! -r $CONFIG ]]; then
  echo "shortcuts-window.sh: cannot read $CONFIG" >&2
  exit 1
fi

# --- The toggle -------------------------------------------------------------
# Only the literal token `off` disables it; anything else (including a deleted
# line) leaves the window on, so a typo cannot silently take the cheat sheet
# away from someone still learning the keys.
startup_enabled() {
  local v
  v=$(sed -nE 's@^[[:space:]]*//[[:space:]]*shortcuts-at-startup[[:space:]]+([^[:space:]]+).*@\1@p' \
      "$CONFIG" | head -1)
  [[ ${v,,} != off ]]
}

render() {
  local width=${COLUMNS:-0}
  (( width < 40 )) && width=$( (tput cols) 2>/dev/null || echo 80 )
  (( width < 40 )) && width=80
  (( width > 110 )) && width=110

  CONFIG_PATH="$CONFIG" REAL_PATH="$REAL_CONFIG" WIDTH="$width" \
  awk -v cfg="$CONFIG" '
    BEGIN {
      width   = ENVIRON["WIDTH"] + 0
      cfgpath = ENVIRON["CONFIG_PATH"]
      realp   = ENVIRON["REAL_PATH"]
      nsec = 0
      section = "General"
    }

    # Only look inside binds { ... }
    /^binds[[:space:]]*\{/ { inbinds = 1; next }
    inbinds && /^\}/       { inbinds = 0; next }
    !inbinds { next }

    # `// --- Applications ---` headings become section headings.
    /^[[:space:]]*\/\/[[:space:]]*-+[[:space:]]*.*[[:space:]]*-+[[:space:]]*$/ {
      s = $0
      sub(/^[[:space:]]*\/\/[[:space:]]*-+[[:space:]]*/, "", s)
      sub(/[[:space:]]*-+[[:space:]]*$/, "", s)
      if (s != "") section = s
      next
    }
    /^[[:space:]]*\/\// { next }          # ordinary comments
    !/\{/ { next }                        # not a bind line

    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      key = line
      sub(/[[:space:]].*$/, "", key)
      rest = substr(line, length(key) + 1)

      # Explicitly hidden from niris own overlay: hide here too.
      if (rest ~ /hotkey-overlay-title[[:space:]]*=[[:space:]]*null/) next

      label = ""
      if (match(rest, /hotkey-overlay-title[[:space:]]*=[[:space:]]*"[^"]*"/)) {
        label = substr(rest, RSTART, RLENGTH)
        sub(/^[^"]*"/, "", label)
        sub(/"$/, "", label)
      } else {
        # Fall back to the action itself.
        a = rest
        sub(/^[^{]*\{[[:space:]]*/, "", a)
        sub(/[[:space:]]*\}[[:space:]]*$/, "", a)
        sub(/;[[:space:]]*$/, "", a)
        if (a ~ /^spawn/) {
          # Join the quoted arguments: spawn "loginctl" "lock-session".
          out = ""
          tmp = a
          while (match(tmp, /"[^"]*"/)) {
            arg = substr(tmp, RSTART + 1, RLENGTH - 2)
            tmp = substr(tmp, RSTART + RLENGTH)
            sub(/^.*\//, "", arg)          # ~/.local/bin/x.sh -> x.sh
            out = (out == "" ? arg : out " " arg)
          }
          label = out
        } else {
          # Only the action name loses its dashes; arguments stay verbatim, or
          # set-column-width "-10%" renders as `set column width " 10%"`.
          nm = a; ar = ""
          if (match(a, /[[:space:]]/)) { nm = substr(a, 1, RSTART - 1); ar = substr(a, RSTART) }
          gsub(/-/, " ", nm)
          label = nm ar
        }
        if (length(label) > 44) label = substr(label, 1, 41) "..."
      }
      if (label == "") next

      # Merge every key that does the same thing in the same section onto one
      # row: the arrows and HJKL are one binding to learn, not eight.
      k = section SUBSEP label
      if (!(k in keys)) {
        if (!(section in seen_sec)) { seen_sec[section] = 1; secs[++nsec] = section }
        order[section, ++cnt[section]] = label
        keys[k] = key
      } else {
        keys[k] = keys[k] " / " key
      }
      if (length(keys[k]) > kw) kw = length(keys[k])
    }

    function rep(ch, n,   s) { s = ""; while (n-- > 0) s = s ch; return s }
    function pad(s, n) { return s rep(" ", n - length(s)) }
    function row(s,   n) {
      n = width - 4 - length(s)
      if (n < 0) n = 0
      print "  \342\224\202 " s rep(" ", n) " \342\224\202"
    }

    END {
      if (kw > 30) kw = 30
      inner = width - 4

      print ""
      print "  \342\224\214" rep("\342\224\200", inner + 2) "\342\224\220"
      row("LifeBranch \342\200\224 keyboard shortcuts")
      row("Mod is the Super key (the one with the Windows/Command logo).")
      print "  \342\224\234" rep("\342\224\200", inner + 2) "\342\224\244"

      for (i = 1; i <= nsec; i++) {
        s = secs[i]
        if (i > 1) row("")
        h = s
        sub(/[[:space:]]*\(.*$/, "", h)
        row(toupper(h))
        for (j = 1; j <= cnt[s]; j++) {
          lab = order[s, j]
          k = keys[s SUBSEP lab]
          if (length(k) <= kw) {
            row("  " pad(k, kw) "  " lab)
          } else {
            row("  " k)
            row("  " rep(" ", kw) "  " lab)
          }
        }
      }

      print "  \342\224\234" rep("\342\224\200", inner + 2) "\342\224\244"
      row("Edit these bindings:")
      row("    nano " cfgpath)
      if (realp != cfgpath) row("    (a symlink to " realp " \342\200\224 your git checkout)")
      row("Check it before logging out:   niri validate")
      row("")
      row("To stop this window opening at login, set that line to off:")
      row("    // shortcuts-at-startup off")
      print "  \342\224\224" rep("\342\224\200", inner + 2) "\342\224\230"
      print ""
    }
  ' "$CONFIG"
}

open_window() {
  if ! command -v kitty >/dev/null 2>&1; then
    echo "shortcuts-window.sh: kitty is not installed" >&2
    exit 1
  fi
  # --class gives niri an app-id to match its floating window rule against.
  exec kitty --class "$APP_ID" --title "LifeBranch shortcuts" \
       -- "$0" --show
}

case ${1:---open} in
  --print) render ;;
  --show)
    if command -v less >/dev/null 2>&1; then
      # -R keeps the box drawing, -X leaves the text on screen, and the prompt
      # states the one key that matters. Paging only kicks in when it must.
      render | less -R -X -P " q = close    up/down, PgUp/PgDn = scroll "
    else
      render
      read -rsn1 -p "  press any key to close "
    fi
    ;;
  --at-startup)
    startup_enabled || exit 0
    open_window
    ;;
  --open|*) open_window ;;
esac
