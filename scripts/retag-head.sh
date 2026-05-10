#!/usr/bin/env bash
#
# Move the release tag v<Cargo.toml package version> to the current HEAD and
# force-push only that tag to origin (same idea as deleting the tag, re-tagging
# HEAD, then pushing — without rewriting unrelated tags).
#
# Usage:
#   scripts/retag-head.sh
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/^version = "([^"]+)"/\1/')"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not read package version from Cargo.toml" >&2
    exit 1
fi
TAG="v${VERSION}"

if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    git tag -d "${TAG}"
else
    echo "note: no local tag ${TAG} (creating fresh at HEAD)"
fi

git tag "${TAG}"
echo "Tagged ${TAG} at $(git rev-parse --short HEAD)"

git push origin "${TAG}" --force
