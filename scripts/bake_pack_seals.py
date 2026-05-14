#!/usr/bin/env python3
"""Bake a wax seal + foil spine onto each booster-pack cover PNG.

The pack covers shipped from `generate_pack_covers.py` are mostly
transparent, with a small per-kind sigil floating in the center. At
thumbnail size on the shop counter, the foil tint dominates and every
pack reads as a similarly-shaped pastel slab. This script composites
two extra elements onto each cover so the cuboid gains a focal point
and a rarity-tied edge:

- A centered wax seal: a flat circular disc in the per-kind seal
  color, with a thin highlight ring and a small impressed insignia.
  Sells "this is sealed, valuable, worth opening" in one glance.
- A vertical foil/spine stripe along the right edge: a thin metallic
  band tinted by the per-kind foil color, so the rarity tint reads
  even when the wrapper is otherwise occluded.

Idempotent: on first run, the unbaked PNG is moved to
`assets/textures/tile_packs/source/`; subsequent runs re-bake from source
and overwrite the live PNG. Re-runs do not double-stamp.

Usage:
    python3 scripts/bake_pack_seals.py             # bake all
    python3 scripts/bake_pack_seals.py --name honors
    python3 scripts/bake_pack_seals.py --restore   # restore unbaked
"""

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)


PACK_DIR = (
    Path(__file__).resolve().parent.parent / "assets" / "textures" / "tile_packs"
)
SOURCE_DIR = PACK_DIR / "source"

# (slug, seal_rgba, foil_rgba, insignia)
# seal_rgba mirrors TilePackKind::seal_color (deep wax reds, shifted per
# kind so the seal contrasts the wrapper rather than disappearing into it).
# foil_rgba mirrors TilePackKind::foil_tint and is used for the right-edge
# spine stripe so the rarity tint survives even at thumbnail size.
# insignia is a single character drawn in inset on the seal — a small
# pictogram that distinguishes packs at a glance without needing legible
# text. Matches the calligraphic register of the shrine plaques.
PACKS = [
    # Insignia chars are picked from a stable ASCII subset so the default
    # Arial Bold (or Pillow's bundled font as a last-resort fallback)
    # always renders them. Each one is on-vocabulary for the pack theme.
    ("honors",         (189,  46,  41), (235, 199,  97), "H"),  # Honor
    ("terminals",      (143,  36,  31), (199, 133,  82), "9"),  # 1/9 terminal
    ("flowers",        (133,  36,  77), (235, 158, 179), "F"),  # Flower
    ("bamboo_grove",   (199,  46,  36), (122, 199, 133), "B"),  # Bamboo
    ("coin_cache",     (148,  26,  46), (199, 209, 224), "C"),  # Coin / Circles
    ("scroll_library", (184,  46,  46), (107, 122, 199), "S"),  # Scroll
    ("polychrome",     (148,  26,  46), (235, 235, 235), "P"),  # Polychrome
]


def stage_source(slug: str) -> Path:
    """Move the live PNG into source/ on first run, return source path."""
    SOURCE_DIR.mkdir(parents=True, exist_ok=True)
    src = SOURCE_DIR / f"pack_{slug}.png"
    live = PACK_DIR / f"pack_{slug}.png"
    if not src.exists():
        if not live.exists():
            return src
        # First bake: snapshot the unmodified PNG so future bakes
        # always start from the original art, never from a previously
        # baked output.
        src.write_bytes(live.read_bytes())
    return src


def draw_wax_seal(canvas: Image.Image, seal_rgba: tuple, insignia: str) -> None:
    """Draw a centered wax-seal disc with a highlight ring and an inset
    insignia. Seal sits at ~58% of pack height — visually centered in
    the upper sigil region, sharing space with the existing decal."""
    w, h = canvas.size
    cx = w // 2
    # Seat the seal in the lower title-bar region (under the gold
    # divider authored at ~70% down) so it complements the existing
    # central sigil rather than overlapping it. Reading order then
    # becomes: foil tint → sigil at top → seal at bottom.
    cy = int(h * 0.83)
    # Disc radius: 13% of the short side keeps the seal proportionate
    # to the title-bar region (only ~25% of the pack height) while
    # still reading as a focal point at thumbnail size.
    radius = int(min(w, h) * 0.13)

    # Soft ambient shadow under the seal — sells the impression of a
    # raised wax dollop sitting on the pack face.
    shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    pad = 3
    sd.ellipse(
        (cx - radius - pad, cy - radius + pad,
         cx + radius + pad, cy + radius + pad * 2),
        fill=(0, 0, 0, 110),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=4))
    canvas.alpha_composite(shadow)

    d = ImageDraw.Draw(canvas)
    r, g, b = seal_rgba
    # Main disc — opaque wax body.
    d.ellipse(
        (cx - radius, cy - radius, cx + radius, cy + radius),
        fill=(r, g, b, 255),
    )
    # Inner highlight ring — thin lighter-wax band just inside the
    # rim, suggesting the meniscus where the wax pooled when it set.
    inner = int(radius * 0.86)
    d.ellipse(
        (cx - inner, cy - inner, cx + inner, cy + inner),
        outline=(min(r + 38, 255), min(g + 24, 255), min(b + 24, 255), 220),
        width=2,
    )
    # Specular highlight: a small soft warm-white crescent on the
    # upper-left quadrant. Sold as a glossy lacquered wax drop.
    spec = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    sp = ImageDraw.Draw(spec)
    sp.ellipse(
        (cx - int(radius * 0.55), cy - int(radius * 0.65),
         cx - int(radius * 0.10), cy - int(radius * 0.20)),
        fill=(255, 232, 210, 110),
    )
    spec = spec.filter(ImageFilter.GaussianBlur(radius=4))
    canvas.alpha_composite(spec)
    # Insignia: a single pictogram impressed into the wax. Uses the
    # default Pillow font sized to fit; we draw it slightly darker
    # than the disc to mimic an embossed indent rather than printed ink.
    glyph_color = (
        max(r - 60, 0),
        max(g - 30, 0),
        max(b - 30, 0),
        255,
    )
    # Approximate centering with bbox math; ImageFont default has
    # reliable extents for ASCII glyphs.
    try:
        from PIL import ImageFont
        try:
            font = ImageFont.truetype(
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                size=int(radius * 1.1),
            )
        except OSError:
            font = ImageFont.load_default()
    except ImportError:
        font = None

    if font is not None:
        bbox = d.textbbox((0, 0), insignia, font=font)
        tw = bbox[2] - bbox[0]
        th = bbox[3] - bbox[1]
        d.text(
            (cx - tw / 2 - bbox[0], cy - th / 2 - bbox[1]),
            insignia,
            fill=glyph_color,
            font=font,
        )


def draw_foil_spine(canvas: Image.Image, foil_rgba: tuple) -> None:
    """Draw a vertical foil stripe along the right edge of the pack.

    Reads as a metallic spine — a thin bright band tinted by the
    per-kind foil color so the rarity signal persists even when the
    central sigil is occluded by gameplay elements (cursor, tooltip).
    Uses a vertical highlight gradient to fake a polished metal sheen."""
    w, h = canvas.size
    r, g, b = foil_rgba
    # Spine width: 5% of the pack width sits cleanly inside the rounded
    # corner without spilling past it. Inset 2px from the absolute edge
    # so it doesn't get clipped on the rounded silhouette of build_pack_mesh.
    spine_w = max(8, int(w * 0.05))
    inset = 2
    x1 = w - inset - spine_w
    x2 = w - inset
    y1 = int(h * 0.06)
    y2 = int(h * 0.94)

    spine = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(spine)
    # Base metallic band.
    sd.rectangle((x1, y1, x2, y2), fill=(r, g, b, 235))
    # Vertical highlight column — a brighter sliver in the middle that
    # suggests a polished metal reflection. Three-stop ramp keeps it
    # from looking like a hard line.
    hi_w = max(2, spine_w // 3)
    hi_x = x1 + (spine_w - hi_w) // 2
    hi = (
        min(r + 70, 255),
        min(g + 60, 255),
        min(b + 60, 255),
        220,
    )
    sd.rectangle((hi_x, y1, hi_x + hi_w, y2), fill=hi)
    # Soft outer glow to integrate into the pack face.
    spine = spine.filter(ImageFilter.GaussianBlur(radius=1))
    canvas.alpha_composite(spine)


def bake_one(slug: str) -> bool:
    src = stage_source(slug)
    live = PACK_DIR / f"pack_{slug}.png"
    if not src.exists():
        print(f"  [skip] no source for {slug}")
        return False
    pack = next((p for p in PACKS if p[0] == slug), None)
    if pack is None:
        print(f"  [skip] unknown pack slug {slug}")
        return False
    _, seal_rgba, foil_rgba, insignia = pack
    img = Image.open(src).convert("RGBA")
    draw_foil_spine(img, foil_rgba)
    draw_wax_seal(img, seal_rgba, insignia)
    img.save(live)
    print(f"  baked {live.name}")
    return True


def restore_one(slug: str) -> bool:
    src = SOURCE_DIR / f"pack_{slug}.png"
    live = PACK_DIR / f"pack_{slug}.png"
    if not src.exists():
        print(f"  [skip] no source snapshot for {slug}")
        return False
    live.write_bytes(src.read_bytes())
    print(f"  restored {live.name}")
    return True


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--name", help="bake only this slug")
    p.add_argument("--restore", action="store_true",
                   help="restore unbaked source PNGs to the live location")
    p.add_argument("--list", action="store_true", help="list slugs and exit")
    args = p.parse_args()

    if args.list:
        for slug, *_ in PACKS:
            print(f"  {slug}")
        return

    targets = [args.name] if args.name else [s for s, *_ in PACKS]
    op = restore_one if args.restore else bake_one
    n = 0
    for slug in targets:
        if op(slug):
            n += 1
    verb = "restored" if args.restore else "baked"
    print(f"Done. {verb}={n}")


if __name__ == "__main__":
    main()
