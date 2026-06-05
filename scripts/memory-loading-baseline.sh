#!/usr/bin/env bash
# Capture memory/loading baseline metrics for the memory-loading strategy plan.
# See docs/agents/memory-loading-budgets.md.
#
# Usage:
#   ./scripts/memory-loading-baseline.sh              # full soak (quit game when done)
#   ./scripts/memory-loading-baseline.sh --startup    # auto-stop after sync+async boot profiles
#   ./scripts/memory-loading-baseline.sh --summarize baseline-captures/20250604-120000

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MODE="soak"
SUMMARIZE_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --startup)
      MODE="startup"
      shift
      ;;
    --summarize)
      SUMMARIZE_DIR="${2:?pass capture directory}"
      shift 2
      ;;
    -h | --help)
      sed -n '2,8p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

# game.log may be empty on older captures; time -l merged stderr there too.
combined_log() {
  local dir="$1"
  if [[ -f "$dir/time.log" && -s "$dir/time.log" ]]; then
    cat "$dir/time.log"
  elif [[ -f "$dir/game.log" ]]; then
    cat "$dir/game.log"
  else
    return 1
  fi
}

summarize_capture() {
  local dir="$1"
  local log="$dir/game.log"
  local rss="$dir/rss-samples.tsv"
  local out="$dir/summary.txt"
  local merged
  merged="$(mktemp)"
  combined_log "$dir" >"$merged" || {
    echo "missing game.log / time.log in $dir" >&2
    return 1
  }

  {
    echo "=== memory-loading baseline summary ==="
    echo "capture: $dir"
    echo "machine: $(uname -srm)  $(sysctl -n hw.memsize 2>/dev/null | awk '{printf "RAM %.1f GiB", $1/1073741824}' || true)"
    echo

    echo "--- sync boot wall ---"
    grep -E "startup profile: sync boot.*\(wall [0-9]" "$merged" | tail -1 || echo "(not found)"
    echo

    echo "--- async boot wall ---"
    grep -E "startup profile: async boot.*\(wall [0-9]" "$merged" | tail -1 || echo "(not found)"
    echo

    echo "--- top startup scopes (sync boot table) ---"
    awk '
      /startup profile: sync boot/ { in_sync=1; next }
      /startup profile: async boot/ { in_sync=0 }
      in_sync && /startup_profile\]/ && !/──/ {
        line=$0
        sub(/^.*startup_profile\] +/, "", line)
        print line
      }
      /── end startup profile ──/ && in_sync { in_sync=0 }
    ' "$merged" | head -15
    echo

    echo "--- gpu mem profile (allocator snapshots) ---"
    grep "gpu mem profile:" "$merged" || echo "(none)"
    echo

    echo "--- room gpu profile (decode + upload) ---"
    grep "room gpu profile:" "$merged" || echo "(none — run hub tour during soak)"
    echo

    echo "--- hitches (prev frame dt >= 33 ms) ---"
    grep "room gpu profile:" "$merged" | grep -E "HITCH|dt [3-9][0-9]\.|dt [0-9]{3,}" || echo "(none logged)"
    echo

    echo "--- device lost / OOM ---"
    grep -Ei "device lost|out of memory|vk_error" "$merged" || echo "(none)"
    echo
  } >"$out"

  if [[ -f "$rss" ]]; then
    {
      echo "--- process RSS samples (KiB, every 5 s) ---"
      awk 'BEGIN { max=0 } { if ($2>max) max=$2; sum+=$2; n++ }
           END {
             if (n==0) { print "(no samples)"; exit }
             printf "samples=%d peak=%.1f MiB mean=%.1f MiB\n", n, max/1024, (sum/n)/1024
           }' "$rss"
    } >>"$out"
  fi

  if [[ -f "$log" ]]; then
    {
      echo "--- /usr/bin/time -l (macOS max RSS, bytes) ---"
      grep "maximum resident set size" "$log" | tail -1 || true
    } >>"$out"
  elif [[ -f "$dir/time.log" ]]; then
    {
      echo "--- /usr/bin/time -l (macOS max RSS, bytes) ---"
      grep "maximum resident set size" "$dir/time.log" | tail -1 || true
    } >>"$out"
  fi

  rm -f "$merged"
  cat "$out"
}

if [[ -n "$SUMMARIZE_DIR" ]]; then
  summarize_capture "$SUMMARIZE_DIR"
  exit 0
fi

BIN="$ROOT/target/release/mahjuro"
if [[ ! -x "$BIN" ]]; then
  echo "building release binary..."
  cargo build --release
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
CAPTURE_DIR="$ROOT/baseline-captures/$STAMP"
mkdir -p "$CAPTURE_DIR"

export MAHJURO_STARTUP_PROFILE=1
export MAHJURO_GPU_MEM_PROFILE=1
export MAHJURO_GRAPHICS_MODE=low_memory
export RUST_LOG=mahjuro=info

LOG="$CAPTURE_DIR/game.log"
TIME_LOG="$CAPTURE_DIR/time.log"
RSS_LOG="$CAPTURE_DIR/rss-samples.tsv"
META="$CAPTURE_DIR/meta.env"

{
  echo "MAHJURO_STARTUP_PROFILE=1"
  echo "MAHJURO_GPU_MEM_PROFILE=1"
  echo "MAHJURO_GRAPHICS_MODE=low_memory"
  echo "RUST_LOG=mahjuro=info"
  echo "mode=$MODE"
  echo "binary=$BIN"
  echo "started=$(date -Iseconds)"
} >"$META"

echo "capture dir: $CAPTURE_DIR"
echo "mode: $MODE"
if [[ "$MODE" == "soak" ]]; then
  echo
  echo "Manual checklist (see docs/agents/memory-loading-budgets.md):"
  echo "  1. Borderless 1920×1080"
  echo "  2. Hub tour: menu → shop → hallway → archive → gameplay → back"
  echo "  3. Options: switch tileset once"
  echo "  4. Play 30+ min, then quit"
  echo
fi

run_game() {
  /usr/bin/time -l "$BIN" --no-steam >>"$LOG" 2>&1
}

run_game &
GAME_PID=$!

echo "mahjuro pid=$GAME_PID"

# `/usr/bin/time` is the background job; sample the actual game process.
resolve_game_pid() {
  pgrep -x mahjuro 2>/dev/null | head -1 || echo "$GAME_PID"
}

(
  sample_pid=""
  for _ in $(seq 1 50); do
    sample_pid="$(resolve_game_pid)"
    [[ -n "$sample_pid" && "$sample_pid" != "$GAME_PID" ]] && break
    sleep 0.2
  done
  [[ -z "$sample_pid" ]] && sample_pid="$GAME_PID"
  while kill -0 "$sample_pid" 2>/dev/null; do
    rss="$(ps -o rss= -p "$sample_pid" 2>/dev/null | tr -d ' ' || true)"
    if [[ -n "$rss" ]]; then
      printf "%s\t%s\n" "$(date +%s)" "$rss"
    fi
    sleep 5
  done
) >"$RSS_LOG" &
RSS_PID=$!

if [[ "$MODE" == "startup" ]]; then
  (
    start_epoch=$(date +%s)
    deadline=$((start_epoch + 90))
    while kill -0 "$GAME_PID" 2>/dev/null && (( $(date +%s) < deadline )); do
      if [[ -f "$LOG" ]]; then
        merged_count="$(grep -c "── end startup profile ──" "$LOG" 2>/dev/null || true)"
        if [[ "$merged_count" -ge 2 ]]; then
          sleep 8
          kill -INT "$GAME_PID" 2>/dev/null || true
          break
        fi
      fi
      sleep 1
    done
    if kill -0 "$GAME_PID" 2>/dev/null; then
      echo "startup watchdog: timeout — sending INT" >&2
      kill -INT "$GAME_PID" 2>/dev/null || true
    fi
  ) &
  WATCH_PID=$!
fi

wait "$GAME_PID" || true
kill "$RSS_PID" 2>/dev/null || true
[[ "${WATCH_PID:-}" ]] && wait "$WATCH_PID" 2>/dev/null || true

echo "finished=$(date -Iseconds)" >>"$META"
echo
summarize_capture "$CAPTURE_DIR"
