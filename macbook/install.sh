#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Install the niri olive setup on the 2019 MacBook Pro (T2 / Arch):
# packages + symlinks + validate. Idempotent — safe to re-run.
# Needs your sudo password for the package step.
set -euo pipefail

MAC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # <repo>/macbook
REPO="$(cd "$MAC/.." && pwd)"                          # <repo> (shared theme files)

if [[ $(id -u) -eq 0 ]]; then
  echo "!! Run this as your own user, not as root." >&2
  exit 1
fi

# AUR helper, bootstrapped by curl when missing. No-op if yay is present.
bash "$REPO/scripts/ensure-yay.sh"

# Shared stack + brightnessctl for the backlight keys/module.
# phinger-cursors is AUR-only; the rest are official repos (adw-gtk-theme is in extra).
# kitty is the terminal the whole rice assumes: Mod+T, the Ctrl+Alt+Return
# recovery bind, the waybar htop clicks, and the `kitten panel` Game of Life
# wallpaper all need it.
# qt6-wayland/qt5-wayland are the Qt Wayland platform plugins — the actual fix
# for drag-and-drop. Without the plugin Qt falls back to XWayland, and
# xwayland-satellite can't bridge DnD across the X11/Wayland boundary
# (Supreeeme/xwayland-satellite#133), so drags out of Dolphin die at the border.
# The platform variables that go with them live in
# environment.d/50-niri-platform.conf, which documents the trade-off.
PKGS=(niri rust
      kitty fuzzel waybar mako swaybg xwayland-satellite wl-clipboard cliphist wev brightnessctl
      adw-gtk-theme wob jq
      swaylock swayidle ttf-sharetech-mono-nerd ttf-cousine-nerd
      xdg-desktop-portal-gnome qt6ct qt6-wayland qt5-wayland polkit-kde-agent
      network-manager-applet blueman udiskie wlsunset wf-recorder playerctl
      # Everyday applications (same set as the desktop installer).
      nano dolphin libreoffice-fresh element-desktop kleopatra)

# AUR-only, kept separate so a build failure names itself.
AUR_PKGS=(phinger-cursors vesktop-bin)

echo "==> Installing packages from the official repos"
sudo pacman -S --needed "${PKGS[@]}"

echo "==> Installing AUR packages: ${AUR_PKGS[*]}"
if command -v yay >/dev/null 2>&1; then
  yay -S --needed "${AUR_PKGS[@]}"
else
  echo "    !! no AUR helper — skipping ${AUR_PKGS[*]}."
fi

if [[ -t 0 ]]; then
  echo
  echo "==> Anything else you want installed?"
  echo "    Repos and the AUR are both available. Common picks:"
  echo "      firefox thunderbird vlc gimp obsidian signal-desktop keepassxc btop"
  read -rp "    Packages (space-separated, blank to skip): " -a extra_pkgs || extra_pkgs=()
  wanted=()
  for p in "${extra_pkgs[@]:-}"; do
    [[ -z $p ]] && continue
    if [[ $p =~ ^[a-zA-Z0-9][a-zA-Z0-9@._+-]*$ ]]; then
      wanted+=("$p")
    else
      echo "    skipping '$p' — that is not a package name."
    fi
  done
  if (( ${#wanted[@]} )); then
    yay -S --needed "${wanted[@]}" || echo "    !! some of those did not install; carrying on."
  fi
fi

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
# Laptop-specific (from macbook/):
link "$MAC/niri/config.kdl"      "$HOME/.config/niri/config.kdl"
link "$MAC/waybar/config.jsonc"  "$HOME/.config/waybar/config.jsonc"
# Shared olive theme (from the repo root):
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
  printf '\n# Olive rice extras (transparency; managed in ~/niri).\ninclude rice.conf\n' >> "$KITTY_CONF"
  echo "    appended 'include rice.conf' to $KITTY_CONF"
fi

# kitty's listen_on lives under $XDG_RUNTIME_DIR, which kitty will not create;
# without the rule every kitty start prints "Invalid listen_on=..., ignoring"
# and lifeconf can never re-theme a running terminal. Not silenced: a failure
# here is worth seeing rather than being told the directory exists.
if systemd-tmpfiles --user --create "$HOME/.config/user-tmpfiles.d/kitty.conf"; then
  echo "    created ${XDG_RUNTIME_DIR:-\$XDG_RUNTIME_DIR}/kitty (kitty remote-control sockets)"
else
  echo "    note: the kitty socket dir was not created now — it will be at next login."
fi

# waybar is a supervised systemd user unit, not a niri spawn — the niri configs
# stopped spawning it, so without this the bar never starts at all.
echo "==> Enabling the waybar user unit"
systemctl --user daemon-reload
systemctl --user enable waybar.service

echo "==> Installing scripts into ~/.local/bin"
# One list drives both chmod and symlink — the desktop installer's shape, so a
# script added there cannot silently go missing here (bright-osd.sh is the one
# laptop-only addition).
SCRIPTS=(clip-menu.sh power-menu.sh lifebg-toggle.sh vol-osd.sh
         dnd-toggle.sh float-snap.sh scratch-term.sh notif-menu.sh
         rec-toggle.sh pinentry-fuzzel.sh shortcuts-window.sh
         detect-trackpad.sh setup-locale.sh lite-profile.sh bright-osd.sh)
chmod +x "$REPO/scripts/life.py"
mkdir -p "$HOME/.local/bin"
for s in "${SCRIPTS[@]}"; do
  chmod +x "$REPO/scripts/$s"
  ln -sfn "$REPO/scripts/$s" "$HOME/.local/bin/$s"
done

# --- Hardware and locale -----------------------------------------------------
# shellcheck source=scripts/config-region.sh
source "$REPO/scripts/config-region.sh"
NIRI_CFG="$HOME/.config/niri/config.kdl"

# The Apple pad's settings are already written into macbook/niri/config.kdl, but
# re-detect anyway: the same script runs on any hardware, and an external pad
# reports different capabilities from the internal one.
echo "==> Looking for a touchpad"
if bash "$REPO/scripts/detect-trackpad.sh" && has_region "$(readlink -f "$NIRI_CFG")" touchpad; then
  tp_block=$(mktemp)
  bash "$REPO/scripts/detect-trackpad.sh" --niri-block > "$tp_block"
  echo
  echo "    Proposed niri settings:"
  sed 's/^/    /' "$tp_block"
  echo
  if [[ -t 0 ]]; then
    read -rp "    Replace the touchpad block in your niri config with these? [y/N] " a
    [[ ${a:-N} =~ ^[Yy]$ ]] && write_region "$NIRI_CFG" touchpad "$tp_block" niri validate --config
  fi
  rm -f "$tp_block"
fi

# Keyboard layout and night-light location, defaulted from what the system
# already knows. Same fenced regions as the desktop installer.
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
  # The internal Apple board has no numpad, so the default here is off.
  read -rp "    Start with NumLock on (only if you have a numpad)? [y/N] " a
  [[ ${a:-N} =~ ^[Yy]$ ]] && kb_numlock=1 || kb_numlock=0
  if [[ -n $det_lat && -n $det_lon ]]; then
    read -rp "    Night-light coordinates 'lat lon' [${det_lat} ${det_lon}]: " a
    if [[ -n $a ]]; then
      read -r in_lat in_lon <<<"$a"
      if [[ $in_lat =~ ^-?[0-9]+(\.[0-9]+)?$ && $in_lon =~ ^-?[0-9]+(\.[0-9]+)?$ ]]; then
        kb_lat=$in_lat kb_lon=$in_lon
      fi
    fi
  fi
fi

if has_region "$(readlink -f "$NIRI_CFG")" keyboard; then
  kb_block=$(mktemp)
  bash "$REPO/scripts/setup-locale.sh" --keyboard-block \
       "$kb_layout" "$kb_variant" pc105 "$kb_numlock" > "$kb_block"
  write_region "$NIRI_CFG" keyboard "$kb_block" niri validate --config
  rm -f "$kb_block"
fi
if [[ -n $kb_lat && -n $kb_lon ]] && has_region "$(readlink -f "$NIRI_CFG")" nightlight; then
  nl_block=$(mktemp)
  bash "$REPO/scripts/setup-locale.sh" --nightlight-block "$kb_lat" "$kb_lon" > "$nl_block"
  write_region "$NIRI_CFG" nightlight "$nl_block" niri validate --config
  rm -f "$nl_block"
fi

echo "==> Game of Life wallpaper (~/.local/bin/lifebg)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifewall" && cargo build --release)
  ln -sfn "$REPO/lifewall/target/release/lifewall" "$HOME/.local/bin/lifebg"
else
  echo "    cargo not found — using the python fallback (scripts/life.py)"
  ln -sfn "$REPO/scripts/life.py" "$HOME/.local/bin/lifebg"
fi

# lifelock — the Game of Life lock screen (desktop parity; wired into swayidle
# in macbook/niri/config.kdl). swaylock stays installed as the emergency
# fallback behind Mod+Shift+Alt+Escape.
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

# lifenote — box-drawing-framed notification daemon (desktop parity). Replaces
# mako in spawn-at-startup; mako stays installed as the fallback (pkill lifenote
# && mako). The waybar #/DND modules talk to `lifenote ctl`.
echo "==> lifenote notification daemon (~/.local/bin/lifenote)"
if command -v cargo >/dev/null 2>&1; then
  (cd "$REPO/lifenote" && cargo build --release)
  ln -sfn "$REPO/lifenote/target/release/lifenote" "$HOME/.local/bin/lifenote"
else
  echo "    ERROR: cargo not found — niri spawns lifenote for notifications."
  echo "    Install rust, or point the spawn-at-startup line back at mako."
  exit 1
fi

# lifeconf — the theming/settings front-end (shared build from the repo root).
# Drives the palette files (gitignored build artifacts — see lifeconf/README.md
# "Generated files & git") + the niri regions in macbook/niri/config.kdl.
# Seeded from the olive preset on first run: no-visual-diff.
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

# T2 system plumbing (root): s2idle suspend fix + module lifecycle hooks,
# hibernate kept dead, lid = suspend, UPower poweroff-on-critical, ALSA/JACK ->
# PipeWire routing, 48 kHz pin, NCM-ethernet silencing, BT firmware from macOS.
# Idempotent; see macbook/system/apply-system.sh for the full list.
echo "==> T2 system plumbing (suspend/audio/network fixes; needs sudo)"
read -rp "    Run macbook/system/apply-system.sh now? [Y/n] " a
[[ "${a:-Y}" =~ ^[Yy]?$ ]] && sudo bash "$MAC/system/apply-system.sh"

echo "==> Performance profile"
if bash "$REPO/scripts/lite-profile.sh" --check; then
  bash "$REPO/scripts/lite-profile.sh" --why
  bash "$REPO/scripts/lite-profile.sh" --report
  if [[ -t 0 ]]; then
    read -rp "    Apply the lite profile? [Y/n] " a
    [[ ${a:-Y} =~ ^[Yy]?$ ]] && bash "$REPO/scripts/lite-profile.sh" --apply
  fi
else
  echo "    hardware looks comfortable — keeping the full look."
fi

echo "==> Validating niri config"
niri validate

cat <<'EOF'

==> Done.
    - Log out and pick "Niri" at the login screen (Mod = Super = Command).
    - A keyboard cheat sheet opens once at every login, built from your own
      config. Mod+Slash reopens it; it says how to edit the bindings and how
      to stop it appearing.
    - Native Wayland (drag-and-drop): Qt, Electron and Brave are pinned to
      Wayland by ~/.config/environment.d/50-niri-platform.conf. The systemd
      user manager reads that file only when the session starts, so LOG OUT AND
      BACK IN before testing a drag — otherwise you are testing the old session
      and nothing will have changed. Trade-off, on purpose: file drags from
      Dolphin into X11-only apps (Bitwig) stop working, drags into Brave /
      vesktop / VS Code start working. Per-app opt-out is in that file.
    - Brightness keys need brightnessctl's udev rules OR your user in the `video` group:
        sudo usermod -aG video "$USER"   # then re-login
    - Lock: Mod+Alt+Escape (or 10 min idle) -> lifelock, the Game of Life cube
      (desktop parity). Mod+Shift+Alt+Escape force-swaps in swaylock if the
      locker ever wedges.
      Screens off at 15 min. Sleep is NOT masked here (unlike the desktop) —
      lid close suspends (s2idle; see macbook/system/), locking first.
      Hibernate stays masked on purpose: the T2 cannot survive it.
    - Power menu: Mod+Shift+E (lock/suspend/logout/reboot/poweroff — Suspend
      shows here because sleep.target isn't masked).
    - Volume/brightness keys flash a wob OSD bar (olive; ~/.config/wob/wob.ini).
    - Do-not-disturb: Mod+N (or click the DND label in waybar).
    - swayidle starts with niri — log out/in (or run the spawn line by hand) to arm it.
    - Restart kitty windows to pick up transparency + font + olive palette (rice.conf).
    - Optional olive login screen: bash ../greeter-install.sh
      (greetd + lifegreet — the Game of Life cube greeter; tuigreet fallback).
    - See macbook/README.md for T2 notes: audio firmware, hid_apple options, touch bar.
EOF
