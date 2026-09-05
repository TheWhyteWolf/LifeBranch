#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Clipboard-history picker: cliphist + fuzzel.
# Bound to Mod+P in niri. Requires the wl-paste --watch cliphist store
# watchers started via spawn-at-startup.
set -euo pipefail
cliphist list | fuzzel --dmenu --prompt "clip> " | cliphist decode | wl-copy
