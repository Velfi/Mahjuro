#!/usr/bin/env python3
"""Refresh `assets/textures/shop/candle_sss.png` (UV1 lightmap for shop.glb candle wax).

Blender workflow (preferred):
  1. Open `assets/3d/source/Shop.blend`, select all `Candle*` meshes.
  2. Bake subsurface / diffuse lighting to a new image using the **Lightmap** UV
     (`TEXCOORD_1` in the exported GLB — small atlas patch near UV center).
  3. Save as `assets/textures/shop/candle_sss.png` (linear or sRGB warm pass).

This script writes a soft placeholder when Blender is unavailable (CI / first setup).
Re-run after moving candles or changing shop lighting.
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets/textures/shop/candle_sss.png"
SIZE = 1024


def main() -> None:
    # Observed TEXCOORD_1 span on exported candles ≈ (0.45–0.55, 0.41–0.59).
    cx = int(0.5 * SIZE)
    cy = int(0.5 * SIZE)
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    for i in range(10):
        ang = i * (2.0 * math.pi / 10.0)
        ox = int(math.cos(ang) * 72.0)
        oy = int(math.sin(ang) * 52.0)
        layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        d = ImageDraw.Draw(layer)
        x0 = cx + ox - 55
        y0 = cy + oy - 75
        d.ellipse((x0, y0, x0 + 110, y0 + 150), fill=(255, 198, 128, 200))
        img = Image.alpha_composite(img, layer)
    img = img.filter(ImageFilter.GaussianBlur(radius=22))
  # Premultiply warm scatter into RGB; keep alpha for optional masking.
    px = img.load()
    for y in range(SIZE):
        for x in range(SIZE):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            k = a / 255.0
            px[x, y] = (
                min(255, int(r * k * 1.15)),
                min(255, int(g * k * 1.10)),
                min(255, int(b * k * 0.95)),
                255,
            )
    OUT.parent.mkdir(parents=True, exist_ok=True)
    img.save(OUT)
    print(f"wrote {OUT.relative_to(ROOT)} ({SIZE}x{SIZE})")


if __name__ == "__main__":
    main()
