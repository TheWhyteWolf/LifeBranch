#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# idle-suspend.sh — the last step of the swayidle chain, on machines that have
# one at all (lifeconf's idle.suspend_minutes; 0, the default, omits the step).
#
# Why a script instead of `systemctl suspend` straight in the swayidle argv:
#
#   1. Only on battery. A plugged-in laptop that has been idle for half an hour
#      is usually idle on purpose — compiling, downloading, serving something —
#      and the chain has already locked it and turned the screen off, which is
#      all that was wanted. Unplugged, the same half hour is just battery
#      burning down to nothing.
#   2. Only when sleep is actually available. install.sh offers to mask the
#      sleep targets on a machine that must never sleep; if someone does that
#      on a box whose theme.toml still asks for idle suspend, `systemctl
#      suspend` would fail every timeout and journal an error each time.
#
# Locking is NOT done here: swayidle's own before-sleep hook runs lifelock on
# the way down, and doing it twice races the locker against the suspend.
#
# LIFEBRANCH_FORCE_SUSPEND=1 skips the battery check (for testing the step
# without unplugging).
set -uo pipefail

log() { systemd-cat -t idle-suspend -p info echo "$*" 2>/dev/null || echo "idle-suspend: $*" >&2; }

# On battery when a charger exists and every one of them reads offline. A
# machine with no charger at all is a desktop: it never idle-suspends from
# here, whatever theme.toml says, because "unplugged" is not a state it has.
on_battery() {
  local found=0 f online
  for f in /sys/class/power_supply/*/type; do
    [[ -r $f && $(<"$f") == Mains ]] || continue
    # A charger whose `online` we cannot read tells us nothing, so it does not
    # count either way — same conservative rule as lifewall's Power::refresh.
    online="${f%/type}/online"
    [[ -r $online ]] || continue
    found=1
    [[ $(<"$online") == 0 ]] || return 1
  done
  (( found ))
}

if [[ ${LIFEBRANCH_FORCE_SUSPEND:-0} != 1 ]] && ! on_battery; then
  exit 0
fi

if systemctl is-enabled suspend.target 2>/dev/null | grep -qx masked; then
  log "suspend.target is masked; idle suspend skipped (unmask it, or set idle.suspend_minutes = 0 in lifeconf)"
  exit 0
fi

log "idle on battery — suspending"
exec systemctl suspend
