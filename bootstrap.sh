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
set -euo pipefail

REMOTE=${LIFEBRANCH_REMOTE:-https://github.com/TheWhyteWolf/LifeBranch.git}
BRANCH=${LIFEBRANCH_BRANCH:-main}
DEST=${LIFEBRANCH_DIR:-$HOME/LifeBranch}

# `curl | bash` hands the script a pipe as stdin, so the installer's prompts
# (extra packages, touchpad, suspend policy) would all read EOF and be skipped.
# Reattach the terminal if there is one — this is the whole reason the bootstrap
# is a separate file rather than piping install.sh straight into bash.
# The subshell probes it first: a redirection error on `exec` kills a
# non-interactive shell outright, and there are contexts (a container, a
# systemd unit) with no controlling terminal to attach at all.
if [[ ! -t 0 ]] && (: < /dev/tty) 2>/dev/null; then
  exec < /dev/tty
fi

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

if [[ $(id -u) -eq 0 ]]; then
  echo "!! Run this as your own user, not as root — it sudos where it needs to." >&2
  exit 1
fi
if ! command -v pacman >/dev/null 2>&1; then
  echo "!! LifeBranch is an Arch Linux setup (pacman/yay). This machine has no pacman." >&2
  exit 1
fi
if [[ $REMOTE == *YOUR-GITHUB-USER* || -z $REMOTE ]]; then
  echo "!! This bootstrap has no real git URL in it." >&2
  echo "   Set one:  LIFEBRANCH_REMOTE=https://github.com/<you>/LifeBranch.git bash $0" >&2
  exit 1
fi

say "Installing git"
sudo pacman -S --needed --noconfirm git

if [[ -d $DEST/.git ]]; then
  say "Updating the existing checkout at $DEST"
  git -C "$DEST" pull --ff-only origin "$BRANCH"
elif [[ -e $DEST ]]; then
  echo "!! $DEST exists and is not a git checkout. Move it aside, or set" >&2
  echo "   LIFEBRANCH_DIR to somewhere else." >&2
  exit 1
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
    if [[ -t 0 ]]; then
      read -rp "    Use the MacBook installer (T2 suspend/audio/keyboard fixes)? [Y/n] " a
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
exec bash "$installer"
