#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Brightness-key wrapper (laptop): brightnessctl + an OSD flash (lifeosd, else
# wob).
# Bound to XF86MonBrightness* / Mod+F1/F2 in macbook/niri/config.kdl.
set -euo pipefail

case "${1:-}" in
  up)   brightnessctl set 5%+ >/dev/null ;;
  down) brightnessctl set 5%- >/dev/null ;;
  *)    echo "usage: bright-osd.sh up|down" >&2; exit 2 ;;
esac

# lifebar re-reads BRT now (the kernel's uevent would tell it shortly too).
pkill -RTMIN+10 -x lifebar 2>/dev/null || true

run="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
osd="$run/lifeosd.fifo" sock="$run/wob.sock"
[[ -p "$osd" || -p "$sock" ]] || exit 0   # no OSD running; brightness changed anyway

# "intel_backlight,backlight,48000,50%,96000" -> 50
pct=$(brightnessctl -m | awk -F, '{ gsub("%", "", $4); print $4 }')
# The listener holds the FIFO open for reading, so this never blocks in
# practice; the timeout guards against a dead listener wedging the script.
if [[ -p "$osd" ]]; then
  timeout 0.2 bash -c "echo 'brightness $pct' > '$osd'" || true
else
  timeout 0.2 bash -c "echo $pct > '$sock'" || true
fi
