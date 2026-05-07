#!/usr/bin/env bash
#
# Stage Mahjuro release artifacts and upload them to Steam via steamcmd.
#
# Usage:
#   scripts/steam-upload.sh [flags] <version>
#
# Flags:
#   --local        Stage from local builds (host platform only) instead of
#                  downloading the GitHub release. Useful for smoke testing.
#   --preview      Build the depots and validate, but do NOT upload to Steam.
#                  Always run this first when changing the VDFs.
#   --branch NAME  Set the build live on this beta branch after upload.
#                  Default: empty (build is uploaded but not promoted; use the
#                  Steamworks partner UI to promote).
#   --beta         Same as --branch, but the branch name defaults to "beta"
#                  (or STEAM_BETA_BRANCH if set). For pushing public betas.
#   --skip-login   Don't pass +login to steamcmd; assume an existing cached
#                  session. Useful when re-running after a successful login.
#
# Example:
#   STEAM_BUILD_USER=mahjuro_ci scripts/steam-upload.sh --preview 0.4.2
#   STEAM_BUILD_USER=mahjuro_ci scripts/steam-upload.sh --beta 0.4.2
#   STEAM_BUILD_USER=mahjuro_ci scripts/steam-upload.sh --branch internal 0.4.2
#
# Environment:
#   STEAM_SDK_ROOT       Path to the vendored Steamworks SDK.
#                        Default: ./steam_sdk
#   STEAM_BUILD_USER     Steam account with "Publish Builds" partner permission.
#                        Required (unless --skip-login).
#   STEAM_BUILD_PASSWORD If set, passed to steamcmd; otherwise interactive prompt.
#   STEAM_APP_ID         Override Mahjuro's AppID. Default: 4636490
#   STEAM_DEPOT_WINDOWS  Default: 4636491
#   STEAM_DEPOT_MACOS    Default: 4636492
#   STEAM_DEPOT_LINUX    Default: 4636493
#   STEAM_BETA_BRANCH    Used with --beta when you want a default other than
#                        the branch literally named "beta" (e.g. "publicbeta").

set -euo pipefail

# ─────────────────────────── Arg parsing ───────────────────────────
LOCAL=0
PREVIEW=0
SKIP_LOGIN=0
BETA=0
BRANCH=""
VERSION=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --local)       LOCAL=1; shift ;;
        --preview)     PREVIEW=1; shift ;;
        --skip-login)  SKIP_LOGIN=1; shift ;;
        --beta)        BETA=1; shift ;;
        --branch)      BRANCH="$2"; shift 2 ;;
        -h|--help)     sed -n '3,37p' "$0"; exit 0 ;;
        -*)            echo "unknown flag: $1" >&2; exit 1 ;;
        *)
            if [[ -n "$VERSION" ]]; then
                echo "error: version specified twice ('$VERSION' and '$1')" >&2
                exit 1
            fi
            VERSION="$1"; shift ;;
    esac
done

if [[ $BETA -eq 1 ]]; then
    if [[ -n "$BRANCH" ]]; then
        echo "error: use either --beta or --branch, not both" >&2
        exit 1
    fi
    BRANCH="${STEAM_BETA_BRANCH:-beta}"
fi

if [[ -z "$VERSION" ]]; then
    echo "error: version is required (e.g. 0.4.2)" >&2
    exit 1
fi
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "error: '$VERSION' is not a valid semver version" >&2
    exit 1
fi
TAG="v${VERSION}"

# ─────────────────────────── Resolve config ───────────────────────────
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STEAM_SDK_ROOT="${STEAM_SDK_ROOT:-${REPO_ROOT}/steam_sdk}"
STEAM_APP_ID="${STEAM_APP_ID:-4636490}"
STEAM_DEPOT_WINDOWS="${STEAM_DEPOT_WINDOWS:-4636491}"
STEAM_DEPOT_MACOS="${STEAM_DEPOT_MACOS:-4636492}"
STEAM_DEPOT_LINUX="${STEAM_DEPOT_LINUX:-4636493}"

if [[ ! -d "$STEAM_SDK_ROOT" ]]; then
    echo "error: STEAM_SDK_ROOT does not exist: $STEAM_SDK_ROOT" >&2
    echo "       Vendor the Steamworks SDK there, or set STEAM_SDK_ROOT." >&2
    exit 1
fi

case "$(uname)" in
    Darwin) STEAMCMD="$STEAM_SDK_ROOT/tools/ContentBuilder/builder_osx/steamcmd.sh" ;;
    Linux)  STEAMCMD="$STEAM_SDK_ROOT/tools/ContentBuilder/builder_linux/steamcmd.sh" ;;
    *) echo "error: unsupported host OS: $(uname)" >&2; exit 1 ;;
esac
if [[ ! -x "$STEAMCMD" ]]; then
    echo "error: steamcmd not found or not executable: $STEAMCMD" >&2
    exit 1
fi

# ─────────────────────────── Staging tree ───────────────────────────
STAGING="$REPO_ROOT/build-staging"
CONTENT="$STAGING/content"
OUTPUT="$STAGING/output"
SCRIPTS="$STAGING/scripts"
DOWNLOADS="$STAGING/dl"

rm -rf "$STAGING"
mkdir -p "$CONTENT/windows" "$CONTENT/macos" "$CONTENT/linux" \
         "$OUTPUT" "$SCRIPTS" "$DOWNLOADS"

# ─────────────────────────── Stage content ───────────────────────────
stage_local () {
    local host
    host="$(uname)"
    case "$host" in
        Darwin)
            local app="$REPO_ROOT/Mahjuro.app"
            if [[ ! -d "$app" ]]; then
                echo "error: --local on macOS expects Mahjuro.app at repo root." >&2
                echo "       Run scripts/package-macos.sh first." >&2
                exit 1
            fi
            cp -R "$app" "$CONTENT/macos/"
            echo "staged: macos/Mahjuro.app (from $app)"
            echo "warning: --local stages only the host platform; windows/ and linux/ are empty." >&2
            ;;
        Linux)
            local bin="$REPO_ROOT/target/release/mahjuro"
            if [[ ! -x "$bin" ]]; then
                echo "error: --local on Linux expects target/release/mahjuro." >&2
                echo "       Run 'cargo build --release' first." >&2
                exit 1
            fi
            cp "$bin" "$CONTENT/linux/mahjuro"
            chmod +x "$CONTENT/linux/mahjuro"
            local so="$REPO_ROOT/target/release/libsteam_api.so"
            if [[ ! -f "$so" ]]; then
                echo "error: $so not found." >&2
                echo "       Run a release build so build.rs copies the Steam redistributable next to the binary." >&2
                exit 1
            fi
            cp "$so" "$CONTENT/linux/libsteam_api.so"
            cp "$REPO_ROOT/packaging/steam_input/game_actions_4636490.vdf" \
                "$CONTENT/linux/game_actions_4636490.vdf"
            echo "staged: linux/mahjuro (from $bin)"
            echo "staged: linux/libsteam_api.so (from $so)"
            echo "staged: linux/game_actions_4636490.vdf"
            echo "warning: --local stages only the host platform; windows/ and macos/ are empty." >&2
            ;;
        *)
            echo "error: --local is only supported on macOS or Linux hosts." >&2
            exit 1
            ;;
    esac
}

stage_release () {
    if ! command -v gh >/dev/null 2>&1; then
        echo "error: gh CLI is required to download release artifacts." >&2
        exit 1
    fi

    echo "Downloading $TAG release artifacts..."
    gh release download "$TAG" \
        --pattern "mahjuro-${TAG}-windows-x86_64.zip" \
        --pattern "mahjuro-${TAG}-linux-x86_64.tar.gz" \
        --pattern "mahjuro-${TAG}-macos-universal.dmg" \
        --dir "$DOWNLOADS"

    # Windows: zip contains mahjuro.exe at the root.
    unzip -q "$DOWNLOADS/mahjuro-${TAG}-windows-x86_64.zip" -d "$CONTENT/windows/"
    echo "staged: windows/mahjuro.exe"

    # Linux: tar.gz contains mahjuro + libsteam_api.so (Steamworks redistributable).
    tar -xzf "$DOWNLOADS/mahjuro-${TAG}-linux-x86_64.tar.gz" -C "$CONTENT/linux/"
    chmod +x "$CONTENT/linux/mahjuro"
    echo "staged: linux/mahjuro"
    echo "staged: linux/libsteam_api.so (from release tarball)"

    # macOS: mount the DMG and copy the .app out of it (signed + notarized + stapled).
    if [[ "$(uname)" != "Darwin" ]]; then
        echo "error: macOS .app extraction requires running on a macOS host." >&2
        echo "       (DMG mount uses hdiutil.) Run this script on macOS, or" >&2
        echo "       extract Mahjuro.app manually into build-staging/content/macos/." >&2
        exit 1
    fi
    local mount
    mount="$(hdiutil attach -nobrowse -readonly \
        "$DOWNLOADS/mahjuro-${TAG}-macos-universal.dmg" \
        | awk '/\/Volumes\//{for(i=3;i<=NF;i++) printf "%s ", $i; print ""}' \
        | sed 's/ *$//')"
    if [[ -z "$mount" || ! -d "$mount/Mahjuro.app" ]]; then
        echo "error: failed to find Mahjuro.app inside the mounted DMG" >&2
        [[ -n "$mount" ]] && hdiutil detach "$mount" >/dev/null || true
        exit 1
    fi
    cp -R "$mount/Mahjuro.app" "$CONTENT/macos/"
    hdiutil detach "$mount" >/dev/null
    echo "staged: macos/Mahjuro.app"
}

if [[ $LOCAL -eq 1 ]]; then
    stage_local
else
    stage_release
fi

# ─────────────────────────── Render VDFs ───────────────────────────
render () {
    local src="$1"
    local dst="$2"
    sed \
        -e "s|__APPID__|${STEAM_APP_ID}|g" \
        -e "s|__DESC__|Mahjuro ${TAG}|g" \
        -e "s|__PREVIEW__|${PREVIEW}|g" \
        -e "s|__SETLIVE__|${BRANCH}|g" \
        -e "s|__CONTENT_ROOT__|${CONTENT}|g" \
        -e "s|__BUILD_OUTPUT__|${OUTPUT}|g" \
        -e "s|__DEPOT_WINDOWS__|${STEAM_DEPOT_WINDOWS}|g" \
        -e "s|__DEPOT_MACOS__|${STEAM_DEPOT_MACOS}|g" \
        -e "s|__DEPOT_LINUX__|${STEAM_DEPOT_LINUX}|g" \
        "$src" > "$dst"
}

render "packaging/steam/app_build.vdf.template"           "$SCRIPTS/app_build.vdf"
render "packaging/steam/depot_build_windows.vdf.template" "$SCRIPTS/depot_build_windows.vdf"
render "packaging/steam/depot_build_macos.vdf.template"   "$SCRIPTS/depot_build_macos.vdf"
render "packaging/steam/depot_build_linux.vdf.template"   "$SCRIPTS/depot_build_linux.vdf"

echo
echo "Rendered VDFs in $SCRIPTS:"
ls -1 "$SCRIPTS"

# ─────────────────────────── steamcmd ───────────────────────────
LOGIN_ARGS=()
PIPE_PASSWORD=0
if [[ $SKIP_LOGIN -eq 0 ]]; then
    : "${STEAM_BUILD_USER:?STEAM_BUILD_USER is required (or pass --skip-login)}"
    LOGIN_ARGS=(+login "$STEAM_BUILD_USER")
    # If a password is in the environment, feed it on stdin rather than as a
    # command-line arg — argv is visible to other processes via `ps` and gets
    # logged by some CI runners. steamcmd prompts on stdin when the password
    # is omitted from +login.
    if [[ -n "${STEAM_BUILD_PASSWORD:-}" ]]; then
        PIPE_PASSWORD=1
    fi
fi

echo
if [[ $PREVIEW -eq 1 ]]; then
    echo "── PREVIEW BUILD (no upload) ──"
else
    echo "── REAL BUILD (will upload to Steam AppID $STEAM_APP_ID) ──"
fi
echo "  staging:    $STAGING"
echo "  steamcmd:   $STEAMCMD"
echo "  setlive:    ${BRANCH:-<none — promote in partner UI>}"
echo

if [[ $PIPE_PASSWORD -eq 1 ]]; then
    # Pipe password on stdin so it never appears in argv (and thus not in `ps`,
    # set -x output, or CI logs that capture command lines).
    printf '%s\n' "$STEAM_BUILD_PASSWORD" | "$STEAMCMD" \
        "${LOGIN_ARGS[@]}" \
        +run_app_build "$SCRIPTS/app_build.vdf" \
        +quit
else
    "$STEAMCMD" \
        "${LOGIN_ARGS[@]}" \
        +run_app_build "$SCRIPTS/app_build.vdf" \
        +quit
fi

echo
echo "Done. Logs in: $OUTPUT"
if [[ $PREVIEW -eq 0 ]]; then
    echo "View the build: https://partner.steamgames.com/apps/builds/${STEAM_APP_ID}"
fi
