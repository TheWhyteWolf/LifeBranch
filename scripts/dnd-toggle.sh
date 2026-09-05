#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Toggle lifenote's do-not-disturb mode. Bound to Mod+N in niri and to the
# waybar DND label's on-click; the RTMIN+8 ping refreshes that label.
set -euo pipefail

~/.local/bin/lifenote ctl dnd toggle >/dev/null
pkill -RTMIN+8 waybar || true
