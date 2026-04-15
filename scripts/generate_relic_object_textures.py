#!/usr/bin/env python3
"""
Generate only `source/<slug>_object.png` (RGBA color renders).

Thin wrapper around `scripts/generate_relic_art.py --artifact object`. Used as
albedo **fallback** when `relics/<slug>.png` is missing (`relic_pipeline.rs`).
Usually also run `derive_relic_runtime_textures.py` to produce the preferred
runtime albedo.

Usage:
    python scripts/generate_relic_object_textures.py --dry-run
    python scripts/generate_relic_object_textures.py --name kan_drum
    python scripts/generate_relic_object_textures.py --force
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate only relic object textures via the shared OpenAI relic generator."
    )
    parser.add_argument("--relic", type=int, default=None)
    parser.add_argument("--name", type=str, default=None)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--model", type=str, default=None)
    parser.add_argument("--size", type=str, default=None)
    parser.add_argument("--output-dir", type=str, default=None)
    parser.add_argument("--delay", type=float, default=None)
    args = parser.parse_args()

    script = Path(__file__).resolve().parent / "generate_relic_art.py"
    cmd = [sys.executable, str(script), "--artifact", "object"]

    if args.relic is not None:
        cmd.extend(["--relic", str(args.relic)])
    if args.name is not None:
        cmd.extend(["--name", args.name])
    if args.list:
        cmd.append("--list")
    if args.dry_run:
        cmd.append("--dry-run")
    if args.force:
        cmd.append("--force")
    if args.model is not None:
        cmd.extend(["--model", args.model])
    if args.size is not None:
        cmd.extend(["--size", args.size])
    if args.output_dir is not None:
        cmd.extend(["--output-dir", args.output_dir])
    if args.delay is not None:
        cmd.extend(["--delay", str(args.delay)])

    raise SystemExit(subprocess.call(cmd))


if __name__ == "__main__":
    main()
