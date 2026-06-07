#!/usr/bin/env bash
# Install Apple WWDR intermediate certs so Distribution / Installer identities
# appear in `security find-identity`. Required after downloading .cer files from
# the developer portal when Keychain shows certs but codesign cannot build the chain.
#
# See: https://www.apple.com/certificateauthority/

set -euo pipefail

KEYCHAIN="${1:-$HOME/Library/Keychains/login.keychain-db}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

install_one() {
    local url="$1"
    local name="$2"
    local out="$TMP/${name}.cer"
    echo "Installing $name…"
    curl -fsSL -o "$out" "$url"
    security import "$out" -k "$KEYCHAIN" -T /usr/bin/codesign -T /usr/bin/security
}

# G3 — required for current Apple Distribution / Mac Developer certs (2026+).
install_one "https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer" "AppleWWDRCAG3"
# G2 — older certs; harmless to have both.
install_one "https://www.apple.com/certificateauthority/AppleWWDRCAG2.cer" "AppleWWDRCAG2" || true

echo
echo "Valid signing identities:"
security find-identity -v -p codesigning 2>/dev/null | rg -i "distribution|installer|developer" || true
