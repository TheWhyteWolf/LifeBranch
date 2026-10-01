#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# test-snapshots.sh: run snapshots.sh's restore against a throwaway btrfs
# image, never the running system. Needs root for the loop device and mounts:
#
#   sudo bash scripts/test-snapshots.sh
#
# Builds @ and @snapshots/1/snapshot (read-only, as snapper makes them) in a
# 512 MB sparse file, changes @ afterwards, restores snapshot 1, and checks
# that @ is the old content and writable, the changed root was set aside, the
# default subvolume followed, a missing snapshot and a dry run change nothing,
# and leftovers --delete clears the set-aside root.
set -euo pipefail
(( EUID == 0 )) || { echo "run as root: sudo bash $0" >&2; exit 1; }

HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d /tmp/lb-snaptest.XXXXXX)
IMG=$WORK/disk.img
LOOP=
cleanup() {
  for m in "$WORK/check" "$WORK/top" "$WORK/build"; do mountpoint -q "$m" && umount "$m"; done
  [[ -n $LOOP ]] && losetup -d "$LOOP"
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok - $*"; }

truncate -s 512M "$IMG"
LOOP=$(losetup -f --show "$IMG")
mkfs.btrfs -q "$LOOP"
mkdir -p "$WORK/build" "$WORK/check"
mount "$LOOP" "$WORK/build"
B=$WORK/build
btrfs -q subvolume create "$B/@"
btrfs -q subvolume create "$B/@snapshots"
mkdir -p "$B/@/etc" "$B/@snapshots/1"
echo v1 > "$B/@/etc/marker"
btrfs -q subvolume snapshot -r "$B/@" "$B/@snapshots/1/snapshot"
printf '<?xml version="1.0"?>\n<snapshot>\n  <num>1</num>\n  <date>2026-10-01 12:00:00</date>\n  <description>pacman -Syu</description>\n</snapshot>\n' > "$B/@snapshots/1/info.xml"
echo v2 > "$B/@/etc/marker"
btrfs subvolume set-default "$B/@"
old_id=$(btrfs subvolume show "$B/@" | awk '/Subvolume ID:/ {print $3}')
umount "$B"

printf 'UUID=test / btrfs rw,noatime,subvol=/@ 0 0\n' > "$WORK/fstab"
snap() {
  SNAPSHOTS_DEV=$LOOP SNAPSHOTS_FSTAB=$WORK/fstab SNAPSHOTS_SNAP_SUBVOL=@snapshots SNAPSHOTS_TOP=$WORK/top \
    bash "$HERE/snapshots.sh" "$@"
}
check() { mount -o subvolid=5 "$LOOP" "$WORK/check"; }
uncheck() { umount "$WORK/check"; }

# A snapshot that doesn't exist: refused, nothing moved.
if snap restore 99 --yes >/dev/null 2>&1; then fail "restore 99 succeeded"; fi
check; [[ $(cat "$WORK/check/@/etc/marker") == v2 ]] || fail "restore 99 changed @"; uncheck
pass "a missing snapshot is refused and changes nothing"

# Dry run: prints, changes nothing.
snap restore 1 --yes --dry-run | grep -q "would run: mv" || fail "dry run printed no plan"
check; [[ $(cat "$WORK/check/@/etc/marker") == v2 ]] || fail "dry run changed @"; uncheck
pass "a dry run changes nothing"

# The real thing.
snap restore 1 --yes >/dev/null
check
C=$WORK/check
[[ $(cat "$C/@/etc/marker") == v1 ]] || fail "@ isn't snapshot 1"
[[ $(btrfs property get -ts "$C/@" ro) == "ro=false" ]] || fail "@ is read-only"
echo write-test > "$C/@/etc/written" || fail "can't write to the restored @"
aside=$(ls -d "$C"/@.pre-restore-* 2>/dev/null | head -n1)
[[ -n $aside && $(cat "$aside/etc/marker") == v2 ]] || fail "the old root wasn't set aside"
[[ $(btrfs subvolume show "$aside" | awk '/Subvolume ID:/ {print $3}') == "$old_id" ]] || fail "the set-aside root isn't the old subvolume"
new_id=$(btrfs subvolume show "$C/@" | awk '/Subvolume ID:/ {print $3}')
[[ $(btrfs subvolume get-default "$C" | awk '{print $2}') == "$new_id" ]] || fail "the default subvolume still points at the old root"
[[ -d $C/@snapshots/1/snapshot ]] || fail "the snapshot itself was consumed"
uncheck
pass "restore: @ is snapshot 1 and writable, the old root is set aside, the default followed"

snap leftovers | grep -q "@.pre-restore-" || fail "leftovers didn't list the set-aside root"
snap leftovers --delete --yes >/dev/null
check; ! ls -d "$WORK/check"/@.pre-restore-* >/dev/null 2>&1 || fail "leftovers --delete left it"; uncheck
pass "leftovers lists and deletes the set-aside root"

echo "all snapshot restore tests passed"
