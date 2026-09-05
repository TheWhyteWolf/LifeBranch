#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Notification-history menu: lifenote ctl history + fuzzel (clip-menu.sh
# pattern). Bound to the waybar # button. View-only — Esc/Enter both close.
# Viewing marks everything seen: the daemon resets and re-pings the # badge.
# A dead daemon is reported, not passed off as an empty history; --width
# overrides fuzzel.ini's launcher-sized 40 columns so entries stay readable.
set -euo pipefail

if hist="$(~/.local/bin/lifenote ctl history 2>/dev/null)"; then
  [ -n "$hist" ] || hist="(no notifications)"
else
  hist="(lifenote not running — fallback: pkill lifenote && mako)"
fi
printf '%s\n' "$hist" | fuzzel --dmenu --prompt "notif> " --width 100 >/dev/null || true
