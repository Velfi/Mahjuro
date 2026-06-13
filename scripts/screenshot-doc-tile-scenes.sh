#!/usr/bin/env bash
# Capture PNGs of doc-tile / flat-UI 3D tile scenes (guide, tutorial, labs, …).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-${ROOT}/screenshots/doc-tile-scenes}"
mkdir -p "$OUT"

export MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1

echo "Building mahjuro-screenshot…"
cargo build -p mahjuro-headless --bin mahjuro-screenshot --features screenshot -q
BIN="${ROOT}/target/debug/mahjuro-screenshot"

W=2560
H=1600
WARMUP=24

capture() {
  local scene=$1
  local out=$2
  shift 2
  echo "→ ${out#"$ROOT"/}"
  "$BIN" \
    --scene "$scene" \
    --output "$out" \
    --width "$W" \
    --height "$H" \
    --warmup-frames "$WARMUP" \
    "$@"
}

for scene in yaku_journal tile_select material_viewer tile_anchor_lab tile_stress_lab; do
  capture "$scene" "$OUT/${scene}.png"
done

for page in 1 2; do
  capture tutorial "$OUT/tutorial-$(printf '%02d' "$page").png" --page "$page"
done

# Base guide pages (1–7) plus yaku detail pages (all yaku unlocked → 17 total).
for page in $(seq 1 17); do
  capture guide "$OUT/guide-$(printf '%02d' "$page").png" --page "$page"
done

echo "Done — ${OUT}"
