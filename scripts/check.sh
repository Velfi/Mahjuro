#!/usr/bin/env bash
#
# Local pre-push checks — mirrors .github/workflows/ci.yml "build-and-test" job:
#   1. Build offline bake tools (mahjuro-bake, decal/relic bakers)
#   2. cargo build --locked
#   3. cargo test --locked  (xvfb-run on Linux, same as CI)
#
# Does NOT run fmt, clippy, or Python tests (use --extras). CI does not run those either.
#
# Prerequisites: repo assets present (git lfs pull if tests fail on missing files).
#
# Usage:
#   scripts/check.sh [options]
#
# Options:
#   --skip-bake-tools   Skip step 1 (use when bake binaries are already in target/)
#   --target TRIPLE     Pass --target to cargo (default: host triple)
#   --release           Use release profile (CI uses debug)
#   --extras            Also run fmt --check, clippy, and Python unit tests
#   -h, --help          Show this help
#
# Examples:
#   scripts/check.sh
#   scripts/check.sh --extras
#   scripts/check.sh --target aarch64-apple-darwin

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

SKIP_BAKE_TOOLS=0
TARGET=""
PROFILE=debug
EXTRAS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-bake-tools) SKIP_BAKE_TOOLS=1 ;;
        --release)         PROFILE=release ;;
        --extras)          EXTRAS=1 ;;
        --target)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --target requires a value" >&2
                exit 1
            fi
            TARGET="$1"
            ;;
        --target=*) TARGET="${1#--target=}" ;;
        -h|--help)
            sed -n '3,26p' "$0"
            exit 0
            ;;
        *)
            echo "unknown flag: $1" >&2
            exit 1
            ;;
    esac
    shift
done

if [[ -z "$TARGET" ]]; then
    TARGET="$(rustc --print host-tuple)"
fi

CARGO_ARGS=(--locked)
if [[ -n "$TARGET" ]]; then
    CARGO_ARGS+=(--target "$TARGET")
fi
if [[ "$PROFILE" == release ]]; then
    CARGO_ARGS+=(--release)
fi

# Same as .github/actions/build-bake-tools — keep build.rs from re-running GPU bakes
# during the main build/test after tools are already in target/.
export_bake_skip_env() {
    export MAHJURO_SKIP_ROOM_GI_BAKE=1
    export MAHJURO_SKIP_ROOM_SHADOW_BAKE=1
    export MAHJURO_SKIP_SHOWCASE_DECAL_BAKE=1
    export MAHJURO_SKIP_RELIC_BAKE=1
}

build_bake_tools() {
    echo "==> [CI 1/3] Building offline bake tools (target=$TARGET profile=$PROFILE)"
    local bake=(cargo build "${CARGO_ARGS[@]}")
    export_bake_skip_env

    "${bake[@]}" -p mahjuro-headless --bin mahjuro-bake --features bake
    "${bake[@]}" -p mahjuro-render --bin mahjuro-bake-decal-atlases
    "${bake[@]}" -p mahjuro-render --bin mahjuro-bake-relics
}

run_tests() {
    if [[ "$(uname -s)" == "Linux" ]]; then
        if ! command -v xvfb-run >/dev/null 2>&1; then
            echo "error: xvfb-run not found (required for Linux tests, same as CI)" >&2
            exit 1
        fi
        xvfb-run --auto-servernum cargo test "${CARGO_ARGS[@]}"
    else
        cargo test "${CARGO_ARGS[@]}"
    fi
}

run_extras() {
    echo "==> cargo fmt --check (not in CI)"
    cargo fmt --check

    echo "==> cargo clippy (not in CI)"
    cargo clippy --locked

    echo "==> Python unit tests (not in CI)"
    python3 -m unittest discover -s scripts/tests -p 'test_*.py'
}

export_bake_skip_env

if [[ "$SKIP_BAKE_TOOLS" -eq 0 ]]; then
    build_bake_tools
else
    echo "==> Skipping bake tools (--skip-bake-tools); build.rs will use existing target/ binaries"
fi

echo "==> [CI 2/3] cargo build"
cargo build "${CARGO_ARGS[@]}"

echo "==> [CI 3/3] cargo test"
run_tests

if [[ "$EXTRAS" -eq 1 ]]; then
    run_extras
fi

echo "==> All CI checks passed (build-and-test). Push when ready."
