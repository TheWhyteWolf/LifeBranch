#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Power menu: fuzzel dmenu, olive-styled via fuzzel.ini.
# Bound to Mod+Shift+E in niri (Ctrl+Alt+Delete stays as the raw quit fallback).
# Suspend appears only where sleep is allowed — the desktop masks sleep.target
# (live services, must never sleep), the laptop doesn't. before-sleep locks first.
set -euo pipefail

opts=(Lock "Log out" Reboot "Power off")
if ! systemctl is-enabled sleep.target 2>/dev/null | grep -qx masked; then
  opts=(Lock Suspend "Log out" Reboot "Power off")
fi

choice=$(printf '%s\n' "${opts[@]}" \
  | fuzzel --dmenu --prompt "power> " --lines "${#opts[@]}") || exit 0

case "$choice" in
  "Lock")      loginctl lock-session ;;
  "Suspend")   systemctl suspend ;;
  "Log out")   niri msg action quit --skip-confirmation ;;
  "Reboot")    systemctl reboot ;;
  "Power off") systemctl poweroff ;;
esac
