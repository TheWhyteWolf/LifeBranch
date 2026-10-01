#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# sysmon.sh — what the bar's CPU and MEM readouts open when you click them.
#
# The modules used to run `kitty -e htop` directly, so every click opened
# another window and a distracted click or two left a stack of them running.
# This focuses the one that is already open instead, and only spawns a monitor
# when there is none:
#   - a window with app-id "lifebranch-sysmon" exists -> focus it
#   - nothing open                                    -> spawn it, and wait for
#                                                        it to map still holding
#                                                        the lock, so a fast
#                                                        double-click cannot
#                                                        race in a second one
#
# htop is what install.sh installs; btop and top are picked up if they are what
# the machine actually has.
set -euo pipefail

APP_ID="lifebranch-sysmon"

# One click at a time. The lock is released when this script exits, by which
# point the window exists and the branch above takes over.
exec 9>"${XDG_RUNTIME_DIR:-/tmp}/lifebranch-sysmon.lock"
flock -n 9 || exit 0

# niri's IPC is how the window is found; without it (or without jq) there is
# nothing to focus, so fall through to spawning one.
if command -v niri >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  id=$(niri msg --json windows | jq "[.[] | select(.app_id == \"$APP_ID\")][0].id")
  if [[ -n $id && $id != null ]]; then
    niri msg action focus-window --id "$id"
    exit 0
  fi
fi

mon=$(command -v htop || command -v btop || command -v top) || {
  echo "sysmon: no htop, btop or top on this machine" >&2
  exit 1
}

setsid kitty --app-id "$APP_ID" -e "$mon" >/dev/null 2>&1 &

# Hold the lock until it maps (or two seconds pass, if kitty never comes up).
command -v niri >/dev/null 2>&1 && command -v jq >/dev/null 2>&1 || exit 0
for _ in $(seq 1 40); do
  id=$(niri msg --json windows | jq "[.[] | select(.app_id == \"$APP_ID\")][0].id")
  [[ -n $id && $id != null ]] && break
  sleep 0.05
done
