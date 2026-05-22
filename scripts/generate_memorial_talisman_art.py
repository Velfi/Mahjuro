#!/usr/bin/env python3
"""Backward-compatible wrapper — see `generate_talisman_art.py`."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

if __name__ == "__main__":
    if "--set" not in sys.argv:
        sys.argv[1:0] = ["--set", "memorial"]
    from generate_talisman_art import main

    main()
