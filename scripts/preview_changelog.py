#!/usr/bin/env python3
"""Print what the next CHANGELOG section will look like, without changing any files."""

from __future__ import annotations

import sys

from _changelog import load_fragments, render_section


def main() -> int:
    fragments = load_fragments()
    if not fragments:
        print("(no fragments in .changes/ — nothing to preview)")
        return 0
    print(render_section("UNRELEASED", fragments), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
