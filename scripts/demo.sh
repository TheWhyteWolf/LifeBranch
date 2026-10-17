#!/usr/bin/env bash
#
# demo.sh — walk through LifeBranch's features for a screen recording.
#
# Run it from a terminal inside the niri session. It narrates each step in the
# terminal, drives niri/the apps for you, and puts everything back at the end
# (theme preset, wallpaper, DND, floating windows).
#
#   demo.sh            full tour
#   demo.sh --fast     shorter pauses
#
# Not shown on purpose: the lock screen (needs your password on camera) and the
# login screen (a separate session).

set -u

PAUSE=3
[[ "${1:-}" == "--fast" ]] && PAUSE=1.5

BIN="$HOME/.local/bin"
na() { niri msg action "$@" >/dev/null 2>&1; }
pause() { sleep "${1:-$PAUSE}"; }
step() { printf '\n\033[1;32m==> %s\033[0m\n' "$*"; }
say() { printf '    %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1 || [[ -x "$BIN/$1" ]]; }

ORIG_PRESET="$(lifeconf --print 2>/dev/null | sed -n 's/^active_preset *= *"\(.*\)"/\1/p' | head -1)"
ORIG_PRESET="${ORIG_PRESET:-olive}"

cleanup() {
    "$BIN/lifeconf" --preset "$ORIG_PRESET" >/dev/null 2>&1
    "$BIN/lifenote" ctl dnd off >/dev/null 2>&1
    printf '\n\033[1;32mDemo done.\033[0m  Theme restored to %s.\n' "$ORIG_PRESET"
}
trap cleanup EXIT
trap 'exit 130' INT

clear
printf '\033[1;32mLifeBranch\033[0m: a niri desktop built on Conway'"'"'s Game of Life\n'
say "Starting in 5 seconds. Start your recording now."
sleep 5

# ── 1. Wallpaper + bar ──────────────────────────────────────────────────────
step "The desktop: Game of Life wallpaper (lifewall) and text-only bar (lifebar)"
say "Workspaces, clock, NET/VOL/CPU/MEM readings, no icon fonts."
na open-overview; pause 4; na close-overview; pause 1.2

# ── 2. Cheat sheet ──────────────────────────────────────────────────────────
step "Keybind cheat sheet, generated live from the niri config (Mod+Slash)"
"$BIN/shortcuts-window.sh" >/dev/null 2>&1 &
pause 5
na close-window; pause 1

# ── 3. Launcher ─────────────────────────────────────────────────────────────
step "lifemenu: launcher and dmenu (Mod+Space)"
"$BIN/lifemenu" >/dev/null 2>&1 &
LM=$!
pause 4
kill "$LM" 2>/dev/null; pkill -x lifemenu 2>/dev/null; pause 1

step "lifemenu as a dmenu: the scripts' menus use it"
printf 'tiling\nfloating\ntabbed\nscrolling\n' | "$BIN/lifemenu" --dmenu -p 'layout> ' >/dev/null 2>&1 &
pause 4
pkill -x lifemenu 2>/dev/null; pause 1

# ── 4. Tiling / scrolling layout ────────────────────────────────────────────
step "Scrolling tiling: open three terminals, flow across columns"
# Each terminal runs its own fast Game of Life (lifewall in terminal mode), one colour scheme apiece.
# life_kitty BG MATURE NEWBORN
life_kitty() {
    na spawn -- kitty -o confirm_os_window_close=0 -o "background=$1" -e "$BIN/lifebg" \
        --tick 0.08 --fps 24 --fade 2 --density 0.25 --glider-interval 8 \
        --bg "$1" --mature "$2" --newborn "$3"
}
life_kitty '#121412' '#66744c' '#87a540'; pause 0.35
life_kitty '#10141a' '#4c6b8a' '#6fb4e8'; pause 0.35
life_kitty '#1a1012' '#8a4c5a' '#e8708a'; pause 0.35
pause 1
say "Focus left/right (Mod+H/L), resize presets (Mod+R), center (Mod+C)"
na focus-column-left; pause 1; na focus-column-left; pause 1
na switch-preset-column-width; pause 1.2; na switch-preset-column-width; pause 1.2
na center-column; pause 1.5
na focus-column-last; pause 1

step "Tabbed column (Mod+W) with the olive tab strip"
na focus-column-left
na consume-window-into-column; pause 1
na toggle-column-tabbed-display; pause 3
na toggle-column-tabbed-display; pause 1
na expel-window-from-column; pause 1

step "Overview (Mod+O)"
na open-overview; pause 4; na close-overview; pause 1.2

# ── 5. Floating ─────────────────────────────────────────────────────────────
step "Floating windows + snapping (Mod+V, Mod+Alt+arrows)"
na toggle-window-floating; pause 1.5
for d in left right up down center; do "$BIN/float-snap.sh" "$d" >/dev/null 2>&1; pause 1.2; done
"$BIN/float-snap.sh" restore >/dev/null 2>&1; pause 1
na toggle-window-floating; pause 1

step "Dropdown terminal (Mod+Grave)"
"$BIN/scratch-term.sh" >/dev/null 2>&1; pause 3
"$BIN/scratch-term.sh" >/dev/null 2>&1; pause 1

# ── 6. Workspaces ───────────────────────────────────────────────────────────
na close-overview; pause 0.5
step "Workspaces: move a column down, follow it, come back"
na move-column-to-workspace-down; pause 2
na focus-workspace-up; pause 1.5
na focus-workspace-down; pause 1.5

# ── 7. Files ────────────────────────────────────────────────────────────────
step "lifefiles: mouse-driven terminal file browser (Mod+E)"
na spawn -- kitty -o confirm_os_window_close=0 --class lifefiles -e "$BIN/lifefiles"; pause 5
na close-window; pause 1

# ── 8. Notifications ────────────────────────────────────────────────────────
step "lifenote: box-drawing notifications"
notify-send "LifeBranch" "Pure text, no icon fonts." ; pause 3
notify-send -u critical "Urgent" "Critical notifications get the urgent colour." ; pause 4
step "Do not disturb (Mod+N): this one should be held back"
"$BIN/lifenote" ctl dnd on >/dev/null 2>&1; pause 1.5
notify-send "You should not see this" "DND is on."; pause 2.5
"$BIN/lifenote" ctl dnd off >/dev/null 2>&1; pause 1.5

# ── 9. OSD ──────────────────────────────────────────────────────────────────
step "lifeosd: labelled text bar for volume"
"$BIN/vol-osd.sh" up >/dev/null 2>&1; pause 1
"$BIN/vol-osd.sh" up >/dev/null 2>&1; pause 1
"$BIN/vol-osd.sh" down >/dev/null 2>&1; pause 1
"$BIN/vol-osd.sh" down >/dev/null 2>&1; pause 1.5

# ── 10. Quick settings ──────────────────────────────────────────────────────
step "lifepanel: wifi, bluetooth, sound, brightness, power, drives (Mod+A)"
"$BIN/lifepanel" >/dev/null 2>&1 &
pause 6
pkill -x lifepanel 2>/dev/null; pause 1

# ── 11. Clipboard ───────────────────────────────────────────────────────────
step "Clipboard history (Mod+P)"
if have wl-copy; then
    for s in "first clip" "second clip" "third clip"; do printf '%s' "$s" | wl-copy; sleep 0.5; done
fi
"$BIN/clip-menu.sh" >/dev/null 2>&1 &
pause 4
pkill -x lifemenu 2>/dev/null; pause 1

# ── 12. lifeconf: theming ───────────────────────────────────────────────────
step "lifeconf: one theme.toml re-themes everything, live"
for p in olive slate moss; do
    say "preset: $p"
    "$BIN/lifeconf" --preset "$p" >/dev/null 2>&1
    pause 3.5
done
say "back to $ORIG_PRESET"
"$BIN/lifeconf" --preset "$ORIG_PRESET" >/dev/null 2>&1; pause 2

step "lifeconf settings GUI (Mod+S), opened on the palette panel"
"$BIN/lifeconf" --gui --panel palette >/dev/null 2>&1 &
pause 6
na close-window; pause 1

step "lifeconf TUI in a terminal"
na spawn -- kitty -o confirm_os_window_close=0 -e "$BIN/lifeconf"; pause 5
na close-window; pause 1

# ── 13. Wallpaper pause ─────────────────────────────────────────────────────
step "Pause / resume the wallpaper (Mod+Shift+G)"
"$BIN/lifebg-toggle.sh" >/dev/null 2>&1; pause 3
"$BIN/lifebg-toggle.sh" >/dev/null 2>&1; pause 2

# ── 14. Screenshot ──────────────────────────────────────────────────────────
step "lifeshot: freeze, select, annotate (Shift+Print)"
"$BIN/lifeshot" --full >/dev/null 2>&1 &
pause 6
say "(press Esc yourself if the overlay is still up)"
pkill -x lifeshot 2>/dev/null; pause 1

# ── 15. Power menu ──────────────────────────────────────────────────────────
step "Power menu (lock / suspend / logout, not executed)"
"$BIN/power-menu.sh" >/dev/null 2>&1 &
pause 4
pkill -x lifemenu 2>/dev/null; pause 1

# ── Outro ───────────────────────────────────────────────────────────────────
step "Closing: tidy up leftover windows"
for _ in 1 2 3; do na close-window; sleep 0.4; done
na focus-workspace 1
pause 2
