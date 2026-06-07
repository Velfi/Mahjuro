#!/usr/bin/env bash
# Verify Mac App Store signing identities before packaging/upload.

set -euo pipefail

missing=0

have_app="$(security find-identity -v -p codesigning 2>/dev/null \
    | grep -E 'Apple Distribution|3rd Party Mac Developer Application' \
    | head -1 || true)"
have_installer="$(security find-identity -v 2>/dev/null \
    | grep '3rd Party Mac Developer Installer' \
    | head -1 || true)"

# Cert visible in Keychain Access but not in find-identity → broken WWDR chain.
have_app_cert="$(security find-certificate -c 'Apple Distribution' 2>/dev/null | head -1 || true)"
have_installer_cert="$(security find-certificate -c '3rd Party Mac Developer Installer' 2>/dev/null | head -1 || true)"

echo "== Mac App Store preflight =="

if [[ -n "${APPLE_MAS_APP_SIGNING_IDENTITY:-}" ]]; then
    echo "  app identity (env): $APPLE_MAS_APP_SIGNING_IDENTITY"
elif [[ -n "$have_app" ]]; then
    echo "  app identity (keychain): $have_app"
else
    echo "  MISSING: valid Mac App Distribution signing identity" >&2
    if [[ -n "$have_app_cert" ]]; then
        echo "    Certificate is in Keychain but not trusted for codesigning." >&2
        echo "    Fix: scripts/install-apple-wwdr-intermediates.sh" >&2
        echo "    (missing Apple Worldwide Developer Relations G3 intermediate)" >&2
    else
        echo "    Create at https://developer.apple.com/account/resources/certificates/list" >&2
        echo "    → Apple Distribution (Mac App Store)" >&2
    fi
    missing=1
fi

if [[ -n "${APPLE_MAS_INSTALLER_SIGNING_IDENTITY:-}" ]]; then
    echo "  installer identity (env): $APPLE_MAS_INSTALLER_SIGNING_IDENTITY"
elif [[ -n "$have_installer" ]]; then
    echo "  installer identity (keychain): $have_installer"
else
    echo "  MISSING: valid Mac Installer Distribution signing identity" >&2
    if [[ -n "$have_installer_cert" ]]; then
        echo "    Certificate is in Keychain but not trusted for codesigning." >&2
        echo "    Fix: scripts/install-apple-wwdr-intermediates.sh" >&2
    else
        echo "    Create at https://developer.apple.com/account/resources/certificates/list" >&2
        echo "    → Mac Installer Distribution" >&2
    fi
    missing=1
fi

if [[ $missing -ne 0 ]]; then
    exit 1
fi

echo "  preflight OK"
