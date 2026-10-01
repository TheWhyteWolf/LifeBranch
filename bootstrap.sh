#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# LifeBranch bootstrap: one command from a fresh Arch install to the full rice.
#
#     curl -fsSL https://raw.githubusercontent.com/TheWhyteWolf/LifeBranch/main/bootstrap.sh | bash
#
# or, if you would rather read it before running it (recommended, and it is
# short on purpose so that is a reasonable thing to do):
#
#     curl -fsSL https://raw.githubusercontent.com/TheWhyteWolf/LifeBranch/main/bootstrap.sh -o lifebranch.sh
#     less lifebranch.sh
#     bash lifebranch.sh
#
# All it does: install git, clone the repo to ~/LifeBranch, and run install.sh
# from it. Everything interesting happens there, in a file you can read on disk.
#
# Knobs (environment variables):
#   LIFEBRANCH_REMOTE  git URL to clone (default: the repo this came from)
#   LIFEBRANCH_BRANCH  branch to check out (default: main)
#   LIFEBRANCH_DIR     where to put the checkout (default: ~/LifeBranch)
#   LIFEBRANCH_VARIANT desktop | macbook (default: asked, or guessed from the hardware)
#   LIFEBRANCH_EASY    1 = easy install (recommended answer to every question),
#                      0 = full control. Default: the installer asks.
#   LIFEBRANCH_LOG     where to write the transcript (default: ~/lifebranch-install.log,
#                      "none" to keep the output on screen only)
set -euo pipefail

REMOTE=${LIFEBRANCH_REMOTE:-https://github.com/TheWhyteWolf/LifeBranch.git}
BRANCH=${LIFEBRANCH_BRANCH:-main}
DEST=${LIFEBRANCH_DIR:-$HOME/LifeBranch}

# `curl | bash` hands the installer's prompts (extra packages, touchpad,
# suspend policy) a pipe as stdin, where they would read EOF and be skipped.
# Getting a real terminal back to them is the whole reason this bootstrap is a
# separate file rather than piping install.sh straight into bash.
#
# It must NOT be done with `exec < /dev/tty`, which is what this script used to
# do: under `curl | bash` that same pipe is where bash is reading the script
# ITSELF from, one chunk at a time, so repointing fd 0 makes bash read the rest
# of this file from the terminal. Every line below the exec then silently never
# runs (the first thing typed comes back as "command not found"), no clone
# happens, and the whole install looks like it did nothing at all.
#
# So the terminal is attached per command instead, never to this shell: prompts
# here redirect from $ASK_FROM, and install.sh gets it on the exec at the end.
# The subshell probes /dev/tty first, since there are contexts (a container, a
# systemd unit) with no controlling terminal to open at all.
if [[ -t 0 ]]; then
  ASK_FROM=/dev/stdin           # run from a terminal already
elif (: < /dev/tty) 2>/dev/null; then
  ASK_FROM=/dev/tty             # piped in, but there is a terminal behind it
else
  ASK_FROM=/dev/null            # nobody there to ask
fi

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

# Keep a transcript. "It didn't appear to work" is the hardest kind of bug
# report to act on, and a `curl | bash` run scrolls its own evidence away.
# The exec'd installer inherits these descriptors, so the log covers the whole
# install, not just this file.
LOG=${LIFEBRANCH_LOG:-$HOME/lifebranch-install.log}
if [[ $LOG != none ]] && : >>"$LOG" 2>/dev/null; then
  exec > >(tee -a "$LOG") 2>&1
  printf '\n===== LifeBranch %s =====\n' "$(date -Is)"
  echo "(this run is being logged to $LOG)"
else
  LOG=none
fi
if [[ $ASK_FROM == /dev/null ]]; then
  echo "note: there is no terminal here to ask questions at, so the installer"
  echo "      will take the recommended answer to all of them (easy mode)."
fi

# set -e on its own exits silently, which is exactly what makes a failed
# install read as "nothing happened". Name the line that stopped it. The checks
# below explain themselves and exit through die(), so the trap keeps quiet for
# those rather than following a clear message with a vaguer one.
fail_line=
explained=0

# A checked failure that has already said what to do about itself.
die() {
  explained=1
  local line
  for line in "$@"; do echo "$line" >&2; done
  exit 1
}

on_exit() {
  local rc=$?
  (( rc )) || return 0
  (( explained )) && return 0
  echo
  if [[ -n $fail_line ]]; then
    echo "!! bootstrap.sh stopped at line $fail_line (exit $rc)."
  else
    echo "!! bootstrap.sh stopped (exit $rc)."
  fi
  echo "   Nothing was installed by this script beyond what the lines above say."
  [[ $LOG != none ]] && echo "   Full transcript: $LOG"
  return 0
}

trap 'fail_line=$LINENO' ERR
trap on_exit EXIT

if [[ $(id -u) -eq 0 ]]; then
  die "!! Run this as your own user, not as root — it sudos where it needs to."
fi
if ! command -v pacman >/dev/null 2>&1; then
  die "!! LifeBranch is an Arch Linux setup (pacman/yay). This machine has no pacman."
fi
# sudo, and the ability to actually use it. A fresh Arch install has neither by
# default, and without this check the first thing anybody sees is a package
# command failing several screens in.
if ! command -v sudo >/dev/null 2>&1; then
  die "!! No sudo on this machine, and installing packages needs it." \
      "   As root (su -), once:" \
      "       pacman -S sudo" \
      "       usermod -aG wheel $(id -un)" \
      "       EDITOR=nano visudo     # uncomment: %wheel ALL=(ALL:ALL) ALL" \
      "   Then log out, log back in, and run this again."
fi
say "Checking sudo (it may ask for your password)"
if ! sudo -v; then
  die "!! sudo would not authenticate $(id -un)." \
      "   Usually that means the account is not in the wheel group, or the" \
      "   %wheel line in /etc/sudoers is still commented out. Fix that as" \
      "   root (su -), then run this again."
fi
if [[ $REMOTE == *YOUR-GITHUB-USER* || -z $REMOTE ]]; then
  die "!! This bootstrap has no real git URL in it." \
      "   Set one:  LIFEBRANCH_REMOTE=https://github.com/<you>/LifeBranch.git bash $0"
fi

say "Installing git"
sudo pacman -S --needed --noconfirm git

if [[ -d $DEST/.git ]]; then
  say "Updating the existing checkout at $DEST"
  git -C "$DEST" pull --ff-only origin "$BRANCH"
elif [[ -e $DEST ]]; then
  die "!! $DEST exists and is not a git checkout. Move it aside, or set" \
      "   LIFEBRANCH_DIR to somewhere else."
else
  say "Cloning $REMOTE -> $DEST"
  git clone --branch "$BRANCH" "$REMOTE" "$DEST"
fi

# Two installers: the general one, and the T2 MacBook variant (HiDPI panel,
# Apple keyboard, brightness keys, plus the T2 suspend/audio/wifi plumbing).
variant=${LIFEBRANCH_VARIANT:-}
if [[ -z $variant ]]; then
  product=$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)
  if [[ $product == MacBook* ]]; then
    say "This looks like a $product"
    if [[ $ASK_FROM != /dev/null ]]; then
      read -rp "    Use the MacBook installer (T2 suspend/audio/keyboard fixes)? [Y/n] " a \
           < "$ASK_FROM"
      [[ ${a:-Y} =~ ^[Yy]?$ ]] && variant=macbook || variant=desktop
    else
      variant=macbook
    fi
  else
    variant=desktop
  fi
fi

case $variant in
  macbook) installer="$DEST/macbook/install.sh" ;;
  *)       installer="$DEST/install.sh" ;;
esac

say "Running $installer"
echo "    (everything from here is in the repo — read it any time: $installer)"
[[ $LOG != none ]] && echo "    (transcript: $LOG)"
# The redirect is what hands the installer a terminal to ask its questions at.
# It also keeps the leftovers of this script's own pipe away from its prompts.
exec bash "$installer" < "$ASK_FROM"
