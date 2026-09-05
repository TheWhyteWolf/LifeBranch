#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# detect-trackpad.sh: find the touchpad, report what it can do, and emit the
# matching niri `touchpad { ... }` block.
#
# Everything here reads sysfs and udev, which need no root and no X/Wayland
# session, so it works from the installer, from a TTY, or over SSH:
#   /sys/class/input/eventN/device/name              the model string
#   /sys/class/input/eventN/device/capabilities/abs  ABS_MT_* -> multitouch
#   /sys/class/input/eventN/device/capabilities/key  BTN_TOOL_*TAP -> finger count
#   /sys/class/input/eventN/device/properties        INPUT_PROP_BUTTONPAD -> clickpad
#   udevadm ID_INPUT_TOUCHPAD                        "this is a touchpad", not a mouse
#
# Usage:
#   detect-trackpad.sh              report every touchpad found (exit 1 if none)
#   detect-trackpad.sh --niri-block print the niri config block for the first one
#   detect-trackpad.sh --quiet      exit status only
set -uo pipefail

# --- Linux input codes we care about (include/uapi/linux/input-event-codes.h)
ABS_MT_SLOT=47          # 0x2f — MT protocol B (per-finger slots)
ABS_MT_POSITION_X=53    # 0x35 — any multitouch at all
KEY_BTN_LEFT=272        # 0x110
KEY_BTN_RIGHT=273       # 0x111
KEY_BTN_MIDDLE=274      # 0x112
KEY_BTN_TOOL_FINGER=325 # 0x145
KEY_BTN_TOOL_QUINTTAP=328
KEY_BTN_TOOL_DOUBLETAP=333
KEY_BTN_TOOL_TRIPLETAP=334
KEY_BTN_TOOL_QUADTAP=335
PROP_BUTTONPAD=2        # whole pad is the button (no separate physical buttons)
PROP_SEMI_MT=3          # reports a bounding box, not real per-finger positions

# bit_set BITMASK BIT. sysfs prints capability bitmasks as space-separated
# 64-bit words, most significant word FIRST. Indexed by nibble rather than with
# arithmetic so a word with bit 63 set cannot overflow bash's signed integers.
bit_set() {
  local mask=$1 bit=$2
  local -a w
  read -ra w <<<"$mask"
  local nwords=${#w[@]}
  local widx=$(( bit / 64 )) within=$(( bit % 64 ))
  local i=$(( nwords - 1 - widx ))
  (( i < 0 )) && return 1
  local word=${w[i]}
  local nib=$(( within / 4 )) off=$(( within % 4 ))
  local pos=$(( ${#word} - 1 - nib ))
  (( pos < 0 )) && return 1
  local d=${word:pos:1}
  (( (16#$d >> off) & 1 ))
}

# is_touchpad EVENTNAME. udev's classification is authoritative (it is what
# libinput itself keys off). Without udevadm, fall back to the shape of the
# device: a pointer that reports finger tools and multitouch positions.
is_touchpad() {
  local ev=$1
  if command -v udevadm >/dev/null 2>&1; then
    udevadm info --query=property --name="/dev/input/$ev" 2>/dev/null \
      | grep -q '^ID_INPUT_TOUCHPAD=1$' && return 0
    return 1
  fi
  local key abs
  key=$(cat "/sys/class/input/$ev/device/capabilities/key" 2>/dev/null) || return 1
  abs=$(cat "/sys/class/input/$ev/device/capabilities/abs" 2>/dev/null) || return 1
  bit_set "$key" "$KEY_BTN_TOOL_FINGER" && bit_set "$abs" "$ABS_MT_POSITION_X"
}

udev_prop() {
  command -v udevadm >/dev/null 2>&1 || return 1
  udevadm info --query=property --name="/dev/input/$1" 2>/dev/null \
    | sed -n "s/^$2=//p" | head -1
}

# Fills the globals describing one device.
probe() {
  local ev=$1 d="/sys/class/input/$1/device"
  TP_EVENT=$ev
  TP_NAME=$(cat "$d/name" 2>/dev/null || echo "unknown touchpad")
  TP_ABS=$(cat "$d/capabilities/abs" 2>/dev/null || echo 0)
  TP_KEY=$(cat "$d/capabilities/key" 2>/dev/null || echo 0)
  TP_PROP=$(cat "$d/properties" 2>/dev/null || echo 0)

  TP_MT=0; TP_MT_PROTO=none
  if bit_set "$TP_ABS" "$ABS_MT_POSITION_X"; then
    TP_MT=1
    TP_MT_PROTO=A
    bit_set "$TP_ABS" "$ABS_MT_SLOT" && TP_MT_PROTO=B
  fi

  TP_FINGERS=1
  bit_set "$TP_KEY" "$KEY_BTN_TOOL_DOUBLETAP" && TP_FINGERS=2
  bit_set "$TP_KEY" "$KEY_BTN_TOOL_TRIPLETAP" && TP_FINGERS=3
  bit_set "$TP_KEY" "$KEY_BTN_TOOL_QUADTAP"   && TP_FINGERS=4
  bit_set "$TP_KEY" "$KEY_BTN_TOOL_QUINTTAP"  && TP_FINGERS=5

  TP_BUTTONPAD=0; bit_set "$TP_PROP" "$PROP_BUTTONPAD" && TP_BUTTONPAD=1
  TP_SEMI_MT=0;   bit_set "$TP_PROP" "$PROP_SEMI_MT"   && TP_SEMI_MT=1
  TP_RIGHT_BTN=0; bit_set "$TP_KEY" "$KEY_BTN_RIGHT"   && TP_RIGHT_BTN=1
  TP_MIDDLE_BTN=0; bit_set "$TP_KEY" "$KEY_BTN_MIDDLE" && TP_MIDDLE_BTN=1
  bit_set "$TP_KEY" "$KEY_BTN_LEFT" || true

  TP_INTEGRATION=$(udev_prop "$ev" ID_INPUT_TOUCHPAD_INTEGRATION || true)
  TP_INTEGRATION=${TP_INTEGRATION:-$(udev_prop "$ev" ID_INTEGRATION || true)}
  TP_INTEGRATION=${TP_INTEGRATION:-unknown}
  TP_WIDTH=$(udev_prop "$ev" ID_INPUT_WIDTH_MM || true)
  TP_HEIGHT=$(udev_prop "$ev" ID_INPUT_HEIGHT_MM || true)

  # Apple pads are the one family where clickfinger (two-finger = right click)
  # is the behaviour people actually expect, because macOS trained them on it.
  TP_APPLE=0
  [[ ${TP_NAME,,} == *apple* ]] && TP_APPLE=1
}

find_touchpads() {
  local ev found=()
  for path in /sys/class/input/event*; do
    [[ -e $path ]] || continue
    ev=${path##*/}
    is_touchpad "$ev" && found+=("$ev")
  done
  # The guard is load-bearing: `printf '%s\n' "${empty[@]}"` prints one BLANK
  # LINE, which mapfile would happily read as a device named "" — so a desktop
  # with no touchpad would report finding one.
  (( ${#found[@]} )) && printf '%s\n' "${found[@]}"
  return 0
}

report() {
  local size='' pos=''
  [[ -n ${TP_WIDTH:-} && -n ${TP_HEIGHT:-} ]] && size=" ${TP_WIDTH}x${TP_HEIGHT} mm"
  echo "    $TP_NAME  (/dev/input/$TP_EVENT, $TP_INTEGRATION$size)"
  if (( TP_MT )); then
    if (( TP_SEMI_MT )); then
      pos="semi-MT — reports a two-finger bounding box, not real finger positions"
    else
      pos="MT protocol $TP_MT_PROTO"
    fi
    echo "      multitouch : yes ($pos), up to $TP_FINGERS finger(s) reported"
  else
    echo "      multitouch : NO. Single touch only; no two-finger scroll or tap"
  fi
  if (( TP_BUTTONPAD )); then
    echo "      buttons    : clickpad (the whole surface is the button)"
  elif (( TP_RIGHT_BTN )); then
    echo "      buttons    : physical left/right$( (( TP_MIDDLE_BTN )) && echo /middle )"
  else
    echo "      buttons    : left only"
  fi
}

# The generated niri block. Every line is a plain niri option, kept commented
# where the right answer is a preference rather than a fact about the hardware.
niri_block() {
  local click scroll
  if (( TP_BUTTONPAD )); then
    # clickfinger: 2-finger click = right. button-areas: bottom-right corner =
    # right, which is what Windows and most PC laptops do — the safer default
    # for someone arriving from Windows. Needs >=2 fingers either way.
    if (( TP_APPLE )) && (( TP_FINGERS >= 2 )); then click=clickfinger; else click=button-areas; fi
  else
    click=button-areas
  fi
  if (( TP_MT )) && (( TP_FINGERS >= 2 )); then scroll=two-finger; else scroll=edge; fi

  echo "    // Detected by scripts/detect-trackpad.sh — re-run it after a hardware"
  echo "    // change. Everything here is an ordinary niri option: edit freely."
  echo "    //   $TP_NAME ($TP_INTEGRATION)"
  if (( TP_MT )); then
    echo "    //   multitouch: yes, up to $TP_FINGERS fingers$( (( TP_SEMI_MT )) && echo ' (semi-MT: bounding box only)' )"
  else
    echo "    //   multitouch: no. Edge scrolling instead of two-finger"
  fi
  echo "    touchpad {"
  echo "        tap                          // tap the pad to click"
  echo "        dwt                          // disable-while-typing"
  if (( TP_APPLE )); then
    echo "        natural-scroll               // content follows the fingers (Apple default)"
  else
    echo "        // natural-scroll           // uncomment for macOS-style reversed scrolling"
  fi
  echo "        click-method \"$click\""
  if [[ $click == button-areas ]]; then
    echo "                                     // bottom-right of the pad = right click"
  else
    echo "                                     // two-finger click = right, three = middle"
  fi
  echo "        scroll-method \"$scroll\""
  echo "        accel-profile \"adaptive\""
  if (( TP_MT )) && (( TP_FINGERS >= 3 )) && (( TP_BUTTONPAD )) && [[ $click == button-areas ]]; then
    echo "        middle-emulation             // left+right together = middle click"
  fi
  echo "        // disabled-on-external-mouse // uncomment to switch the pad off when a mouse is plugged in"
  echo "    }"
}

mode=${1:---report}
mapfile -t pads < <(find_touchpads | grep .)
(( ${#pads[@]} )) || { [[ $mode == --quiet ]] || echo "    no touchpad found (desktop, or the driver did not load)"; exit 1; }

case $mode in
  --quiet) exit 0 ;;
  --niri-block)
    probe "${pads[0]}"
    niri_block
    ;;
  *)
    echo "==> Touchpad(s) found: ${#pads[@]}"
    for ev in "${pads[@]}"; do probe "$ev"; report; done
    ;;
esac
