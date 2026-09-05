#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# LifeBranch: install the rice. Packages, symlinks and hardware detection.
# Idempotent — safe to re-run. Needs your sudo password for the package step.
#
#     bash ~/LifeBranch/install.sh
#
# Arch (or an Arch derivative) only: everything here goes through pacman/yay.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

# An AUR helper, bootstrapped by curl if it is missing (phinger-cursors and
# vesktop-bin are AUR-only). No-op when yay is already installed.
bash "$REPO/scripts/ensure-yay.sh"

# --- Packages ----------------------------------------------------------------
# niri itself heads the list: the installer validates a niri config at the end
# and the whole thing is useless without the compositor.
# rust is not optional either — lifelock (the screen locker) and lifenote (the
# notification daemon) are built from source below and are wired into the config.
# kitty is the terminal the whole rice assumes: Mod+T, the Ctrl+Alt+Return
# recovery bind, the waybar htop clicks, the cheat-sheet window and the
# `kitten panel` Game of Life wallpaper all need it.
# qt6-wayland/qt5-wayland are the Qt Wayland platform plugins — the actual fix
# for drag-and-drop. Without the plugin Qt falls back to XWayland, and
# xwayland-satellite can't bridge DnD across the X11/Wayland boundary
# (Supreeeme/xwayland-satellite#133), so drags out of Dolphin die at the border.
# The platform variables that go with them live in
# environment.d/50-niri-platform.conf, which documents the trade-off.
PKGS=(niri rust
      kitty fuzzel waybar mako swaybg xwayland-satellite wl-clipboard cliphist wev
      adw-gtk-theme wob jq
      swaylock swayidle ttf-sharetech-mono-nerd ttf-cousine-nerd
      xdg-desktop-portal-gnome qt6ct qt6-wayland qt5-wayland
      network-manager-applet blueman
      polkit-kde-agent udiskie wlsunset wf-recorder playerctl
      # Everyday applications.
      nano dolphin libreoffice-fresh element-desktop kleopatra)
#     libreoffice-fresh is the current release; swap in libreoffice-still on
#     older/slower hardware — it is the same suite, a version behind.

# AUR. Kept separate so the bulk of the install goes through pacman directly
# (faster, and a build failure here names itself instead of taking the lot down).
AUR_PKGS=(phinger-cursors vesktop-bin)

echo "==> Installing packages from the official repos"
sudo pacman -S --needed "${PKGS[@]}"

echo "==> Installing AUR packages: ${AUR_PKGS[*]}"
if command -v yay >/dev/null 2>&1; then
  yay -S --needed "${AUR_PKGS[@]}"
else
  echo "    !! no AUR helper — skipping ${AUR_PKGS[*]}."
  echo "       The cursor theme and vesktop will be missing; install them later with yay."
fi

# Anything else this particular person wants, while we already have their
# attention and a working AUR helper.
if [[ -t 0 ]]; then
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
# link SRC DST — back up a real file at DST to DST.bak (once), then symlink.
link() {
  local src="$1" dst="$2"
  mkdir -p "$(dirname "$dst")"
  if [[ -e "$dst" && ! -L "$dst" ]]; then
    echo "    backing up $dst -> $dst.bak"
    mv "$dst" "$dst.bak"
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
mkdir -p "$REPO/waybar" "$REPO/fuzzel" "$REPO/kitty" "$REPO/lifenote" "$REPO/swaylock"
link "$REPO/niri/config.kdl"     "$HOME/.config/niri/config.kdl"
link "$REPO/waybar/config.jsonc" "$HOME/.config/waybar/config.jsonc"
link "$REPO/waybar/style.css"    "$HOME/.config/waybar/style.css"
link "$REPO/fuzzel/fuzzel.ini"   "$HOME/.config/fuzzel/fuzzel.ini"
link "$REPO/mako/config"         "$HOME/.config/mako/config"     # fallback daemon
link "$REPO/lifenote/config"     "$HOME/.config/lifenote/config"
link "$REPO/kitty/rice.conf"     "$HOME/.config/kitty/rice.conf"
link "$REPO/kitty/olive.conf"    "$HOME/.config/kitty/olive.conf"
link "$REPO/tmpfiles/kitty.conf" "$HOME/.config/user-tmpfiles.d/kitty.conf"
link "$REPO/systemd/waybar.service" "$HOME/.config/systemd/user/waybar.service"
link "$REPO/wob/wob.ini"         "$HOME/.config/wob/wob.ini"
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
         dnd-toggle.sh float-snap.sh scratch-term.sh notif-menu.sh
         rec-toggle.sh pinentry-fuzzel.sh shortcuts-window.sh
         detect-trackpad.sh setup-locale.sh lite-profile.sh)
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

# waybar runs supervised, not from niri's spawn-at-startup: a scope that exits
# leaves no bar and no log. Restart=always brings it back; the journal keeps the
# evidence (journalctl --user -u waybar -b).
echo "==> Enabling the waybar user unit"
systemctl --user daemon-reload
systemctl --user enable waybar.service
echo "    enabled (starts with graphical-session.target; start now with"
echo "     systemctl --user start waybar.service)"

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
  if [[ -t 0 ]]; then
    read -rp "    Write these into your niri config? [Y/n] " a
    [[ ${a:-Y} =~ ^[Yy]?$ ]] && write_region "$NIRI_CFG" touchpad "$tp_block" niri validate --config
  else
    echo "    (non-interactive — not writing; re-run from a terminal to apply)"
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
if [[ -t 0 ]]; then
  echo "    Detected from this system: layout '${det_layout}'${det_variant:+ (variant ${det_variant})}, timezone ${det_tz:-unknown}"
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
  write_region "$NIRI_CFG" keyboard "$kb_block" niri validate --config
  rm -f "$kb_block"
fi

if [[ -n $kb_lat && -n $kb_lon ]] && has_region "$(readlink -f "$NIRI_CFG")" nightlight; then
  nl_block=$(mktemp)
  bash "$REPO/scripts/setup-locale.sh" --nightlight-block "$kb_lat" "$kb_lon" > "$nl_block"
  echo "    night light: ${kb_lat}, ${kb_lon}"
  write_region "$NIRI_CFG" nightlight "$nl_block" niri validate --config
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
  mask_sleep=n
  if [[ -t 0 ]]; then
    read -rp "    Mask sleep/suspend/hibernate so this machine never sleeps? [y/N] " a
    [[ ${a:-N} =~ ^[Yy]$ ]] && mask_sleep=y
  else
    echo "    (non-interactive — leaving sleep enabled)"
  fi
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
# spawn-at-startup; mako stays installed as the fallback (pkill lifenote && mako).
echo "==> lifenote notification daemon (~/.local/bin/lifenote)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifenote" && cargo build --release)
  ln -sfn "$REPO/lifenote/target/release/lifenote" "$HOME/.local/bin/lifenote"
else
  echo "    ERROR: cargo not found — niri spawns lifenote for notifications."
  echo "    Install rust, or point the spawn-at-startup line back at mako."
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
  for gen in waybar/style.css fuzzel/fuzzel.ini kitty/olive.conf \
             lifenote/config swaylock/config; do
    [[ -s "$REPO/$gen" ]] || theme_missing=1
  done
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

echo "==> GTK dark theme + cursor (GTK apps; Qt/KDE keeps its own settings)"
if command -v gsettings >/dev/null 2>&1; then
  gsettings set org.gnome.desktop.interface gtk-theme "adw-gtk3-dark"
  gsettings set org.gnome.desktop.interface color-scheme "prefer-dark"
  gsettings set org.gnome.desktop.interface cursor-theme "phinger-cursors-light"
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
  if [[ -t 0 ]]; then
    read -rp "    Apply the lite profile? [Y/n] " a
    [[ ${a:-Y} =~ ^[Yy]?$ ]] && bash "$REPO/scripts/lite-profile.sh" --apply
  else
    echo "    (non-interactive — not applied; run: lite-profile.sh --apply)"
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
      Restart it live:  pkill -f '[l]ifebg'; then re-run the kitten panel line
      from niri/config.kdl. Flags: `lifebg --help` (tick/fps/fade/colours/char).
    - Restart kitty windows to pick up the transparency + font + olive palette.
    - Lock: Mod+Alt+Escape (or 10 min idle) -> lifelock, the Game of Life cube;
      the Mod+Shift+Alt+Escape recovery bind force-swaps in swaylock if it
      ever wedges. Power menu: Mod+Shift+E.
    - Volume keys flash a wob OSD bar (~/.config/wob/wob.ini).
    - Notifications: lifenote — pure-text popups in box-drawing frames, top
      right. Style/colours/alpha: ~/.config/lifenote/config. The waybar #
      button counts unseen notifications; mako stays installed as the fallback
      (pkill lifenote && mako). Do-not-disturb: Mod+N.
    - Theming: run `lifeconf` (TUI) or `lifeconf --gui` to change the palette.
    - Optional, deliberate extras:
        bash greeter-install.sh   replace the login screen with lifegreet
EOF
