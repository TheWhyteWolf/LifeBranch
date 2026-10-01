#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# packages.sh: sourced by install.sh and macbook/install.sh. One package list
# for both machines, so they stop drifting (the macbook list had already lost
# clang, which lifelock's build needs). Not executable on its own; `source` it.
#
# Every external command lifeconf or a script calls must come from a package
# here, not from whatever happened to be installed already: a fresh Arch box has
# none of wpctl, bluetoothctl, powerprofilesctl... and the Settings panels just
# say "unavailable". scripts/check-deps.sh enforces that in CI.

# niri itself heads the list: the installer validates a niri config at the end
# and the whole thing is useless without the compositor.
# rust is not optional either — the life* components are built from source and
# wired into the config. clang comes with it: lifelock's pam-client dependency
# pulls in bindgen, whose clang-sys build script panics without libclang.so.
# kitty is the terminal the whole rice assumes: Mod+T, the Ctrl+Alt+Return
# recovery bind, the waybar htop clicks, the cheat-sheet window and the
# `kitten panel` Game of Life wallpaper all need it.
# qt6-wayland/qt5-wayland are the Qt Wayland platform plugins — the actual fix
# for drag-and-drop. Without the plugin Qt falls back to XWayland, and
# xwayland-satellite can't bridge DnD across the X11/Wayland boundary
# (Supreeeme/xwayland-satellite#133), so drags out of Dolphin die at the border.
# The platform variables that go with them live in
# environment.d/50-niri-platform.conf, which documents the trade-off.
# shellcheck disable=SC2034  # read by the sourcing installer
PKGS=(niri rust clang
      kitty fuzzel waybar mako xwayland-satellite wl-clipboard cliphist wev
      adw-gtk-theme wob jq
      swaylock swayidle ttf-sharetech-mono-nerd ttf-cousine-nerd
      xdg-desktop-portal-gnome qt6ct qt6-wayland qt5-wayland
      network-manager-applet blueman
      polkit-kde-agent udiskie wlsunset wf-recorder playerctl
      # Plumbing the Settings panels and scripts call into. niri only pulls in
      # libpipewire, so the sound server itself has to be asked for.
      pipewire pipewire-pulse wireplumber   # wpctl: Sound panel, vol-osd.sh
      networkmanager                        # nmcli: Network panel, net-menu.sh
      bluez bluez-utils                     # bluetoothctl: Bluetooth panel
      udisks2                               # udisksctl: lifepanel's drives
      htop                                  # the bar's CPU/MEM click
      power-profiles-daemon upower brightnessctl  # Power panel, bright-osd.sh
      libnotify                             # notify-send: the scripts' feedback
      xdg-utils                             # xdg-mime: Default apps panel
      # greetd/pam/greetd unlocks this at login; without the package those
      # lines skip silently and Brave/Element have nowhere to keep logins.
      gnome-keyring libsecret
      # Fallback fonts: without them emoji and CJK text render as boxes. Only
      # glyphs actually used get loaded, so the cost is disk, not RAM.
      noto-fonts noto-fonts-emoji noto-fonts-cjk
      # Everyday applications.
      nano dolphin libreoffice-fresh element-desktop kleopatra)
#     libreoffice-fresh is the current release; swap in libreoffice-still on
#     older/slower hardware — it is the same suite, a version behind.

# AUR. Kept separate so the bulk of the install goes through pacman directly
# (faster, and a build failure here names itself instead of taking the lot down).
# shellcheck disable=SC2034
AUR_PKGS=(phinger-cursors vesktop-bin)

# "wanted:installed-rival" pairs that pacman would answer with a remove-the-
# other-one prompt. Its default is N, which aborts the WHOLE transaction — one
# existing tlp would leave a fresh machine with nothing installed. Someone who
# chose pulseaudio or tlp keeps it; the panel that needs the skipped package
# says so itself.
CONFLICTS=(pipewire-pulse:pulseaudio
           power-profiles-daemon:tlp
           power-profiles-daemon:auto-cpufreq
           power-profiles-daemon:tuned-ppd)

# drop_conflicts: remove from PKGS anything whose rival is already installed.
drop_conflicts() {
  local pair want rival keep=() p skip
  local -A drop=()
  for pair in "${CONFLICTS[@]}"; do
    want=${pair%%:*} rival=${pair#*:}
    if pacman -Qq "$rival" >/dev/null 2>&1 && ! pacman -Qq "$want" >/dev/null 2>&1; then
      drop[$want]=1
      echo "    keeping your $rival, so skipping $want (they conflict)"
    fi
  done
  for p in "${PKGS[@]}"; do
    skip=${drop[$p]:-}
    [[ -n $skip ]] || keep+=("$p")
  done
  PKGS=("${keep[@]}")
}

# enable_services: the system daemons the panels talk to. Installing a package
# does not start its service on Arch.
enable_services() {
  local other
  # NetworkManager only when nothing else already manages the network: two
  # managers fighting over one interface is a machine that drops off wifi.
  if ! systemctl is-enabled -q NetworkManager.service 2>/dev/null; then
    for other in systemd-networkd connman netctl; do
      if systemctl is-enabled -q "$other.service" 2>/dev/null; then
        echo "    $other is managing the network — leaving NetworkManager off"
        echo "      (the Network panel and wifi menu need it; switch by hand if you want them)"
        other=found
        break
      fi
    done
    if [[ $other != found ]]; then
      sudo systemctl enable --now NetworkManager.service && echo "    enabled NetworkManager"
    fi
  fi
  # bluetooth.service is harmless without an adapter; it just idles.
  if pacman -Qq bluez >/dev/null 2>&1 && ! systemctl is-enabled -q bluetooth.service 2>/dev/null; then
    sudo systemctl enable --now bluetooth.service && echo "    enabled bluetooth"
  fi
  # D-Bus can activate power-profiles-daemon, but only an enabled unit restores
  # the chosen profile at boot.
  if pacman -Qq power-profiles-daemon >/dev/null 2>&1 \
     && ! systemctl is-enabled -q power-profiles-daemon.service 2>/dev/null; then
    sudo systemctl enable --now power-profiles-daemon.service && echo "    enabled power-profiles-daemon"
  fi
  # upower is D-Bus activated; pipewire and wireplumber are socket/preset-
  # enabled user units. Nothing to do for them.
}
