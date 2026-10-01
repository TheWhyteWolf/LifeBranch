#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# config-region.sh: sourced helper for rewriting fenced regions in a config.
#
#     // LIFEBRANCH:BEGIN <name>
#     ...installer-owned lines...
#     // LIFEBRANCH:END <name>
#
# Same idea as lifeconf's LIFECONF regions, for the things the INSTALLER owns:
# touchpad settings, keyboard layout, night-light coordinates, terminal opacity.
# Hand-editable in place, and rewritten without appending duplicates.
# Not executable on its own; `source` it.

# has_region CONFIG NAME
has_region() { grep -q "LIFEBRANCH:BEGIN $2\$" "$1" 2>/dev/null; }

# write_region CONFIG NAME BLOCKFILE [VALIDATOR...]
#
# Replaces the region's body with BLOCKFILE. The config is normally a symlink
# into the repo, so the real file is resolved first: writing to the link path
# would replace the link with a regular file and quietly detach the config from
# git. If VALIDATOR is given it is run against the result (with the config path
# appended) and the previous file is restored when it fails — a generated block
# that does not parse must never be what you find at your next login.
write_region() {
  local cfg="$1" name="$2" block="$3"; shift 3
  local real backup rc
  real=$(readlink -f "$cfg") || return 1
  if ! has_region "$real" "$name"; then
    echo "    no LIFEBRANCH:BEGIN $name region in $real — skipping" >&2
    return 1
  fi
  backup="$real.lifebranch-prev"
  cp "$real" "$backup" || return 1
  awk -v blk="$block" -v name="$name" '
    $0 ~ ("LIFEBRANCH:BEGIN " name "$") {
      print
      while ((getline l < blk) > 0) print l
      close(blk)
      skip = 1
      next
    }
    $0 ~ ("LIFEBRANCH:END " name "$") { skip = 0 }
    !skip { print }
  ' "$backup" > "$real"
  rc=0
  if (( $# )); then
    "$@" "$real" >/dev/null 2>&1 || rc=1
  fi
  if (( rc )); then
    mv "$backup" "$real"
    echo "    !! generated '$name' block did not validate — config restored, nothing changed." >&2
    return 1
  fi
  echo "    wrote the '$name' region (previous copy: $backup)"
  return 0
}

# get_toml FILE SECTION KEY: print the value of `key` inside [section], or
# nothing when either is absent. Numbers and bare words come back verbatim.
get_toml() {
  awk -v sec="[$2]" -v key="$3" '
    /^[[:space:]]*\[/ { in_sec = ($0 ~ "^[[:space:]]*\\" sec "[[:space:]]*$"); next }
    in_sec && $0 ~ ("^[[:space:]]*" key "[[:space:]]*=") {
      sub(/^[^=]*=[[:space:]]*/, ""); sub(/[[:space:]]*$/, ""); print; exit
    }
  ' "$1" 2>/dev/null
}

# offer_idle_suspend THEME_TOML: on a laptop, offer to add the suspend step to
# the idle chain. Lives here rather than in both installers because it is 20
# lines of prompt and TOML editing that would otherwise be copy-pasted twice
# and drift — the failure CI's installer-agreement job exists to catch.
#
# Needs ask_yn from prompt.sh, which every installer that sources this file has
# already sourced. Only asks while the value is still 0 (the shipped default),
# so a deliberate "no" is re-asked on the next run but a deliberate "yes" is
# never second-guessed.
#
# Every path returns 0 on purpose: the installers run under `set -e` and call
# this as a bare statement, so a machine without a battery, an unreadable
# theme.toml or a failed rewrite must not take the whole install down over an
# optional power setting.
offer_idle_suspend() {
  local theme="$1" cur
  compgen -G "/sys/class/power_supply/BAT*" >/dev/null || return 0   # desktop
  [[ -f $theme ]] || return 0
  cur=$(get_toml "$theme" idle suspend_minutes || true)
  [[ ${cur:-0} == 0 ]] || return 0

  echo "==> Idle suspend: battery detected"
  echo "    The idle chain locks the screen and then blanks it, and stops there —"
  echo "    an idle laptop with the lid open stays awake until the battery is flat."
  echo "    Suspending after 30 min of idle applies ONLY on battery; plugged in,"
  echo "    nothing changes. Adjust or disable it later in lifeconf (Idle ->"
  echo "    suspend_minutes; 0 is off)."
  ask_yn "Suspend after 30 minutes idle on battery?" y || return 0

  if ! set_toml "$theme" idle suspend_minutes 30; then
    echo "    !! could not write $theme — idle suspend not enabled" >&2
    return 0
  fi
  if "$HOME/.local/bin/lifeconf" --apply >/dev/null 2>&1; then
    echo "    idle suspend enabled (30 min, battery only)"
  else
    echo "    !! lifeconf --apply failed; run it by hand to arm the new timeout" >&2
  fi
}

# set_toml FILE SECTION KEY VALUE: set `key = value` inside [section].
# awk rather than a TOML library: this runs during a fresh install, before
# anything guarantees python or a parser is present.
#
# The key is INSERTED when the section exists but does not have it yet, and the
# section is appended when it is missing too. That matters on upgrades: lifeconf
# only rewrites theme.toml when the preset changes, so a machine installed
# before a field existed still has a theme.toml without it, and a replace-only
# version of this silently did nothing there. A key appended at the end of a
# section is still inside it — a blank line does not close a TOML table, only
# the next [header] does.
set_toml() {
  local file="$1" section="$2" key="$3" value="$4" tmp
  tmp=$(mktemp)
  awk -v sec="[$section]" -v key="$key" -v val="$value" '
    # Blank lines inside our section are held back rather than printed, so an
    # inserted key lands against the last real key instead of after the blank
    # line that separates the section from the next one.
    function flush(   i) { for (i = 0; i < nblank; i++) print ""; nblank = 0 }
    # A table header: if we are leaving our section without having written the
    # key, this is the last moment it would still land inside it.
    /^[[:space:]]*\[/ {
      if (in_sec && !done) { print key " = " val; done = 1 }
      flush()
      in_sec = ($0 ~ "^[[:space:]]*\\" sec "[[:space:]]*$")
      if (in_sec) seen_sec = 1
      print
      next
    }
    in_sec && !done && $0 ~ ("^[[:space:]]*" key "[[:space:]]*=") {
      flush(); print key " = " val; done = 1; next
    }
    in_sec && /^[[:space:]]*$/ { nblank++; next }
    { flush(); print }
    END {
      if (in_sec && !done) { print key " = " val; done = 1 }
      flush()
      if (!done) {
        if (!seen_sec) { print ""; print sec }
        print key " = " val
      }
    }
  ' "$file" > "$tmp" && mv "$tmp" "$file"
}
