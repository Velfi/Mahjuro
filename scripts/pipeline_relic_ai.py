#!/usr/bin/env python3
"""
Run the full Gemini → derived runtime textures pipeline for one or more relics.

Steps:

  1. `generate_relic_art.py` — writes `assets/textures/relics/<slug>_object.png`
     and `<slug>_height.png` (defaults: height from text, object from height via edit).

  2. `derive_relic_runtime_textures.py` — writes `assets/textures/relics/<slug>.png`
     (preferred albedo) and, with `--emit-masks`, `<slug>_mask.png` for extrusion.

  3. Rebuild or re-bake packs — see `tools/bake_assets/README.md`.

Runtime loading is implemented in `src/render/relic_pipeline.rs`: albedo
(`relics/<slug>.png` then `<slug>_object.png`), optional masks, linear
`<slug>_height.png` for 3D relief.

Usage:
    export GEMINI_API_KEY=...
    pip install google-genai pillow
    python scripts/pipeline_relic_ai.py --list
    python scripts/pipeline_relic_ai.py                  # all relics (long; many API calls)
    python scripts/pipeline_relic_ai.py star_tile
    python scripts/pipeline_relic_ai.py pair_power kan_drum --force
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    gen = root / "scripts" / "generate_relic_art.py"
    derive = root / "scripts" / "derive_relic_runtime_textures.py"

    parser = argparse.ArgumentParser(
        description="Chain Gemini relic generation + derive runtime textures."
    )
    parser.add_argument(
        "names",
        nargs="*",
        metavar="SLUG",
        help="Relic slug(s) from generate_relic_art.py RELICS. Omit to run the full list.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all relics (same as generate_relic_art.py --list); no API calls.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Pass --force to both generate and derive.",
    )
    parser.add_argument(
        "--skip-generate",
        action="store_true",
        help="Only run derive (use when source PNGs are already present).",
    )
    parser.add_argument(
        "--skip-derive",
        action="store_true",
        help="Only run Gemini generation.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default="gemini-3.1-flash-image-preview",
        help=(
            "Gemini image model for generate_relic_art.py "
            "(default: gemini-3.1-flash-image-preview)."
        ),
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds between relics when generating multiple (default: 2).",
    )
    args = parser.parse_args()

    if args.list:
        r = subprocess.run([sys.executable, str(gen), "--list"], cwd=root)
        sys.exit(r.returncode)

    def generate_cmd_base() -> list[str]:
        return [
            sys.executable,
            str(gen),
            "--artifact",
            "both",
            "--height-mode",
            "generate",
            "--object-mode",
            "reference",
            "--model",
            args.model,
            "--delay",
            str(args.delay),
        ]

    def derive_cmd_base() -> list[str]:
        return [sys.executable, str(derive), "--emit-masks"]

    all_relics = not args.names

    if not args.skip_generate:
        if all_relics:
            cmd = generate_cmd_base()
            if args.force:
                cmd.append("--force")
            print("Running (all relics):", " ".join(cmd))
            r = subprocess.run(cmd, cwd=root)
            if r.returncode != 0:
                sys.exit(r.returncode)
        else:
            for name in args.names:
                cmd = generate_cmd_base()
                cmd.extend(["--name", name])
                if args.force:
                    cmd.append("--force")
                print("Running:", " ".join(cmd))
                r = subprocess.run(cmd, cwd=root)
                if r.returncode != 0:
                    sys.exit(r.returncode)

    if not args.skip_derive:
        if all_relics:
            cmd = derive_cmd_base()
            if args.force:
                cmd.append("--force")
            print("Running (all object sources):", " ".join(cmd))
            r = subprocess.run(cmd, cwd=root)
            if r.returncode != 0:
                sys.exit(r.returncode)
        else:
            for name in args.names:
                cmd = derive_cmd_base()
                cmd.extend(["--name", name])
                if args.force:
                    cmd.append("--force")
                print("Running:", " ".join(cmd))
                r = subprocess.run(cmd, cwd=root)
                if r.returncode != 0:
                    sys.exit(r.returncode)

    print(
        "\nNext: rebuild the game so embedded assets update, e.g.\n"
        "  cargo build\n"
        "(Re-bake asset packs or use loose assets/ in dev — tools/bake_assets/README.md.)"
    )


if __name__ == "__main__":
    main()
