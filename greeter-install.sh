#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Switch the login screen to greetd + lifegreet (the Game of Life greeter that
# matches the lifelock lock screen; tuigreet stays installed as the fallback).
# Separate from install.sh because it replaces the display manager — run it
# once, deliberately: bash ~/LifeBranch/greeter-install.sh
# Idempotent — safe to re-run (also how you deploy a rebuilt lifegreet).
# Needs sudo.
#
# Rollback (from a TTY, Ctrl+Alt+F3):
#   back to tuigreet: sudo install -Dm644 ~/LifeBranch/greetd/config-tuigreet.toml /etc/greetd/config.toml
#                     sudo systemctl restart greetd
#   off greetd:       sudo systemctl disable greetd && sudo systemctl enable <your old DM> && reboot
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# id -un, not $USER: $USER is unset under `env -i` (fatal under `set -u`), and
# it is `root` when the script is run with sudo — where the PAM verification
# below would test root's password-locked account and fail a good stack.
me=$(id -un)
if [[ $me == root ]]; then
  echo "!! Run this as your own user, not with sudo — it sudos where it needs to." >&2
  exit 1
fi

# yay: pamtester is AUR-only, and it is what proves the PAM stack works before
# anyone reboots into it. scripts/ensure-yay.sh is a no-op if yay is present.
bash "$REPO/scripts/ensure-yay.sh"

echo "==> Installing greetd + tuigreet (fallback) + cage + rice font + pamtester"
if command -v yay >/dev/null 2>&1; then
  yay -S --needed greetd greetd-tuigreet cage ttf-sharetech-mono-nerd pamtester
else
  sudo pacman -S --needed greetd greetd-tuigreet cage ttf-sharetech-mono-nerd
  echo "    (no yay — pamtester skipped; the PAM stack cannot be verified)"
fi

echo "==> Building lifegreet (Game of Life greeter)"
(cd "$REPO/lifegreet" && cargo build --release)
# /usr/local/bin, not ~/.local/bin: the binary runs as the `greeter` user.
sudo install -Dm755 "$REPO/lifegreet/target/release/lifegreet" /usr/local/bin/lifegreet
# Wrapper: wipes vt1 (cursor + text + scrollback) so the KMS handoffs flash
# pure black, then execs cage+lifegreet with output in the journal, not the VT.
sudo install -Dm755 "$REPO/greetd/lifegreet-cage" /usr/local/bin/lifegreet-cage

echo "==> Installing /etc/greetd/config.toml"
if [[ -f /etc/greetd/config.toml && ! -f /etc/greetd/config.toml.bak ]] \
   && ! cmp -s "$REPO/greetd/config.toml" /etc/greetd/config.toml; then
  sudo cp /etc/greetd/config.toml /etc/greetd/config.toml.bak
  echo "    backed up previous config to config.toml.bak"
fi
sudo install -Dm644 "$REPO/greetd/config.toml" /etc/greetd/config.toml

# PAM: greetd's own stack plus gnome-keyring auto-unlock, so logging in unlocks
# the login keyring and Brave/Element/Kleopatra stop prompting. Arch ships these
# lines in /etc/pam.d/sddm; moving to greetd left them behind. Ordering inside
# the file is load-bearing — see the header comment in greetd/pam/greetd.
echo "==> Installing /etc/pam.d/greetd (login + gnome-keyring unlock)"

# The live stack is NEVER the thing under test. A broken /etc/pam.d/greetd means
# nobody can log in graphically, and installing-then-verifying had two holes: on
# a machine with no .bak (a fresh box, or one whose live file already matched)
# the failure path's "restoring the previous stack" was a silent no-op, and on a
# machine with no pamtester the unverified file stayed and the script went on to
# enable greetd. So stage the new stack under a scratch service name,
# authenticate against THAT, and only promote it once it answers.
VERIFY_SVC=greetd-verify
trap 'sudo rm -f "/etc/pam.d/$VERIFY_SVC"' EXIT

sudo install -Dm644 "$REPO/greetd/pam/greetd" "/etc/pam.d/$VERIFY_SVC"

install_pam=0
if command -v pamtester >/dev/null 2>&1; then
  echo "    verifying the staged stack as $me (you will be asked for your password)"
  # authenticate + acct_mgmt is exactly what greetd asks of this stack.
  # open_session/close_session are deliberately NOT tested: pam_open_session
  # does not simulate anything — it really runs pam_gnome_keyring's auto_start,
  # spawning and tearing down a keyring daemon for $me out of a root sudo
  # environment (no session bus, wrong XDG_RUNTIME_DIR) while the real desktop
  # keyring is running.
  if sudo pamtester "$VERIFY_SVC" "$me" authenticate acct_mgmt; then
    echo "    verified — promoting to /etc/pam.d/greetd"
    install_pam=1
  else
    echo "    !! pamtester FAILED. /etc/pam.d/greetd is UNTOUCHED — nothing to roll back." >&2
    echo "       Fix greetd/pam/greetd and re-run." >&2
    exit 1
  fi
else
  echo "    !! pamtester is not installed, so the stack CANNOT be verified."
  echo "       A broken stack here means no graphical login. Keep a way back in"
  echo "       open first: a TTY (Ctrl+Alt+F3) or SSH from another machine."
  if [[ -t 0 ]]; then
    read -rp "    Install the UNVERIFIED PAM stack anyway? [y/N] " a
    [[ ${a:-N} =~ ^[Yy]$ ]] && install_pam=1
  else
    echo "       Non-interactive run: skipping. Re-run from a terminal to confirm."
  fi
  (( install_pam )) || echo "    skipped — /etc/pam.d/greetd left exactly as it was."
fi

if (( install_pam )); then
  if [[ -f /etc/pam.d/greetd && ! -f /etc/pam.d/greetd.bak ]] \
     && ! cmp -s "$REPO/greetd/pam/greetd" /etc/pam.d/greetd; then
    sudo cp /etc/pam.d/greetd /etc/pam.d/greetd.bak
    echo "    backed up previous PAM stack to /etc/pam.d/greetd.bak"
  fi
  sudo install -Dm644 "$REPO/greetd/pam/greetd" /etc/pam.d/greetd
  # The .bak may itself be a hand-edited file. The package's own stack is the
  # one guaranteed-pristine copy, so name the way back to it.
  echo "    (to restore the stock stack: sudo rm /etc/pam.d/greetd && sudo pacman -S greetd)"
fi

# Service drop-in: LimitMEMLOCK=infinity (lifegreet mlockalls ~112 MiB; the
# 8 MiB systemd default killed it at boot) and StartLimitIntervalSec=0 (a
# login screen must retry forever, never start-limit-hit into a dead vt1).
echo "==> Installing greetd service drop-in (memlock + restart policy)"
sudo install -Dm644 "$REPO/greetd/greetd.service.d/lifegreet.conf" \
  /etc/systemd/system/greetd.service.d/lifegreet.conf
sudo systemctl daemon-reload
sudo systemctl reset-failed greetd 2>/dev/null || true

# tuigreet (the fallback) needs a writable cache dir for --remember.
echo "==> Creating /var/cache/tuigreet (fallback greeter's remember cache)"
sudo install -d -o greeter -g greeter -m 755 /var/cache/tuigreet

# GRUB prints "Loading Linux linux ..." / "Loading initial ramdisk ..." before
# the kernel starts — the only text left on an otherwise quiet boot. Arch's
# /etc/grub.d/10_linux hardcodes the echoes (no quiet option), but grub.cfg is
# static — it only changes when grub-mkconfig is run by hand (e.g. after a grub
# package update), so deleting the lines from the generated file holds. Re-run
# this script after any grub-mkconfig to re-silence them.
if [[ -f /boot/grub/grub.cfg ]] \
   && sudo grep -q "^[[:space:]]*echo[[:space:]]*'Loading " /boot/grub/grub.cfg; then
  echo "==> Silencing GRUB's 'Loading Linux ...' boot echoes"
  sudo sed -i "/^[[:space:]]*echo[[:space:]]*'Loading /d" /boot/grub/grub.cfg
fi

# Swap display managers. enable/disable only touch next boot — the current
# session keeps running; greetd takes over vt1 after a reboot.
current="$(basename "$(readlink /etc/systemd/system/display-manager.service 2>/dev/null)" .service || true)"
if [[ -n "$current" && "$current" != greetd ]]; then
  echo "==> Disabling current display manager: $current"
  sudo systemctl disable "$current"
fi
echo "==> Enabling greetd"
sudo systemctl enable greetd

cat <<'EOF'

==> Done. greetd + lifegreet take over at next reboot. Never restart greetd
    from inside a session it started — that kills the session. Reboot instead.
    - Pre-reboot check (only from a session greetd did NOT start, e.g. a TTY
      login): sudo systemctl start greetd flips to the greeter on vt1 — look,
      switch back to your VT (Ctrl+Alt+F3...), then sudo systemctl stop greetd.
    - Type your username into the box (required EVERY login — by design),
      Enter grows the cube, then type your password: no text, only panel
      flares (rust flash + collapse back to the box on a wrong password).
    - Esc on an empty password backs out to the username box.
    - F3 = session picker (Niri / others), Ctrl+Alt+Del = reboot
      (no suspend and no shutdown key — by design).
    - Logs: journalctl -t lifegreet (greeter/cage) and -t greetd-session
      (session startup) — nothing writes to vt1, so handoffs flash black.
    - If it ever breaks: Ctrl+Alt+F3 to a TTY, then either
      tuigreet fallback:  sudo install -Dm644 ~/LifeBranch/greetd/config-tuigreet.toml /etc/greetd/config.toml
                          sudo systemctl restart greetd
      or leave greetd:    sudo systemctl disable greetd && sudo systemctl enable <old DM> && reboot
EOF
