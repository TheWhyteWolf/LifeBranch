#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# check-deps.sh: fail if the code calls an external command whose package is
# not in scripts/packages.sh. Run by CI; runs anywhere (no pacman needed).
#
# The failure it guards against: a panel or script starts using a tool that
# happens to be installed on the dev machine, and a fresh install shows
# "unavailable" or a silent no-op. Add a row below when you start calling
# something new.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/packages.sh
source "$REPO/scripts/packages.sh"

# command:package
MAP=(wpctl:wireplumber
     nmcli:networkmanager
     bluetoothctl:bluez-utils
     powerprofilesctl:power-profiles-daemon
     upower:upower
     brightnessctl:brightnessctl
     notify-send:libnotify
     xdg-mime:xdg-utils
     secret-tool:libsecret
     playerctl:playerctl
     wl-copy:wl-clipboard
     wl-paste:wl-clipboard
     cliphist:cliphist
     jq:jq
     wf-recorder:wf-recorder
     swaylock:swaylock
     swayidle:swayidle
     udisksctl:udisks2
     htop:htop
     checkupdates:pacman-contrib
     kitty:kitty)

# Code that runs on the user's machine. Comments count as uses too, which errs
# on the safe side.
SRC=("$REPO"/scripts/*.sh "$REPO"/life*/src "$REPO"/niri/config.kdl "$REPO"/waybar/config.jsonc)

declare -A have=()
for p in "${PKGS[@]}"; do have[$p]=1; done

status=0
for row in "${MAP[@]}"; do
  cmd=${row%%:*} pkg=${row#*:}
  # -w treats '-' as a word boundary, so match the command delimited by
  # anything that cannot be part of a command name.
  if grep -rqE "(^|[^A-Za-z0-9_.-])$cmd([^A-Za-z0-9_.-]|\$)" "${SRC[@]}" --exclude=check-deps.sh --exclude=packages.sh; then
    if [[ -z ${have[$pkg]:-} ]]; then
      echo "::error::'$cmd' is called but its package '$pkg' is not in scripts/packages.sh"
      status=1
    fi
  fi
done
(( status )) || echo "all ${#MAP[@]} known commands map to installed packages"
exit $status
