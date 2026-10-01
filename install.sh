#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# LifeBranch: install the rice. Packages, symlinks and hardware detection.
# Idempotent — safe to re-run. Needs your sudo password for the package step.
#
#     bash ~/LifeBranch/install.sh
#
# It asks first whether to run in easy mode (take the recommended answer to
# everything) or full control (ask about each step). To skip that question:
#
#     LIFEBRANCH_EASY=1 bash ~/LifeBranch/install.sh   # easy
#     LIFEBRANCH_EASY=0 bash ~/LifeBranch/install.sh   # full control
#
# Arch (or an Arch derivative) only: everything here goes through pacman/yay.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# A `set -e` exit prints nothing at all, which is how a failed install ends up
# described as "it didn't appear to do anything". Name the line that stopped,
# and point at bootstrap.sh's transcript when there is one.
fail_line=

on_exit() {
  local rc=$? log
  (( rc )) || return 0
  echo
  if [[ -n $fail_line ]]; then
    echo "!! install.sh stopped at line $fail_line (exit $rc)."
  else
    echo "!! install.sh stopped (exit $rc)."
  fi
  echo "   Fix what the error above says and run it again: this installer is"
  echo "   idempotent, so a second run picks up where this one stopped."
  log=${LIFEBRANCH_LOG:-$HOME/lifebranch-install.log}
  [[ $log != none && -f $log ]] && echo "   Transcript: $log"
  return 0
}

trap 'fail_line=$LINENO' ERR
trap on_exit EXIT

# --- Preflight ---------------------------------------------------------------
if ! command -v pacman >/dev/null 2>&1; then
  echo "!! LifeBranch installs with pacman — this does not look like an Arch system." >&2
  exit 1
fi
if [[ $(id -u) -eq 0 ]]; then
  echo "!! Run this as your own user, not as root: it sudos where it needs to," >&2
  echo "   and everything else belongs in YOUR home directory." >&2
  exit 1
fi

# Easy or full control. Sets EASY, CONFIRM (pacman/yay --noconfirm in easy
# mode) and interactive(), and exports LIFEBRANCH_EASY so the scripts this one
# calls make the same choice.
# shellcheck source=scripts/prompt.sh
source "$REPO/scripts/prompt.sh"
pick_mode

# An AUR helper, bootstrapped by curl if it is missing: the extra packages
# asked for below may be AUR names. No-op when yay is already installed.
bash "$REPO/scripts/ensure-yay.sh"

# --- Packages ----------------------------------------------------------------
# The list lives in scripts/packages.sh, shared with the other installer, along
# with why each package is there.
# shellcheck source=scripts/packages.sh
source "$REPO/scripts/packages.sh"
drop_conflicts

echo "==> Installing packages from the official repos"
sudo pacman -S --needed "${CONFIRM[@]}" "${PKGS[@]}"

if (( ! ${#AUR_PKGS[@]} )); then
  :  # nothing the desktop needs is AUR-only at the moment
elif command -v yay >/dev/null 2>&1; then
  echo "==> Installing AUR packages: ${AUR_PKGS[*]}"
  clear_orphan_debug
  # Not fatal: a package that will not build is no reason
  # to abandon the rest of the desktop. Retry later with yay -S <name>.
  if ! yay -S --needed "${CONFIRM[@]}" "${AUR_PKGS[@]}"; then
    echo "    !! AUR install failed (see above); carrying on without: ${AUR_PKGS[*]}"
    echo "       retry later with: yay -S ${AUR_PKGS[*]}"
  fi
else
  echo "    !! no AUR helper — skipping ${AUR_PKGS[*]}; install them later with yay."
fi

echo "==> Enabling the system services the Settings panels use"
enable_services
offer_tailscale_operator
offer_snapshots

# Anything else this particular person wants, while we already have their
# attention and a working AUR helper. Easy mode installs nothing extra: the
# desktop is complete without it, and anything can be added later with
# `yay -S <package>`.
if interactive; then
  echo
  echo "==> Anything else you want installed?"
  echo "    Both repos and the AUR are available. Common picks:"
  echo "      firefox thunderbird vlc gimp obsidian spotify steam"
  echo "      signal-desktop keepassxc syncthing btop neovim git-delta"
  read -rp "    Packages (space-separated, blank to skip): " -a extra_pkgs || extra_pkgs=()
  wanted=()
  for p in "${extra_pkgs[@]:-}"; do
    [[ -z $p ]] && continue
    # Package names only: no flags, no shell metacharacters slipping through.
    if [[ $p =~ ^[a-zA-Z0-9][a-zA-Z0-9@._+-]*$ ]]; then
      wanted+=("$p")
    else
      echo "    skipping '$p' — that is not a package name."
    fi
  done
  # Same conflict checks as the main list: a rival you already have is kept.
  WANTED=("${wanted[@]}")
  (( ${#WANTED[@]} )) && filter_wanted
  wanted=("${WANTED[@]}")
  if (( ${#wanted[@]} )); then
    echo "    installing: ${wanted[*]}"
    # Never fatal: a typo in this list must not abort the whole install.
    if command -v yay >/dev/null 2>&1; then
      yay -S --needed "${wanted[@]}" || echo "    !! some of those did not install; carrying on."
    else
      sudo pacman -S --needed "${wanted[@]}" || echo "    !! some of those did not install; carrying on."
    fi
  fi
fi

# --- Config symlinks ---------------------------------------------------------
# link SRC DST — back up a real file at DST to DST.bak, then symlink. The first
# .bak is the pre-LifeBranch original and is never overwritten: a later real
# file at DST (a re-run after something replaced the link) goes to a
# timestamped DST.bak.<epoch> instead.
link() {
  local src="$1" dst="$2" bak
  mkdir -p "$(dirname "$dst")"
  if [[ -e "$dst" && ! -L "$dst" ]]; then
    bak="$dst.bak"
    [[ -e $bak || -L $bak ]] && bak="$dst.bak.$(date +%s)"
    echo "    backing up $dst -> $bak"
    mv "$dst" "$bak"
  fi
  ln -sfn "$src" "$dst"
  echo "    linked $dst -> $src"
}

echo "==> Symlinking configs into ~/.config"
# The five lifeconf-generated theme files are gitignored build artifacts, so a
# fresh clone carries neither the files nor — for fuzzel/ and swaylock/, whose
# only tracked content WAS the generated file — the directories themselves.
# Create them before linking: the symlinks below point into the repo, and
# `lifeconf --apply` writes through them, so a missing directory turns into a
# write failure and a dangling symlink (unthemed bar/launcher/locker).
mkdir -p "$REPO/fuzzel" "$REPO/kitty" "$REPO/lifenote" "$REPO/swaylock"
link "$REPO/niri/config.kdl"     "$HOME/.config/niri/config.kdl"
link "$REPO/fuzzel/fuzzel.ini"   "$HOME/.config/fuzzel/fuzzel.ini"
link "$REPO/lifenote/config"     "$HOME/.config/lifenote/config"
link "$REPO/kitty/rice.conf"     "$HOME/.config/kitty/rice.conf"
link "$REPO/kitty/olive.conf"    "$HOME/.config/kitty/olive.conf"
link "$REPO/tmpfiles/kitty.conf" "$HOME/.config/user-tmpfiles.d/kitty.conf"
link "$REPO/swaylock/config"     "$HOME/.config/swaylock/config"
link "$REPO/xdg/portals.conf"    "$HOME/.config/xdg-desktop-portal/portals.conf"
link "$REPO/qt6ct/qt6ct.conf"    "$HOME/.config/qt6ct/qt6ct.conf"
# Wayland platform vars: environment.d so the systemd user manager exports them
# to D-Bus-activated and systemd-launched apps, not just to niri's own children.
link "$REPO/environment.d/50-niri-platform.conf" \
     "$HOME/.config/environment.d/50-niri-platform.conf"
# Brave is Chromium, so it ignores ELECTRON_OZONE_PLATFORM_HINT and needs its
# own flags file to stay pinned to Wayland.
link "$REPO/brave/brave-flags.conf" "$HOME/.config/brave-flags.conf"

# Wire rice.conf into kitty.conf (appended -> last-wins over the stock config).
KITTY_CONF="$HOME/.config/kitty/kitty.conf"
touch "$KITTY_CONF"
if ! grep -qxF 'include rice.conf' "$KITTY_CONF"; then
  printf '\n# LifeBranch extras (transparency; managed in the repo).\ninclude rice.conf\n' >> "$KITTY_CONF"
  echo "    appended 'include rice.conf' to $KITTY_CONF"
fi

# kitty listen_on points into $XDG_RUNTIME_DIR, which is tmpfs and has no
# kitty/ dir; kitty will not create it. Without the tmpfiles rule linked above,
# every kitty start prints "Invalid listen_on=..., ignoring". Apply it now so
# this session works before the next boot — scoped to the one file just
# installed, and NOT silenced: over SSH or on a TTY there may be no user
# session for %t to resolve against, and swallowing that error would leave the
# installer claiming a directory it never created.
if systemd-tmpfiles --user --create "$HOME/.config/user-tmpfiles.d/kitty.conf"; then
  echo "    created ${XDG_RUNTIME_DIR:-\$XDG_RUNTIME_DIR}/kitty (kitty remote-control sockets)"
else
  echo "    note: the kitty socket dir was not created now — it will be at next login."
fi

echo "==> Installing scripts into ~/.local/bin"
# One list drives both chmod and symlink. life.py stays outside: chmod'd here
# but only linked (as lifebg) in the no-cargo fallback below.
SCRIPTS=(clip-menu.sh power-menu.sh lifebg-toggle.sh vol-osd.sh
         dnd-toggle.sh float-snap.sh scratch-term.sh notif-menu.sh net-menu.sh
         rec-toggle.sh pinentry-fuzzel.sh shortcuts-window.sh sysmon.sh
         detect-trackpad.sh setup-locale.sh lite-profile.sh idle-suspend.sh)
chmod +x "$REPO/scripts/life.py"
for s in "${SCRIPTS[@]}"; do
  chmod +x "$REPO/scripts/$s"
  link "$REPO/scripts/$s" "$HOME/.local/bin/$s"
done

echo "==> GPG passphrase prompts (pinentry-fuzzel: installed, NOT enabled)"
# Deliberately opt-in. pinentry-fuzzel routes passphrase prompts through fuzzel,
# which takes an *exclusive* layer-shell keyboard grab: if the prompt ever wedges,
# it takes the whole session's input with it and the only way out is a VT switch
# (Ctrl+Alt+F3). That is not something to switch on unattended from an installer,
# so the script is linked into ~/.local/bin and left inert.
#
# To enable, after testing it standalone (see scripts/pinentry-fuzzel.sh header):
#     echo "pinentry-program $HOME/.local/bin/pinentry-fuzzel.sh" >> ~/.gnupg/gpg-agent.conf
#     gpgconf --kill gpg-agent
# To back out, delete that line and run gpgconf --kill gpg-agent.
echo "    linked ~/.local/bin/pinentry-fuzzel.sh (inert until gpg-agent.conf points at it)"


# --- Hardware and locale -----------------------------------------------------
# Everything in this section is a fenced LIFEBRANCH:BEGIN region in the niri
# config: rewritten in place, hand-editable afterwards, and validated before it
# is kept. write_region restores the previous file if the result does not parse.
# shellcheck source=scripts/config-region.sh
source "$REPO/scripts/config-region.sh"
NIRI_CFG="$HOME/.config/niri/config.kdl"

# Nothing about a touchpad is guessable: tap-to-click, two-finger scrolling and
# where the right button lives all depend on what the hardware reports. Read it
# and offer the matching config rather than shipping someone else's laptop's.
echo "==> Looking for a touchpad"
if bash "$REPO/scripts/detect-trackpad.sh" && has_region "$(readlink -f "$NIRI_CFG")" touchpad; then
  tp_block=$(mktemp)
  bash "$REPO/scripts/detect-trackpad.sh" --niri-block > "$tp_block"
  echo
  echo "    Proposed niri settings:"
  sed 's/^/    /' "$tp_block"
  echo
  if ask_yn "Write these into your niri config?" y; then
    # Never fatal: write_region has already put the previous config back, and
    # a block that will not validate is no reason to abandon the whole install.
    write_region "$NIRI_CFG" touchpad "$tp_block" niri validate --config || true
  fi
  rm -f "$tp_block"
fi

# The keyboard layout is the one setting that is wrong in the worst possible
# place if it is wrong: at the login password prompt, before you can read any
# documentation about it. The shipped config pins a UK board; ask, defaulting
# to whatever the Arch install already configured.
echo "==> Keyboard layout and location"
det_layout=us det_variant='' det_tz='' det_lat='' det_lon='' det_numlock=0
while IFS='=' read -r k v; do
  case $k in
    layout)          det_layout=$v ;;
    variant)         det_variant=$v ;;
    timezone)        det_tz=$v ;;
    lat)             det_lat=$v ;;
    lon)             det_lon=$v ;;
    numlock_default) det_numlock=$v ;;
  esac
done < <(bash "$REPO/scripts/setup-locale.sh" --detect)

kb_layout=$det_layout kb_variant=$det_variant
kb_numlock=$det_numlock kb_lat=$det_lat kb_lon=$det_lon
echo "    Detected from this system: layout '${det_layout}'${det_variant:+ (variant ${det_variant})}, timezone ${det_tz:-unknown}"
if interactive; then
  read -rp "    Keyboard layout [${det_layout}]: " a; kb_layout=${a:-$det_layout}
  read -rp "    Layout variant, or 'none' [${det_variant:-none}]: " a
  case ${a:-keep} in
    keep) kb_variant=$det_variant ;;
    none) kb_variant='' ;;
    *)    kb_variant=$a ;;
  esac
  if (( det_numlock )); then
    read -rp "    Start with NumLock on (full-size keyboard)? [Y/n] " a
    [[ ${a:-Y} =~ ^[Yy]?$ ]] && kb_numlock=1 || kb_numlock=0
  else
    read -rp "    Start with NumLock on (only if you have a numpad)? [y/N] " a
    [[ ${a:-N} =~ ^[Yy]$ ]] && kb_numlock=1 || kb_numlock=0
  fi
  if [[ -n $det_lat && -n $det_lon ]]; then
    echo "    Night light needs your rough latitude/longitude (from your timezone)."
    read -rp "    Coordinates 'lat lon' [${det_lat} ${det_lon}]: " a
    if [[ -n $a ]]; then
      read -r in_lat in_lon <<<"$a"
      if [[ $in_lat =~ ^-?[0-9]+(\.[0-9]+)?$ && $in_lon =~ ^-?[0-9]+(\.[0-9]+)?$ ]]; then
        kb_lat=$in_lat kb_lon=$in_lon
      else
        echo "    not two numbers — keeping ${det_lat} ${det_lon}"
      fi
    fi
  fi
fi

if has_region "$(readlink -f "$NIRI_CFG")" keyboard; then
  kb_block=$(mktemp)
  bash "$REPO/scripts/setup-locale.sh" --keyboard-block \
       "$kb_layout" "$kb_variant" pc105 "$kb_numlock" > "$kb_block"
  echo "    keyboard: layout '${kb_layout}'${kb_variant:+ variant '${kb_variant}'}, numlock $(( kb_numlock )) "
  write_region "$NIRI_CFG" keyboard "$kb_block" niri validate --config || true
  rm -f "$kb_block"
fi

if [[ -n $kb_lat && -n $kb_lon ]] && has_region "$(readlink -f "$NIRI_CFG")" nightlight; then
  nl_block=$(mktemp)
  bash "$REPO/scripts/setup-locale.sh" --nightlight-block "$kb_lat" "$kb_lon" > "$nl_block"
  echo "    night light: ${kb_lat}, ${kb_lon}"
  write_region "$NIRI_CFG" nightlight "$nl_block" niri validate --config || true
  rm -f "$nl_block"
fi

# --- Suspend policy ----------------------------------------------------------
# The desktop this rice grew up on runs services that must never sleep, so it
# masks the sleep targets. That is exactly the wrong default on a laptop, where
# it means a closed lid keeps running until the battery is flat — so ask, and
# let the hardware pick the default answer.
if compgen -G "/sys/class/power_supply/BAT*" >/dev/null; then
  echo "==> Suspend: battery detected, leaving sleep ENABLED (right for a laptop)"
  echo "    To mask it anyway (a machine that must never sleep):"
  echo "      sudo systemctl mask sleep.target suspend.target hibernate.target hybrid-sleep.target"
else
  echo "==> Suspend: no battery detected (desktop?)"
  # Default no, in both modes: a machine that must never sleep is a deliberate
  # choice, never something an installer should decide for you.
  mask_sleep=n
  ask_yn "Mask sleep/suspend/hibernate so this machine never sleeps?" n && mask_sleep=y
  if [[ $mask_sleep == y ]]; then
    sudo systemctl mask sleep.target suspend.target hibernate.target hybrid-sleep.target
    echo "    masked (undo: sudo systemctl unmask sleep.target suspend.target hibernate.target hybrid-sleep.target)"
  fi
fi

# --- Rust components ---------------------------------------------------------
echo "==> Game of Life wallpaper (~/.local/bin/lifebg)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifewall" && cargo build --release)
  ln -sfn "$REPO/lifewall/target/release/lifewall" "$HOME/.local/bin/lifebg"
else
  echo "    cargo not found — using the python fallback (scripts/life.py)"
  ln -sfn "$REPO/scripts/life.py" "$HOME/.local/bin/lifebg"
fi

# lifelock — the Game of Life lock screen. Builds the binary and installs its
# PAM service file (required: lifelock refuses to start without it). It is
# wired into swayidle in niri/config.kdl; swaylock stays installed as the
# emergency fallback behind Mod+Shift+Alt+Escape.
echo "==> lifelock screen locker (~/.local/bin/lifelock)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifelock" && cargo build --release)
  ln -sfn "$REPO/lifelock/target/release/lifelock" "$HOME/.local/bin/lifelock"
  echo "    installing PAM service -> /etc/pam.d/lifelock"
  sudo install -Dm644 "$REPO/lifelock/pam/lifelock" /etc/pam.d/lifelock
else
  echo "    ERROR: cargo not found — swayidle is wired to lifelock and needs it."
  echo "    Install rust, or point the swayidle line back at swaylock -f."
  exit 1
fi

# lifenote — box-drawing-framed notification daemon. Replaces mako in
# spawn-at-startup.
echo "==> lifenote notification daemon (~/.local/bin/lifenote)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifenote" && cargo build --release)
  ln -sfn "$REPO/lifenote/target/release/lifenote" "$HOME/.local/bin/lifenote"
else
  echo "    ERROR: cargo not found — niri spawns lifenote for notifications."
  echo "    Install rust and run this again."
  exit 1
fi
# KDE ships a DBus activation file for org.freedesktop.Notifications that
# resurrects plasmashell whenever a notification is sent while the name is
# unowned (e.g. during a lifenote restart) — plasma then squats on the name
# and lifenote can't start. A user-level override masks it; delete the file
# to restore KDE's lazy activation.
echo "    masking KDE's notification DBus activation (plasmashell squatting)"
mkdir -p "$HOME/.local/share/dbus-1/services"
printf '[D-BUS Service]\nName=org.freedesktop.Notifications\nExec=/usr/bin/false\n' \
  > "$HOME/.local/share/dbus-1/services/org.kde.plasma.Notifications.service"

# lifemenu — the launcher (Mod+Return/D/Space) and the menu every script
# opens (power, clipboard, wifi, notifications, GPG PIN). Fuzzel-compatible
# flags; themed from the fuzzel.ini lifeconf generates. fuzzel stays installed
# as the scripts' fallback.
echo "==> lifemenu launcher (~/.local/bin/lifemenu)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifemenu" && cargo build --release)
  ln -sfn "$REPO/lifemenu/target/release/lifemenu" "$HOME/.local/bin/lifemenu"
else
  echo "    WARNING: cargo not found — skipping lifemenu (the scripts fall back to fuzzel)."
fi

# lifeauth — the polkit agent (admin prompts: mounting disks, pkexec, lifeconf
# writing /etc). Asks through lifemenu's password box; niri falls back to
# polkit-kde-agent, still installed, when this isn't built.
echo "==> lifeauth polkit agent (~/.local/bin/lifeauth)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifeauth" && cargo build --release)
  ln -sfn "$REPO/lifeauth/target/release/lifeauth" "$HOME/.local/bin/lifeauth"
else
  echo "    WARNING: cargo not found — skipping lifeauth (polkit-kde-agent stays the agent)."
fi

# lifepanel — quick settings (wifi, bluetooth, sound, brightness, power,
# do-not-disturb, drives), and with --watch the drive automounter. Replaces
# nm-applet, blueman-applet and udiskie, which niri still starts when this
# isn't built.
echo "==> lifepanel quick settings (~/.local/bin/lifepanel)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifepanel" && cargo build --release)
  ln -sfn "$REPO/lifepanel/target/release/lifepanel" "$HOME/.local/bin/lifepanel"
  # Their packages also ship XDG autostart entries, which niri's session runs
  # regardless of its own config: keep those out of niri (Plasma keeps them).
  bash "$REPO/scripts/hide-autostart.sh" nm-applet blueman
else
  echo "    WARNING: cargo not found — skipping lifepanel (the tray applets stay)."
fi

# lifeosd — the volume/brightness OSD (a labelled text bar), replacing wob,
# which niri still starts when this isn't built.
echo "==> lifeosd volume/brightness OSD (~/.local/bin/lifeosd)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifeosd" && cargo build --release)
  ln -sfn "$REPO/lifeosd/target/release/lifeosd" "$HOME/.local/bin/lifeosd"
else
  echo "    WARNING: cargo not found — skipping lifeosd (no volume OSD)."
fi

# lifecursor — LifeBranch's own cursor themes (LifeBranch-dark, -light), drawn
# from shapes and written to ~/.local/share/icons. Settings > Cursor picks one.
echo "==> lifecursor cursor themes (~/.local/share/icons/LifeBranch-{dark,light})"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifecursor" && cargo build --release)
  "$REPO/lifecursor/target/release/lifecursor"
else
  echo "    WARNING: cargo not found — skipping lifecursor (no LifeBranch cursors)."
fi

# lifebar — the status bar (workspaces, clock, readings, a text tray),
# replacing waybar. A machine upgraded from the waybar days gets that unit
# disabled here.
echo "==> lifebar status bar (~/.local/bin/lifebar)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifebar" && cargo build --release)
  ln -sfn "$REPO/lifebar/target/release/lifebar" "$HOME/.local/bin/lifebar"
  link "$REPO/systemd/lifebar.service" "$HOME/.config/systemd/user/lifebar.service"
  systemctl --user daemon-reload
  systemctl --user disable waybar.service 2>/dev/null || true
  systemctl --user enable lifebar.service
  echo "    lifebar.service enabled (starts at next login; now: systemctl --user start lifebar)"
else
  echo "    WARNING: cargo not found — skipping lifebar (there will be no bar)."
fi

# lifeportal — apps' Open/Save dialogs become lifefiles (--pick), through
# xdg-desktop-portal's FileChooser. Per-user install: a .portal file names the
# backend, a D-Bus service file starts it on the first dialog, and
# xdg/portals.conf routes FileChooser to it (gtk's dialog when it's missing).
echo "==> lifeportal file dialogs (~/.local/bin/lifeportal)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifeportal" && cargo build --release)
  ln -sfn "$REPO/lifeportal/target/release/lifeportal" "$HOME/.local/bin/lifeportal"
  mkdir -p "$HOME/.local/share/xdg-desktop-portal/portals" "$HOME/.local/share/dbus-1/services"
  ln -sfn "$REPO/lifeportal/lifebranch.portal" "$HOME/.local/share/xdg-desktop-portal/portals/lifebranch.portal"
  printf '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.lifebranch\nExec=%s\n' \
    "$HOME/.local/bin/lifeportal" \
    > "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lifebranch.service"
  # A running portal only reads backends at start.
  if systemctl --user -q is-active xdg-desktop-portal.service 2>/dev/null; then
    systemctl --user restart xdg-desktop-portal.service || true
  fi
else
  echo "    WARNING: cargo not found — skipping lifeportal (file dialogs stay gtk's)."
fi

# lifefiles — mouse-driven terminal file browser (Mod+E). Themed by lifeconf via
# ~/.config/lifefiles/theme, and registered as the folder handler.
echo "==> lifefiles file browser (~/.local/bin/lifefiles)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifefiles" && cargo build --release)
  ln -sfn "$REPO/lifefiles/target/release/lifefiles" "$HOME/.local/bin/lifefiles"
  apps="$HOME/.local/share/applications"
  mkdir -p "$apps"
  sed "s|-e lifefiles|-e $HOME/.local/bin/lifefiles|" \
    "$REPO/lifefiles/lifefiles.desktop" > "$apps/lifefiles.desktop"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" 2>/dev/null || true
  # Folder links in browsers/portals ("show in folder") open here — but only
  # take the slot when it is unset or still the Dolphin this installer put there;
  # a file manager you chose yourself stays. (lifeconf's Apps panel switches it.)
  if command -v xdg-mime >/dev/null 2>&1; then
    cur_fm="$(xdg-mime query default inode/directory 2>/dev/null || true)"
    case "$cur_fm" in
      ""|org.kde.dolphin.desktop) xdg-mime default lifefiles.desktop inode/directory ;;
      *) echo "    keeping $cur_fm as the folder handler (change it in lifeconf > Apps)" ;;
    esac
  fi
else
  echo "    WARNING: cargo not found — skipping lifefiles (Mod+E will do nothing)."
fi

# lifeshot — screenshot + annotation overlay (Shift+Print). Themed by lifeconf
# via ~/.config/lifeshot/theme; copies through wl-copy (wl-clipboard).
echo "==> lifeshot screenshot tool (~/.local/bin/lifeshot)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifeshot" && cargo build --release)
  ln -sfn "$REPO/lifeshot/target/release/lifeshot" "$HOME/.local/bin/lifeshot"
else
  echo "    WARNING: cargo not found — skipping lifeshot (Shift+Print will do nothing)."
fi

# lifeconf — the theming/settings front-end. One ~/.config/lifeconf/theme.toml
# drives waybar/kitty/fuzzel/lifenote/swaylock/lifelock/lifegreet/niri; `lifeconf
# --apply` regenerates them all. Seeded from the olive preset on first run.
echo "==> lifeconf theming front-end (~/.local/bin/lifeconf)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifeconf" && cargo build --release)
  ln -sfn "$REPO/lifeconf/target/release/lifeconf" "$HOME/.local/bin/lifeconf"
  # --apply is what materialises the five gitignored theme files. Run it when
  # theme.toml is missing (first run — seeds olive), but ALSO whenever any of
  # those files is absent: on an existing machine theme.toml already exists, so
  # the old first-run-only test left a fresh clone's symlinks dangling and the
  # bar fell back to waybar's built-in stylesheet.
  theme_missing=0
  for gen in fuzzel/fuzzel.ini kitty/olive.conf \
             lifenote/config swaylock/config; do
    [[ -s "$REPO/$gen" ]] || theme_missing=1
  done
  # lifefiles' theme lives in ~/.config, not the repo.
  [[ -s "$HOME/.config/lifefiles/theme" ]] || theme_missing=1
  [[ -s "$HOME/.config/lifeshot/theme" ]] || theme_missing=1
  [[ -s "$HOME/.config/lifebar/config" ]] || theme_missing=1
  if [[ ! -f "$HOME/.config/lifeconf/theme.toml" ]]; then
    echo "    seeding ~/.config/lifeconf/theme.toml (olive) and applying"
    "$HOME/.local/bin/lifeconf" --apply
  elif (( theme_missing )); then
    echo "    regenerating the gitignored theme files (lifeconf --apply)"
    "$HOME/.local/bin/lifeconf" --apply
  fi
  # Desktop entry so the GUI shows up in the launcher / app menu. The Exec is
  # rewritten to an absolute path since a launcher may not carry ~/.local/bin.
  apps="$HOME/.local/share/applications"
  mkdir -p "$apps"
  sed "s|^Exec=lifeconf|Exec=$HOME/.local/bin/lifeconf|" \
    "$REPO/lifeconf/lifeconf.desktop" > "$apps/lifeconf.desktop"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" 2>/dev/null || true
  echo "    installed lifeconf.desktop (launch 'lifeconf' from fuzzel)"
else
  echo "    cargo not found — skipping lifeconf (optional; configs stay as-is)."
fi

# --- Idle suspend ------------------------------------------------------------
# The idle chain locks at 10 minutes and blanks the screens at 15, and then
# stops: the desktop this rice grew up on runs services and must never sleep.
# On a laptop that means an idle machine with the lid open stays fully awake
# until the battery is flat, which is the same wrong default the sleep-target
# question above already corrects for — so correct it here too, by the same
# rule: let the hardware pick the answer.
#
# This has to run after lifeconf, because theme.toml is what it edits and
# lifeconf is what creates it. The step itself (scripts/idle-suspend.sh) fires
# only while discharging, so saying yes on a docked laptop costs nothing.
offer_idle_suspend "$HOME/.config/lifeconf/theme.toml"

# The tray applets, waybar, wob, udiskie, fuzzel, mako and polkit-kde-agent that
# the life* components replaced: offer to remove what is left of them.
offer_remove_legacy

echo "==> GTK dark theme + cursor (GTK apps; Qt/KDE keeps its own settings)"
if command -v gsettings >/dev/null 2>&1; then
  gsettings set org.gnome.desktop.interface gtk-theme "adw-gtk3-dark"
  gsettings set org.gnome.desktop.interface color-scheme "prefer-dark"
  gsettings set org.gnome.desktop.interface cursor-theme "LifeBranch-dark"
  gsettings set org.gnome.desktop.interface cursor-size 24
fi

# --- Performance profile -----------------------------------------------------
# A full-screen Game of Life at 30 fps, composited under translucent terminals,
# is the one part of this that an old machine will genuinely struggle with.
echo "==> Performance profile"
if bash "$REPO/scripts/lite-profile.sh" --check; then
  echo "    This machine looks like it would rather not run the full budget:"
  bash "$REPO/scripts/lite-profile.sh" --why
  echo "    The lite profile changes:"
  bash "$REPO/scripts/lite-profile.sh" --report
  if ask_yn "Apply the lite profile?" y; then
    bash "$REPO/scripts/lite-profile.sh" --apply
  fi
else
  echo "    hardware looks comfortable — keeping the full look."
  echo "    (turn it down any time: ~/.local/bin/lite-profile.sh --apply)"
fi

echo "==> Validating niri config"
niri validate

cat <<'EOF'

==> Done.
    - Log out and pick "Niri" at the login screen (a real session; Mod = Super).
    - A keyboard cheat sheet opens once at every login, built from your own
      config. Mod+Slash reopens it; the window itself says how to edit the
      bindings and how to stop it appearing.
    - Native Wayland (drag-and-drop): Qt, Electron and Brave are pinned to
      Wayland by ~/.config/environment.d/50-niri-platform.conf. The systemd
      user manager reads that file only when the session starts, so LOG OUT AND
      BACK IN before testing a drag — otherwise you are testing the old session
      and nothing will have changed. Trade-off, on purpose: file drags from
      Dolphin into X11-only apps stop working, drags into Brave / vesktop /
      VS Code start working. Per-app opt-out is in that file.
    - Touchpad: re-run `detect-trackpad.sh` any time to see what your hardware
      reports; the settings live in the LIFEBRANCH:BEGIN touchpad region of
      ~/.config/niri/config.kdl and are ordinary niri options.
    - Game of Life wallpaper starts with niri. Preview in a terminal: `lifebg`
      Restart it live:  pkill -f '[l]ifebg'; then re-run the lifebg line from
      niri/config.kdl (or: lifeconf --apply). Flags: `lifebg --help` (tick/fps/fade/colours/char).
    - Restart kitty windows to pick up the transparency + font + olive palette.
    - Lock: Mod+Alt+Escape (or 10 min idle) -> lifelock, the Game of Life cube;
      the Mod+Shift+Alt+Escape recovery bind force-swaps in swaylock if it
      ever wedges. Power menu: Mod+Shift+E.
    - Volume keys flash the lifeosd bar.
    - Notifications: lifenote — pure-text popups in box-drawing frames, top
      right. Style/colours/alpha: ~/.config/lifenote/config. The bar's #
      button counts unseen notifications. Do-not-disturb: Mod+N.
    - Theming: run `lifeconf` (TUI) or `lifeconf --gui` to change the palette.
    - Optional, deliberate extras:
        bash greeter-install.sh   replace the login screen with lifegreet
EOF
