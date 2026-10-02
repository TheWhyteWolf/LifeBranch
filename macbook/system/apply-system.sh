#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Root-level T2 system setup for the 2019 MBP (t2arch). Idempotent — safe to
# re-run. Everything it installs lives next to it in macbook/system/ (git).
#
#   sudo bash macbook/system/apply-system.sh
#
# What it does (see macbook/README.md "System plumbing"):
#   1. installs the sleep hook + drop-ins + udev/NM/pipewire rules
#   2. unmasks suspend (s2idle), keeps hibernate dead, lid = suspend
#   3. UPower critical battery -> clean poweroff; saner faillock limits
#   4. kernel cmdline: mem_sleep_default=s2idle pm_async=off (+ runtime apply)
#   5. packages: pipewire-alsa, jack2 -> pipewire-jack, tray/laptop bits;
#      removes plymouth (segfaulted every boot; splash was invisible anyway)
#   6. mkinitcpio: drop the stale apple-bce module, no plymouth hook; grub
#   7. network: silence the T2 NCM iface, drop stale/dupe NM profiles
#   8. Bluetooth firmware from the macOS APFS volume (best-effort)
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[[ $EUID -eq 0 ]] || { echo "run me with sudo"; exit 1; }

echo "==> [1/8] System config files"
install -Dm755 "$DIR/t2-sleep.sh"             /usr/local/lib/t2/t2-sleep.sh
install -Dm755 "$DIR/t2-bt-firmware.sh"       /usr/local/lib/t2/t2-bt-firmware.sh
install -Dm644 "$DIR/t2-suspend-fix.service"  /etc/systemd/system/t2-suspend-fix.service
install -Dm644 "$DIR/sleep-t2.conf"           /etc/systemd/sleep.conf.d/t2.conf
install -Dm644 "$DIR/logind-t2.conf"          /etc/systemd/logind.conf.d/t2.conf
install -Dm644 "$DIR/99-touchbar-power.rules" /etc/udev/rules.d/99-touchbar-power.rules
install -Dm644 "$DIR/99-network-t2-ncm.rules" /etc/udev/rules.d/99-network-t2-ncm.rules
install -Dm644 "$DIR/nm-t2-ncm.conf"          /etc/NetworkManager/conf.d/99-network-t2-ncm.conf
install -Dm644 "$DIR/pipewire-t2-rate.conf"   /etc/pipewire/pipewire.conf.d/10-t2-rate.conf

echo "==> [2/8] Sleep targets: suspend on, hibernate stays dead, lid = suspend"
systemctl unmask sleep.target suspend.target
systemctl mask hibernate.target hybrid-sleep.target 2>/dev/null || true
# The lid lines were hand-set to ignore in the main logind.conf; comment them
# out so the logind.conf.d/t2.conf drop-in (lid = suspend) wins.
sed -i -E 's/^(HandleLidSwitch(ExternalPower|Docked)?=)/#\1/' /etc/systemd/logind.conf
systemctl daemon-reload
systemctl enable t2-suspend-fix.service
# (logind picks the lid drop-in up on reboot; not restarting it here to avoid
# yanking device access from the live Wayland session)

echo "==> [3/8] UPower critical action + faillock limits"
sed -i 's/^CriticalPowerAction=.*/CriticalPowerAction=PowerOff/' /etc/UPower/UPower.conf
systemctl restart upower 2>/dev/null || true
for kv in "deny = 10" "unlock_time = 60"; do
  k="${kv%% *}"
  if grep -qE "^\s*${k}\s*=" /etc/security/faillock.conf; then
    sed -i -E "s/^\s*${k}\s*=.*/${kv}/" /etc/security/faillock.conf
  else
    echo "$kv" >> /etc/security/faillock.conf
  fi
done

echo "==> [4/8] Kernel cmdline (s2idle, pm_async=off, no splash)"
# Add only what is missing, never replace the line: it may carry options this
# script knows nothing about (cryptdevice=, rd.luks.*, resume=) and dropping
# one of those is a machine that no longer boots. A key that is already set
# to something else is left alone and reported. Backed up before any edit.
T2_OPTS=(intel_iommu=on iommu=pt pcie_ports=compat mem_sleep_default=s2idle pm_async=off)
if [[ ! -f /etc/default/grub ]]; then
  echo "    no /etc/default/grub (not GRUB?) — add these to your bootloader's kernel"
  echo "    command line by hand: ${T2_OPTS[*]}"
elif ! grep -q '^GRUB_CMDLINE_LINUX=".*"$' /etc/default/grub; then
  echo "    GRUB_CMDLINE_LINUX in /etc/default/grub is not a plain \"...\" line — add by hand: ${T2_OPTS[*]}"
else
  cur=$(sed -n 's/^GRUB_CMDLINE_LINUX="\(.*\)"$/\1/p' /etc/default/grub | tail -n1)
  new=$cur
  for opt in "${T2_OPTS[@]}"; do
    key=${opt%%=*}
    if [[ " $new " == *" $opt "* ]]; then
      continue
    elif [[ " $new " == *" $key="* ]]; then
      echo "    keeping your own $key= (wanted $opt)"
    else
      new="${new:+$new }$opt"
    fi
  done
  if [[ $new != "$cur" ]]; then
    bak="/etc/default/grub.lifebranch-$(date +%Y%m%d-%H%M%S)"
    cp /etc/default/grub "$bak"
    # | as the delimiter; escape the replacement's own specials (& \ |).
    esc=$(printf '%s' "$new" | sed 's/[&\\|]/\\&/g')
    sed -i "s|^GRUB_CMDLINE_LINUX=\".*\"\$|GRUB_CMDLINE_LINUX=\"$esc\"|" /etc/default/grub
    echo "    GRUB_CMDLINE_LINUX=\"$new\" (previous file: $bak)"
  else
    echo "    kernel cmdline already has the T2 options"
  fi
fi
# apply at runtime too so suspend can be tested before the reboot
echo s2idle > /sys/power/mem_sleep
echo 0     > /sys/power/pm_async

echo "==> [5/8] Packages"
if pacman -Qq plymouth &>/dev/null; then
  pacman -Rns --noconfirm plymouth || pacman -Rdd --noconfirm plymouth
fi
if pacman -Qq jack2 &>/dev/null; then
  pacman -Rdd --noconfirm jack2   # dep-less removal; pipewire-jack provides jack next
fi
# Not the tray applets or udiskie: lifepanel replaced them, and the installer's
# offer_remove_legacy removes them just before this script runs.
pacman -S --needed --noconfirm pipewire-alsa pipewire-jack \
  wlsunset wf-recorder playerctl \
  || { echo "pacman failed (stale mirrors?) — run 'pacman -Syu', then re-run this script"; exit 1; }
systemctl enable t2fanrd tiny-dfr 2>/dev/null || true

echo "==> [6/8] mkinitcpio (no plymouth) + grub"
# No T2 module goes in MODULES=(). This used to add apple-bce, which upstream
# has since renamed to t2bce_{core,dma,audio,vhci} — so mkinitcpio failed with
# "module not found: 'apple_bce'", and because of set -e that aborted this
# script before grub-mkconfig and steps 7-8 ever ran. Naming the t2bce modules
# instead would fail too: mkinitcpio -P builds a preset for EVERY installed
# kernel from this one config, and the stock `linux` package ships none of
# them. Nothing needs them early anyway — HOOKS carries no encrypt hook, so
# there is no boot-time prompt wanting the T2 keyboard before the root
# filesystem mounts; the drivers load from disk in the normal way.
sed -i -E 's/^MODULES=\(apple-bce\)$/MODULES=()/' /etc/mkinitcpio.conf
sed -i -E 's/^(HOOKS=.*) plymouth(.*)/\1\2/' /etc/mkinitcpio.conf
mkinitcpio -P
if command -v grub-mkconfig >/dev/null 2>&1 && [[ -f /boot/grub/grub.cfg ]]; then
  grub-mkconfig -o /boot/grub/grub.cfg
else
  echo "    no GRUB here — regenerate your bootloader entries yourself"
fi

echo "==> [7/8] Network cleanup"
udevadm control --reload
# Drop the wired profile NetworkManager auto-created for the T2's internal NCM
# interface before 99-network-t2-ncm.rules named it — and only that one: it is
# found by the interface's fixed MAC or its new name, never by a profile name
# like "Wired connection 1", which on another machine is a real connection.
T2_NCM_MAC=ac:de:48:00:11:22
nmcli -t -f UUID,TYPE connection show 2>/dev/null \
  | while IFS=: read -r uuid type; do
      [[ $type == 802-3-ethernet ]] || continue
      bound=$(nmcli --escape no -g connection.interface-name,802-3-ethernet.mac-address \
                connection show "$uuid" 2>/dev/null | tr 'A-Z' 'a-z' | tr '\n' ' ')
      if [[ " $bound " == *" t2_ncm "* || " $bound " == *" $T2_NCM_MAC "* ]]; then
        nmcli connection delete "$uuid" >/dev/null 2>&1 && echo "    removed the T2 NCM wired profile ($uuid)"
      fi
    done || true

echo "==> [8/8] Bluetooth (rfkill unblock; ROM firmware is normal on this chip)"
bash "$DIR/t2-bt-firmware.sh" || echo "    (Bluetooth still off — see message above; everything else is done)"

cat <<'EOF'

==> Done. Next steps:
    - reboot once to pick up the new initramfs/cmdline (recommended before
      the first suspend test)
    - suspend test: 'systemctl suspend', wake with the keyboard/lid; check
      Wi-Fi, sound, and the Touch Bar afterwards
    - hibernate stays disabled on purpose: the T2 cannot survive it
EOF
