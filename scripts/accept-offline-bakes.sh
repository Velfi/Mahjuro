#!/usr/bin/env bash
#
# Restore committed offline bake outputs (+ stamps) from a git ref, discarding
# local copies. Use when a teammate pushed new bakes and you want theirs, or to
# finish a merge/pull without hand-resolving binary conflicts.
#
# macOS / Linux: run from repo root (bash is fine on macOS).
#
# Usage:
#   scripts/accept-offline-bakes.sh [options] [ref] [kinds…]
#
# Ref (optional): commit/branch to take bakes from. Defaults:
#   - MERGE_HEAD while a merge is in progress (incoming side of git pull)
#   - @{upstream} when the current branch has one
#   - origin/HEAD otherwise
#
# Kinds (default: all) — same names as scripts/rebake-offline.sh:
#   lightmap, shadow, gi, room, decal, relic, all
#
# Options:
#   --lfs          Run `git lfs pull` before restoring (recommended after pull)
#   --verify       Run `cargo test -p mahjuro-bake-stamp` after restore
#   --no-stage     Do not `git add` restored paths (default: stage during merge)
#   -h, --help     Show this help
#
# Examples:
#   git pull   # binary bake conflicts
#   scripts/accept-offline-bakes.sh --lfs
#
#   scripts/accept-offline-bakes.sh origin/main shadow
#   scripts/accept-offline-bakes.sh --verify all

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

DO_LFS=0
DO_VERIFY=0
DO_STAGE=1
REF=""
kinds=()

usage() {
    sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --lfs) DO_LFS=1 ;;
        --verify) DO_VERIFY=1 ;;
        --no-stage) DO_STAGE=0 ;;
        -h|--help)
            usage
            exit 0
            ;;
        all|room|gi|lightmap|shadow|decal|relic)
            kinds+=("$1")
            ;;
        *)
            if [[ -z "$REF" ]]; then
                REF="$1"
            else
                echo "unexpected argument: $1" >&2
                exit 1
            fi
            ;;
    esac
    shift
done

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
        gi|lightmap) want_gi=1 ;;
        shadow) want_shadow=1 ;;
        decal) want_decal=1 ;;
        relic) want_relic=1 ;;
        *)
            echo "unknown kind: $kind (use lightmap, shadow, room, decal, relic, or all)" >&2
            exit 1
            ;;
    esac
done

resolve_ref() {
    if [[ -n "$REF" ]]; then
        if ! git rev-parse -q --verify "${REF}^{commit}" >/dev/null; then
            echo "error: invalid ref: $REF" >&2
            exit 1
        fi
        echo "$REF"
        return
    fi
    if git rev-parse -q --verify MERGE_HEAD >/dev/null; then
        echo MERGE_HEAD
        return
    fi
    if git rev-parse -q --verify @{upstream} >/dev/null; then
        echo @{upstream}
        return
    fi
    if git rev-parse -q --verify origin/HEAD >/dev/null; then
        echo origin/HEAD
        return
    fi
    echo "error: could not determine ref; pass one explicitly (e.g. origin/main)" >&2
    exit 1
}

merge_in_progress() {
    git rev-parse -q --verify MERGE_HEAD >/dev/null
}

restore_tree_from_ref() {
    local ref="$1"
    local path="$2"
    local count
    count="$(git ls-tree -r --name-only "$ref" -- "$path" 2>/dev/null | wc -l | tr -d ' ')"
    if [[ "$count" -eq 0 ]]; then
        echo "  skip $path (not in $ref)"
        return 0
    fi
    echo "  restore $path from $ref ($count file(s))"
    git checkout "$ref" -- "$path"
}

restore_tracked_under() {
    local ref="$1"
    local prefix="$2"
    local pattern="${3:-}"
    local listed
    listed="$(git ls-tree -r --name-only "$ref" -- "$prefix" 2>/dev/null || true)"
    if [[ -z "$listed" ]]; then
        echo "  skip $prefix (no tracked files in $ref)"
        return 0
    fi
    local paths=()
    while IFS= read -r rel; do
        [[ -z "$rel" ]] && continue
        if [[ -n "$pattern" ]] && [[ ! "$rel" =~ $pattern ]]; then
            continue
        fi
        paths+=("$rel")
    done <<<"$listed"
    if [[ ${#paths[@]} -eq 0 ]]; then
        echo "  skip $prefix (no paths matched)"
        return 0
    fi
    echo "  restore ${#paths[@]} file(s) under $prefix from $ref"
    git checkout "$ref" -- "${paths[@]}"
}

resolve_merge_theirs() {
    local path="$1"
    if merge_in_progress && git ls-files -u -- "$path" | grep -q .; then
        echo "  conflict → theirs: $path"
        git checkout --theirs -- "$path" 2>/dev/null || git checkout MERGE_HEAD -- "$path"
    fi
}

stage_paths() {
    local path="$1"
    if [[ "$DO_STAGE" -eq 1 ]] && merge_in_progress; then
        git add -- "$path"
    fi
}

REF_RESOLVED="$(resolve_ref)"
echo "==> accepting offline bakes from $REF_RESOLVED"

if [[ "$DO_LFS" -eq 1 ]]; then
    echo "==> git lfs pull"
    if command -v git-lfs >/dev/null 2>&1 || git lfs version >/dev/null 2>&1; then
        git lfs install --local 2>/dev/null || true
        git lfs pull
    else
        echo "warning: git-lfs not installed; stamp checks may fail until LFS objects are present" >&2
    fi
fi

if [[ "$want_shadow" -eq 1 ]]; then
    echo "==> room shadow (.msh)"
    resolve_merge_theirs assets/data/room_shadow
    restore_tree_from_ref "$REF_RESOLVED" assets/data/room_shadow
    stage_paths assets/data/room_shadow
fi

if [[ "$want_gi" -eq 1 ]]; then
    echo "==> room GI lightmaps"
    resolve_merge_theirs assets/data/room_lightmap
    restore_tree_from_ref "$REF_RESOLVED" assets/data/room_lightmap
    stage_paths assets/data/room_lightmap
fi

if [[ "$want_decal" -eq 1 ]]; then
    echo "==> showcase decal atlases"
    resolve_merge_theirs assets/textures/tile_sets/.decal_bake_stamp
    restore_tracked_under "$REF_RESOLVED" assets/textures/tile_sets '/showcase_decal_atlas\.png$'
    git checkout "$REF_RESOLVED" -- assets/textures/tile_sets/.decal_bake_stamp 2>/dev/null || true
    stage_paths assets/textures/tile_sets
fi

if [[ "$want_relic" -eq 1 ]]; then
    echo "==> relic RLC2"
    resolve_merge_theirs assets/data/relic_baked
    restore_tracked_under "$REF_RESOLVED" assets/data/relic_baked '\.rlc$'
    git checkout "$REF_RESOLVED" -- assets/data/relic_baked/.inputs_stamp 2>/dev/null || true
    stage_paths assets/data/relic_baked
fi

if merge_in_progress && [[ "$DO_STAGE" -eq 1 ]]; then
    echo "==> staged restored bake paths (continue merge with: git commit)"
fi

if [[ "$DO_VERIFY" -eq 1 ]]; then
    echo "==> verifying input stamps"
    cargo test -p mahjuro-bake-stamp hash_matches_committed_stamp -- --nocapture
fi

echo "done — local bake outputs now match $REF_RESOLVED"
