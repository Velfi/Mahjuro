#!/usr/bin/env bash
# Lit-mesh shader A/B matrix for shop relic inspect @ 1280x720, Visuals + high shadows.
set -euo pipefail
cd "$(dirname "$0")/.."

COMMON_ENV=(
  RUST_LOG=info,mahjuro_render::gpu_profiler=debug
  MAHJURO_GRAPHICS_MODE=visuals
  MAHJURO_HEADLESS_SHADOW_QUALITY=high
  MAHJURO_HEADLESS_GPU_PROFILE_FRAMES=40
)

SCENE_ARGS=(
  --scene shop
  --shop-focus relic:0
  --item-inspect
  --warmup-frames 60
  --width 1280
  --height 720
)

profiles=(
  baseline
  no_per_light_shadow
  no_combined_shadow
  no_shadow
  no_pcf
  no_spec
  one_light
  diffuse_only
  no_pcf,no_per_light_shadow
)

printf "%-28s %8s %8s %8s\n" "profile" "main" "shadow" "total"
printf "%-28s %8s %8s %8s\n" "-------" "----" "------" "-----"

for profile in "${profiles[@]}"; do
  out="/tmp/lit_mesh_profile_${profile//,/_}.png"
  log="$(env "${COMMON_ENV[@]}" MAHJURO_LIT_MESH_PROFILE="$profile" \
    cargo run -q -p mahjuro-headless --bin mahjuro-screenshot --features screenshot -- \
    "${SCENE_ARGS[@]}" --output "$out" 2>&1)"

  main="$(printf '%s\n' "$log" | rg '^\s+main\s+' | tail -1 | awk '{print $2}')"
  shadow="$(printf '%s\n' "$log" | rg '^\s+shadow\s+' | tail -1 | awk '{print $2}')"
  total="$(printf '%s\n' "$log" | rg '^\s+TOTAL\s+' | tail -1 | awk '{print $2}')"
  printf "%-28s %8s %8s %8s\n" "$profile" "${main:-?}" "${shadow:-?}" "${total:-?}"
done
