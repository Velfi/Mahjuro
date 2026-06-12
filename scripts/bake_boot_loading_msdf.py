#!/usr/bin/env python3
"""Bake a single-channel SDF atlas for the boot \"loading...\" label.

Output (committed to assets/):
  assets/textures/boot_loading_msdf.png  — R channel = distance (0.5 = edge)
  assets/data/boot_loading_msdf.json     — layout + spread for the boot shader

Uses Instrument Serif (same face as HUD). Regenerate when copy or font changes:
  python3 scripts/bake_boot_loading_msdf.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError as e:
    raise SystemExit(
        "bake_boot_loading_msdf.py needs Pillow: pip install -r scripts/requirements.txt"
    ) from e

ROOT = Path(__file__).resolve().parents[1]
FONT_PATH = ROOT / "assets/fonts/Instrument_Serif/InstrumentSerif-Regular.ttf"
OUT_PNG = ROOT / "assets/textures/boot_loading_msdf.png"
OUT_JSON = ROOT / "assets/data/boot_loading_msdf.json"

TEXT = "loading..."
# 4× the original 48px bake — sharper on Retina when the boot plate downscales the atlas.
FONT_PX = 192
PAD = FONT_PX // 2
SPREAD_PX = FONT_PX / 6.0


def edt_outside(mask: list[list[bool]]) -> list[list[float]]:
    """8-neighbor chamfer distance (approx EDT) for False cells to nearest True."""
    h, w = len(mask), len(mask[0])
    inf = float(h + w)
    dist = [[0.0 if mask[y][x] else inf for x in range(w)] for y in range(h)]
    # Forward
    for y in range(h):
        for x in range(w):
            if mask[y][x]:
                continue
            best = dist[y][x]
            if y > 0:
                best = min(best, dist[y - 1][x] + 1.0)
                if x > 0:
                    best = min(best, dist[y - 1][x - 1] + 1.4142135)
                if x + 1 < w:
                    best = min(best, dist[y - 1][x + 1] + 1.4142135)
            if x > 0:
                best = min(best, dist[y][x - 1] + 1.0)
            dist[y][x] = best
    # Backward
    for y in range(h - 1, -1, -1):
        for x in range(w - 1, -1, -1):
            if mask[y][x]:
                continue
            best = dist[y][x]
            if y + 1 < h:
                best = min(best, dist[y + 1][x] + 1.0)
                if x > 0:
                    best = min(best, dist[y + 1][x - 1] + 1.4142135)
                if x + 1 < w:
                    best = min(best, dist[y + 1][x + 1] + 1.4142135)
            if x + 1 < w:
                best = min(best, dist[y][x + 1] + 1.0)
            dist[y][x] = best
    return dist


def edt_inside(mask: list[list[bool]]) -> list[list[float]]:
    inv = [[not mask[y][x] for x in range(len(mask[0]))] for y in range(len(mask))]
    return edt_outside(inv)


def encode_sdf(outside: list[list[float]], inside: list[list[float]], spread: float) -> Image.Image:
    h, w = len(outside), len(outside[0])
    rgba = Image.new("RGBA", (w, h))
    px = rgba.load()
    for y in range(h):
        for x in range(w):
            dist = outside[y][x] - inside[y][x]
            norm = 0.5 + dist / (2.0 * spread)
            norm = max(0.0, min(1.0, norm))
            v = int(norm * 255.0 + 0.5)
            px[x, y] = (v, v, v, 255)
    return rgba


def main() -> int:
    if not FONT_PATH.is_file():
        print(f"font not found: {FONT_PATH}", file=sys.stderr)
        return 1

    font = ImageFont.truetype(str(FONT_PATH), FONT_PX)
    bbox = font.getbbox(TEXT)
    text_w = bbox[2] - bbox[0]
    text_h = bbox[3] - bbox[1]
    atlas_w = text_w + PAD * 2
    atlas_h = text_h + PAD * 2

    mask_img = Image.new("L", (atlas_w, atlas_h), 0)
    draw = ImageDraw.Draw(mask_img)
    draw.text((PAD - bbox[0], PAD - bbox[1]), TEXT, fill=255, font=font)

    mask = [[mask_img.getpixel((x, y)) > 127 for x in range(atlas_w)] for y in range(atlas_h)]
    outside = edt_outside(mask)
    inside = edt_inside(mask)
    sdf = encode_sdf(outside, inside, SPREAD_PX)

    OUT_PNG.parent.mkdir(parents=True, exist_ok=True)
    OUT_JSON.parent.mkdir(parents=True, exist_ok=True)
    sdf.save(OUT_PNG)

    meta = {
        "text": TEXT,
        "font_px": FONT_PX,
        "spread_px": SPREAD_PX,
        "atlas_w": atlas_w,
        "atlas_h": atlas_h,
        "text_w": text_w,
        "text_h": text_h,
        "pad": PAD,
        "color_stone": [0.716, 0.683, 0.645, 1.0],
    }
    OUT_JSON.write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {OUT_PNG} ({atlas_w}x{atlas_h})")
    print(f"wrote {OUT_JSON}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
