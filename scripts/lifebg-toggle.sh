#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Pause/resume the Game of Life wallpaper (SIGSTOP/SIGCONT on lifebg).
#   (no args)     manual toggle with a notification — bound to Mod+Shift+G
#   dpms-pause    silent; swayidle runs it when the monitors power off
#   dpms-resume   silent; only resumes what dpms-pause froze, so it never
#                 clobbers a manual pause
# Frozen wallpaper costs zero CPU; the last frame stays on screen. lifewall
# resyncs its clock on resume.
set -euo pipefail

mode="${1:-toggle}"
marker="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/lifebg.dpms-paused"

# Matches the rust binary (…/bin/lifebg) or the python fallback
# (python3 …/bin/lifebg) but NOT the kitty panel, whose multi-word cmdline
# also ends in bin/lifebg — [^ ]* cannot span its spaces.
pattern='^(python[0-9.]* )?[^ ]*bin/lifebg( |$)'
pid=$(pgrep -f "$pattern" | head -1) || {
  if [[ "$mode" == toggle ]]; then
    notify-send -t 2000 "Game of Life" "wallpaper is not running"
  fi
  exit 0
}

paused() { [[ "$(ps -o stat= -p "$pid" | tr -d ' ')" == T* ]]; }

case "$mode" in
  toggle)
    if paused; then
      kill -CONT "$pid"
      rm -f "$marker" # a manual resume also releases any dpms claim
      notify-send -t 1500 "Game of Life" "resumed"
    else
      kill -STOP "$pid"
      notify-send -t 1500 "Game of Life" "paused"
    fi
    ;;
  dpms-pause)
    if ! paused; then
      kill -STOP "$pid"
      touch "$marker"
    fi
    ;;
  dpms-resume)
    if [[ -e "$marker" ]]; then
      rm -f "$marker"
      kill -CONT "$pid"
    fi
    ;;
  *)
    echo "usage: lifebg-toggle.sh [toggle|dpms-pause|dpms-resume]" >&2
    exit 2
    ;;
esac
