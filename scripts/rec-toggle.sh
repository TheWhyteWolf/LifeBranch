#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Screen-record toggle: Mod+Print starts wf-recorder on the (single) output,
# a second press stops it — SIGINT lets it finalize the file. Start/stop are
# announced through lifenote (notify-send). vol-osd.sh/dnd-toggle.sh pattern.
set -euo pipefail

dir="$(xdg-user-dir VIDEOS 2>/dev/null || true)"
[ -n "$dir" ] || dir="$HOME/Videos"

if pgrep -x wf-recorder >/dev/null; then
  pkill -INT -x wf-recorder
  notify-send "rec" "recording saved in $dir"
else
  command -v wf-recorder >/dev/null || {
    notify-send -u critical "rec" "wf-recorder not installed"
    exit 1
  }
  mkdir -p "$dir"
  out="$dir/rec-$(date +%F_%H-%M-%S).mp4"
  setsid wf-recorder -f "$out" </dev/null >/dev/null 2>&1 &
  notify-send "rec" "recording started — Mod+Print stops"
fi
