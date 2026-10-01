#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# snapshots.sh: btrfs snapshots of / that a bad update can be undone from.
#
# snapper takes the snapshots: snap-pac before and after every pacman
# transaction, its timer hourly and daily. Getting back to one is the part
# Arch leaves to you, and `snapper rollback` doesn't fit the usual layout:
# fstab and the kernel command line name the root subvolume (`subvol=@`), so
# changing btrfs's default subvolume changes nothing. This script restores the
# way that layout wants: put @ aside, make a writable copy of the snapshot
# under the name @, reboot. The old root stays as @.pre-restore-<time> until
# `leftovers --delete` removes it.
#
# Supported layout: root in a named subvolume (@) and snapshots in a sibling
# subvolume mounted at /.snapshots (@snapshots), as archinstall sets it up.
# `setup` creates that layout when snapper isn't configured yet.
#
# Usage:
#   snapshots.sh status                   key=value lines (no root needed)
#   snapshots.sh setup [--yes]            configure snapper for /           (root)
#   snapshots.sh allow-group GROUP        let GROUP list/create snapshots   (root)
#   snapshots.sh restore N [--yes] [--dry-run]                              (root)
#   snapshots.sh leftovers [--delete]     the roots set aside by restore    (root)
#
# A restore works from the running system and from a console after booting
# a snapshot from the GRUB menu (that boot is read-only; the script writes
# only to the top-level mount it makes under /run).
set -euo pipefail

# Test hooks (scripts/test-snapshots.sh): point everything at a scratch
# filesystem instead of the running system's.
TOP=${SNAPSHOTS_TOP:-/run/lifebranch-snapshots/top}
FSTAB=${SNAPSHOTS_FSTAB:-/etc/fstab}
DRY=0
YES=0

die() { echo "snapshots: $*" >&2; exit 1; }
say() { echo "==> $*"; }
run() {
  if (( DRY )); then printf '    would run:'; printf ' %q' "$@"; echo; else "$@"; fi
}
need_root() { (( EUID == 0 )) || die "run this as root (sudo or pkexec)"; }
confirm() {
  (( YES )) && return 0
  local a
  read -r -p "$1 Type 'yes' to go ahead: " a
  [[ $a == yes ]] || die "cancelled"
}

# The root's subvolume as fstab names it ("@"), not as mounted now: booted
# into a snapshot, / is @snapshots/N/snapshot but the system's root is still @.
root_subvol() {
  awk '$2 == "/" && $3 == "btrfs" {
         n = split($4, o, ",")
         for (i = 1; i <= n; i++) if (o[i] ~ /^subvol=/) { s = substr(o[i], 8); sub(/^\//, "", s); print s }
       }' "$FSTAB" | head -n1
}
root_fs() { if [[ -n ${SNAPSHOTS_DEV:-} ]]; then echo btrfs; else findmnt -no FSTYPE /; fi; }
# @snapshots, if it is its own subvolume mounted at /.snapshots.
snap_subvol() {
  if [[ -n ${SNAPSHOTS_SNAP_SUBVOL:-} ]]; then echo "$SNAPSHOTS_SNAP_SUBVOL"; return 0; fi
  local r
  r=$(findmnt -no FSROOT /.snapshots 2>/dev/null) || return 0
  [[ $(findmnt -no FSTYPE /.snapshots) == btrfs ]] || return 0
  r=${r#/}
  [[ -n $r && $r != "$(root_subvol)"/* ]] && echo "$r"
  return 0
}
root_dev() {
  if [[ -n ${SNAPSHOTS_DEV:-} ]]; then echo "$SNAPSHOTS_DEV"; return 0; fi
  local s; s=$(findmnt -no SOURCE /); echo "${s%%[*}"
}
# The snapshot this boot is running from, if any.
booted_snapshot() {
  local r; r=$(findmnt -no FSROOT /)
  [[ $r =~ /([0-9]+)/snapshot$ ]] && echo "${BASH_REMATCH[1]}"
  return 0
}

mount_top() {
  mkdir -p "$TOP"
  mountpoint -q "$TOP" || mount -o subvolid=5 "$(root_dev)" "$TOP"
}
umount_top() { mountpoint -q "$TOP" && umount "$TOP"; rmdir "$TOP" 2>/dev/null || true; }

cmd_status() {
  local fs; fs=$(findmnt -no FSTYPE /)
  echo "fs=$fs"
  echo "root_subvol=$(root_subvol)"
  echo "snap_subvol=$(snap_subvol)"
  echo "snapper=$([[ -f /etc/snapper/configs/root ]] && echo yes || echo no)"
  echo "readable=$(snapper -c root --csvout list >/dev/null 2>&1 && echo yes || echo no)"
  echo "booted_snapshot=$(booted_snapshot)"
  echo "grub=$([[ -f /boot/grub/grub.cfg ]] && echo yes || echo no)"
  echo "grub_btrfs=$(systemctl is-active grub-btrfsd 2>/dev/null || true)"
  local ok=yes
  [[ $fs == btrfs && -n $(root_subvol) && -n $(snap_subvol) ]] || ok=no
  echo "restorable=$ok"
}

cmd_restore() {
  local n=${1:-}
  [[ $n =~ ^[0-9]+$ ]] || die "restore takes a snapshot number (see: snapper -c root list)"
  need_root
  [[ $(root_fs) == btrfs ]] || die "/ is not btrfs"
  local root snaps
  root=$(root_subvol)
  snaps=$(snap_subvol)
  [[ -n $root ]] || die "fstab mounts / without subvol=: the root must be a named subvolume such as @ to be swapped"
  [[ -n $snaps ]] || die "/.snapshots isn't its own subvolume beside $root; this layout can't be restored by swapping (run: snapshots.sh setup)"
  mount_top
  trap umount_top EXIT
  local src="$TOP/$snaps/$n/snapshot"
  btrfs subvolume show "$src" >/dev/null 2>&1 || die "no snapshot $n (looked for $snaps/$n/snapshot)"
  [[ -d $TOP/$root ]] || die "no subvolume $root at the top of the filesystem"
  local stamp aside desc
  stamp=$(date +%Y%m%d-%H%M%S)
  aside="$root.pre-restore-$stamp"
  desc=$(sed -n 's:.*<description>\(.*\)</description>.*:\1:p' "$TOP/$snaps/$n/info.xml" 2>/dev/null | head -n1)
  local date; date=$(sed -n 's:.*<date>\(.*\)</date>.*:\1:p' "$TOP/$snaps/$n/info.xml" 2>/dev/null | head -n1)
  echo "Restore snapshot $n${date:+ ($date UTC)}${desc:+: $desc}"
  echo "  the current root ($root) is kept as $aside"
  echo "  a writable copy of snapshot $n becomes $root"
  echo "  /home, /var/log and the package cache are separate subvolumes and are not touched"
  echo "  takes effect at the next boot"
  confirm "Restore snapshot $n?"
  local default_id root_id
  default_id=$(btrfs subvolume get-default "$TOP" | awk '{print $2}')
  root_id=$(btrfs subvolume show "$TOP/$root" | awk '/Subvolume ID:/ {print $3}')
  run mv "$TOP/$root" "$TOP/$aside"
  if ! run btrfs subvolume snapshot "$src" "$TOP/$root"; then
    run mv "$TOP/$aside" "$TOP/$root"
    die "copying the snapshot failed; nothing changed"
  fi
  # A default subvolume pointing at the old root would boot it wherever a
  # bootloader goes by the default rather than by name; follow the name.
  if [[ $default_id == "$root_id" ]]; then
    run btrfs subvolume set-default "$TOP/$root"
  fi
  run sync
  say "Done. Reboot to start snapshot $n. The old root is $aside; once you're happy, remove it with: snapshots.sh leftovers --delete"
}

cmd_leftovers() {
  need_root
  local root; root=$(root_subvol)
  [[ -n $root ]] || die "the root isn't a named subvolume"
  mount_top
  trap umount_top EXIT
  local found=()
  for d in "$TOP/$root".pre-restore-*; do [[ -d $d ]] && found+=("$d"); done
  if (( ! ${#found[@]} )); then echo "no set-aside roots"; return 0; fi
  for d in "${found[@]}"; do echo "${d#"$TOP"/}"; done
  [[ ${1:-} == --delete ]] || return 0
  confirm "Delete ${#found[@]} set-aside root(s)? They can't be restored afterwards."
  # A root may hold nested subvolumes (systemd makes some under /var/lib).
  for d in "${found[@]}"; do run btrfs subvolume delete --recursive "$d"; done
}

cmd_allow_group() {
  need_root
  local g=${1:?group}
  run snapper -c root set-config "ALLOW_GROUPS=$g" "SYNC_ACL=yes"
}

cmd_setup() {
  need_root
  [[ $(findmnt -no FSTYPE /) == btrfs ]] || die "/ is not btrfs: snapshots need btrfs"
  local root; root=$(root_subvol)
  [[ -n $root ]] || die "/ is the filesystem's top level, not a named subvolume; restoring needs a layout like archinstall's (@, @snapshots)"
  if [[ -f /etc/snapper/configs/root ]]; then
    say "snapper is already set up for /; letting the wheel group read it"
    cmd_allow_group wheel
    return 0
  fi
  local pkgs=(snapper snap-pac)
  [[ -f /boot/grub/grub.cfg ]] && pkgs+=(grub-btrfs)
  say "Installing ${pkgs[*]}"
  run pacman -S --needed --noconfirm "${pkgs[@]}"

  if mountpoint -q /.snapshots; then
    # archinstall's layout: @snapshots exists and is mounted. snapper wants
    # to create /.snapshots itself, so step it aside, then put it back.
    say "Using the existing /.snapshots subvolume"
    run umount /.snapshots
    run rmdir /.snapshots
    run snapper --no-dbus -c root create-config /
    run btrfs subvolume delete /.snapshots
    run mkdir /.snapshots
    run mount /.snapshots
  else
    # No @snapshots: make one beside the root and mount it from fstab.
    local snaps="@snapshots" dev uuid opts
    dev=$(root_dev)
    uuid=$(blkid -s UUID -o value "$dev")
    opts=$(awk '$2 == "/" && $3 == "btrfs" {print $4}' /etc/fstab | head -n1)
    opts=$(sed -E 's/(^|,)subvol(id)?=[^,]*//g; s/^,//' <<<"$opts")
    say "Creating $snaps beside $root"
    run snapper --no-dbus -c root create-config /
    run btrfs subvolume delete /.snapshots
    mount_top
    trap umount_top EXIT
    [[ -e $TOP/$snaps ]] || run btrfs subvolume create "$TOP/$snaps"
    run cp /etc/fstab "/etc/fstab.lifebranch-$(date +%Y%m%d-%H%M%S)"
    if (( DRY )); then
      echo "    would add to fstab: UUID=$uuid /.snapshots btrfs ${opts:+$opts,}subvol=/$snaps 0 0"
    else
      printf 'UUID=%s\t/.snapshots\tbtrfs\t%s\t0 0\n' "$uuid" "${opts:+$opts,}subvol=/$snaps" >> /etc/fstab
    fi
    run mkdir -p /.snapshots
    run systemctl daemon-reload
    run mount /.snapshots
  fi
  run chmod 750 /.snapshots

  # Modest limits: a week of dailies, a few hours, ten package transactions.
  run snapper -c root set-config \
    ALLOW_GROUPS=wheel SYNC_ACL=yes \
    TIMELINE_CREATE=yes TIMELINE_CLEANUP=yes NUMBER_CLEANUP=yes \
    TIMELINE_LIMIT_HOURLY=5 TIMELINE_LIMIT_DAILY=7 TIMELINE_LIMIT_WEEKLY=0 \
    TIMELINE_LIMIT_MONTHLY=0 TIMELINE_LIMIT_QUARTERLY=0 TIMELINE_LIMIT_YEARLY=0 \
    NUMBER_LIMIT=2-10 NUMBER_LIMIT_IMPORTANT=4-10 SPACE_LIMIT=0.5 FREE_LIMIT=0.2
  run systemctl enable --now snapper-timeline.timer snapper-cleanup.timer
  if [[ -f /boot/grub/grub.cfg ]]; then
    run systemctl enable --now grub-btrfsd.service
    run grub-mkconfig -o /boot/grub/grub.cfg
  fi
  run snapper -c root create -d "LifeBranch: snapshots set up"
  say "Snapshots are on: before and after every pacman transaction, hourly and daily."
}

main() {
  local cmd=${1:-status}; shift || true
  local args=()
  for a in "$@"; do
    case $a in
      --yes) YES=1 ;;
      --dry-run) DRY=1 ;;
      *) args+=("$a") ;;
    esac
  done
  case $cmd in
    status) cmd_status ;;
    setup) cmd_setup ;;
    allow-group) cmd_allow_group "${args[@]}" ;;
    restore) cmd_restore "${args[@]}" ;;
    leftovers) cmd_leftovers "${args[@]}" ;;
    -h|--help|help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    *) die "unknown command $cmd (try --help)" ;;
  esac
}

main "$@"
