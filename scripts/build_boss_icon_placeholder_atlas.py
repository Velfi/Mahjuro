#!/usr/bin/env python3
"""Build `assets/textures/boss_icons/atlas.png` from `atlas.toml`.

Writes simple distinct placeholder tiles (hue ramp + vignette) so the game
has valid RGBA art before hand-painted icons land. For Gemini-generated art
instead, use ``scripts/generate_boss_icons.py`` (writes ``source/boss_{slug}.png``,
post-processes, and repacks the atlas).

Replace cells by dropping 512×512 PNGs into
``assets/textures/boss_icons/source/{slug}.png`` and re-run with Pillow to
composite those over the placeholders.

Usage:
  pip install pillow
  python3 scripts/build_boss_icon_placeholder_atlas.py
"""

from __future__ import annotations

import math
import sys
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT_DIR = REPO / "assets" / "textures" / "boss_icons"
SOURCE_DIR = OUT_DIR / "source"

try:
    from PIL import Image, ImageDraw
except ImportError:
    print("Install Pillow: pip install pillow", file=sys.stderr)
    sys.exit(1)


def load_atlas_meta(toml_path: Path) -> tuple[int, int, int, list[str]]:
    data = tomllib.loads(toml_path.read_text())
    layout = data["layout"]
    return int(data["tile_width"]), int(data["tile_height"]), int(data["columns"]), layout


def cell_image(slug: str, idx: int, w: int, h: int) -> Image.Image:
    """Fallback tile when ``source/{slug}.png`` is missing."""
    hue = (idx * 47) % 360 / 360.0
    # HSV-ish via simple lerp (good enough for placeholders)
    r = 0.35 + 0.45 * abs(math.sin(hue * math.tau))
    g = 0.28 + 0.45 * abs(math.sin((hue + 0.33) * math.tau))
    b = 0.22 + 0.45 * abs(math.sin((hue + 0.66) * math.tau))
    img = Image.new("RGBA", (w, h), (int(r * 255), int(g * 255), int(b * 255), 255))
    draw = ImageDraw.Draw(img, "RGBA")
    margin = int(min(w, h) * 0.08)
    draw.rounded_rectangle(
        [margin, margin, w - margin, h - margin],
        radius=margin,
        outline=(20, 16, 12, 200),
        width=max(2, margin // 4),
    )
    return img


def main() -> None:
    toml_path = OUT_DIR / "atlas.toml"
    tw, th, cols, layout = load_atlas_meta(toml_path)
    ncells = len(layout)
    rows = (ncells + cols - 1) // cols
    sheet = Image.new("RGBA", (cols * tw, rows * th), (0, 0, 0, 0))
    idx = 0
    for i, slug in enumerate(layout):
        if not slug:
            continue
        col = i % cols
        row = i // cols
        src = SOURCE_DIR / f"{slug}.png"
        if src.is_file():
            tile = Image.open(src).convert("RGBA")
            tile = tile.resize((tw, th), Image.Resampling.LANCZOS)
        else:
            tile = cell_image(slug, idx, tw, th)
            idx += 1
        sheet.paste(tile, (col * tw, row * th), tile)
    png_path = OUT_DIR / "atlas.png"
    sheet.save(png_path)
    print(f"wrote {png_path} ({sheet.width}×{sheet.height})")


if __name__ == "__main__":
    main()
