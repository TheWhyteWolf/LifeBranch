#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# T2 suspend module lifecycle (2019 MBP16,1). Run by t2-suspend-fix.service:
#   pre  — before sleep.target: stop tiny-dfr, drop the Touch Bar HID stack and
#          brcmfmac so they can't wedge suspend entry or come back dead.
#   post — on resume: reload in the order the hardware needs, with the delays
#          the T2 needs to re-enumerate, then re-enumerate the Touch Bar USB
#          device and restart tiny-dfr.
# NEVER unload apple_bce here: the T2 sound card pins it and rmmod hangs
# suspend entry (t2linux wiki's old advice predates that finding).
set -uo pipefail

# Touch Bar Display is 05ac:8302 on the T2's internal bus.
tb_dev() {
  local d
  for d in /sys/bus/usb/devices/*; do
    [[ -f "$d/idVendor" && -f "$d/idProduct" ]] || continue
    [[ "$(cat "$d/idVendor")" == "05ac" && "$(cat "$d/idProduct")" == "8302" ]] && { echo "$d"; return; }
  done
}

case "${1:-}" in
  pre)
    systemctl stop tiny-dfr 2>/dev/null || true
    rmmod hid_appletb_kbd 2>/dev/null || true
    rmmod hid_appletb_bl  2>/dev/null || true
    rmmod appletbdrm      2>/dev/null || rmmod -f appletbdrm 2>/dev/null || true
    rmmod brcmfmac_wcc    2>/dev/null || true
    rmmod brcmfmac        2>/dev/null || true
    ;;
  post)
    modprobe brcmfmac || true
    sleep 4
    modprobe appletbdrm 2>/dev/null || true
    modprobe hid_appletb_bl || true
    sleep 2
    modprobe hid_appletb_kbd || true
    # Re-enumerate the Touch Bar USB device (works around the kernel-7.x
    # resume race that leaves it -ETIMEDOUT dead).
    d="$(tb_dev)"
    if [[ -n "${d:-}" && -w "$d/bConfigurationValue" ]]; then
      cfg="$(cat "$d/bConfigurationValue" 2>/dev/null)"
      [[ -n "$cfg" ]] || cfg=2
      echo 0     > "$d/bConfigurationValue" 2>/dev/null || true
      sleep 1
      echo "$cfg" > "$d/bConfigurationValue" 2>/dev/null || true
    fi
    sleep 3
    systemctl start tiny-dfr 2>/dev/null || true
    ;;
  *)
    echo "usage: $0 pre|post" >&2
    exit 1
    ;;
esac
exit 0
