#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Bluetooth on T2 Macs, the facts (2019 MBP16,1 / BCM4364B3 "Trinidad"):
#
# - The chip is UART-attached and runs its ROM firmware. The kernel's
#   "BCM: firmware Patch file not found, tried: 'brcm/BCM.hcd'" line is
#   EXPECTED and harmless — t2linux ships no patchram for UART BCM4364,
#   and macOS only carries it as unconverted Intel-HEX (MiniDriver/Updater
#   pairs under /usr/share/firmware/bluetooth on the APFS System volume).
#   Converting those with hex2hcd is undocumented for this chip and a bad
#   patchram can wedge the controller until reboot — not worth it while
#   ROM firmware scans/pairs fine.
# - Only MacBookPro15,4 / MacBookPro16,3 / MacBookAir9,1 (PCIe BCM4377,
#   hci_bcm4377) need real firmware extraction — the t2linux firmware.sh
#   flow (https://wiki.t2linux.org/guides/wifi-bluetooth/) covers them.
# - The usual reason BT looks dead here is an rfkill soft-block, which
#   systemd-rfkill then faithfully restores on every boot.
#
# So: unblock, power on, verify. Idempotent; run as root (or with rfkill
# group access).
set -uo pipefail

rfkill unblock bluetooth 2>/dev/null || true
systemctl start bluetooth 2>/dev/null || true
sleep 1

if bluetoothctl show 2>/dev/null | grep -q 'PowerState: on'; then
  echo "t2-bt: controller powered on (ROM firmware — the missing-BCM.hcd dmesg line is normal)"
else
  bluetoothctl power on >/dev/null 2>&1 || true
  sleep 1
  if bluetoothctl show 2>/dev/null | grep -q 'PowerState: on'; then
    echo "t2-bt: controller powered on"
  else
    echo "t2-bt: controller still off — check 'rfkill list' and 'bluetoothctl show'" >&2
    exit 1
  fi
fi
exit 0
