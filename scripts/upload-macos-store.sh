#!/usr/bin/env bash
#
# Build, sign, and upload a Mac App Store .pkg to App Store Connect.
#
# Usage:
#   scripts/upload-macos-store.sh [--universal]
#
# Required (signing — install from developer.apple.com → Certificates):
#   APPLE_MAS_APP_SIGNING_IDENTITY        "Apple Distribution: …"
#   APPLE_MAS_INSTALLER_SIGNING_IDENTITY  "3rd Party Mac Developer Installer: …"
#
# Required (upload — App Store Connect → Users and Access → Integrations):
#   APPLE_API_ISSUER                      Issuer UUID
#   APPLE_API_KEY                         Key ID (default: 33B59YFTBZ from ~/.private_keys)
#   APPLE_API_KEY_PATH                    Path to AuthKey_<KEY>.p8
#
# Optional:
#   MAS_BUILD_NUMBER   must increase for every upload (default from mas-version.sh)
#   MAS_BUNDLE_ID      default com.zelda-built-this.Mahjuro.store

set -euo pipefail

UNIVERSAL=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        -h|--help) sed -n '3,22p' "$0"; exit 0 ;;
        *) echo "unknown flag: $arg" >&2; exit 1 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

API_KEY="${APPLE_API_KEY:-33B59YFTBZ}"
API_KEY_PATH="${APPLE_API_KEY_PATH:-$HOME/.private_keys/AuthKey_${API_KEY}.p8}"
: "${APPLE_API_ISSUER:?Set APPLE_API_ISSUER (App Store Connect → Users and Access → Integrations → Issuer ID)}"

if [[ ! -f "$API_KEY_PATH" ]]; then
    echo "error: API key not found at $API_KEY_PATH" >&2
    exit 1
fi

scripts/mas-preflight.sh

PKG_ARGS=(--sign --validate)
[[ $UNIVERSAL -eq 1 ]] && PKG_ARGS=(--universal "${PKG_ARGS[@]}")
scripts/package-macos-store.sh "${PKG_ARGS[@]}"

PKG="$(ls -t mahjuro-store-v*-macos-*.pkg 2>/dev/null | head -1)"
if [[ -z "$PKG" || ! -f "$PKG" ]]; then
    echo "error: signed .pkg not found after packaging" >&2
    exit 1
fi

echo "Validating $PKG with App Store Connect…"
xcrun altool --validate-app \
    -f "$PKG" \
    -t macos \
    --apiKey "$API_KEY" \
    --apiIssuer "$APPLE_API_ISSUER"

echo "Uploading ${PKG}..."
xcrun altool --upload-app \
    -f "$PKG" \
    -t macos \
    --apiKey "$API_KEY" \
    --apiIssuer "$APPLE_API_ISSUER"

echo "Upload submitted. Processing usually takes 5–30 minutes; check App Store Connect → TestFlight / macOS builds."
