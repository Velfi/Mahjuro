#!/usr/bin/env bash
#
# Rebake committed offline outputs and refresh their stamp files.
# Run from repo root after changing bake inputs (room GLBs, shaders, tile sets, relics, …).
#
# Usage:
#   scripts/rebake-offline.sh [kinds…]
#
# Kinds (default: all):
#   gi, shadow   room bakes (mahjuro-bake)
#   room         gi + shadow
#   decal        showcase atlases
#   relic        relic RLC2
#   all          room + decal + relic
#
# Examples:
#   scripts/rebake-offline.sh
#   scripts/rebake-offline.sh gi shadow
#   scripts/rebake-offline.sh decal

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

kinds=("$@")
if [[ ${#kinds[@]} -eq 0 ]]; then
    kinds=(all)
fi

want_gi=0
want_shadow=0
want_decal=0
want_relic=0

for kind in "${kinds[@]}"; do
    case "$kind" in
        all)
            want_gi=1
            want_shadow=1
            want_decal=1
            want_relic=1
            ;;
        room)
            want_gi=1
            want_shadow=1
            ;;
        gi) want_gi=1 ;;
        shadow) want_shadow=1 ;;
        decal) want_decal=1 ;;
        relic) want_relic=1 ;;
        *)
            echo "unknown kind: $kind (use gi, shadow, room, decal, relic, or all)" >&2
            exit 1
            ;;
    esac
done

if [[ "$want_gi" -eq 1 || "$want_shadow" -eq 1 ]]; then
    bake_kinds=()
    [[ "$want_gi" -eq 1 ]] && bake_kinds+=(gi)
    [[ "$want_shadow" -eq 1 ]] && bake_kinds+=(shadow)
    kinds_csv=$(IFS=,; echo "${bake_kinds[*]}")

    echo "==> room bakes ($kinds_csv)"
    MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo build -p mahjuro-headless --bin mahjuro-bake --features bake
    MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds "$kinds_csv"
fi

if [[ "$want_decal" -eq 1 ]]; then
    echo "==> showcase decal atlases"
    cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases
fi

if [[ "$want_relic" -eq 1 ]]; then
    echo "==> relic RLC2 bakes"
    cargo run -p mahjuro-render --bin mahjuro-bake-relics
fi

echo "==> verifying stamps (cargo build --locked)"
cargo build --locked

echo "done — commit baked outputs + stamp files"
