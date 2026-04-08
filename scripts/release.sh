#!/usr/bin/env bash
#
# Tag and start a Mahjuro release.
#
# Usage:
#   scripts/release.sh <version>
#
# Example:
#   scripts/release.sh 0.2.0
#
# This will:
#   1. Verify the working tree is clean and on `main`
#   2. Update the version in Cargo.toml (and refresh Cargo.lock)
#   3. Commit the version bump
#   4. Create an annotated `v<version>` tag
#   5. Push the commit and tag to `origin`, which triggers .github/workflows/release.yml

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <version>   (e.g. $0 0.2.0)" >&2
    exit 1
fi

VERSION="$1"
TAG="v${VERSION}"

# Validate semver-ish: MAJOR.MINOR.PATCH with optional -prerelease
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "error: '$VERSION' is not a valid semver version (expected MAJOR.MINOR.PATCH[-pre])" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Must be on main
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]]; then
    echo "error: must be on 'main' branch (currently on '$BRANCH')" >&2
    exit 1
fi

# Working tree must be clean
if ! git diff-index --quiet HEAD --; then
    echo "error: working tree has uncommitted changes" >&2
    git status --short >&2
    exit 1
fi

# Tag must not already exist (locally or on the remote)
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    echo "error: tag ${TAG} already exists locally" >&2
    exit 1
fi
git fetch --tags origin >/dev/null
if git ls-remote --exit-code --tags origin "refs/tags/${TAG}" >/dev/null 2>&1; then
    echo "error: tag ${TAG} already exists on origin" >&2
    exit 1
fi

# Make sure local main is up to date with origin/main
git fetch origin main >/dev/null
LOCAL="$(git rev-parse @)"
REMOTE="$(git rev-parse @{u})"
if [[ "$LOCAL" != "$REMOTE" ]]; then
    echo "error: local main is not in sync with origin/main" >&2
    echo "  local:  $LOCAL" >&2
    echo "  remote: $REMOTE" >&2
    exit 1
fi

# Bump version in the [package] section of Cargo.toml using a Python helper
# (avoids touching dependency version strings).
python3 - "$VERSION" <<'PY'
import re, sys, pathlib
new_version = sys.argv[1]
path = pathlib.Path("Cargo.toml")
text = path.read_text()
pattern = re.compile(r'(\[package\][^\[]*?\nversion\s*=\s*")([^"]+)(")', re.DOTALL)
new_text, n = pattern.subn(rf'\g<1>{new_version}\g<3>', text, count=1)
if n != 1:
    sys.exit("error: could not find [package] version in Cargo.toml")
path.write_text(new_text)
PY

# Refresh Cargo.lock so the version bump is recorded
cargo update --workspace --quiet

git add Cargo.toml Cargo.lock
git commit -m "Release ${TAG}"
git tag -a "${TAG}" -m "Release ${TAG}"

echo
echo "About to push the following to origin:"
echo "  - commit: $(git rev-parse --short HEAD)  Release ${TAG}"
echo "  - tag:    ${TAG}"
echo
read -r -p "Push now? [y/N] " reply
case "$reply" in
    [yY]|[yY][eE][sS])
        git push origin main
        git push origin "${TAG}"
        echo
        echo "Pushed. The release workflow should now be running:"
        echo "  https://github.com/Velfi/Mahjuro/actions/workflows/release.yml"
        ;;
    *)
        echo "Skipped push. To finish the release later, run:"
        echo "  git push origin main && git push origin ${TAG}"
        echo "To undo locally:"
        echo "  git tag -d ${TAG} && git reset --hard HEAD~1"
        ;;
esac
