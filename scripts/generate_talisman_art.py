#!/usr/bin/env python3
"""
Generate grayscale heightmaps and octagon silhouette masks for talisman tablets.

Outputs per kind:
  - Shop:     talisman_{slug}.png + talisman_{slug}_mask.png
  - Memorial: memorial_{slug}.png + memorial_{slug}_mask.png

Heightmaps are full mid-gray plates with carved relief (no black void — the mask
handles cutout). Masks are procedural octagons aligned with `talisman_face_uv`.

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python scripts/generate_talisman_art.py                    # missing only, all sets
    python scripts/generate_talisman_art.py --force            # regenerate everything
    python scripts/generate_talisman_art.py --set shop         # shop talismans only
    python scripts/generate_talisman_art.py --set memorial
    python scripts/generate_talisman_art.py --masks-only       # masks from existing heights
    python scripts/generate_talisman_art.py --name pearl --set shop
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import DEFAULT_MODEL, generate_image_bytes, init_client, parse_size  # noqa: E402
from _talisman_art_common import postprocess_heightmap, write_octagon_mask  # noqa: E402

try:
    from PIL import Image
except ImportError:
    print("Error: pillow required. Run: pip install pillow")
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = ROOT / "assets" / "textures" / "talismans"
SHOP_DATA = ROOT / "assets" / "data" / "talismans.json"
MEMORIAL_DATA = ROOT / "assets" / "data" / "memorial_talismans.json"
OUT_SIZE = 256

SHOP_STYLE = (
    "A flat, top-down grayscale HEIGHTMAP texture for a single regular octagonal "
    "jade merchant talisman (eight equal sides), centered and nearly filling a "
    "square frame. A **flat horizontal edge** runs along the **bottom** of the "
    "octagon (stop-sign resting on its edge, not a point). DISPLACEMENT MAP for "
    "real-time 3D — tonal value is literal surface height.\n\n"
    "Construction: crisp shop engraving on polished tablet. Raised motifs read "
    "near-white; recessed field and groove moats read mid-gray (#808080). The "
    "**entire square** stays on the mid-gray plate — **do not paint black outside "
    "the octagon** (silhouette is a separate mask texture).\n\n"
    "Tonal key (discrete plateaus):\n"
    "  • #ffffff: highest relief peaks and rim highlights\n"
    "  • #e0–#f0: secondary highs inside motifs\n"
    "  • #808080: uniform flat field between motifs\n"
    "  • #3a3a3a: narrow recess grooves outlining raised elements\n\n"
    "No cast shadows, no specular, no perspective, no color, no Latin letters. "
    "Orthographic heightfield only. Premium mahjong-house merchandise — readable "
    "at thumbnail size."
)

MEMORIAL_STYLE = (
    "A flat, top-down grayscale HEIGHTMAP texture for a single regular octagonal "
    "jade memorial tablet (eight equal sides), centered and nearly filling a "
    "square frame. A **flat horizontal edge** runs along the **bottom** of the "
    "octagon (stop-sign resting on its edge, not a point). DISPLACEMENT MAP for "
    "real-time 3D — tonal value is literal surface height.\n\n"
    "Construction: shallow carved relief like an ancient stone rubbing or worn "
    "temple plaque. Raised lines read near-white; recessed field and groove moats "
    "read mid-gray (#808080). The **entire square** stays on the mid-gray plate — "
    "**do not paint black outside the octagon** (silhouette is a separate mask).\n\n"
    "Tonal key (discrete plateaus):\n"
    "  • #ffffff: highest relief peaks and rim highlights\n"
    "  • #e0–#f0: secondary highs inside motifs\n"
    "  • #808080: uniform flat field between motifs\n"
    "  • #3a3a3a: narrow recess grooves\n\n"
    "No shading, no cast shadows, no specular, no perspective, no color, no "
    "Latin letters. Worn, solemn memorial — not glossy shop merchandise."
)

SHOP_MOTIFS: dict[str, str] = {
    "pearl": (
        "Motif — Pearl: a single large nacreous disk in the center with concentric "
        "growth rings and a small raised boss, rimmed by a thin double-line octagon "
        "border. Reads as lustrous pearl inlay."
    ),
    "gilded": (
        "Motif — Gilded: three overlapping ancient coins with square holes, stacked "
        "diagonally, each coin a raised ring with dark square recess. Suggests gold "
        "payout on scored tiles."
    ),
    "polychrome": (
        "Motif — Polychrome: a six-point starburst mandala radiating from center, "
        "each arm a raised wedge separated by groove moats, implying multicolor "
        "mult bonus."
    ),
    "bamboo": (
        "Motif — Bamboo: three vertical bamboo stalks with joint nodes, simplified "
        "segments, leaves as small raised chevrons at the top. Suit-transform tablet."
    ),
    "dots": (
        "Motif — Dots: a large concentric circle target (one bold ring, one inner "
        "disk) like a simplified pin / circle suit pip, centered on the tablet."
    ),
    "characters": (
        "Motif — Characters: a bold wan / character suit square frame in the center "
        "with a raised horizontal bar and two side pillars — abstract, not a real "
        "kanji glyph."
    ),
    "honors": (
        "Motif — Honors: three small dragon-scale shields in a triangle arrangement, "
        "each shield a raised teardrop with a central groove line."
    ),
    "wildflower": (
        "Motif — Wildflower: a five-petal flower viewed from above, petals as raised "
        "teardrops around a central stamen boss, stem groove curving to the bottom edge."
    ),
    "conformity": (
        "Motif — Conformity: nine identical small squares in a 3×3 grid, each "
        "square a raised tile blank, suggesting every tile becoming the same."
    ),
}

MEMORIAL_MOTIFS: dict[str, str] = {
    "exhausted": (
        "Motif — The Exhausted: concentric rings radiating from a small central "
        "boss, like ripples fading to stillness. Outer rings broken and irregular."
    ),
    "frozen_hand": (
        "Motif — The Frozen Hand: stylized open handprint in the center, rimmed by "
        "cracked ice facets and short fracture lines radiating outward."
    ),
    "skipper": (
        "Motif — The Skipper: stepping stones along a winding path with gaps — "
        "missing stones as dark recess voids between raised stones."
    ),
    "hoarder": (
        "Motif — The Hoarder: stack of seven ancient coins with square holes piled "
        "in the center, overlapping disks."
    ),
    "full_dish": (
        "Motif — The Full Dish: wide shallow bowl from above, brim with three small "
        "raised pebbles on the rim lip."
    ),
    "discarded": (
        "Motif — The Discarded: three blank mahjong tile rectangles tumbling "
        "downward with trailing ripple grooves."
    ),
    "boss_mark": (
        "Motif — The Boss's Mark: heavy house seal / stern angular mask emblem "
        "inside a thick circular ring with four radial tick marks."
    ),
    "buff_saint": (
        "Motif — The Buff Saint: six small raised seal stamps in a ring around a "
        "central dot, each a different simple geometric glyph."
    ),
    "transformer": (
        "Motif — The Transformer: bamboo stalk, dot circle, and wan square in a "
        "column connected by morphing groove lines."
    ),
    "tag_bearer": (
        "Motif — The Tag Bearer: five rectangular house tokens fanning from a "
        "central nail head."
    ),
    "meld_mason": (
        "Motif — The Meld Mason: interlocking triplet bar, sequence chain, and "
        "pair block fitted like masonry."
    ),
    "deep_walker": (
        "Motif — The Deep Walker: paired footprints along a coiled maze path from "
        "rim toward center with depth tick marks."
    ),
}


def height_path(prefix: str, slug: str) -> Path:
    return OUTPUT_DIR / f"{prefix}_{slug}.png"


def mask_path(prefix: str, slug: str) -> Path:
    return OUTPUT_DIR / f"{prefix}_{slug}_mask.png"


def load_shop_entries() -> list[dict]:
    raw = json.loads(SHOP_DATA.read_text(encoding="utf-8"))
    out: list[dict] = []
    for row in raw:
        slug = row["id"]
        if slug not in SHOP_MOTIFS:
            raise SystemExit(f"talismans.json id {slug!r} missing from SHOP_MOTIFS")
        out.append(
            {
                "prefix": "talisman",
                "slug": slug,
                "name": row["name"],
                "style": SHOP_STYLE,
                "motif": SHOP_MOTIFS[slug],
            }
        )
    return out


def load_memorial_entries() -> list[dict]:
    raw = json.loads(MEMORIAL_DATA.read_text(encoding="utf-8"))
    out: list[dict] = []
    for row in raw:
        slug = row["id"]
        if slug not in MEMORIAL_MOTIFS:
            raise SystemExit(f"memorial_talismans.json id {slug!r} missing from MEMORIAL_MOTIFS")
        out.append(
            {
                "prefix": "memorial",
                "slug": slug,
                "name": row["name"],
                "style": MEMORIAL_STYLE,
                "motif": MEMORIAL_MOTIFS[slug],
            }
        )
    return out


def build_prompt(entry: dict) -> str:
    return f"{entry['style']}\n\n{entry['motif']}"


def write_mask_for_entry(entry: dict, out_size: int) -> Path:
    mpath = mask_path(entry["prefix"], entry["slug"])
    write_octagon_mask(mpath, out_size)
    return mpath


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate talisman heightmaps and masks")
    parser.add_argument(
        "--set",
        choices=("shop", "memorial", "all"),
        default="all",
        help="Which talisman set to process",
    )
    parser.add_argument("--name", type=str, help="Single slug (e.g. pearl, boss_mark)")
    parser.add_argument("--force", action="store_true", help="Regenerate even if on disk")
    parser.add_argument(
        "--masks-only",
        action="store_true",
        help="Write procedural octagon masks only (no API)",
    )
    parser.add_argument("--dry-run", action="store_true", help="Print prompts only")
    parser.add_argument("--list", action="store_true", help="List slugs and exit")
    parser.add_argument("--model", type=str, default=DEFAULT_MODEL)
    parser.add_argument("--size", type=str, default="1:1@1K")
    parser.add_argument("--out-size", type=int, default=OUT_SIZE)
    parser.add_argument("--sleep", type=float, default=2.0, help="Seconds between API calls")
    args = parser.parse_args()

    entries: list[dict] = []
    if args.set in ("shop", "all"):
        entries.extend(load_shop_entries())
    if args.set in ("memorial", "all"):
        entries.extend(load_memorial_entries())

    if args.list:
        for e in entries:
            hp = height_path(e["prefix"], e["slug"])
            mp = mask_path(e["prefix"], e["slug"])
            hm = "✓" if hp.is_file() else "·"
            mm = "✓" if mp.is_file() else "·"
            print(f"  {hm}{mm} {e['prefix']}_{e['slug']:16}  {e['name']}")
        return

    if args.name:
        entries = [e for e in entries if e["slug"] == args.name]
        if not entries:
            raise SystemExit(f"unknown --name {args.name!r}")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    if args.dry_run:
        for e in entries:
            print(f"\n=== {e['prefix']}_{e['slug']}.png ===\n")
            print(build_prompt(e))
        return

    if args.masks_only:
        for e in entries:
            mpath = write_mask_for_entry(e, args.out_size)
            print(f"  mask {mpath.name}")
        print("Done (masks only).")
        return

    client = init_client()
    aspect_ratio, image_size = parse_size(args.size)

    for i, entry in enumerate(entries):
        hp = height_path(entry["prefix"], entry["slug"])
        mp = mask_path(entry["prefix"], entry["slug"])
        need_height = args.force or not hp.is_file()
        need_mask = args.force or not mp.is_file()

        if need_height:
            prompt = build_prompt(entry)
            print(f"[{i + 1}/{len(entries)}] generating {hp.name} …")
            raw = generate_image_bytes(
                client,
                prompt,
                model=args.model,
                aspect_ratio=aspect_ratio,
                image_size=image_size,
            )
            cleaned = postprocess_heightmap(raw, args.out_size)
            cleaned.save(hp, "PNG", optimize=True)
            print(f"  wrote {hp.name}")
            if i + 1 < len(entries) and args.sleep > 0:
                time.sleep(args.sleep)
        elif not need_mask:
            print(f"  skip {hp.name} (exists)")
        else:
            print(f"  skip {hp.name} (exists, mask pending)")

        if need_mask or need_height:
            write_mask_for_entry(entry, args.out_size)
            print(f"  wrote {mp.name}")

    print("Done.")


if __name__ == "__main__":
    main()
