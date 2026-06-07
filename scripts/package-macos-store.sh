#!/usr/bin/env bash
#
# Build a sandboxed Mahjuro.app for Mac App Store / TestFlight (dist-mas).
#
# Usage:
#   scripts/package-macos-store.sh [--universal] [--sign] [--validate]
#
# Environment (optional):
#   MAS_BUNDLE_ID                    default: com.zelda-built-this.Mahjuro.store
#   MAS_BUILD_NUMBER                 CFBundleVersion override (must increase per upload)
#   APPLE_MAS_APP_SIGNING_IDENTITY   "Apple Distribution: …" or "3rd Party Mac Developer Application: …"
#   APPLE_MAS_INSTALLER_SIGNING_IDENTITY  "3rd Party Mac Developer Installer: …" (for .pkg)
#   MAS_PROVISIONING_PROFILE         .provisionprofile to embed when signing
#
# Output:
#   Mahjuro-Store.app
#   mahjuro-store-v<version>-macos-<arch>.pkg   (with --sign)

set -euo pipefail

UNIVERSAL=0
SIGN=0
VALIDATE=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        --sign)      SIGN=1 ;;
        --validate)  VALIDATE=1 ;;
        -h|--help)
            sed -n '3,22p' "$0"
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
# shellcheck source=scripts/mas-version.sh
source "$REPO_ROOT/scripts/mas-version.sh"
mas_resolve_versions

BUNDLE_ID="${MAS_BUNDLE_ID:-com.zelda-built-this.Mahjuro.store}"
FEATURES="game,dist-mas"
APP="Mahjuro-Store.app"

echo "MAS versions: short=$MAS_SHORT_VERSION build=$MAS_BUILD_NUMBER bundle=$BUNDLE_ID"

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

sed -e "s/__SHORT_VERSION__/${MAS_SHORT_VERSION}/g" \
    -e "s/__BUILD_VERSION__/${MAS_BUILD_NUMBER}/g" \
    -e "s/com.zelda-built-this.Mahjuro.store/${BUNDLE_ID}/g" \
    packaging/Info.mas.plist > "$APP/Contents/Info.plist"

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
cp "$BAKE_OUT/mahjuro-pack-music.zip" "$APP/Contents/Resources/"
cp "$BAKE_OUT/mahjuro-pack-gameplay-bulk.zip" "$APP/Contents/Resources/"

mas_entitlements_plist() {
    local out="$1"
    cp Entitlements.mas.plist "$out"
    if [[ -z "${MAS_PROVISIONING_PROFILE:-}" || ! -f "$MAS_PROVISIONING_PROFILE" ]]; then
        return
    fi
    local profile_decoded app_id team_id
    profile_decoded="$(mktemp -t mahjuro-profile).plist"
    security cms -D -i "$MAS_PROVISIONING_PROFILE" > "$profile_decoded"
    app_id="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.application-identifier' "$profile_decoded" 2>/dev/null || true)"
    team_id="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.developer.team-identifier' "$profile_decoded" 2>/dev/null || true)"
    rm -f "$profile_decoded"
    if [[ -n "$app_id" ]]; then
        /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $app_id" "$out" 2>/dev/null \
            || /usr/libexec/PlistBuddy -c "Set :com.apple.application-identifier $app_id" "$out"
    fi
    if [[ -n "$team_id" ]]; then
        /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $team_id" "$out" 2>/dev/null \
            || /usr/libexec/PlistBuddy -c "Set :com.apple.developer.team-identifier $team_id" "$out"
    fi
}

mas_strip_xattrs() {
    # ITMS-91109: reject quarantine from Downloads copies. codesign also adds
    # com.apple.provenance; strip all xattrs after signing, before productbuild.
    if xattr -lr "$APP" 2>/dev/null | grep -q .; then
        echo "Clearing extended attributes under $APP"
        xattr -cr "$APP"
    fi
}

mas_sign_app() {
    local app_identity="${APPLE_MAS_APP_SIGNING_IDENTITY:-}"
    if [[ -z "$app_identity" ]]; then
        app_identity="$(security find-identity -v -p codesigning \
            | grep -E 'Apple Distribution|3rd Party Mac Developer Application' \
            | head -1 | sed 's/.*"\(.*\)".*/\1/')"
    fi
    if [[ -z "$app_identity" ]]; then
        echo "error: no Mac App Distribution identity found." >&2
        echo "       Set APPLE_MAS_APP_SIGNING_IDENTITY or install cert from App Store Connect." >&2
        exit 1
    fi
    echo "Signing app with: $app_identity"

    if [[ -n "${MAS_PROVISIONING_PROFILE:-}" && -f "$MAS_PROVISIONING_PROFILE" ]]; then
        cp "$MAS_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
        echo "Embedded provisioning profile: $MAS_PROVISIONING_PROFILE"
    else
        echo "warning: MAS_PROVISIONING_PROFILE not set — upload may fail (missing embedded.provisionprofile)" >&2
    fi
    mas_strip_xattrs

    local entitlements
    entitlements="$(mktemp -t mahjuro-mas-ents).plist"
    mas_entitlements_plist "$entitlements"

    local -a sign_args=(
        --sign "$app_identity"
        --entitlements "$entitlements"
        --options runtime
        --timestamp
        --force
        --identifier "$BUNDLE_ID"
    )

    # Sign the main binary first, then the bundle (avoid blind --deep on MAS).
    codesign "${sign_args[@]}" "$APP/Contents/MacOS/mahjuro"
    codesign "${sign_args[@]}" "$APP"
    rm -f "$entitlements"
    codesign --verify --strict --verbose=2 "$APP"
    mas_strip_xattrs
    codesign --verify --strict --verbose=2 "$APP"
}

if [[ $SIGN -eq 1 ]]; then
    mas_sign_app

    installer_identity="${APPLE_MAS_INSTALLER_SIGNING_IDENTITY:-}"
    if [[ -z "$installer_identity" ]]; then
        installer_identity="$(security find-identity -v \
            | grep '3rd Party Mac Developer Installer' \
            | head -1 | sed 's/.*"\(.*\)".*/\1/')"
    fi
    if [[ -z "$installer_identity" ]]; then
        echo "error: no Mac Installer Distribution identity found for .pkg." >&2
        echo "       Set APPLE_MAS_INSTALLER_SIGNING_IDENTITY." >&2
        exit 1
    fi
    echo "Signing pkg with: $installer_identity"

    PKG="mahjuro-store-v${MAS_SHORT_VERSION}-b${MAS_BUILD_NUMBER}-macos-${ARCH_LABEL}.pkg"
    productbuild \
        --component "$APP" /Applications \
        --sign "$installer_identity" \
        "$PKG"
    echo "Wrote $PKG — upload with Transporter or: xcrun altool --upload-app -f $PKG -t macos -u …"
fi

if [[ $VALIDATE -eq 1 || $SIGN -eq 1 ]]; then
    scripts/validate-macos-store.sh "$APP"
fi

echo "Wrote $APP (features=$FEATURES, arch=$ARCH_LABEL)"
