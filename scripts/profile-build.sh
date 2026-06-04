#!/usr/bin/env bash
#
# Wall-clock profile of local CI-equivalent build steps (see scripts/check.sh).
# Use after a normal build for "hot" numbers (incremental / no-op rebuild).
#
# Usage:
#   scripts/profile-build.sh [options]
#
# Options:
#   --hot               Skip warmup; time one pass (assumes target/ is already warm)
#   --timings           Pass --timings to cargo (HTML report under target/cargo-timings/)
#   --build-bake-tools  Include offline bake tool builds (same as check.sh)
#   --target TRIPLE     Pass --target to cargo (default: host triple)
#   --release           Use release profile (CI uses debug)
#   --extras            Also time fmt, clippy, and Python unit tests
#   -h, --help          Show this help
#
# Examples:
#   scripts/profile-build.sh --hot
#   scripts/profile-build.sh --hot --timings
#   scripts/profile-build.sh --release --build-bake-tools

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

BUILD_BAKE_TOOLS=0
TARGET=""
PROFILE=debug
EXTRAS=0
HOT=0
CARGO_TIMINGS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --build-bake-tools) BUILD_BAKE_TOOLS=1 ;;
        --release)         PROFILE=release ;;
        --extras)          EXTRAS=1 ;;
        --hot)             HOT=1 ;;
        --timings)         CARGO_TIMINGS=1 ;;
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
            sed -n '3,22p' "$0"
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
if [[ "$CARGO_TIMINGS" -eq 1 ]]; then
    CARGO_ARGS+=(--timings)
fi

# label -> seconds (bash 3.2 on macOS has no associative arrays)
PROFILE_LABELS=()
PROFILE_SECONDS=()

now_s() {
    python3 -c 'import time; print(time.perf_counter())'
}

record_step() {
    local label=$1
    local start=$2
    local end
    end=$(now_s)
    local elapsed
    elapsed=$(python3 -c "print(round(float('$end') - float('$start'), 2))")
    PROFILE_LABELS+=("$label")
    PROFILE_SECONDS+=("$elapsed")
    printf "    finished in %.2fs\n" "$elapsed"
}

run_step() {
    local label=$1
    shift
    echo "==> $label"
    local start
    start=$(now_s)
    "$@"
    record_step "$label" "$start"
}

export_bake_skip_env() {
    export MAHJURO_SKIP_OFFLINE_BAKES=1
    export MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1
}

build_bake_tools() {
    local bake=(cargo build "${CARGO_ARGS[@]}")
    export_bake_skip_env

    run_step "bake: mahjuro-bake" "${bake[@]}" -p mahjuro-headless --bin mahjuro-bake --features bake
    run_step "bake: decal atlases" "${bake[@]}" -p mahjuro-render --bin mahjuro-bake-decal-atlases
    run_step "bake: relics" "${bake[@]}" -p mahjuro-render --bin mahjuro-bake-relics
}

run_tests() {
    if [[ "$(uname -s)" == "Linux" ]]; then
        if ! command -v xvfb-run >/dev/null 2>&1; then
            echo "error: xvfb-run not found (required for Linux tests, same as CI)" >&2
            exit 1
        fi
        run_step "cargo test (xvfb)" xvfb-run --auto-servernum cargo test "${CARGO_ARGS[@]}"
    else
        run_step "cargo test" cargo test "${CARGO_ARGS[@]}"
    fi
}

run_extras() {
    run_step "cargo fmt --check" cargo fmt --check
    run_step "cargo clippy" cargo clippy --locked
    run_step "python unit tests" python3 -m unittest discover -s scripts/tests -p 'test_*.py'
}

echo "Build profile: target=$TARGET profile=$PROFILE hot=$([[ $HOT -eq 1 ]] && echo yes || echo no) timings=$([[ $CARGO_TIMINGS -eq 1 ]] && echo yes || echo no)"

if [[ "$HOT" -eq 0 ]]; then
    echo "==> warmup (untimed cargo build — fills incremental cache)"
    cargo build "${CARGO_ARGS[@]}"
fi

if [[ "$BUILD_BAKE_TOOLS" -eq 1 ]]; then
    build_bake_tools
fi

run_step "cargo build" cargo build "${CARGO_ARGS[@]}"
run_tests

if [[ "$EXTRAS" -eq 1 ]]; then
    run_extras
fi

total=0
echo ""
echo "=== Summary (wall clock) ==="
for i in "${!PROFILE_LABELS[@]}"; do
    printf "  %-28s %6.2fs\n" "${PROFILE_LABELS[$i]}" "${PROFILE_SECONDS[$i]}"
    total=$(python3 -c "print(round($total + ${PROFILE_SECONDS[$i]}, 2))")
done
printf "  %-28s %6.2fs\n" "total (timed steps)" "$total"

if [[ "$CARGO_TIMINGS" -eq 1 ]]; then
    timings_dir="target/cargo-timings"
    if [[ -n "$TARGET" ]]; then
        timings_dir="target/$TARGET/cargo-timings"
    fi
    echo ""
    echo "Cargo --timings HTML: $REPO_ROOT/$timings_dir/ (open the latest .html)"
fi
