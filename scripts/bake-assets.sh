#!/usr/bin/env bash
# Wrapper: bake assets/ into ZIP packs + pack_manifest.json (see tools/bake_assets/README.md).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$REPO_ROOT/tools/bake_assets/bake_assets.py" "$@"
