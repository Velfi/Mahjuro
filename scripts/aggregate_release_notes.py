#!/usr/bin/env python3
"""Build GitHub release notes from CHANGELOG.md for a given tag.

Collects every changelog section that is strictly after the most recent *stable*
( non-prerelease ) version older than the tag, and not newer than the tag's
version — in document order (newest first). This way a prerelease or final
0.5.0 publish includes notes for 0.5.0-0, 0.5.0-1, etc. since 0.4.5.

Usage:
    python3 scripts/aggregate_release_notes.py <version-or-tag>

Prints markdown to stdout. Exits 1 if CHANGELOG.md is missing or the tag's
section is not found.
"""

from __future__ import annotations

import pathlib
import re
import sys

CHANGELOG_PATH = pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"

# First line of each release block: version, optional em dash / ASCII hyphen + date/title.
SECTION_HEAD = re.compile(
    r"^## ([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)(?:\s+[\u2014-].*)?\s*$",
    re.MULTILINE,
)


def is_stable(version: str) -> bool:
    return "-" not in version


def _pre_tuple(pre: str) -> tuple:
    """Semver pre-release identifier sequence for comparison."""
    if not pre:
        return ()
    out: list = []
    for part in pre.split("."):
        if part.isdigit():
            out.append(int(part))
        else:
            out.append(part)
    return tuple(out)


def split_semver(v: str) -> tuple[tuple[int, int, int], tuple]:
    if "-" in v:
        core_s, pre_s = v.split("-", 1)
    else:
        core_s, pre_s = v, ""
    maj_s, min_s, pat_s = core_s.split(".")
    return (int(maj_s), int(min_s), int(pat_s)), _pre_tuple(pre_s)


def semver_cmp(a: str, b: str) -> int:
    """Return -1 if a<b, 0 if a==b, 1 if a>b (semver 2.0 ordering)."""
    c1, p1 = split_semver(a)
    c2, p2 = split_semver(b)
    if c1 != c2:
        return -1 if c1 < c2 else 1
    # Same MAJOR.MINOR.PATCH
    if not p1 and not p2:
        return 0
    if not p1 and p2:
        return 1
    if p1 and not p2:
        return -1
    for x, y in zip(p1, p2):
        if x == y:
            continue
        if isinstance(x, int) and isinstance(y, int):
            return -1 if x < y else 1
        if isinstance(x, int):
            return -1
        if isinstance(y, int):
            return 1
        return -1 if str(x) < str(y) else 1
    return -1 if len(p1) < len(p2) else (1 if len(p1) > len(p2) else 0)


def parse_sections(text: str) -> list[tuple[str, str]]:
    """Return [(version, full_block_including_heading_line), ...] newest-first."""
    matches = list(SECTION_HEAD.finditer(text))
    out: list[tuple[str, str]] = []
    for i, m in enumerate(matches):
        ver = m.group(1)
        start = m.start()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        block = text[start:end].rstrip()
        out.append((ver, block))
    return out


def aggregate_notes(sections: list[tuple[str, str]], current: str) -> list[str] | None:
    """Return list of markdown blocks to include, or None if current missing."""
    idx_current = next((i for i, (v, _) in enumerate(sections) if v == current), None)
    if idx_current is None:
        return None

    anchor_idx: int | None = None
    for i, (v, _) in enumerate(sections):
        if is_stable(v) and semver_cmp(v, current) < 0:
            anchor_idx = i
            break

    if anchor_idx is None:
        # No older stable in the log (e.g. first release) — only the current heading stack.
        blocks = [sections[j][1] for j in range(0, idx_current + 1)]
        return blocks

    v_anchor = sections[anchor_idx][0]
    blocks: list[str] = []
    for j in range(0, anchor_idx):
        vj = sections[j][0]
        if semver_cmp(vj, v_anchor) > 0 and semver_cmp(vj, current) <= 0:
            blocks.append(sections[j][1])
    return blocks


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: aggregate_release_notes.py <version-or-tag>", file=sys.stderr)
        return 2

    current = sys.argv[1].lstrip("v").strip()
    if not CHANGELOG_PATH.exists():
        print(f"error: {CHANGELOG_PATH} does not exist", file=sys.stderr)
        return 1

    text = CHANGELOG_PATH.read_text(encoding="utf-8")
    sections = parse_sections(text)
    if not sections:
        print(f"error: no version sections found in {CHANGELOG_PATH.name}", file=sys.stderr)
        return 1

    blocks = aggregate_notes(sections, current)
    if blocks is None:
        print(
            f"error: no section for version '{current}' in {CHANGELOG_PATH.name}",
            file=sys.stderr,
        )
        return 1

    print("\n\n---\n\n".join(blocks))
    return 0


if __name__ == "__main__":
    sys.exit(main())
