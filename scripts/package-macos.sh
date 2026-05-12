#!/usr/bin/env bash
#
# Build a local Mahjuro.app bundle and .dmg on macOS.
#
# Usage:
#   scripts/package-macos.sh [--universal] [--sign] [--notarize]
#
# Flags:
#   --universal   Build a universal (arm64 + x86_64) binary. Default: host arch only.
#   --sign        Codesign the .app and .dmg. Requires APPLE_SIGNING_IDENTITY
#                 (e.g. "Developer ID Application: Name (TEAMID)") in the env,
#                 or the script will pick the first "Developer ID Application"
#                 identity it finds.
#   --notarize    Submit to Apple notary service and staple. Implies --sign.
#                 Requires APPLE_API_KEY_PATH, APPLE_API_KEY, APPLE_API_ISSUER.
#
# Output:
#   Mahjuro.app
#   mahjuro-v<version>-macos-<arch>.dmg    (arch = "universal" with --universal)

set -euo pipefail

UNIVERSAL=0
SIGN=0
NOTARIZE=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        --sign)      SIGN=1 ;;
        --notarize)  SIGN=1; NOTARIZE=1 ;;
        -h|--help)   sed -n '3,22p' "$0"; exit 0 ;;
        *) echo "unknown flag: $arg" >&2; exit 1 ;;
    esac
done

if [[ "$(uname)" != "Darwin" ]]; then
    echo "error: this script only runs on macOS" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(awk -F'"' '/^\[package\]/{p=1} p && /^version/{print $2; exit}' Cargo.toml)"
TAG="v${VERSION}"

# ─────────────────────────── Build binary ───────────────────────────
if [[ $UNIVERSAL -eq 1 ]]; then
    ARCH_LABEL="universal"
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/release-universal
    lipo -create \
        target/aarch64-apple-darwin/release/mahjuro \
        target/x86_64-apple-darwin/release/mahjuro \
        -output target/release-universal/mahjuro
    BIN="target/release-universal/mahjuro"
else
    case "$(uname -m)" in
        arm64)  ARCH_LABEL="arm64" ;;
        x86_64) ARCH_LABEL="x86_64" ;;
        *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
    esac
    cargo build --release
    BIN="target/release/mahjuro"
fi

# ─────────────────────────── .app bundle ───────────────────────────
APP="Mahjuro.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/mahjuro"
chmod +x "$APP/Contents/MacOS/mahjuro"
# Steamworks redistributable. The dylib's install_name is
# `@loader_path/libsteam_api.dylib`, so it must live next to the binary
# inside Contents/MacOS/. Prefer the SDK copy when STEAM_SDK_LOCATION is
# set; otherwise reuse the file build.rs placed next to the release binary
# (same sources as .github/workflows/release.yml for universal builds).
STEAM_DYLIB_DST="$APP/Contents/MacOS/libsteam_api.dylib"
SDK_DYLIB="${STEAM_SDK_LOCATION:-}/redistributable_bin/osx/libsteam_api.dylib"
if [[ -n "${STEAM_SDK_LOCATION:-}" && -f "$SDK_DYLIB" ]]; then
    cp "$SDK_DYLIB" "$STEAM_DYLIB_DST"
elif [[ $UNIVERSAL -eq 1 && -f target/aarch64-apple-darwin/release/libsteam_api.dylib ]]; then
    cp target/aarch64-apple-darwin/release/libsteam_api.dylib "$STEAM_DYLIB_DST"
elif [[ -f target/release/libsteam_api.dylib ]]; then
    cp target/release/libsteam_api.dylib "$STEAM_DYLIB_DST"
else
    echo "error: libsteam_api.dylib not found for bundling." >&2
    echo "       Run this script after \`cargo build --release\` (or --universal), or set" >&2
    echo "       STEAM_SDK_LOCATION to a Steamworks SDK with redistributable_bin/osx/." >&2
    exit 1
fi

sed "s/__VERSION__/${VERSION}/g" packaging/Info.plist > "$APP/Contents/Info.plist"

ICONSET="AppIcon.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for spec in "16:icon_16x16.png" "32:icon_16x16@2x.png" \
            "32:icon_32x32.png" "64:icon_32x32@2x.png" \
            "128:icon_128x128.png" "256:icon_128x128@2x.png" \
            "256:icon_256x256.png" "512:icon_256x256@2x.png" \
            "512:icon_512x512.png" "1024:icon_512x512@2x.png"; do
    size="${spec%%:*}"
    name="${spec##*:}"
    sips -z "$size" "$size" icon.png --out "$ICONSET/$name" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

BAKE_OUT="$REPO_ROOT/target/mahjuro-bake-packs"
rm -rf "$BAKE_OUT"
python3 "$REPO_ROOT/tools/bake_assets/bake_assets.py" --out "$BAKE_OUT"
cp "$BAKE_OUT/pack_manifest.json" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-shared.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-gameplay.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-scene-main_menu.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-music.zip" "$APP/Contents/Resources/"

# ─────────────────────────── Sign (optional) ───────────────────────────
if [[ $SIGN -eq 1 ]]; then
    if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
        APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning \
            | grep "Developer ID Application" | head -1 \
            | sed 's/.*"\(.*\)".*/\1/')"
    fi
    if [[ -z "$APPLE_SIGNING_IDENTITY" ]]; then
        echo "error: no Developer ID Application identity found in keychain" >&2
        exit 1
    fi
    echo "Signing with: $APPLE_SIGNING_IDENTITY"
    codesign --sign "$APPLE_SIGNING_IDENTITY" \
        --entitlements Entitlements.plist \
        --options runtime \
        --force --deep --timestamp \
        "$APP"
    codesign --verify --strict --verbose=2 "$APP"
fi

# ─────────────────────────── Notarize (optional) ───────────────────────────
if [[ $NOTARIZE -eq 1 ]]; then
    : "${APPLE_API_KEY_PATH:?APPLE_API_KEY_PATH not set}"
    : "${APPLE_API_KEY:?APPLE_API_KEY not set}"
    : "${APPLE_API_ISSUER:?APPLE_API_ISSUER not set}"
    ditto -c -k --keepParent "$APP" Mahjuro-notarize.zip
    xcrun notarytool submit Mahjuro-notarize.zip \
        --key "$APPLE_API_KEY_PATH" \
        --key-id "$APPLE_API_KEY" \
        --issuer "$APPLE_API_ISSUER" \
        --wait
    rm Mahjuro-notarize.zip
    xcrun stapler staple "$APP"
fi

# ─────────────────────────── DMG ───────────────────────────
DMG="mahjuro-${TAG}-macos-${ARCH_LABEL}.dmg"
STAGING="dmg-staging"
rm -rf "$STAGING" "$DMG"
mkdir "$STAGING"
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
hdiutil create \
    -volname "Mahjuro" \
    -srcfolder "$STAGING" \
    -ov -format UDZO \
    "$DMG"
rm -rf "$STAGING"

if [[ $SIGN -eq 1 ]]; then
    codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
fi
if [[ $NOTARIZE -eq 1 ]]; then
    xcrun notarytool submit "$DMG" \
        --key "$APPLE_API_KEY_PATH" \
        --key-id "$APPLE_API_KEY" \
        --issuer "$APPLE_API_ISSUER" \
        --wait
    xcrun stapler staple "$DMG"
fi

echo
echo "Built: $DMG"
