#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# prompt.sh: sourced helper. The one place that decides whether an installer
# stops to ask a question or just takes the answer already in the brackets.
#
# Two modes:
#   easy  every question answers itself with the default and says so out loud,
#         and pacman/yay run with --noconfirm (see $CONFIRM).
#   full  the installer asks about each step, exactly as it always did.
#
# Set it from outside with LIFEBRANCH_EASY=1 (easy) or LIFEBRANCH_EASY=0 (full),
# or let pick_mode ask once at the top of the installer. A run with no terminal
# at all (a pipe, a container, a systemd unit) is always easy: nobody is there
# to answer, and a half-configured desktop is worse than a defaulted one.
#
# Not executable on its own; `source` it.

# EASY and CONFIRM are what the installers read. _set_mode keeps them in step.
# CONFIRM is only ever used by the file that sources this one, hence the
# directive: shellcheck cannot see across the source boundary from here.
EASY=0
# shellcheck disable=SC2034
CONFIRM=()

_set_mode() {
  EASY=$1
  LIFEBRANCH_EASY=$1
  export LIFEBRANCH_EASY          # ensure-yay.sh and friends are child processes
  # shellcheck disable=SC2034      # read by the sourcing installer, not here
  if (( EASY )); then
    CONFIRM=(--noconfirm)
  else
    CONFIRM=()
  fi
}

# Whether the mode was already settled before this file was sourced, recorded
# before _set_mode writes its own answer back into LIFEBRANCH_EASY (which it
# has to, so child processes inherit the choice). Without this pick_mode would
# always find the variable set and never ask.
_MODE_CHOSEN=0
case ${LIFEBRANCH_EASY:-} in
  1) _set_mode 1; _MODE_CHOSEN=1 ;;
  0) _set_mode 0; _MODE_CHOSEN=1 ;;
  *) if [[ -t 0 ]]; then _set_mode 0; else _set_mode 1; fi ;;
esac

# True only when there is somebody at a terminal AND they asked to be asked.
interactive() { (( ! EASY )) && [[ -t 0 ]]; }

# pick_mode: ask which mode this run is, once, at the top of an installer.
# Skipped entirely when LIFEBRANCH_EASY already said, or when there is no
# terminal to ask at.
pick_mode() {
  (( _MODE_CHOSEN )) && return 0
  [[ -t 0 ]] || return 0
  echo
  echo "==> Install mode"
  echo "    1) Easy          take the recommended answer to every question:"
  echo "                     your detected keyboard and touchpad, no extra"
  echo "                     packages, pacman and yay with --noconfirm."
  echo "    2) Full control  ask about each step (extra packages, touchpad,"
  echo "                     keyboard, suspend, performance profile)."
  echo "    Either way you get the same desktop, and everything asked here can"
  echo "    be changed afterwards (lifeconf, or re-run this installer)."
  local a
  while true; do
    read -rp "    Mode [1]: " a || a=''
    case ${a:-1} in
      1|e|easy) _set_mode 1; echo "    easy mode: not asking anything else."; return 0 ;;
      2|f|full) _set_mode 0; echo "    full control: asking about each step."; return 0 ;;
      *)        echo "    1 or 2, please." ;;
    esac
  done
}

# ask_yn PROMPT DEFAULT(y|n) -> 0 when the answer is yes.
ask_yn() {
  local prompt="$1" default="$2" hint a
  [[ $default == y ]] && hint='[Y/n]' || hint='[y/N]'
  if interactive; then
    read -rp "    $prompt $hint " a || a=''
    a=${a:-$default}
  else
    a=$default
    echo "    $prompt $hint $a"
  fi
  [[ $a =~ ^[Yy] ]]
}
