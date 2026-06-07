#!/usr/bin/env bash
# Preflight checks for a Mahjuro-Store.app before Transporter upload.
#
# Usage: scripts/validate-macos-store.sh [path/to/Mahjuro-Store.app]

set -euo pipefail

APP="${1:-Mahjuro-Store.app}"
if [[ ! -d "$APP" ]]; then
    echo "error: $APP not found" >&2
    exit 1
fi

PLIST="$APP/Contents/Info.plist"
fail=0

check() {
    if ! "$@"; then
        fail=1
    fi
}

echo "== Info.plist =="
SHORT="$(plutil -extract CFBundleShortVersionString raw "$PLIST" 2>/dev/null || true)"
BUILD="$(plutil -extract CFBundleVersion raw "$PLIST" 2>/dev/null || true)"
BUNDLE="$(plutil -extract CFBundleIdentifier raw "$PLIST" 2>/dev/null || true)"
echo "  CFBundleIdentifier:      $BUNDLE"
echo "  CFBundleShortVersionString: $SHORT"
echo "  CFBundleVersion:         $BUILD"

if [[ -z "$SHORT" ]]; then
    echo "error: CFBundleShortVersionString is empty" >&2
    fail=1
fi
if [[ -z "$BUILD" ]]; then
    echo "error: CFBundleVersion is empty" >&2
    fail=1
fi
if [[ -n "$BUILD" && ! "$BUILD" =~ ^[0-9]+$ ]]; then
    echo "warning: CFBundleVersion should be numeric for App Store Connect" >&2
fi

echo "== Asset packs =="
for f in pack_manifest.json mahjuro-pack-shared.zip mahjuro-pack-rooms.zip \
         mahjuro-pack-gameplay-bulk.zip mahjuro-pack-music.zip; do
    if [[ ! -f "$APP/Contents/Resources/$f" ]]; then
        echo "error: missing Resources/$f" >&2
        fail=1
    else
        echo "  ok Resources/$f"
    fi
done

echo "== Provisioning profile =="
PROFILE="$APP/Contents/embedded.provisionprofile"
if [[ -f "$PROFILE" ]]; then
    echo "  ok Contents/embedded.provisionprofile"
    PROFILE_NAME="$(security cms -D -i "$PROFILE" 2>/dev/null | plutil -extract Name raw - 2>/dev/null || true)"
    if [[ -n "$PROFILE_NAME" ]]; then
        echo "  profile name: $PROFILE_NAME"
    fi
else
    echo "error: missing Contents/embedded.provisionprofile (set MAS_PROVISIONING_PROFILE when signing)" >&2
    fail=1
fi

echo "== Extended attributes =="
if xattr -lr "$APP" 2>/dev/null | grep -q 'com.apple.quarantine'; then
    echo "error: com.apple.quarantine present (ITMS-91109); packaging should run xattr -cr after signing" >&2
    xattr -lr "$APP" 2>/dev/null | grep 'com.apple.quarantine' >&2 || true
    fail=1
else
    echo "  ok (no quarantine)"
fi

echo "== Codesign =="
if codesign --verify --deep --strict --verbose=2 "$APP" 2>/dev/null; then
    echo "  codesign verify: ok"
    echo "  entitlements:"
    codesign -d --entitlements :- "$APP" 2>/dev/null | plutil -p - 2>/dev/null || true
else
    echo "warning: app is unsigned or codesign verify failed (expected before --sign)" >&2
fi

if [[ $fail -ne 0 ]]; then
    echo "validation FAILED" >&2
    exit 1
fi
echo "validation OK"
