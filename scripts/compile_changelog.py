#!/usr/bin/env python3
"""Compile `.changes/*.md` fragments into a new CHANGELOG.md section.

Usage:
    python3 scripts/compile_changelog.py <version> [--date YYYY-MM-DD] [--keep-fragments]

On success:
  - Prepends a new `## <version> — <date>` section to CHANGELOG.md (creating
    the file if absent).
  - Deletes the fragment files (unless --keep-fragments is passed).
"""

from __future__ import annotations

import argparse
import datetime as _dt
import re
import sys

from _changelog import (
    CHANGELOG_PATH,
    load_fragments,
    render_section,
)

CHANGELOG_HEADER = """# Changelog

All notable changes to Mahjuro are listed here. Entries are grouped by
release; the most recent release is on top.

"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="Version being released, e.g. 0.3.3")
    parser.add_argument(
        "--date",
        default=_dt.date.today().isoformat(),
        help="Release date in YYYY-MM-DD (default: today, local)",
    )
    parser.add_argument(
        "--keep-fragments",
        action="store_true",
        help="Don't delete fragment files after compiling (useful for testing)",
    )
    args = parser.parse_args()

    fragments = load_fragments()
    if not fragments:
        print("error: no changelog fragments in .changes/", file=sys.stderr)
        print(
            "       add at least one `.changes/<slug>.md` before releasing",
            file=sys.stderr,
        )
        return 1

    heading = f"{args.version} — {args.date}"
    new_section = render_section(heading, fragments)

    if CHANGELOG_PATH.exists():
        existing = CHANGELOG_PATH.read_text(encoding="utf-8")
        # Insert the new section immediately above the first existing `## `
        # version heading, so any preamble (# Changelog, intro paragraph) stays
        # at the top.
        m = re.search(r"^## ", existing, re.MULTILINE)
        if m:
            preamble = existing[: m.start()].rstrip() + "\n\n"
            rest = existing[m.start() :]
            combined = f"{preamble}{new_section}\n{rest}"
        else:
            # No existing sections — treat whole file as preamble.
            preamble = existing.rstrip() + "\n\n" if existing.strip() else CHANGELOG_HEADER
            combined = f"{preamble}{new_section}"
    else:
        combined = f"{CHANGELOG_HEADER}{new_section}"

    CHANGELOG_PATH.write_text(combined, encoding="utf-8")

    if not args.keep_fragments:
        for fragment in fragments:
            fragment.path.unlink()

    print(
        f"compiled {len(fragments)} fragment(s) into {CHANGELOG_PATH.name} "
        f"under '{heading}'"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
