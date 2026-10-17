#!/usr/bin/env bash
#
# mic-boost.sh — Boost microphone volume to 24 dB using PulseAudio/PipeWire
#
# 24 dB in PulseAudio's linear volume scale is approximately 103868 (out of
# 65536 base = 0 dB).  The formula is:
#
#   pa_volume = 65536 * 10^(dB / 20)
#
# 65536 * 10^(24/20) = 65536 * 15.8489... ≈ 1038888   (but pactl also accepts
# a dB value directly, so we use that for clarity).

set -euo pipefail

TARGET_DB="24"

# ── Resolve the source to boost ──────────────────────────────────────────────
SOURCE="${1:-$(pactl get-default-source)}"

echo "🎤 Boosting source: ${SOURCE}"
echo "   Target gain:     +${TARGET_DB} dB"

# ── Apply the boost ──────────────────────────────────────────────────────────
# pactl set-source-volume accepts a dB value when suffixed with "dB".
pactl set-source-volume "${SOURCE}" "${TARGET_DB}dB"

# ── Verify ───────────────────────────────────────────────────────────────────
CURRENT=$(pactl get-source-volume "${SOURCE}")
echo "   New volume:      ${CURRENT}"
echo "✅ Done."
