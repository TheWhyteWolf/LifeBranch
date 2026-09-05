#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# pinentry-fuzzel: an Assuan pinentry that prompts with fuzzel.
#
# The stock /usr/bin/pinentry wrapper picks pinentry-gnome3 under niri (it hits
# the `*` case in its backend guess), which draws an Adwaita dialog — light,
# icon'd, nothing to do with the theme. fuzzel is already the launcher, the
# power menu, the clipboard menu and the notification history, and its colours
# come out of lifeconf, so routing passphrases through it themes them for free.
#
# --prompt-only gives an input box that does not wait on stdin and implies
# --lines=0; --password masks the typing. fuzzel allows custom (unmatched)
# entries by default, so the typed text comes back on stdout.
#
# Wire it up in ~/.gnupg/gpg-agent.conf:
#     pinentry-program $HOME/.local/bin/pinentry-fuzzel.sh
# then: gpgconf --kill gpg-agent
#
# Tell it apart from the launcher: drop a fuzzel config at
# ~/.config/fuzzel/pinentry.ini and this uses it for passphrase prompts only
# (an urgent-red border makes a spoofed prompt obvious). Nothing else can
# redirect the prompt — the fuzzel binary is resolved here, never taken from
# the environment.
set -uo pipefail

# --- Backend selection -------------------------------------------------------
# gpg-agent execs a pinentry for EVERY passphrase, including ones raised where
# no compositor is in reach: over SSH, on a TTY, from a systemd unit or a timer.
# /usr/bin/pinentry falls back to curses/tty there; naming this script in
# gpg-agent.conf replaces that whole chain, so without a fallback of our own
# every signature and decrypt off the desktop would fail with a misleading
# "Operation cancelled". exec, not a subprocess: stdin/stdout ARE the Assuan
# pipe, and the replacement inherits them.
if [[ -z ${WAYLAND_DISPLAY:-} && -z ${SSH_CONNECTION:-} && -z ${SSH_TTY:-} ]]; then
  # A gpg-agent started before the graphical session (a TTY login, a socket
  # activation at boot) never got WAYLAND_DISPLAY, and prompting on a TTY the
  # user is not looking at reads as a hang. Adopt the session's socket — but
  # only when this is not a remote connection, where the terminal is the point.
  for _sock in "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"/wayland-[0-9]*; do
    [[ -S $_sock ]] || continue
    export WAYLAND_DISPLAY=${_sock##*/}
    break
  done
  unset _sock
fi

FUZZEL_BIN=$(command -v fuzzel 2>/dev/null) || FUZZEL_BIN=''
if [[ -z ${WAYLAND_DISPLAY:-} || -z $FUZZEL_BIN ]]; then
  for _fallback in /usr/bin/pinentry-curses /usr/bin/pinentry-tty /usr/bin/pinentry; do
    [[ -x $_fallback ]] && exec "$_fallback" "$@"
  done
  echo "pinentry-fuzzel: no Wayland display and no terminal pinentry installed" >&2
  exit 1
fi

# Hard ceiling on any prompt. A pinentry that never returns holds fuzzel's
# exclusive keyboard grab open, which locks up the whole session.
PIN_TIMEOUT=${PIN_TIMEOUT:-120}

# Passphrase prompts may carry their own fuzzel config so they cannot be
# mistaken for the launcher. Only this exact path is honoured.
PIN_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/fuzzel/pinentry.ini"
FUZZEL=("$FUZZEL_BIN" --log-level=none)
[[ -f $PIN_CONFIG ]] && FUZZEL+=(--config "$PIN_CONFIG")

desc=''      # SETDESC  — what is being unlocked
prompt=''    # SETPROMPT
errtext=''   # SETERROR — set on a retry after a bad passphrase
title=''     # SETTITLE — used as the message when there is no description
repeat=''    # SETREPEAT prompt; non-empty means GETPIN must ask twice
repeaterr='Passphrases do not match'   # SETREPEATERROR

# Assuan percent-decoding, into the variable named by $1. Strict: only a `%`
# followed by exactly two hex digits is an escape. A bare `%` is data and must
# survive — the loose `${s//%/\\x}` form silently ate the following character
# and made printf complain on stderr.
pct_decode() {
  local -n __dst=$1
  local s=$2 out='' i c
  for ((i = 0; i < ${#s}; i++)); do
    c=${s:i:1}
    if [[ $c == '%' && ${s:i+1:2} =~ ^[0-9a-fA-F]{2}$ ]]; then
      printf -v c '%b' "\\x${s:i+1:2}"
      ((i += 2))
    fi
    out+=$c
  done
  __dst=$out
}

# Only %, CR and LF must be escaped on the way back out.
pct_encode() {
  local s=$1 out='' i c
  for ((i = 0; i < ${#s}; i++)); do
    c=${s:i:1}
    case $c in
      '%')    out+='%25' ;;
      $'\n')  out+='%0A' ;;
      $'\r')  out+='%0D' ;;
      *)      out+=$c ;;
    esac
  done
  printf '%s' "$out"
}

# First line only: gpg-agent sends a multi-line description (key uid, algorithm,
# creation date) and fuzzel's prompt/message widgets are single-line.
first_line() { printf '%s' "${1%%$'\n'*}"; }

# ask_pin [PROMPT] [MESSAGE] — echoes the typed text, exit status says whether
# the user submitted at all. An empty line submitted with Enter is a real (empty)
# passphrase and exits 0; Escape, the timeout and a fuzzel that cannot start all
# exit non-zero.
ask_pin() {
  local p=${1:-${prompt:-Passphrase}} mesg=${2-} args
  # fuzzel renders the prompt inline, so keep it short and trailing-spaced.
  args=(--dmenu --password --prompt-only "$(first_line "${p%:}") ")
  [[ -n $mesg ]] && args+=(--mesg "$(first_line "$mesg")")
  # stdin MUST be /dev/null: this script's own stdin is the Assuan pipe from
  # gpg-agent, and fuzzel would otherwise inherit it and eat the protocol
  # stream. fuzzel holds an exclusive layer-shell keyboard grab, so a fuzzel
  # that blocks here takes the whole session's input down with it.
  # The timeout is the backstop for the same reason: never grab forever.
  timeout -k 5 "$PIN_TIMEOUT" "${FUZZEL[@]}" "${args[@]}" </dev/null 2>/dev/null
}

ask_confirm() {
  local choice
  choice=$(printf 'No\nYes\n' \
    | timeout -k 5 "$PIN_TIMEOUT" "${FUZZEL[@]}" --dmenu --only-match --lines 2 \
              --prompt "$(first_line "${desc:-confirm}")> " 2>/dev/null) || return 1
  [[ $choice == Yes ]]
}

# An OK box. --only-match with no entries can never return — fuzzel documents it
# as "will not return if no matching entry is selected" — so Enter would do
# nothing and the exclusive keyboard grab would be held for the full timeout.
# Feed it one entry so Enter dismisses the window.
show_message() {
  printf 'OK\n' \
    | timeout -k 5 "$PIN_TIMEOUT" "${FUZZEL[@]}" --dmenu --only-match --lines 1 \
              --prompt "$(first_line "$1") " >/dev/null 2>&1
}

printf 'OK Pleased to meet you\n'

while IFS= read -r line; do
  cmd=${line%% *}
  arg=''
  [[ $line == *' '* ]] && arg=${line#* }

  case ${cmd^^} in
    SETDESC)    pct_decode desc "$arg";    printf 'OK\n' ;;
    SETPROMPT)  pct_decode prompt "$arg";  printf 'OK\n' ;;
    SETERROR)   pct_decode errtext "$arg"; printf 'OK\n' ;;
    SETTITLE)   pct_decode title "$arg";   printf 'OK\n' ;;

    # A new passphrase is being set (gpg --full-gen-key, --change-passphrase):
    # ask twice and confirm with an `S PIN_REPEATED` status line. Answering a
    # bare OK here without ever sending that line is how a typo in a brand-new
    # key passphrase gets accepted unconfirmed — and the key becomes unopenable.
    SETREPEAT)      pct_decode repeat "${arg:-Repeat}"; printf 'OK\n' ;;
    SETREPEATERROR) pct_decode repeaterr "$arg";        printf 'OK\n' ;;

    GETPIN)
      pin=''
      ok=0
      if [[ -n $repeat ]]; then
        # Three rounds, then give up rather than loop at the user forever.
        mesg=${errtext:-${desc:-$title}}
        for _ in 1 2 3; do
          if ! pin=$(ask_pin "$prompt" "$mesg"); then break; fi
          if ! again=$(ask_pin "$repeat" "$mesg"); then break; fi
          if [[ $pin == "$again" ]]; then ok=1; break; fi
          mesg=$repeaterr
        done
        unset again
      else
        pin=$(ask_pin "$prompt" "${errtext:-${desc:-$title}}") && ok=1
      fi

      if (( ok )); then
        # The repeat is confirmed BEFORE the data line, the same order pinentry
        # uses; gpg-agent will not accept the passphrase without it.
        [[ -n $repeat ]] && printf 'S PIN_REPEATED 1\n'
        printf 'D %s\n' "$(pct_encode "$pin")"
        printf 'OK\n'
      else
        printf 'ERR 83886179 Operation cancelled\n'
      fi
      # A SETERROR only applies to the attempt it was set for.
      errtext=''
      unset pin ok
      ;;

    CONFIRM)
      if ask_confirm; then printf 'OK\n'
      else printf 'ERR 83886193 Not confirmed\n'; fi
      ;;

    MESSAGE)
      msg=${desc:-$title}
      [[ -n $msg ]] && show_message "$msg"
      printf 'OK\n'
      unset msg
      ;;

    GETINFO)
      case $arg in
        pid)     printf 'D %s\nOK\n' "$$" ;;
        version) printf 'D 1.0.0\nOK\n' ;;
        flavor)  printf 'D fuzzel\nOK\n' ;;
        ttyinfo) printf 'D - - - - 0/0 0\nOK\n' ;;
        *)       printf 'OK\n' ;;
      esac
      ;;

    RESET)
      desc=''; prompt=''; errtext=''; title=''; repeat=''
      repeaterr='Passphrases do not match'
      printf 'OK\n'
      ;;

    BYE)
      printf 'OK closing connection\n'
      exit 0
      ;;

    # OPTION, SETOK, SETCANCEL, SETNOTOK, SETQUALITYBAR, ... — accept and ignore.
    # A pinentry may decline to implement a *feature* it is asked about, but
    # ignoring one it just said OK to is worse than declining: see SETREPEAT.
    *) printf 'OK\n' ;;
  esac
done
