#!/usr/bin/env python3
"""Extract the section for a given version from CHANGELOG.md.

Used by the GitHub Actions release workflow to populate release body text.

Usage:
    python3 scripts/extract_release_notes.py <version-or-tag>

Accepts either a bare version (`0.3.3`) or a tag (`v0.3.3`). Prints the
section body (everything under the matching `## <version> — ...` heading,
up to the next `## ` line) to stdout. Exits 1 if not found.
"""

from __future__ import annotations

import pathlib
import re
import sys

CHANGELOG_PATH = pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: extract_release_notes.py <version-or-tag>", file=sys.stderr)
        return 2

    version = sys.argv[1].lstrip("v").strip()
    if not CHANGELOG_PATH.exists():
        print(f"error: {CHANGELOG_PATH} does not exist", file=sys.stderr)
        return 1

    text = CHANGELOG_PATH.read_text(encoding="utf-8")

    # Match: "## <version>" followed by either end-of-line or " — ..." / " - ..." date suffix.
    pattern = re.compile(
        rf"^## {re.escape(version)}(?:\s+[\u2014-].*)?\s*$", re.MULTILINE
    )
    m = pattern.search(text)
    if not m:
        print(
            f"error: no section for version '{version}' in {CHANGELOG_PATH.name}",
            file=sys.stderr,
        )
        return 1

    start = m.end()
    next_section = re.compile(r"^## ", re.MULTILINE).search(text, pos=start)
    end = next_section.start() if next_section else len(text)
    body = text[start:end].strip("\n")
    print(body)
    return 0


if __name__ == "__main__":
    sys.exit(main())
