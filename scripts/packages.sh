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
# Only what the desktop itself needs: chat apps and the like are the user's
# choice, offered as "common picks" by the installers.
# shellcheck disable=SC2034
AUR_PKGS=(phinger-cursors)

# Rivals pacman does NOT know about: packages that install fine side by side
# but fight at runtime (two power managers, two sound servers). Declared
# conflicts are found automatically by drop_conflicts below; this list is only
# for the undeclared ones. "wanted:installed-rival".
CONFLICTS=(pipewire-pulse:pulseaudio
           power-profiles-daemon:tlp
           power-profiles-daemon:auto-cpufreq
           power-profiles-daemon:tuned-ppd)

# app_of NAME: the program a package is a build of, without its flavour:
# vesktop, vesktop-bin, vesktop-git and vesktop-debug are all "vesktop".
# Sets APP rather than printing it: called once per installed package, and a
# $(subshell) each time costs seconds on a machine with 1500 of them.
app_of() {
  local s
  APP=${1%-debug}
  for s in -bin -git -appimage -nightly -beta; do APP=${APP%"$s"}; done
}

# _conflict_table: pacman/yay -Si or -Qi text on stdin ->
# "NAME<TAB>conflicts<TAB>provides" lines, each list space-separated with
# version constraints stripped.
# LC_ALL=C on the caller's side keeps the field names English.
_conflict_table() {
  awk '
    function clean(v) { gsub(/[<>=][^ ]*/, "", v); return v == "None" ? "" : v }
    function out() {
      if (name != "") print name "\t" clean(conf) "\t" clean(prov)
      name = ""; conf = ""; prov = ""
    }
    /^[A-Za-z][A-Za-z ]*[^ ] *: / {
      key = $0; sub(/ *:.*/, "", key)
      val = $0; sub(/^[^:]*: /, "", val)
      if (key == "Name") { out(); name = val }
      else if (key == "Conflicts With") conf = val
      else if (key == "Provides") prov = val
      next
    }
    /^ +[^ ]/ && key == "Conflicts With" { line = $0; sub(/^ +/, "", line); conf = conf " " line; next }
    /^ +[^ ]/ && key == "Provides" { line = $0; sub(/^ +/, "", line); prov = prov " " line; next }
    /^[^ ]/ { key = "" }
    END { out() }'
}

# drop_conflicts: take out of PKGS and AUR_PKGS anything that would collide
# with a package already installed, and keep the installed one. pacman answers
# a conflict with "Remove X? [y/N]", and its N (also what --noconfirm picks)
# aborts the WHOLE transaction, so one rival would otherwise leave a fresh
# machine with nothing installed. Four ways a rival shows up:
#   1. the wanted package declares a conflict with something installed
#      (pipewire-pulse vs pulseaudio);
#   2. something installed declares a conflict with the wanted package, or
#      with a name it provides (niri-git vs niri);
#   3. another build of the same program is installed (vesktop vs
#      vesktop-bin), which AUR packages often fail to declare;
#   4. the undeclared runtime rivals in CONFLICTS above.
# Whoever chose the installed one keeps it; anything that needed the skipped
# package says so itself.
drop_conflicts() {
  local want c rival name conf prov p
  local -A drop=() installed=() app_installed=() blocks=() provider=()
  while read -r p; do
    installed[$p]=1
    [[ $p == *-debug ]] && continue
    app_of "$p"; app_installed[$APP]+="$p "
  done < <(pacman -Qq)

  # 2: who blocks what, from every installed package's own Conflicts With;
  # and who provides what, to name the real package behind a virtual one.
  while IFS=$'\t' read -r name conf prov; do
    for c in $conf; do blocks[$c]+="$name "; done
    for c in $prov; do provider[$c]=$name; done
  done < <(LC_ALL=C pacman -Qi | _conflict_table)

  _rival() {   # want rival why
    [[ -n ${drop[$1]:-} ]] && return
    drop[$1]=1
    echo "    keeping your $2, so skipping $1 ($3)"
  }
  _check() {   # want, its declared conflicts, the names it provides
    local want=$1 c r n
    [[ -n ${installed[$want]:-} ]] && return     # already have it: nothing to decide
    for c in $2; do                              # 1
      if [[ -n ${installed[$c]:-} ]]; then _rival "$want" "$c" "they conflict"; return; fi
      if [[ -n ${provider[$c]:-} && ${provider[$c]} != "$want" ]]; then
        _rival "$want" "${provider[$c]}" "they conflict"; return
      fi
    done
    for n in "$want" $3; do                      # 2
      for r in ${blocks[$n]:-}; do
        [[ $r == "$want" ]] || { _rival "$want" "$r" "it conflicts with $want"; return; }
      done
    done
    app_of "$want"
    for r in ${app_installed[$APP]:-}; do        # 3
      [[ $r == "$want" ]] || { _rival "$want" "$r" "another build of the same program"; return; }
    done
  }

  while IFS=$'\t' read -r name conf prov; do
    _check "$name" "$conf" "$prov"
  done < <(LC_ALL=C pacman -Si "${PKGS[@]}" 2>/dev/null | _conflict_table)
  if (( ${#AUR_PKGS[@]} )) && command -v yay >/dev/null 2>&1; then
    while IFS=$'\t' read -r name conf prov; do
      _check "$name" "$conf" "$prov"
    done < <(LC_ALL=C yay -Si --aur "${AUR_PKGS[@]}" 2>/dev/null | _conflict_table)
  fi
  for c in "${CONFLICTS[@]}"; do                 # 4
    want=${c%%:*} rival=${c#*:}
    [[ -n ${installed[$rival]:-} && -z ${installed[$want]:-} ]] && _rival "$want" "$rival" "they fight at runtime"
  done

  local keep=() keep_aur=()
  for p in "${PKGS[@]}"; do [[ -n ${drop[$p]:-} ]] || keep+=("$p"); done
  for p in "${AUR_PKGS[@]}"; do [[ -n ${drop[$p]:-} ]] || keep_aur+=("$p"); done
  PKGS=("${keep[@]}")
  AUR_PKGS=("${keep_aur[@]}")
  unset -f _rival _check
}

# clear_orphan_debug: with `debug` in makepkg.conf's OPTIONS (Arch's default
# since 2024), every AUR build also installs a BASE-debug package, and removing
# the program later leaves that behind, still owning its files. A later build
# of the same program then fails at the AUR step:
#   vesktop-bin-debug: /usr/lib/debug/.build-id/... exists in filesystem (owned by vesktop-debug)
# An orphan is a -debug package whose build (pkgbase) no longer has any package
# installed and that nothing requires. Those for a program about to be
# installed are offered for removal; the rest are only counted.
clear_orphan_debug() {
  local db=/var/lib/pacman/local dbg base p req n
  local -A wanted=() per_base=()
  # How many installed packages each build (pkgbase) has, in one pass.
  while read -r base n; do per_base[$base]=$n; done < <(
    awk '/^%BASE%$/ { getline; c[$0]++ } END { for (b in c) print b, c[b] }' "$db"/*/desc)
  for p in "${PKGS[@]}" "${AUR_PKGS[@]}"; do app_of "$p"; wanted[$APP]=1; done
  local colliding=() other=0
  local -A have=()
  while read -r p; do have[$p]=1; done < <(pacman -Qq)
  while read -r dbg; do
    base=${dbg%-debug}
    [[ -n ${have[$base]:-} ]] && continue
    # A split build's debug package is named after its pkgbase, not after any
    # one package it made: the debug package's own %BASE% is "<base>" too, so
    # any OTHER installed package with that base means the build is still here.
    (( ${per_base[$base]:-0} > 1 )) && continue
    req=$(LC_ALL=C pacman -Qi "$dbg" | sed -n 's/^Required By *: //p')
    [[ $req == None ]] || continue
    app_of "$base"
    if [[ -n ${wanted[$APP]:-} ]]; then colliding+=("$dbg"); else other=$((other + 1)); fi
  done < <(printf '%s\n' "${!have[@]}" | grep -- '-debug$')
  (( other )) && echo "    ($other other orphaned -debug package(s) from removed AUR builds; harmless here)"
  (( ${#colliding[@]} )) || return 0
  echo "    leftover debug package(s) from a removed build: ${colliding[*]}"
  echo "    they own files the new build's debug package needs, so the AUR step would fail."
  if ask_yn "Remove ${colliding[*]}?" y; then
    sudo pacman -Rns "${CONFIRM[@]}" "${colliding[@]}" || true
  fi
}

# filter_wanted: the same checks over the extra packages someone types at the
# installer's "anything else?" question, which may be repo or AUR names alike.
# Reads and rewrites the array WANTED.
filter_wanted() {
  local saved=("${PKGS[@]}") saved_aur=("${AUR_PKGS[@]}")
  # Each name goes in both lists: pacman -Si answers for the repo ones, yay
  # -Si --aur for the AUR ones, and a drop removes a name from both.
  PKGS=("${WANTED[@]}") AUR_PKGS=("${WANTED[@]}")
  drop_conflicts
  clear_orphan_debug
  WANTED=("${PKGS[@]}")
  PKGS=("${saved[@]}") AUR_PKGS=("${saved_aur[@]}")
}

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
