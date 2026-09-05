#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# ensure-yay.sh: make `yay` available before anything asks for an AUR package.
#
# A few things this rice needs are AUR-only (phinger-cursors, vesktop-bin,
# pamtester), and the usual bootstrap — clone the PKGBUILD, run makepkg — needs
# git and base-devel already in place and a manual detour out of the installer.
# yay publishes a static binary with every release, so curl it instead.
#
# TRUST: the tarball comes from Jguer/yay's GitHub releases over HTTPS. There is
# no upstream signature to check it against, so this is trust-on-first-use in
# the binary and in GitHub. If you would rather build it yourself:
#     sudo pacman -S --needed git base-devel
#     git clone https://aur.archlinux.org/yay-bin.git && cd yay-bin && makepkg -si
# and this script becomes a no-op.
#
# Idempotent: exits immediately if yay is already on PATH.
set -euo pipefail

# Only used if the GitHub API cannot be reached (rate limit, no DNS yet).
YAY_PINNED_VERSION=13.0.1

if command -v yay >/dev/null 2>&1; then
  echo "==> yay present: $(yay --version 2>/dev/null | head -1)"
  exit 0
fi

if ! command -v pacman >/dev/null 2>&1; then
  echo "!! ensure-yay: this is an Arch/pacman setup — no pacman, nothing to do." >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64)  arch=x86_64 ;;
  aarch64) arch=aarch64 ;;
  armv7l)  arch=armv7h ;;
  *)
    echo "!! ensure-yay: no yay release build for $(uname -m)." >&2
    echo "   Install an AUR helper by hand, then re-run." >&2
    exit 1 ;;
esac

# yay ships as a single binary, but BUILDING anything from the AUR still needs
# these. Installing them now means the first `yay -S <aur-pkg>` just works.
echo "==> Installing AUR build prerequisites (base-devel, git, curl)"
sudo pacman -S --needed --noconfirm base-devel git curl

# Latest release, with the pin as the fallback. Parsed with grep/sed on purpose:
# jq is one of the packages the installer has not installed yet at this point.
echo "==> Finding the latest yay release"
ver=$(curl -fsSL --max-time 20 https://api.github.com/repos/Jguer/yay/releases/latest 2>/dev/null \
      | grep -m1 '"tag_name"' | sed -E 's/.*"v?([0-9][0-9.]*)".*/\1/') || ver=''
if [[ ! $ver =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
  ver=$YAY_PINNED_VERSION
  echo "    could not reach the GitHub API — falling back to the pinned v$ver"
fi

tarball="yay_${ver}_${arch}.tar.gz"
url="https://github.com/Jguer/yay/releases/download/v${ver}/${tarball}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "==> Downloading $tarball"
if ! curl -fL --proto '=https' --tlsv1.2 --max-time 120 -o "$tmp/$tarball" "$url"; then
  echo "!! ensure-yay: download failed ($url)" >&2
  exit 1
fi

tar -xzf "$tmp/$tarball" -C "$tmp"
bin="$tmp/yay_${ver}_${arch}/yay"
if [[ ! -f $bin ]]; then
  echo "!! ensure-yay: $tarball did not contain the expected yay binary." >&2
  exit 1
fi

# Run it before trusting it with sudo: a wrong-arch or truncated download fails
# here rather than halfway through the package step.
chmod +x "$bin"
if ! "$bin" --version >/dev/null 2>&1; then
  echo "!! ensure-yay: the downloaded yay does not run on this machine." >&2
  exit 1
fi

echo "==> Installing yay to /usr/local/bin (plus man page and completions)"
sudo install -Dm755 "$bin" /usr/local/bin/yay
src="$tmp/yay_${ver}_${arch}"
[[ -f $src/yay.8 ]] && sudo install -Dm644 "$src/yay.8"  /usr/local/share/man/man8/yay.8
[[ -f $src/bash  ]] && sudo install -Dm644 "$src/bash"   /usr/local/share/bash-completion/completions/yay
[[ -f $src/zsh   ]] && sudo install -Dm644 "$src/zsh"    /usr/local/share/zsh/site-functions/_yay
[[ -f $src/fish  ]] && sudo install -Dm644 "$src/fish"   /usr/local/share/fish/vendor_completions.d/yay.fish
echo "    installed $(/usr/local/bin/yay --version | head -1)"

# Hand yay over to pacman. /usr/local/bin is not package-managed, so the binary
# installed above never gets updates and would shadow a later /usr/bin/yay.
# Installing the AUR package with itself fixes both; if it fails, the curl'd
# binary is still there and working, so this is never fatal.
if [[ -t 0 ]]; then
  read -rp "==> Let pacman manage yay from now on (build yay-bin from the AUR)? [Y/n] " a
  if [[ ${a:-Y} =~ ^[Yy]?$ ]]; then
    if /usr/local/bin/yay -S --needed --removemake yay-bin && [[ -x /usr/bin/yay ]]; then
      sudo rm -f /usr/local/bin/yay /usr/local/share/man/man8/yay.8 \
                 /usr/local/share/bash-completion/completions/yay \
                 /usr/local/share/zsh/site-functions/_yay \
                 /usr/local/share/fish/vendor_completions.d/yay.fish
      echo "    pacman owns yay now (/usr/bin/yay); removed the bootstrap copy."
    else
      echo "    AUR build did not complete — keeping the downloaded /usr/local/bin/yay."
      echo "    (Re-try later with: yay -S yay-bin)"
    fi
  else
    echo "    keeping /usr/local/bin/yay. It will NOT receive updates —"
    echo "    run 'yay -S yay-bin' whenever you want pacman to take it over."
  fi
else
  echo "    non-interactive: keeping /usr/local/bin/yay (unmanaged)."
  echo "    Run 'yay -S yay-bin' later to hand it to pacman."
fi
