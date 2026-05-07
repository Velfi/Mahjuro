#!/usr/bin/env python3
"""Draw small Arabic indices (1…N) on each non-empty tile in an atlas.png.

Reads ``atlas.toml`` beside the PNG for grid geometry. Numbers are placed at
the bottom-right of each cell with a light rounded backing for legibility.

Usage (repo root):

    python3 scripts/annotate_atlas_indices.py assets/sets/classic
    python3 scripts/annotate_atlas_indices.py assets/sets/painted_from_scratch/atlas.png
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def parse_atlas_toml(src: str) -> tuple[int, int, int, list[str]]:
    tw = int(re.search(r"tile_width\s*=\s*(\d+)", src).group(1))
    th = int(re.search(r"tile_height\s*=\s*(\d+)", src).group(1))
    cols = int(re.search(r"columns\s*=\s*(\d+)", src).group(1))
    layout_block = re.search(r"layout\s*=\s*\[(.*?)\]", src, re.S).group(1)
    codes = re.findall(r'"([^"]*)"', layout_block)
    return tw, th, cols, codes


def _load_label_font(size: int) -> ImageFont.FreeTypeFont:
    for path in (
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    ):
        if Path(path).exists():
            return ImageFont.truetype(path, size=size)
    return ImageFont.load_default()


def annotate_atlas_png(
    atlas_path: Path,
    *,
    font_size: int = 24,
    margin: int = 12,
    label_fill: tuple[int, int, int, int] = (52, 48, 44, 255),
    halo_fill: tuple[int, int, int, int] = (252, 246, 232, 230),
) -> int:
    """Overwrites ``atlas_path`` in place. Returns count of labels drawn."""
    toml_path = atlas_path.parent / "atlas.toml"
    if not atlas_path.is_file():
        raise FileNotFoundError(atlas_path)
    if not toml_path.is_file():
        raise FileNotFoundError(toml_path)

    tw, th, cols, layout = parse_atlas_toml(toml_path.read_text())
    im = Image.open(atlas_path).convert("RGBA")
    rows = (len(layout) + cols - 1) // cols
    exp_w, exp_h = cols * tw, rows * th
    aw, ah = im.size
    if (aw, ah) != (exp_w, exp_h):
        print(
            f"warning: atlas is {aw}x{ah}, expected {exp_w}x{exp_h} from atlas.toml; "
            f"resizing for grid alignment",
            file=sys.stderr,
        )
        im = im.resize((exp_w, exp_h), Image.Resampling.LANCZOS)

    draw = ImageDraw.Draw(im)
    font = _load_label_font(font_size)

    n = 0
    for i, code in enumerate(layout):
        if not code:
            continue
        n += 1
        col = i % cols
        row = i // cols
        x0 = col * tw
        y0 = row * th
        text = str(n)
        # Anchor bottom-right of cell so labels line up on the mathematical grid
        # (textbbox at (0,0) ignores font bearings and skews placement).
        ax = x0 + tw - margin
        ay = y0 + th - margin
        bbox = draw.textbbox((ax, ay), text, font=font, anchor="rb")
        pad = 3
        draw.rounded_rectangle(
            [bbox[0] - pad, bbox[1] - pad, bbox[2] + pad, bbox[3] + pad],
            radius=5,
            fill=halo_fill,
            outline=(200, 185, 160, 180),
            width=1,
        )
        draw.text((ax, ay), text, font=font, fill=label_fill, anchor="rb")

    im.save(atlas_path)
    return n


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "target",
        type=Path,
        help="Directory containing atlas.png or path to atlas.png",
    )
    ap.add_argument(
        "--font-size",
        type=int,
        default=24,
        help="Label font size in pixels (default: 24)",
    )
    ap.add_argument(
        "--margin",
        type=int,
        default=12,
        help="Inset from cell bottom-right (default: 12)",
    )
    args = ap.parse_args()
    target = args.target
    atlas = target / "atlas.png" if target.is_dir() else target
    try:
        count = annotate_atlas_png(
            atlas,
            font_size=args.font_size,
            margin=args.margin,
        )
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
    print(f"Labeled {count} tiles in {atlas}")


if __name__ == "__main__":
    main()
