#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Wi-Fi menu: nmcli + fuzzel (notif-menu.sh pattern). Pick a network to join it;
# a secured network you haven't joined before asks for its password in a masked
# fuzzel prompt and hands it to nmcli on STDIN — never as an argument, where
# every process on the machine could read it from /proc.
# Bound to the waybar NET label. lifeconf's Network panel does the same minus
# the new-secured-network case, which is why this exists.
set -euo pipefail

# lifemenu (LifeBranch's own) with fuzzel as the fallback; same flags.
menu=$(command -v lifemenu || command -v "$HOME/.local/bin/lifemenu" || echo fuzzel)

say() { notify-send -a net-menu "$1" "${2:-}" 2>/dev/null || true; }

if [ "$(nmcli radio wifi)" != enabled ]; then
  choice="$(printf '%s\n' 'turn wifi on' | "$menu" --dmenu --prompt 'wifi> ' --width 40)" || exit 0
  [ -n "$choice" ] && nmcli radio wifi on && say "wifi on"
  exit 0
fi

# SSID is the last field and may contain ':', so don't let nmcli escape and
# take the remainder of the line as the name.
declare -a ssids secs lines
declare -A seen
while IFS=: read -r inuse sig sec ssid; do
  [ -n "$ssid" ] || continue          # hidden network
  [ -z "${seen[$ssid]:-}" ] || continue # one row per SSID (strongest first)
  seen[$ssid]=1
  ssids+=("$ssid"); secs+=("$sec")
  mark=' '; [ "$inuse" = '*' ] && mark='*'
  lines+=("$mark $ssid  ${sig}%  ${sec:-open}")
done < <(nmcli --escape no -t -f IN-USE,SIGNAL,SECURITY,SSID device wifi list --rescan auto \
           | LC_ALL=C sort -t: -k1,1r -k2,2nr)

[ "${#ssids[@]}" -gt 0 ] || { say "no wifi networks found"; exit 0; }

idx="$(printf '%s\n' "${lines[@]}" | "$menu" --dmenu --index --prompt 'wifi> ' --width 60)" || exit 0
[ -n "$idx" ] || exit 0
ssid="${ssids[$idx]}"; sec="${secs[$idx]}"
case "$ssid" in -*) say "refusing odd network name"; exit 1 ;; esac

# A saved profile needn't be named after its SSID ("Home 1"), so map each saved
# *wifi* profile to the SSID it joins instead of matching on the name.
declare -A profile
while IFS= read -r row; do
  [ "${row##*:}" = 802-11-wireless ] || continue
  name="${row%:*}"
  s="$(nmcli --escape no -g 802-11-wireless.ssid connection show id "$name" 2>/dev/null)" || continue
  [ -n "$s" ] && profile[$s]="$name"
done < <(nmcli --escape no -t -f NAME,TYPE connection show)

if [ -n "${profile[$ssid]:-}" ]; then
  out="$(nmcli -w 15 connection up id "${profile[$ssid]}" 2>&1)" || { say "could not join $ssid" "${out##*$'\n'}"; exit 1; }
elif [ -z "$sec" ] || [ "$sec" = -- ]; then
  out="$(nmcli -w 15 device wifi connect "$ssid" 2>&1)" || { say "could not join $ssid" "${out##*$'\n'}"; exit 1; }
else
  pw="$("$menu" --dmenu --password --lines 0 --prompt "password for $ssid> " --width 50 </dev/null)" || exit 0
  [ -n "$pw" ] || exit 0
  out="$(printf '%s\n' "$pw" | nmcli --ask -w 15 device wifi connect "$ssid" 2>&1)" \
    || { say "could not join $ssid" "wrong password or out of range"; exit 1; }
  unset pw
fi
say "connected to $ssid"
