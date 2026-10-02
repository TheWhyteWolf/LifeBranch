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

# copy_regions SRC DST: give every fenced region in DST (LIFEBRANCH and
# LIFECONF alike) the body the same-named region has in SRC. Regions SRC lacks
# keep DST's body. DST is rewritten in place (through a symlink: the real file
# is resolved first, as in write_region).
copy_regions() {
  local src="$1" dst real tmp
  real=$(readlink -f "$2") || return 1
  tmp=$(mktemp) || return 1
  awk '
    function key(l) { sub(/.*LIFE(CONF|BRANCH):(BEGIN|END) /, "", l); sub(/[[:space:]]+$/, "", l); return l }
    FNR == NR {
      if ($0 ~ /LIFE(CONF|BRANCH):BEGIN /) { k = key($0); have[k] = 1; body[k] = ""; inr = 1; next }
      if ($0 ~ /LIFE(CONF|BRANCH):END /) { inr = 0; next }
      if (inr) body[k] = body[k] $0 "\n"
      next
    }
    $0 ~ /LIFE(CONF|BRANCH):BEGIN / {
      print; k = key($0)
      if (k in have) { printf "%s", body[k]; skip = 1 }
      next
    }
    $0 ~ /LIFE(CONF|BRANCH):END / { skip = 0 }
    !skip { print }
  ' "$src" "$real" > "$tmp" && cat "$tmp" > "$real"
  local rc=$?
  rm -f "$tmp"
  return $rc
}

# seed_niri_local LOCAL TEMPLATE [OLD...]: create the machine's niri/local.kdl
# (gitignored; the tracked config.kdl includes it) when it doesn't exist yet,
# from TEMPLATE. The region bodies are then carried over from the first OLD
# file that has any regions — an earlier config.kdl from before the regions
# moved out of it — so an upgrade keeps its layout, touchpad and theme.
# A no-op once LOCAL exists: from then on it is the machine's own file.
seed_niri_local() {
  local local_kdl="$1" template="$2" old; shift 2
  [[ -e $local_kdl ]] && return 0
  cp "$template" "$local_kdl" || return 1
  echo "    created $local_kdl (this machine's niri settings; not tracked by git)"
  for old in "$@"; do
    [[ -s $old ]] && grep -q 'LIFE\(CONF\|BRANCH\):BEGIN ' "$old" || continue
    copy_regions "$old" "$local_kdl" && echo "    carried your settings over from your previous config.kdl"
    return 0
  done
  return 0
}

# prepare_niri_local REPO REL: make sure <REPO>/<REL>/local.kdl exists before
# config.kdl is linked (niri rejects a config whose include is missing). REL is
# the config's directory inside the repo: niri or macbook/niri.
#
# Where an upgrade's settings come from: up to this change the regions lived
# in the tracked config.kdl itself, so a machine that had run the installer or
# Settings had local edits there, and `git pull` refused to run over them. The
# way through is `git stash && git pull`, and the newest stash then holds the
# old config — so that is looked at first, then the backup write_region left.
prepare_niri_local() {
  local repo="$1" rel="$2" stashed rc=0
  stashed=$(mktemp) || return 1
  git -C "$repo" show "stash@{0}:$rel/config.kdl" > "$stashed" 2>/dev/null || : > "$stashed"
  seed_niri_local "$repo/$rel/local.kdl" "$repo/$rel/local.kdl.default" \
    "$stashed" "$repo/$rel/config.kdl.lifebranch-prev" || rc=1
  rm -f "$stashed"
  return $rc
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
