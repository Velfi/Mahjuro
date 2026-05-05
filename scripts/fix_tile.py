#!/usr/bin/env python3
"""Spot-fix a single tile (or several) inside an existing atlas.

Workflow:
  1. Read `assets/sets/<name>/atlas.png` + `atlas.toml`.
  2. For each requested tile code, find its cell rect in the atlas, crop it,
     and write the crop to a temp PNG.
  3. Call the OpenAI image-edit API on that crop with a tile-specific content
     prompt (or a user-supplied `--prompt`). The crop is used as BOTH the
     style reference and the subject — the model corrects the composition
     while preserving the existing visual treatment.
  4. Paste the corrected tile back into the same cell of the atlas.
  5. Save the atlas in place; atlas.toml is left untouched.

Cost: one `images.edit` call per tile (~$0.01–0.03). Cheap enough to iterate.

Usage:
    pip install openai pillow
    export OPENAI_API_KEY="sk-..."

    # dry-run: show the prompt, touch nothing
    python3 scripts/fix_tile.py american_spring B5 --dry-run

    # use the built-in spec for this tile code
    python3 scripts/fix_tile.py american_spring B5

    # override with a custom instruction
    python3 scripts/fix_tile.py american_spring B5 \\
        --prompt "four green stalks in the corners, one red stalk centered"

    # fix several tiles in one invocation
    python3 scripts/fix_tile.py american_spring B5 B6 B7

    # keep a backup of the atlas before overwriting
    python3 scripts/fix_tile.py american_spring B5 --backup
"""

from __future__ import annotations

import argparse
import base64
import io
import os
import re
import shutil
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)

from tile_specs import tile_content_spec


REPO_ROOT = Path(__file__).resolve().parent.parent
SETS_DIR = REPO_ROOT / "assets" / "sets"


def parse_atlas_toml(src: str) -> tuple[int, int, int, list[str]]:
    """Pull tile_width/tile_height/columns/layout from the toml our packer
    emits. Ignores unknown fields; returns (tw, th, cols, layout)."""
    tw = int(re.search(r"tile_width\s*=\s*(\d+)", src).group(1))
    th = int(re.search(r"tile_height\s*=\s*(\d+)", src).group(1))
    cols = int(re.search(r"columns\s*=\s*(\d+)", src).group(1))
    layout_block = re.search(r"layout\s*=\s*\[(.*?)\]", src, re.S).group(1)
    # quoted strings, in declaration order; empties ("") included for padding
    codes = re.findall(r'"([^"]*)"', layout_block)
    return tw, th, cols, codes


def tile_rect(code: str, layout: list[str], cols: int,
              tw: int, th: int) -> tuple[int, int, int, int]:
    try:
        idx = layout.index(code)
    except ValueError as e:
        raise SystemExit(f"tile code '{code}' not found in atlas layout") from e
    col = idx % cols
    row = idx // cols
    return col * tw, row * th, col * tw + tw, row * th + th


FIX_PROMPT_TEMPLATE = """\
This is a single mahjong tile face from an existing styled tileset. Keep the \
visual style, material, palette, lighting, and border of the reference image \
EXACTLY as shown — do not restyle or recolor the overall look.

Correct only the COMPOSITION on the tile face so it depicts the following:

{spec}.

Output requirements:
- Same image dimensions as the input ({tw}x{th}, tall 2:3 portrait).
- Same background and tile-face material as the input.
- Same border / frame treatment.
- Orthographic front-on view, no perspective, no 3D thickness, no drop shadow \
outside the tile.
- No text, numbers, labels, or watermarks other than what the specification \
above calls for.
"""


def build_prompt(code: str, tw: int, th: int, user_prompt: str | None) -> str:
    spec = user_prompt if user_prompt else tile_content_spec(code)
    return FIX_PROMPT_TEMPLATE.format(spec=spec, tw=tw, th=th)


def call_edit(client: OpenAI, crop_path: Path, prompt: str,
              model: str, size: str) -> Image.Image:
    with crop_path.open("rb") as fh:
        resp = client.images.edit(
            model=model,
            image=fh,
            prompt=prompt,
            size=size,
            n=1,
        )
    b64 = resp.data[0].b64_json
    img = Image.open(io.BytesIO(base64.b64decode(b64)))
    return img.convert("RGBA")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("set_name",
                    help="Tileset directory name under assets/sets/ "
                         "(e.g. american_spring).")
    ap.add_argument("codes", nargs="+",
                    help="One or more tile codes to fix (e.g. B5 B6 B7).")
    ap.add_argument("--prompt", default=None,
                    help="Override the built-in content spec with a custom "
                         "instruction. Applied to every code in this call.")
    ap.add_argument("--model", default="gpt-image-2",
                    help="OpenAI image model (default: gpt-image-2).")
    ap.add_argument("--dry-run", action="store_true",
                    help="Print prompts, don't call the API or modify the atlas.")
    ap.add_argument("--backup", action="store_true",
                    help="Copy atlas.png to atlas.png.bak before overwriting.")
    ap.add_argument("--keep-crops", action="store_true",
                    help="Keep the temporary before/after crops in the set dir "
                         "for inspection.")
    ap.add_argument("--parallel", type=int, default=4,
                    help="Max concurrent API calls (default 4). Set to 1 to "
                         "run sequentially.")
    args = ap.parse_args()

    set_dir = SETS_DIR / args.set_name
    atlas_path = set_dir / "atlas.png"
    toml_path = set_dir / "atlas.toml"
    if not atlas_path.exists() or not toml_path.exists():
        sys.exit(f"missing atlas.png or atlas.toml in {set_dir}")

    tw, th, cols, layout = parse_atlas_toml(toml_path.read_text())

    # Determine API size: the API supports a few fixed sizes; the closest
    # 2:3-portrait option is 1024x1536. Post-resize back to (tw, th).
    api_size = "1024x1536"

    # Validate codes early so we don't call the API for the first N valid
    # codes before finding an invalid one.
    for code in args.codes:
        rect = tile_rect(code, layout, cols, tw, th)
        _ = rect

    if args.dry_run:
        for code in args.codes:
            print(f"=== {code} ===")
            print(build_prompt(code, tw, th, args.prompt))
            print()
        return

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        sys.exit("Error: OPENAI_API_KEY not set.")
    client = OpenAI(api_key=api_key)

    if args.backup:
        shutil.copy2(atlas_path, atlas_path.with_suffix(".png.bak"))
        print(f"backed up to {atlas_path.name}.bak")

    atlas = Image.open(atlas_path).convert("RGBA")
    crops_dir = set_dir / "_crops"
    if args.keep_crops:
        crops_dir.mkdir(exist_ok=True)

    # Stage 1: write input crops to disk (serial, fast — just Pillow I/O).
    jobs: list[tuple[str, tuple[int, int, int, int], Path]] = []
    for code in args.codes:
        rect = tile_rect(code, layout, cols, tw, th)
        x0, y0, x1, y1 = rect
        crop = atlas.crop((x0, y0, x1, y1))
        crop_path = (crops_dir if args.keep_crops
                     else set_dir) / f"_fix_{code}_before.png"
        crop.save(crop_path)
        jobs.append((code, rect, crop_path))

    # Stage 2: fire the API calls in parallel. Each worker returns the result
    # (or None on failure) without mutating the shared atlas — we paste in the
    # main thread once results come in.
    def run_one(job: tuple[str, tuple[int, int, int, int], Path]):
        code, rect, crop_path = job
        prompt = build_prompt(code, tw, th, args.prompt)
        print(f"[{code}] editing (cell {rect[0]},{rect[1]}→{rect[2]},{rect[3]})...")
        try:
            fixed = call_edit(client, crop_path, prompt, args.model, api_size)
        except Exception as e:
            print(f"  !! {code} failed: {e}", file=sys.stderr)
            return code, rect, crop_path, None
        if fixed.size != (tw, th):
            fixed = fixed.resize((tw, th), Image.LANCZOS)
        return code, rect, crop_path, fixed

    workers = max(1, min(args.parallel, len(jobs)))
    results: list[tuple[str, tuple[int, int, int, int], Path, Image.Image | None]]
    if workers == 1:
        results = [run_one(j) for j in jobs]
    else:
        results = []
        with ThreadPoolExecutor(max_workers=workers) as pool:
            futures = [pool.submit(run_one, j) for j in jobs]
            for fut in as_completed(futures):
                results.append(fut.result())

    # Stage 3: paste successful results into the atlas (main thread).
    for code, rect, crop_path, fixed in results:
        if fixed is None:
            if not args.keep_crops:
                crop_path.unlink(missing_ok=True)
            continue
        x0, y0, _, _ = rect
        if args.keep_crops:
            fixed.save(crops_dir / f"_fix_{code}_after.png")
        else:
            crop_path.unlink(missing_ok=True)
        atlas.paste(fixed, (x0, y0))
        print(f"  pasted fixed {code} into atlas")

    atlas.save(atlas_path)
    print(f"wrote {atlas_path}")


if __name__ == "__main__":
    main()
