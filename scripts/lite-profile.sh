#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# lite-profile.sh: turn the expensive parts down on hardware that cannot spare it.
#
# The default look assumes a machine with cycles to burn: a full-screen Game of
# Life at 30 fps in a terminal panel, translucent terminals composited over it,
# and eased animations. On an old laptop that is the difference between a
# desktop that feels instant and one that feels like treacle.
#
# Nothing here is guesswork about "slow": it turns down the four things that
# actually cost, and says which and why.
#
# Usage:
#   lite-profile.sh --check    exit 0 if this machine looks like it wants it
#   lite-profile.sh --report   print what it would change
#   lite-profile.sh --apply    write the settings and re-apply
set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
# shellcheck source=scripts/config-region.sh
source "$HERE/config-region.sh"

THEME=${LIFEBRANCH_THEME:-${XDG_CONFIG_HOME:-$HOME/.config}/lifeconf/theme.toml}
RICE=${LIFEBRANCH_RICE:-${XDG_CONFIG_HOME:-$HOME/.config}/kitty/rice.conf}

mem_gib() { awk '/^MemTotal:/ { printf "%.1f", $2 / 1048576 }' /proc/meminfo 2>/dev/null || echo 0; }

# Is the disk holding / a spinning one? On a machine of this age that is both
# very likely and the single loudest signal that it predates the assumptions
# this rice was built under.
root_rotational() {
  local src dev
  src=$(findmnt -no SOURCE / 2>/dev/null) || return 1
  dev=$(lsblk -no PKNAME "$src" 2>/dev/null | head -1)
  [[ -z $dev ]] && dev=$(basename "$src" | sed 's/[0-9]*$//')
  [[ -r /sys/block/$dev/queue/rotational ]] || return 1
  [[ $(cat "/sys/block/$dev/queue/rotational") == 1 ]]
}

REASONS=()
check() {
  local mem cores
  mem=$(mem_gib); cores=$(nproc 2>/dev/null || echo 1)
  awk -v m="$mem" 'BEGIN { exit !(m < 6) }' && REASONS+=("${mem} GiB of RAM")
  (( cores <= 2 )) && REASONS+=("$cores CPU core(s)")
  root_rotational && REASONS+=("a spinning disk on /")
  (( ${#REASONS[@]} ))
}

report() {
  cat <<EOF
    Game of Life wallpaper : 30 fps -> 10 fps, 0.3s tick -> 0.5s, sparser board
                             (a full-screen terminal animation is the big one)
    Terminal transparency  : 0.93 -> 1.0 (no compositing over a moving background)
    Window animations      : slowdown 0.6 -> 0.35 (shorter, not disabled)
    Everything else — colours, keybinds, layout — is untouched.
EOF
}

apply() {
  local changed=0
  if [[ -f $THEME ]]; then
    set_toml "$THEME" lifewall   tick     0.5
    set_toml "$THEME" lifewall   fps      10
    set_toml "$THEME" lifewall   fade     2.0
    set_toml "$THEME" lifewall   density  0.12
    set_toml "$THEME" animations slowdown 0.35
    echo "    theme.toml: lifewall turned down, animations shortened"
    changed=1
  else
    echo "    !! $THEME not found — run lifeconf --apply first, then re-run this." >&2
  fi

  if [[ -f $RICE ]] && has_region "$(readlink -f "$RICE")" opacity; then
    local blk
    blk=$(mktemp)
    printf '%s\n' "background_opacity 1.0" > "$blk"
    write_region "$RICE" opacity "$blk"
    rm -f "$blk"
    changed=1
  fi

  if (( changed )) && command -v lifeconf >/dev/null 2>&1; then
    echo "    regenerating configs (lifeconf --apply)"
    lifeconf --apply >/dev/null 2>&1 || echo "    (lifeconf --apply reported a problem; configs may need a re-run)"
  fi
  echo "    done — restart kitty windows and re-login for all of it to take."
}

case ${1:---check} in
  --check)  check ;;
  --report) report ;;
  --apply)  apply ;;
  --why)    check; (( ${#REASONS[@]} )) && printf '  - %s\n' "${REASONS[@]}" ;;
  *) echo "usage: $0 [--check|--report|--apply|--why]" >&2; exit 2 ;;
esac
