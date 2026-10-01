#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# hide-autostart.sh NAME...: keep the system's XDG autostart entry
# /etc/xdg/autostart/NAME.desktop from starting in the niri session.
#
# niri's session starts xdg-desktop-autostart.target, so systemd runs every
# autostart entry as an app-NAME@autostart.service. That is how nm-applet and
# blueman-applet kept coming back after lifepanel replaced them: niri's own
# config no longer starts them, but their packages ship autostart entries.
#
# A per-user copy in ~/.config/autostart with `niri` added to NotShowIn
# overrides the system file for this desktop only: a Plasma session on the
# same machine still starts them. Idempotent; an existing user copy is edited
# in place rather than replaced. To undo, delete ~/.config/autostart/NAME.desktop.
set -euo pipefail

dir=${XDG_CONFIG_HOME:-$HOME/.config}/autostart
mkdir -p "$dir"
for name in "$@"; do
  sys=/etc/xdg/autostart/$name.desktop
  usr=$dir/$name.desktop
  [[ -f $usr ]] || { [[ -f $sys ]] || continue; cp "$sys" "$usr"; }
  if grep -q '^NotShowIn=' "$usr"; then
    grep -q '^NotShowIn=.*\bniri;' "$usr" || sed -i 's/^NotShowIn=\(.*\)$/NotShowIn=\1niri;/; s/;;niri;/;niri;/' "$usr"
  else
    # Inside the [Desktop Entry] group: right after its header.
    sed -i '0,/^\[Desktop Entry\]/s//[Desktop Entry]\nNotShowIn=niri;/' "$usr"
  fi
  # An OnlyShowIn naming niri would contradict it; the spec says NotShowIn
  # wins, but leave nothing ambiguous.
  sed -i '/^OnlyShowIn=/s/niri;//' "$usr"
  echo "    $name: not autostarted in niri (~/.config/autostart/$name.desktop)"
done
