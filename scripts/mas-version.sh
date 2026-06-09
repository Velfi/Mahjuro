#!/usr/bin/env bash
# Resolve marketing + build versions for Mac App Store Info.plist.
# Sourced by package-macos-store.sh — do not execute directly.
#
# Inputs:
#   MAS_BUILD_NUMBER  optional override for CFBundleVersion (must increase per upload)
#
# Outputs (exported):
#   MAS_SHORT_VERSION  CFBundleShortVersionString (e.g. 0.6.0)
#   MAS_BUILD_NUMBER   CFBundleVersion (e.g. 42)

# Echo the raw crate version (e.g. "0.6.1-7"). Reads the workspace-level
# `version = "..."` line (this repo uses `version.workspace = true` in the
# `[package]` section, so the literal lives under `[workspace.package]`), then
# falls back to `cargo pkgid`. Exits non-zero if it cannot be determined.
resolve_crate_version() {
    local raw
    raw="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
    if [[ -z "$raw" ]]; then
        raw="$(cargo pkgid -p mahjuro 2>/dev/null | sed -E 's/.*#([^#]+)$/\1/' | sed 's/^mahjuro@//')"
    fi
    if [[ -z "$raw" ]]; then
        echo "error: could not determine crate version from Cargo.toml" >&2
        return 1
    fi
    printf '%s\n' "$raw"
}

mas_resolve_versions() {
    local raw
    raw="$(resolve_crate_version)" || return 1

    if [[ "$raw" == *-* ]]; then
        MAS_SHORT_VERSION="${raw%%-*}"
        if [[ -z "${MAS_BUILD_NUMBER:-}" ]]; then
            MAS_BUILD_NUMBER="${raw#*-}"
        fi
    else
        MAS_SHORT_VERSION="$raw"
        MAS_BUILD_NUMBER="${MAS_BUILD_NUMBER:-1}"
    fi

    # App Store Connect rejects non-numeric build strings in practice.
    if [[ ! "$MAS_BUILD_NUMBER" =~ ^[0-9]+$ ]]; then
        echo "warning: MAS_BUILD_NUMBER '$MAS_BUILD_NUMBER' is not numeric; using 1" >&2
        MAS_BUILD_NUMBER="1"
    fi

    export MAS_SHORT_VERSION MAS_BUILD_NUMBER
}
