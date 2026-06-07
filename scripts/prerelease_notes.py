#!/usr/bin/env python3
"""Build GitHub pre-release notes as a commit-range link.

Uses the tag reachable from the release commit's parent as the compare base
(same baseline `git describe` would pick when cutting the release).

Usage:
    python3 scripts/prerelease_notes.py <version-or-tag>

Prints markdown to stdout. Exits 1 if the tag is missing or not a pre-release.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys


def is_prerelease(version: str) -> bool:
    return "-" in version


def normalize_tag(tag_or_version: str) -> str:
    v = tag_or_version.strip()
    return v if v.startswith("v") else f"v{v}"


def normalize_version(tag_or_version: str) -> str:
    return normalize_tag(tag_or_version).lstrip("v")


def run_git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True).strip()


def github_repo_slug() -> str:
    env = os.environ.get("GITHUB_REPOSITORY")
    if env:
        return env
    remote = run_git("remote", "get-url", "origin")
    m = re.search(r"github\.com[:/]([^/]+)/([^/.]+)", remote)
    if not m:
        print("error: could not determine GitHub repo from origin remote", file=sys.stderr)
        sys.exit(1)
    return f"{m.group(1)}/{m.group(2).removesuffix('.git')}"


def previous_tag(current_tag: str) -> str | None:
    """Most recent tag on the parent of the release commit."""
    try:
        parent = run_git("rev-parse", f"{current_tag}^")
    except subprocess.CalledProcessError:
        return None
    try:
        return run_git("describe", "--tags", "--abbrev=0", parent)
    except subprocess.CalledProcessError:
        return None


def commit_count(base_tag: str, head_tag: str) -> int:
    out = run_git("rev-list", "--count", f"{base_tag}..{head_tag}")
    return int(out)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prerelease_notes.py <version-or-tag>", file=sys.stderr)
        return 2

    current_tag = normalize_tag(sys.argv[1])
    current = normalize_version(current_tag)

    if not is_prerelease(current):
        print(f"error: '{current}' is not a pre-release version", file=sys.stderr)
        return 1

    try:
        run_git("rev-parse", "--verify", f"refs/tags/{current_tag}")
    except subprocess.CalledProcessError:
        print(f"error: tag {current_tag} not found", file=sys.stderr)
        return 1

    prev_tag = previous_tag(current_tag)
    slug = github_repo_slug()
    lines = [f"Pre-release **{current_tag}**.", ""]

    if prev_tag:
        compare_path = f"{prev_tag}...{current_tag}"
        compare_url = f"https://github.com/{slug}/compare/{compare_path}"
        try:
            n = commit_count(prev_tag, current_tag)
            commit_word = "commit" if n == 1 else "commits"
            lines.append(
                f"**{n}** {commit_word} since [{prev_tag}]({compare_url})."
            )
        except subprocess.CalledProcessError:
            lines.append(f"Changes since [{prev_tag}]({compare_url}).")
        lines.append("")
        lines.append(f"[View full diff]({compare_url})")
    else:
        commits_url = f"https://github.com/{slug}/commits/{current_tag}"
        lines.append(f"[View commits at {current_tag}]({commits_url})")

    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
