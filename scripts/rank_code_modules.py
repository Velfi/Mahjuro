#!/usr/bin/env python3
"""Rank Rust source modules by size.

Scans project code (``src/``, ``crates/``, ``build/`` by default) and prints
``.rs`` files sorted largest-first. Useful for spotting modules that exceed
the 700-line guideline in AGENTS.md.

Examples:
    python3 scripts/rank_code_modules.py
    python3 scripts/rank_code_modules.py --top 20
    python3 scripts/rank_code_modules.py --min-lines 500
    python3 scripts/rank_code_modules.py --sort sloc --csv
    python3 scripts/rank_code_modules.py --include-vendor
"""

from __future__ import annotations

import argparse
import csv
import sys
from dataclasses import dataclass
from pathlib import Path

DEFAULT_ROOTS = ("src", "crates", "build")
DEFAULT_LARGE_THRESHOLD = 700
SKIP_DIR_NAMES = frozenset(
    {
        ".git",
        ".github",
        "node_modules",
        "target",
    }
)


@dataclass(frozen=True)
class ModuleStat:
    path: Path
    rel_path: str
    lines: int
    sloc: int
    bytes: int


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def discover_roots(raw_roots: list[str], root: Path) -> list[Path]:
    if not raw_roots:
        return [root / name for name in DEFAULT_ROOTS if (root / name).is_dir()]

    resolved: list[Path] = []
    for raw in raw_roots:
        path = Path(raw)
        if not path.is_absolute():
            path = (root / path).resolve()
        if not path.exists():
            raise SystemExit(f"error: root does not exist: {path}")
        resolved.append(path)
    return resolved


def should_skip_dir(name: str, *, include_vendor: bool) -> bool:
    if name in SKIP_DIR_NAMES:
        return True
    if name == "vendor" and not include_vendor:
        return True
    return False


def count_module(path: Path, root: Path) -> ModuleStat:
    text = path.read_text(encoding="utf-8")
    lines = text.count("\n") + (0 if text.endswith("\n") or not text else 1)
    sloc = sum(1 for line in text.splitlines() if line.strip())
    rel_path = path.relative_to(root).as_posix()
    return ModuleStat(
        path=path,
        rel_path=rel_path,
        lines=lines,
        sloc=sloc,
        bytes=path.stat().st_size,
    )


def discover_modules(
    roots: list[Path],
    *,
    repo: Path,
    include_vendor: bool,
    extension: str,
) -> list[ModuleStat]:
    pattern = f"*{extension}"
    stats: list[ModuleStat] = []

    for root in roots:
        for path in sorted(root.rglob(pattern)):
            if not path.is_file():
                continue
            if any(should_skip_dir(part, include_vendor=include_vendor) for part in path.parts):
                continue
            stats.append(count_module(path, repo))

    return stats


def sort_key(stat: ModuleStat, metric: str) -> tuple[int, str]:
    if metric == "lines":
        value = stat.lines
    elif metric == "sloc":
        value = stat.sloc
    elif metric == "bytes":
        value = stat.bytes
    else:  # pragma: no cover - argparse choices prevent this
        raise ValueError(f"unknown metric: {metric}")
    return (-value, stat.rel_path)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "roots",
        nargs="*",
        help=f"Directories to scan (default: {', '.join(DEFAULT_ROOTS)}).",
    )
    parser.add_argument(
        "--sort",
        choices=("lines", "sloc", "bytes"),
        default="lines",
        help="Metric to rank by (default: lines).",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=0,
        help="Print only the top N modules (default: all).",
    )
    parser.add_argument(
        "--min-lines",
        type=int,
        default=0,
        help="Only include modules with at least this many lines.",
    )
    parser.add_argument(
        "--large-threshold",
        type=int,
        default=DEFAULT_LARGE_THRESHOLD,
        help=f"Mark modules at or above this line count (default: {DEFAULT_LARGE_THRESHOLD}).",
    )
    parser.add_argument(
        "--ext",
        default=".rs",
        help="File extension to treat as a module (default: .rs).",
    )
    parser.add_argument(
        "--include-vendor",
        action="store_true",
        help="Include files under vendor/.",
    )
    parser.add_argument(
        "--csv",
        action="store_true",
        help="Print CSV instead of a human-readable table.",
    )
    return parser


def print_table(
    stats: list[ModuleStat],
    *,
    sort_metric: str,
    large_threshold: int,
) -> None:
    if not stats:
        print("No modules found.")
        return

    path_width = max(len(stat.rel_path) for stat in stats)
    path_width = max(path_width, len("Path"))

    header = (
        f"{'Rank':>4}  {'Path':<{path_width}}  {'Lines':>6}  {'SLOC':>6}  "
        f"{'KiB':>7}  Flag"
    )
    print(header)
    print("-" * len(header))

    for rank, stat in enumerate(stats, start=1):
        flag = "LARGE" if stat.lines >= large_threshold else ""
        print(
            f"{rank:>4}  {stat.rel_path:<{path_width}}  {stat.lines:>6}  "
            f"{stat.sloc:>6}  {stat.bytes / 1024:>7.1f}  {flag}"
        )

    total_lines = sum(stat.lines for stat in stats)
    total_sloc = sum(stat.sloc for stat in stats)
    total_bytes = sum(stat.bytes for stat in stats)
    large_count = sum(1 for stat in stats if stat.lines >= large_threshold)

    print("-" * len(header))
    print(
        f"{'':>4}  {f'{len(stats)} module(s)':<{path_width}}  {total_lines:>6}  "
        f"{total_sloc:>6}  {total_bytes / 1024:>7.1f}"
    )
    print("")
    print(
        f"Sorted by {sort_metric}. "
        f"{large_count} module(s) >= {large_threshold} lines."
    )


def print_csv(stats: list[ModuleStat], *, large_threshold: int) -> None:
    writer = csv.writer(sys.stdout)
    writer.writerow(("rank", "path", "lines", "sloc", "bytes", "large"))
    for rank, stat in enumerate(stats, start=1):
        writer.writerow(
            (
                rank,
                stat.rel_path,
                stat.lines,
                stat.sloc,
                stat.bytes,
                stat.lines >= large_threshold,
            )
        )


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    root = repo_root()
    try:
        roots = discover_roots(args.roots, root)
    except SystemExit as exc:
        print(exc, file=sys.stderr)
        return 1

    extension = args.ext if args.ext.startswith(".") else f".{args.ext}"
    stats = discover_modules(
        roots,
        repo=root,
        include_vendor=args.include_vendor,
        extension=extension,
    )

    stats = [stat for stat in stats if stat.lines >= args.min_lines]
    stats.sort(key=lambda stat: sort_key(stat, args.sort))

    if args.top > 0:
        stats = stats[: args.top]

    if args.csv:
        print_csv(stats, large_threshold=args.large_threshold)
    else:
        print_table(stats, sort_metric=args.sort, large_threshold=args.large_threshold)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
