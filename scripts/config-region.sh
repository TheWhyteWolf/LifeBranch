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

# set_toml FILE SECTION KEY VALUE: rewrite `key = value` inside [section].
# awk rather than a TOML library: this runs during a fresh install, before
# anything guarantees python or a parser is present.
set_toml() {
  local file="$1" section="$2" key="$3" value="$4" tmp
  tmp=$(mktemp)
  awk -v sec="[$section]" -v key="$key" -v val="$value" '
    /^[[:space:]]*\[/ { in_sec = ($0 ~ "^[[:space:]]*\\" sec "[[:space:]]*$") }
    in_sec && $0 ~ ("^[[:space:]]*" key "[[:space:]]*=") { print key " = " val; done = 1; next }
    { print }
  ' "$file" > "$tmp" && mv "$tmp" "$file"
}
