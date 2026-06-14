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
#   --beta         Promote the main AppID to the "beta" branch (or
#                  STEAM_BETA_BRANCH) and also upload + promote the Steam
#                  Playtest child app (default playtest branch: "default";
#                  override with STEAM_PLAYTEST_BRANCH).
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
#   packaging/steam/targets.env — default AppID / depot IDs (main + playtest).
#   STEAM_BETA_BRANCH    Used with --beta when you want a default other than
#                        the branch literally named "beta" (e.g. "publicbeta").
#   STEAM_PLAYTEST_BRANCH  Playtest branch when using --beta (default: default).

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
TARGETS_ENV="${REPO_ROOT}/packaging/steam/targets.env"
if [[ -f "$TARGETS_ENV" ]]; then
    # shellcheck source=/dev/null
    source "$TARGETS_ENV"
fi
STEAM_APP_ID="${STEAM_APP_ID:-4636490}"
STEAM_DEPOT_WINDOWS="${STEAM_DEPOT_WINDOWS:-4636491}"
STEAM_DEPOT_MACOS="${STEAM_DEPOT_MACOS:-4636492}"
if [[ -z "${STEAM_PLAYTEST_BRANCH+x}" ]]; then
    STEAM_PLAYTEST_BRANCH="default"
fi

validate_app_id () {
    local label="$1"
    local value="$2"
    if ! [[ "$value" =~ ^[0-9]+$ ]] || [[ "$value" -eq 0 ]]; then
        echo "error: invalid $label AppID: '$value'" >&2
        return 1
    fi
}

validate_depot_id () {
    local label="$1"
    local value="$2"
    if ! [[ "$value" =~ ^[0-9]+$ ]] || [[ "$value" -eq 0 ]]; then
        echo "error: invalid $label depot ID: '$value'" >&2
        return 1
    fi
}

if [[ $BETA -eq 1 ]]; then
    missing=()
    [[ -z "${STEAM_PLAYTEST_APP_ID:-}" ]] && missing+=(STEAM_PLAYTEST_APP_ID)
    [[ -z "${STEAM_PLAYTEST_DEPOT_WINDOWS:-}" ]] && missing+=(STEAM_PLAYTEST_DEPOT_WINDOWS)
    [[ -z "${STEAM_PLAYTEST_DEPOT_MACOS:-}" ]] && missing+=(STEAM_PLAYTEST_DEPOT_MACOS)
    if [[ ${#missing[@]} -gt 0 ]]; then
        echo "error: --beta uploads to Mahjuro + Steam Playtest; set in $TARGETS_ENV:" >&2
        for key in "${missing[@]}"; do
            echo "         $key" >&2
        done
        echo "       Playtest depots: partner site → Playtest app → SteamPipe → Depots." >&2
        exit 1
    fi
    validate_app_id "playtest" "$STEAM_PLAYTEST_APP_ID"
    validate_depot_id "playtest windows" "$STEAM_PLAYTEST_DEPOT_WINDOWS"
    validate_depot_id "playtest macos" "$STEAM_PLAYTEST_DEPOT_MACOS"
fi

if [[ ! -d "$STEAM_SDK_ROOT" ]]; then
    echo "error: STEAM_SDK_ROOT does not exist: $STEAM_SDK_ROOT" >&2
    echo "       Vendor the Steamworks SDK there, or set STEAM_SDK_ROOT." >&2
    exit 1
fi

case "$(uname)" in
    Darwin)
        _osx_cb="$STEAM_SDK_ROOT/tools/ContentBuilder/builder_osx"
        # Newer Content Builder ships steamcmd beside steamcmd.sh; the .sh wrapper
        # may still expect Steam.AppBundle (older layout). Prefer the direct binary.
        # Valve sometimes unpacks steamcmd without the execute bit — fix that once.
        if [[ -f "$_osx_cb/steamcmd" ]]; then
            [[ -x "$_osx_cb/steamcmd" ]] || chmod u+x "$_osx_cb/steamcmd"
            STEAMCMD="$_osx_cb/steamcmd"
        else
            STEAMCMD="$_osx_cb/steamcmd.sh"
        fi
        unset _osx_cb
        ;;
    Linux)  STEAMCMD="$STEAM_SDK_ROOT/tools/ContentBuilder/builder_linux/steamcmd.sh" ;;
    *) echo "error: unsupported host OS: $(uname)" >&2; exit 1 ;;
esac
if [[ ! -x "$STEAMCMD" ]]; then
    echo "error: steamcmd not found or not executable: $STEAMCMD" >&2
    exit 1
fi

# Valve's builder_osx/steamcmd.sh execs Steam.AppBundle/.../steamcmd. A partial
# SDK copy fails at runtime with "No such file or directory" on line 37 of the
# wrapper — catch that here when we're not using the standalone steamcmd binary.
if [[ "$(uname)" == "Darwin" && "$STEAMCMD" == *.sh ]]; then
    _steamcmd_embedded="$(dirname "$STEAMCMD")/Steam.AppBundle/Steam/Contents/MacOS/steamcmd"
    if [[ ! -x "$_steamcmd_embedded" ]]; then
        echo "error: Content Builder steamcmd binary missing: $_steamcmd_embedded" >&2
        echo "       Re-download the Steamworks SDK from the partner site and ensure" >&2
        echo "       tools/ContentBuilder/builder_osx/Steam.AppBundle is fully present," >&2
        echo "       or use a layout that includes builder_osx/steamcmd (executable)." >&2
        exit 1
    fi
    unset _steamcmd_embedded
fi

# ─────────────────────────── Staging tree ───────────────────────────
STAGING="$REPO_ROOT/build-staging"
CONTENT="$STAGING/content"
OUTPUT="$STAGING/output"
SCRIPTS="$STAGING/scripts"
DOWNLOADS="$STAGING/dl"

rm -rf "$STAGING"
mkdir -p "$CONTENT/windows" "$CONTENT/macos" \
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
            echo "warning: --local stages only the host platform; windows/ is empty." >&2
            ;;
        *)
            echo "error: --local is only supported on macOS hosts." >&2
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
        --pattern "mahjuro-${TAG}-macos-universal.dmg" \
        --dir "$DOWNLOADS"

    # Windows: zip contains mahjuro.exe, pack_manifest.json + mahjuro-pack-*.zip beside the exe.
    unzip -q "$DOWNLOADS/mahjuro-${TAG}-windows-x86_64.zip" -d "$CONTENT/windows/"
    echo "staged: windows/mahjuro.exe"

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
render_target_vdfs () {
    local target_label="$1"
    local app_id="$2"
    local depot_windows="$3"
    local depot_macos="$4"
    local setlive="$5"
    local desc="$6"
    local target_scripts="$SCRIPTS/$target_label"
    local target_output="$OUTPUT/$target_label"

    mkdir -p "$target_scripts" "$target_output"

    render_one () {
        local src="$1"
        local dst="$2"
        sed \
            -e "s|__APPID__|${app_id}|g" \
            -e "s|__DESC__|${desc}|g" \
            -e "s|__PREVIEW__|${PREVIEW}|g" \
            -e "s|__SETLIVE__|${setlive}|g" \
            -e "s|__CONTENT_ROOT__|${CONTENT}|g" \
            -e "s|__BUILD_OUTPUT__|${target_output}|g" \
            -e "s|__DEPOT_WINDOWS__|${depot_windows}|g" \
            -e "s|__DEPOT_MACOS__|${depot_macos}|g" \
            "$src" > "$dst"
    }

    render_one "packaging/steam/app_build.vdf.template"           "$target_scripts/app_build.vdf"
    render_one "packaging/steam/depot_build_windows.vdf.template" "$target_scripts/depot_build_windows.vdf"
    render_one "packaging/steam/depot_build_macos.vdf.template"   "$target_scripts/depot_build_macos.vdf"

    echo "Rendered $target_label VDFs (AppID $app_id, setlive=${setlive:-<none>}) in $target_scripts"
}

BUILD_TARGETS=()
register_build_target () {
    BUILD_TARGETS+=("$1")
}

render_target_vdfs "main" \
    "$STEAM_APP_ID" \
    "$STEAM_DEPOT_WINDOWS" \
    "$STEAM_DEPOT_MACOS" \
    "$BRANCH" \
    "Mahjuro ${TAG}"
register_build_target "main"

if [[ $BETA -eq 1 ]]; then
    render_target_vdfs "playtest" \
        "$STEAM_PLAYTEST_APP_ID" \
        "$STEAM_PLAYTEST_DEPOT_WINDOWS" \
        "$STEAM_PLAYTEST_DEPOT_MACOS" \
        "$STEAM_PLAYTEST_BRANCH" \
        "Mahjuro ${TAG} (playtest)"
    register_build_target "playtest"
fi

echo

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

STEAMCMD_ARGS=()
for target in "${BUILD_TARGETS[@]}"; do
    STEAMCMD_ARGS+=(+run_app_build "$SCRIPTS/$target/app_build.vdf")
done
STEAMCMD_ARGS+=(+quit)

echo
if [[ $PREVIEW -eq 1 ]]; then
    echo "── PREVIEW BUILD (no upload) ──"
else
    echo "── REAL BUILD ──"
    for target in "${BUILD_TARGETS[@]}"; do
        case "$target" in
            main)
                echo "  main:     AppID $STEAM_APP_ID → branch ${BRANCH:-<none — promote in partner UI>}"
                ;;
            playtest)
                echo "  playtest: AppID $STEAM_PLAYTEST_APP_ID → branch ${STEAM_PLAYTEST_BRANCH:-<none — promote in partner UI>}"
                ;;
        esac
    done
fi
echo "  staging:  $STAGING"
echo "  steamcmd: $STEAMCMD"
echo

if [[ $PIPE_PASSWORD -eq 1 ]]; then
    # Pipe password on stdin so it never appears in argv (and thus not in `ps`,
    # set -x output, or CI logs that capture command lines).
    printf '%s\n' "$STEAM_BUILD_PASSWORD" | "$STEAMCMD" \
        "${LOGIN_ARGS[@]}" \
        "${STEAMCMD_ARGS[@]}"
else
    "$STEAMCMD" \
        "${LOGIN_ARGS[@]}" \
        "${STEAMCMD_ARGS[@]}"
fi

echo
echo "Done. Logs in: $OUTPUT"
if [[ $PREVIEW -eq 0 ]]; then
    echo "View builds:"
    echo "  main:     https://partner.steamgames.com/apps/builds/${STEAM_APP_ID}"
    if [[ $BETA -eq 1 ]]; then
        echo "  playtest: https://partner.steamgames.com/apps/builds/${STEAM_PLAYTEST_APP_ID}"
    fi
fi
