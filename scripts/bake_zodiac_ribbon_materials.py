#!/usr/bin/env python3
"""
Derive linear zodiac ribbon material maps from authored albedo PNGs.

Each `assets/textures/zodiacs/zodiac_<slug>.png` gets a sibling
`zodiac_<slug>_material.png` consumed by `load_zodiac_ribbon_textures()` and
`lit_mesh.wgsl` CatalogPaper shading:

  R = height (paper ≈ 0.5, embroidery higher)
  G = roughness (matte paper ≈ 0.92, glossy thread ≈ 0.62)
  B = embroidered-thread mask
  A = 255

Usage:
    python3 scripts/bake_zodiac_ribbon_materials.py
    python3 scripts/bake_zodiac_ribbon_materials.py --name dragon
    python3 scripts/bake_zodiac_ribbon_materials.py --force
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image, ImageChops, ImageFilter
except ImportError:
    print("Error: pillow required. Run: pip install pillow")
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
ZODIAC_DIR = ROOT / "assets" / "textures" / "zodiacs"

# Slugs in ZodiacKind::all() order (see crates/mahjuro-core/src/core/zodiac.rs).
SLUGS = (
    "mouse",
    "rat",
    "ox",
    "tiger",
    "rabbit",
    "dragon",
    "snake",
    "horse",
    "goat",
    "monkey",
    "rooster",
    "dog",
    "pig",
    "qilin",
    "phoenix",
    "crane",
    "koi",
)

# Shader defaults when no map is bound: vec4(0.5, 0.92, 0.0, 1.0).
PAPER_HEIGHT = 128
PAPER_ROUGH = int(round(0.92 * 255))
THREAD_ROUGH = int(round(0.62 * 255))


def _stretch_gray(img: Image.Image, clip: float = 1.5) -> Image.Image:
    """Percentile clip + autocontrast so embroidery detail spans the full range."""
    hist = img.histogram()
    total = sum(hist)
    if total <= 0:
        return img
    lo_target = total * (clip / 100.0)
    hi_target = total * (1.0 - clip / 100.0)
    lo = 0
    acc = 0
    for v, count in enumerate(hist):
        acc += count
        if acc >= lo_target:
            lo = v
            break
    hi = 255
    acc = 0
    for v in range(255, -1, -1):
        acc += hist[v]
        if acc >= total - hi_target:
            hi = v
            break
    if hi <= lo:
        return img
    scale = 255.0 / max(1, hi - lo)

    def remap(v: int) -> int:
        return int(max(0, min(255, (v - lo) * scale)))

    return img.point(remap, mode="L")


def derive_material(albedo: Image.Image) -> Image.Image:
    """Build an RGBA material map from a full-bleed ribbon albedo."""
    rgb = albedo.convert("RGB")
    luma = rgb.convert("L")

    # Embroidery reads as high-frequency luminance against plain washi ground.
    smooth = luma.filter(ImageFilter.GaussianBlur(radius=4))
    detail = ImageChops.difference(luma, smooth)
    detail = detail.point(lambda v: min(255, int(v * 2.8)))

    # Hair thread is slightly more chromatic than the dyed paper field.
    r, g, b = rgb.split()
    max_c = ImageChops.lighter(r, g)
    max_c = ImageChops.lighter(max_c, b)
    min_c = ImageChops.darker(r, g)
    min_c = ImageChops.darker(min_c, b)
    chroma = ImageChops.subtract(max_c, min_c)
    chroma = chroma.filter(ImageFilter.GaussianBlur(radius=1))

    thread = ImageChops.add(
        detail.point(lambda v: int(v * 0.72)),
        chroma.point(lambda v: int(v * 0.55)),
    )
    thread = thread.filter(ImageFilter.GaussianBlur(radius=1))
    thread = _stretch_gray(thread)
    thread = thread.filter(ImageFilter.GaussianBlur(radius=0.8))

    # Height: flat paper at mid-gray, raised where stitches cluster.
    thread_px = thread.load()
    detail_px = detail.load()
    w, h = thread.size
    height_data = []
    for y in range(h):
        for x in range(w):
            t = thread_px[x, y]
            d = detail_px[x, y]
            v = PAPER_HEIGHT + int((t - 48) * 0.55 + d * 0.22)
            height_data.append(max(0, min(255, v)))
    height = Image.new("L", (w, h))
    height.putdata(height_data)

    # Roughness: matte washi → tighter thread sheen.
    rough_data = []
    for y in range(h):
        for x in range(w):
            t = thread_px[x, y] / 255.0
            v = int(PAPER_ROUGH + t * (THREAD_ROUGH - PAPER_ROUGH))
            rough_data.append(max(0, min(255, v)))
    rough = Image.new("L", (w, h))
    rough.putdata(rough_data)

    alpha = Image.new("L", (w, h), 255)
    return Image.merge("RGBA", (height, rough, thread, alpha))


def bake_one(slug: str, *, force: bool) -> bool:
    albedo_path = ZODIAC_DIR / f"zodiac_{slug}.png"
    material_path = ZODIAC_DIR / f"zodiac_{slug}_material.png"
    if not albedo_path.is_file():
        print(f"  skip {slug}: missing {albedo_path.name}")
        return False
    if material_path.exists() and not force:
        print(f"  skip {slug}: {material_path.name} exists (use --force)")
        return False
    with Image.open(albedo_path) as im:
        material = derive_material(im)
    material.save(material_path, format="PNG")
    print(f"  wrote {material_path.relative_to(ROOT)}")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Bake zodiac ribbon material maps from albedo PNGs"
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Bake only this slug (e.g. dragon).",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite existing material maps.",
    )
    args = parser.parse_args()

    if args.name is not None:
        if args.name not in SLUGS:
            print(f"Error: unknown slug '{args.name}'. Valid: {', '.join(SLUGS)}")
            sys.exit(1)
        targets = [args.name]
    else:
        targets = list(SLUGS)

    ZODIAC_DIR.mkdir(parents=True, exist_ok=True)
    wrote = 0
    for slug in targets:
        if bake_one(slug, force=args.force):
            wrote += 1
    print(f"Done. wrote={wrote} → {ZODIAC_DIR}")


if __name__ == "__main__":
    main()
