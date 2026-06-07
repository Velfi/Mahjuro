#!/usr/bin/env bash
#
# Build a sandboxed Mahjuro.app for Mac App Store / TestFlight (dist-mas).
#
# Usage:
#   scripts/package-macos-store.sh [--universal] [--sign]
#
# Requires:
#   - cargo build with --no-default-features --features game,dist-mas
#   - For --sign: APPLE_MAS_SIGNING_IDENTITY (Mac App Distribution cert)
#
# Output:
#   Mahjuro-Store.app
#   mahjuro-store-v<version>-macos-<arch>.pkg   (when --sign and productbuild succeed)

set -euo pipefail

UNIVERSAL=0
SIGN=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        --sign)      SIGN=1 ;;
        -h|--help)
            sed -n '3,16p' "$0"
            exit 0
            ;;
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
FEATURES="game,dist-mas"
APP="Mahjuro-Store.app"

if [[ $UNIVERSAL -eq 1 ]]; then
    ARCH_LABEL="universal"
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --no-default-features --features "$FEATURES" --target aarch64-apple-darwin
    cargo build --release --no-default-features --features "$FEATURES" --target x86_64-apple-darwin
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
    cargo build --release --no-default-features --features "$FEATURES"
    BIN="target/release/mahjuro"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/mahjuro"
chmod +x "$APP/Contents/MacOS/mahjuro"

sed "s/__VERSION__/${VERSION}/g" packaging/Info.mas.plist > "$APP/Contents/Info.plist"

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
cp "$BAKE_OUT/mahjuro-pack-rooms.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-gameplay-bulk.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-music.zip" "$APP/Contents/Resources/"

if [[ $SIGN -eq 1 ]]; then
    : "${APPLE_MAS_SIGNING_IDENTITY:?APPLE_MAS_SIGNING_IDENTITY not set (Mac App Distribution)}"
    echo "Signing with: $APPLE_MAS_SIGNING_IDENTITY"
    codesign --sign "$APPLE_MAS_SIGNING_IDENTITY" \
        --entitlements Entitlements.mas.plist \
        --options runtime \
        --force --deep --timestamp \
        "$APP"
    codesign --verify --strict --verbose=2 "$APP"

    PKG="mahjuro-store-v${VERSION}-macos-${ARCH_LABEL}.pkg"
    productbuild \
        --component "$APP" /Applications \
        --sign "$APPLE_MAS_SIGNING_IDENTITY" \
        "$PKG"
    echo "Wrote $PKG (upload to App Store Connect / Transporter)"
fi

echo "Wrote $APP (features=$FEATURES, arch=$ARCH_LABEL)"
