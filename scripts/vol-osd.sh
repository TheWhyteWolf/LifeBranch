#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Volume-key wrapper: adjust via wpctl, then flash the OSD bar (lifeosd, else
# wob).
# Bound to Mod+KP_* / XF86Audio* in niri (allow-when-locked — the keys still
# work while locked, but session-lock hides overlays so the bar can't show).
set -euo pipefail

case "${1:-}" in
  up)   wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.05+ -l 1.0 ;;
  down) wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.05- ;;
  mute) wpctl set-mute   @DEFAULT_AUDIO_SINK@ toggle ;;
  *)    echo "usage: vol-osd.sh up|down|mute" >&2; exit 2 ;;
esac

run="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
osd="$run/lifeosd.fifo" sock="$run/wob.sock"
[[ -p "$osd" || -p "$sock" ]] || exit 0   # no OSD running; volume changed anyway

# "Volume: 0.45" / "Volume: 0.45 [MUTED]" -> integer percent.
out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@)
pct=$(awk '{ printf "%d", $2 * 100 + 0.5 }' <<<"$out")
# The listener holds the FIFO open for reading, so this never blocks in
# practice; the timeout guards against a dead listener wedging the script.
if [[ -p "$osd" ]]; then
  line="volume $pct"; [[ "$out" == *MUTED* ]] && line+=" muted"
  timeout 0.2 bash -c "echo '$line' > '$osd'" || true
else
  [[ "$out" == *MUTED* ]] && pct=0   # wob has no muted state: show empty
  timeout 0.2 bash -c "echo $pct > '$sock'" || true
fi
