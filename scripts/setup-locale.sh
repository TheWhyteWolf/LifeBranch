#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# setup-locale.sh: what keyboard is this, and where in the world is it?
#
# The shipped config pins one person's answers (a UK keymap, London for the
# night-light). Both are wrong for everyone else, and the keyboard one is wrong
# in the worst possible place: at a password prompt, before you can read any
# documentation about it. So detect the machine's own answers and offer them.
#
# Everything is read from what the Arch install already set up:
#   localectl / /etc/vconsole.conf     the keyboard layout
#   timedatectl / /etc/localtime       the timezone
#   /usr/share/zoneinfo/zone1970.tab   coordinates for that timezone (tzdata
#                                      ships them; no network, no geolocation)
#   /sys/class/power_supply/BAT*       laptop or desktop, for the NumLock default
#
# Usage:
#   setup-locale.sh --detect                      key=value lines for a caller
#   setup-locale.sh --keyboard-block L V M N      the niri keyboard {} block
#   setup-locale.sh --nightlight-block LAT LON    the niri wlsunset spawn line
set -uo pipefail

# Console keymaps are not xkb layouts. Only the ones that actually differ need
# translating; anything unlisted is passed through, which is right far more
# often than not (us, de, fr, es, it, ...).
vc_to_xkb() {
  case ${1%%.*} in
    uk|gb) echo gb ;;
    de-latin1|de-latin1-nodeadkeys|de_CH-latin1) echo de ;;
    fr-latin1|fr-pc) echo fr ;;
    sv-latin1) echo se ;;
    no-latin1) echo no ;;
    dk-latin1) echo dk ;;
    fi-latin1) echo "fi" ;;
    es|es-cp850) echo es ;;
    pt-latin1) echo pt ;;
    br-abnt2) echo br ;;
    it2) echo it ;;
    ''|us) echo us ;;
    *) echo "${1%%-*}" ;;
  esac
}

# localectl prints the literal "(unset)" (and sometimes "n/a") for a field it
# has no value for, so an unset X11 layout must not win over a set VC keymap.
unset_ok() { [[ -n $1 && $1 != "(unset)" && $1 != "n/a" ]]; }

localectl_field() {
  command -v localectl >/dev/null 2>&1 || return 1
  local v
  v=$(localectl status 2>/dev/null | sed -n "s/.*$1:[[:space:]]*//p" | head -1)
  unset_ok "$v" && printf '%s' "$v"
}

detect_layout() {
  local l
  # X11 layout first: it is already an xkb name, no translation needed.
  l=$(localectl_field "X11 Layout") && { echo "${l%%,*}"; return; }
  l=$(localectl_field "VC Keymap") || l=''
  if [[ -z $l && -r /etc/vconsole.conf ]]; then
    l=$(sed -n 's/^KEYMAP=//p' /etc/vconsole.conf | tr -d '"' | head -1)
  fi
  vc_to_xkb "${l:-us}"
}

detect_variant() {
  local v
  v=$(localectl_field "X11 Variant") || v=''
  printf '%s' "$v"
}

detect_timezone() {
  local tz=''
  command -v timedatectl >/dev/null 2>&1 && tz=$(timedatectl show -p Timezone --value 2>/dev/null)
  [[ -z $tz && -L /etc/localtime ]] && tz=$(readlink -f /etc/localtime | sed 's|.*/zoneinfo/||')
  echo "$tz"
}

# ISO 6709 (+DDMM+DDDMM or +DDMMSS+DDDMMSS) -> two decimal degrees.
iso6709_decimal() {
  local c=$1 lat lon
  if [[ $c =~ ^([+-][0-9]{2})([0-9]{2})([0-9]{2})?([+-][0-9]{3})([0-9]{2})([0-9]{2})?$ ]]; then
    lat=$(awk -v d="${BASH_REMATCH[1]}" -v m="${BASH_REMATCH[2]}" -v s="${BASH_REMATCH[3]:-0}" \
          'BEGIN { sign = (d < 0) ? -1 : 1; printf "%.4f", d + sign * (m / 60 + s / 3600) }')
    lon=$(awk -v d="${BASH_REMATCH[4]}" -v m="${BASH_REMATCH[5]}" -v s="${BASH_REMATCH[6]:-0}" \
          'BEGIN { sign = (d < 0) ? -1 : 1; printf "%.4f", d + sign * (m / 60 + s / 3600) }')
    printf '%s %s\n' "$lat" "$lon"
    return 0
  fi
  return 1
}

# Coordinates for a timezone, straight out of tzdata's own table.
tz_coords() {
  local tz=$1 f c
  [[ -z $tz ]] && return 1
  for f in /usr/share/zoneinfo/zone1970.tab /usr/share/zoneinfo/zone.tab; do
    [[ -r $f ]] || continue
    c=$(awk -F'\t' -v tz="$tz" '$0 !~ /^#/ && $3 == tz { print $2; exit }' "$f")
    [[ -n $c ]] && iso6709_decimal "$c" && return 0
  done
  return 1
}

# Should NumLock start on? This is a GUESS, and it is labelled as one: laptop
# keyboard drivers claim the whole key range, so KEY_KP0 reads as present on
# this very machine — an Apple internal keyboard with no numpad at all.
# Capability bits cannot answer it. A battery can, roughly: full-size boards
# with a numpad live on desktops, and switching NumLock on for a laptop without
# one can turn part of the letter area into a keypad. The installer asks anyway;
# this only picks which answer is offered first.
numlock_default() {
  compgen -G "/sys/class/power_supply/BAT*" >/dev/null && { echo 0; return; }
  echo 1
}

keyboard_block() {
  local layout=${1:-us} variant=${2-} model=${3:-pc105} numlock=${4:-0}
  echo "    keyboard {"
  echo "        // Written by scripts/setup-locale.sh — re-run install.sh, or just"
  echo "        // edit these: they are ordinary xkb names (\`localectl list-x11-keymap-layouts\`)."
  echo "        xkb {"
  echo "            layout \"$layout\""
  [[ -n $variant ]] && echo "            variant \"$variant\""
  echo "            model \"$model\""
  echo "            options \"terminate:ctrl_alt_bksp\""
  echo "        }"
  (( numlock )) && echo "        // This keyboard has a numpad, so NumLock starts on." && echo "        numlock"
  echo "    }"
}

nightlight_block() {
  local lat=$1 lon=$2
  echo "// Night light: warms the gamma after sunset (wlr-gamma-control)."
  echo "// Coordinates come from your timezone via tzdata; -l is latitude, -L longitude."
  echo "spawn-at-startup \"wlsunset\" \"-l\" \"$lat\" \"-L\" \"$lon\""
}

case ${1:---detect} in
  --detect)
    layout=$(detect_layout)
    variant=$(detect_variant)
    tz=$(detect_timezone)
    coords=$(tz_coords "$tz") || coords=''
    printf 'layout=%s\n'   "$layout"
    printf 'variant=%s\n'  "$variant"
    printf 'timezone=%s\n' "$tz"
    printf 'lat=%s\n'      "${coords%% *}"
    printf 'lon=%s\n'      "${coords##* }"
    printf 'numlock_default=%s\n' "$(numlock_default)"
    ;;
  --keyboard-block)   shift; keyboard_block "$@" ;;
  --nightlight-block) shift; nightlight_block "$@" ;;
  *) echo "usage: $0 [--detect|--keyboard-block L V M N|--nightlight-block LAT LON]" >&2; exit 2 ;;
esac
