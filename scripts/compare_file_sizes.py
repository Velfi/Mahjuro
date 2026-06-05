#!/usr/bin/env python3
"""Compare current file sizes against a git revision, with Git LFS support.

By default, compares changed ``*.blend`` files in the worktree against ``HEAD``.

Examples:
    python3 scripts/compare_file_sizes.py
    python3 scripts/compare_file_sizes.py --revision HEAD~1
    python3 scripts/compare_file_sizes.py --all --pathspec "*.glb"
    python3 scripts/compare_file_sizes.py assets/3d/source/Shop.blend
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

LFS_SIZE_RE = re.compile(rb"^size\s+(\d+)\s*$", re.MULTILINE)
DEFAULT_PATHSPEC = "*.blend"


def git(
    repo_root: Path,
    args: list[str],
    *,
    check: bool = True,
    text: bool = True,
) -> subprocess.CompletedProcess[str] | subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *args],
        cwd=repo_root,
        check=check,
        capture_output=True,
        text=text,
    )


def detect_repo_root() -> Path:
    script_repo = Path(__file__).resolve().parent.parent
    result = git(script_repo, ["rev-parse", "--show-toplevel"])
    return Path(result.stdout.strip())


def get_changed_paths(repo_root: Path, pathspecs: list[str]) -> list[str]:
    tracked = git(repo_root, ["diff", "--name-only", "--", *pathspecs]).stdout.splitlines()
    untracked = git(
        repo_root,
        ["ls-files", "--others", "--exclude-standard", "--", *pathspecs],
    ).stdout.splitlines()
    return sorted(set(tracked + untracked))


def get_tracked_paths(repo_root: Path, pathspecs: list[str]) -> list[str]:
    return git(repo_root, ["ls-files", "--", *pathspecs]).stdout.splitlines()


def lfs_pointer_size(blob: bytes) -> int | None:
    # Canonical LFS pointers include a version line and a size line.
    if b"git-lfs.github.com/spec/v1" not in blob:
        return None
    match = LFS_SIZE_RE.search(blob)
    if not match:
        return None
    return int(match.group(1))


def get_size_at_revision(repo_root: Path, revision: str, path: str) -> int | None:
    oid_result = git(
        repo_root,
        ["rev-parse", "--verify", f"{revision}:{path}"],
        check=False,
    )
    if oid_result.returncode != 0:
        return None

    oid = oid_result.stdout.strip()
    object_size = int(git(repo_root, ["cat-file", "-s", oid]).stdout.strip())

    # If the blob is tiny, it may be an LFS pointer. Parse and lift to true content size.
    if object_size <= 2048:
        blob = git(repo_root, ["cat-file", "-p", oid], text=False).stdout
        pointer_size = lfs_pointer_size(blob)
        if pointer_size is not None:
            return pointer_size
    return object_size


def format_mib(size: int | None) -> str:
    if size is None:
        return "-"
    return f"{size / (1024 * 1024):.2f}"


def format_delta_mib(delta: int | None) -> str:
    if delta is None:
        return "-"
    return f"{delta / (1024 * 1024):+.2f}"


def format_pct(old: int | None, delta: int | None) -> str:
    if old is None or delta is None:
        return "-"
    if old == 0:
        return "+inf%" if delta > 0 else "0.00%"
    return f"{(delta / old) * 100:+.2f}%"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="*",
        help="Specific files to compare (overrides --all/changed lookup).",
    )
    parser.add_argument(
        "-r",
        "--revision",
        default="HEAD",
        help="Git revision to compare against (default: HEAD).",
    )
    parser.add_argument(
        "--pathspec",
        action="append",
        default=[],
        help=f"Git pathspec to include (default: {DEFAULT_PATHSPEC!r}). Repeatable.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Compare all tracked files matching pathspec(s), not just changed files.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    repo_root = detect_repo_root()
    pathspecs = args.pathspec or [DEFAULT_PATHSPEC]

    if args.paths:
        paths = sorted(set(args.paths))
    elif args.all:
        paths = get_tracked_paths(repo_root, pathspecs)
    else:
        paths = get_changed_paths(repo_root, pathspecs)

    if not paths:
        print("No matching files found.")
        return 0

    rows: list[tuple[str, int | None, int | None, int | None, str]] = []
    for rel_path in paths:
        current_path = repo_root / rel_path
        current_size = current_path.stat().st_size if current_path.exists() else None
        old_size = get_size_at_revision(repo_root, args.revision, rel_path)
        delta = None if (old_size is None or current_size is None) else current_size - old_size
        pct = format_pct(old_size, delta)
        rows.append((rel_path, old_size, current_size, delta, pct))

    old_total = sum(old for _, old, _, _, _ in rows if old is not None)
    current_total = sum(cur for _, _, cur, _, _ in rows if cur is not None)
    total_delta = current_total - old_total

    path_width = max(4, max(len(path) for path, *_ in rows))
    header = (
        f"{'Path':<{path_width}}  {'Old MiB':>8}  {'New MiB':>8}  "
        f"{'Delta MiB':>9}  {'Delta %':>8}"
    )
    print(header)
    print("-" * len(header))
    for path, old, current, delta, pct in rows:
        print(
            f"{path:<{path_width}}  {format_mib(old):>8}  {format_mib(current):>8}  "
            f"{format_delta_mib(delta):>9}  {pct:>8}"
        )

    print("-" * len(header))
    print(
        f"{'TOTAL':<{path_width}}  {old_total / (1024 * 1024):>8.2f}  "
        f"{current_total / (1024 * 1024):>8.2f}  {total_delta / (1024 * 1024):>+9.2f}  "
        f"{format_pct(old_total, total_delta):>8}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
