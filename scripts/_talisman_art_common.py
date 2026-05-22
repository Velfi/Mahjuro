"""Shared postprocess and octagon mask helpers for talisman heightmap art."""

from __future__ import annotations

import math
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw

# Match `HEIGHT_ALPHA_LO` in generate_relic_art.py / enamel discard in lit_mesh.wgsl.
HEIGHT_ALPHA_LO = 8

# Circumradius and angle offset match `talisman_mesh.rs` (RADIUS = 0.5, OCTAGON_ANGLE_OFFSET).
RADIUS = 0.5
OCTAGON_ANGLE_OFFSET = math.tau / 16.0  # π/8 rad — flat edges at ±X/±Y


def local_xy_from_uv(u: float, v: float) -> tuple[float, float]:
    """Inverse of `talisman_face_uv`: u = x/R*0.5+0.5, v = 0.5 - y/R*0.5 with R=0.5."""
    return (u - 0.5, 0.5 - v)


def uv_from_local_xy(x: float, y: float) -> tuple[float, float]:
    return (x / RADIUS * 0.5 + 0.5, 0.5 - y / RADIUS * 0.5)


def octagon_rim_local() -> list[tuple[float, float]]:
    return [
        (
            math.cos(OCTAGON_ANGLE_OFFSET + i * math.tau / 8.0) * RADIUS,
            math.sin(OCTAGON_ANGLE_OFFSET + i * math.tau / 8.0) * RADIUS,
        )
        for i in range(8)
    ]


def octagon_polygon_pixels(size: int) -> list[tuple[int, int]]:
    """Octagon vertices in image space (flat edge at bottom / −local Y)."""
    last = size - 1
    verts: list[tuple[int, int]] = []
    for x, y in octagon_rim_local():
        u, v = uv_from_local_xy(x, y)
        verts.append((int(round(u * last)), int(round(v * last))))
    return verts


def write_octagon_mask(mask_path: Path, size: int) -> None:
    """Binary L mask: white inside the mesh octagon, black outside."""
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.polygon(octagon_polygon_pixels(size), fill=255)
    mask_path.parent.mkdir(parents=True, exist_ok=True)
    mask.save(mask_path, format="PNG", optimize=True)


def write_mask_from_height(height_path: Path, mask_path: Path) -> bool:
    """Fallback: silhouette from height luminance (legacy assets with black void)."""
    if not height_path.exists():
        return False
    with Image.open(height_path) as im:
        height = im.convert("L")
    mask = height.point(lambda v: 255 if v >= HEIGHT_ALPHA_LO else 0, mode="L")
    mask_path.parent.mkdir(parents=True, exist_ok=True)
    mask.save(mask_path, format="PNG", optimize=True)
    return True


def postprocess_heightmap(raw_bytes: bytes, out_size: int, exaggerate: float = 1.55) -> Image.Image:
    from PIL import ImageOps

    img = Image.open(BytesIO(raw_bytes)).convert("L")
    w, h = img.size
    side = min(w, h)
    left = (w - side) // 2
    top = (h - side) // 2
    img = img.crop((left, top, left + side, top + side))
    img = ImageOps.autocontrast(img, cutoff=1)
    if exaggerate != 1.0:
        inv_exp = 1.0 / max(exaggerate, 1e-3)
        lut = []
        for v in range(256):
            d = max(-1.0, min(1.0, (v - 128) / 127.0))
            sign = 1.0 if d >= 0.0 else -1.0
            pushed = sign * (abs(d) ** inv_exp)
            lut.append(max(0, min(255, int(round(128.0 + pushed * 127.0)))))
        img = img.point(lut)
    if img.size != (out_size, out_size):
        img = img.resize((out_size, out_size), Image.LANCZOS)
    return img
