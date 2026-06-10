#!/usr/bin/env python3
"""
Generate grayscale heightmaps and organic silhouette masks for carved jade talisman art.

Outputs per kind:
  - Shop:     talisman_{slug}.png + talisman_{slug}_mask.png
  - Memorial: memorial_{slug}.png + memorial_{slug}_mask.png

Heightmaps are mid-gray jade plates with deep figurative carving (orthographic heightfield).
Masks are derived via `--mask-method auto` (see _talisman_art_common.py).

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python scripts/generate_talisman_art.py                    # missing only, all sets
    python scripts/generate_talisman_art.py --force            # regenerate everything
    python scripts/generate_talisman_art.py --set shop         # shop talismans only
    python scripts/generate_talisman_art.py --set memorial
    python scripts/generate_talisman_art.py --masks-only       # masks from existing heights
    python scripts/generate_talisman_art.py --name pinzu --set shop
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from io import BytesIO
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import DEFAULT_MODEL, generate_image_bytes, init_client, parse_size  # noqa: E402
from _talisman_art_common import (  # noqa: E402
    MEMORIAL_EXAGGERATE,
    SHOP_EXAGGERATE,
    postprocess_heightmap,
    strip_border_matte_frame,
    write_mask_from_height,
)

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

CARVING_BASE = (
    "Stone rubbing (拓本 taku-hon): orthographic top-down grayscale HEIGHTMAP of a "
    "jade relief carving on a square canvas. Tonal brightness equals carved height only — "
    "flat displacement field, no perspective, no directional lighting, no cast shadows, "
    "no drop shadow, no studio backdrop.\n\n"
    "Edge-to-edge jade plate: mid-gray (#808080) fills every pixel at the canvas border. "
    "The figurative carving reaches the image margins — petal tips, horns, coin rims, "
    "and wing tips shape the outer contour directly at the canvas edge. Mid-gray jade "
    "continues behind the subject all the way to each side. The subject silhouette is "
    "the only outer boundary; there is no black void, matte band, or vignette outside it.\n\n"
    "One clear figurative subject carved into the jade plane — not a separate figurine "
    "on a pedestal, base block, or circular medallion. Readable at thumbnail size. "
    "Asymmetric composition with the top of the carving toward +Y.\n\n"
    "Height tones: white peaks (#ffffff), light gray secondary planes (#e0–#f0), "
    "mid-gray ground (#808080), dark gray grooves and undercuts (#30–#50)."
)

CARVING_NEGATIVE = (
    "Do NOT draw: 3D clay/ZBrush render, photoreal product photo, centered statuette "
    "floating on dark gray, circular lotus or octagonal medallion frame, decorative "
    "bezel rim, bats, phoenixes, generic cloud-scroll filler, coin stacks, or any "
    "subject other than the Subject line below."
)

SHOP_STYLE = (
    f"{CARVING_BASE}\n\n"
    "Polished merchant jade, crisp deep undercut carving."
)

MEMORIAL_STYLE = (
    f"{CARVING_BASE}\n\n"
    "Worn temple-rubbing jade: shallower relief, matte finish, fine crackle grooves "
    "in the ground — still a flat heightfield, not a sculpted maquette."
)

MEMORIAL_NEGATIVE = (
    "Memorial set — do NOT reuse across kinds: ofuda paper fans, money pouches, "
    "three-legged toads, ox or bull heads, generic animal medallions, or identical "
    "vertical blade shapes unless the Subject line names them."
)

SHOP_MOTIFS: dict[str, str] = {
    "pearl": (
        "Moon rabbit (玉兔) crouched beside a raised pearl boss. Rabbit ears and haunch "
        "break the organic outline; pearl is the highest interior peak. Rabbit faces pearl; "
        "tail curl lower-left; ears and pearl boss upper-center."
    ),
    "gilded": (
        "Prosperity beetle (金龟) climbing a single curved sycee ingot. Domed carapace "
        "highest; legs grip the ingot bow; one ginkgo leaf under the ingot tip."
    ),
    "polychrome": (
        "Peacock in full fan display, tail feathers sweeping lower-right as the outer "
        "contour. Crest upper-left; body in profile; each eye-spot a shallow groove."
    ),
    "souzu": (
        "Bamboo culm with three nodes and a cicada (蝉) on the middle node, wings "
        "overlapping a leaf chevron. Left-heavy culm; cicada wing-tips break the right edge."
    ),
    "pinzu": (
        "Azure dragon (青龙) coiled around a bi disc void. Dragon head at top breaking "
        "past the ring; horns, claw, and tail outside the ring define the contour. "
        "Wave band at bottom."
    ),
    "manzu": (
        "Pixiu (貔貅) crouched winged-lion wealth beast holding a raised abstract "
        "wan-frame tablet (horizontal bar and two pillars) in forepaws. Horn, wing-tip, "
        "and haunch at the silhouette edge."
    ),
    "honors": (
        "Three mahjong honor dragon tile faces in a loose triangle — red dragon (abstract "
        "center-bar motif), green dragon (abstract fa-frame motif), white dragon (blank "
        "frame with pearl boss) — plus one east-wind tile roundel linked by curling wind "
        "bands. Tile corners and wind scroll tips break the contour. Abstract carved "
        "tile-glyph shapes only."
    ),
    "wildflower": (
        "Lotus bloom with five asymmetric scalloped petals and kingfisher (翠鸟) on "
        "stem, beak toward stamen. Deep undercut between petals; bird tail adds a right bulge."
    ),
    "conformity": (
        "Two mirror-image koi circling a central blank mahjong tile, bodies interlocked "
        "head-to-tail. Shared scale texture; open water between their arcs."
    ),
}

MEMORIAL_MOTIFS: dict[str, str] = {
    "exhausted": (
        "Silhouette: low horizontal kidney-blob, wider than tall (~1.35×).\n"
        "Sleeping ox curled on bare ground, head on forelegs, dull horns — NOT standing, "
        "NOT a bull-head emblem. Back arc defines the upper edge; one horn tip breaks "
        "the left contour. Shallow worn carving; no bowl, pouch, or paper strips."
    ),
    "frozen_hand": (
        "Silhouette: tall vertical S-curve (~1.25× height), narrow waist.\n"
        "Crane standing on one leg, one wing half-frozen with radiating ice fracture lines "
        "from the lifted talon — NOT a leaping fish, NOT wings fully spread. Neck crest "
        "breaks the top edge; jagged ice splinters the lower-right edge."
    ),
    "skipper": (
        "Silhouette: dynamic diagonal arc lower-left to upper-right, open gap under belly.\n"
        "Carp leaping through a stylized wave gate (鱼跃龙门), body arched mid-jump, "
        "whiskers forward, splash crest at tail — NOT a static fish, NOT a circular "
        "dragon gate medallion. Gate posts break the lower corners."
    ),
    "hoarder": (
        "Silhouette: lumpy asymmetric mound, magpie tail sweeping far left.\n"
        "Magpie (喜鹊) perched on a bulging silk money pouch, beak open — NOT ofuda strips, "
        "NOT a toad, NOT coins scattered around. Pouch drawstring knot lower-right; "
        "magpie wing-tip and tail feather break the left edge."
    ),
    "full_dish": (
        "Silhouette: wide elliptical bowl arc (~1.2× width), low profile.\n"
        "Three-legged money toad (金蟾) squatting on an elliptical offering bowl rim, "
        "one coin held in mouth — NOT a magpie, NOT a flat dish seen from above only. "
        "Toad dorsal bump, bowl rim, and coin rim break the contour."
    ),
    "discarded": (
        "Silhouette: diagonal butterfly wings spanning corner to corner.\n"
        "Butterfly with asymmetric wings over one fallen plum blossom and a half-buried "
        "blank mahjong tile — NOT a rectangular plaque base, NOT ofuda strips. Upper "
        "wing tip top-right; lower wing and tile corner lower-left."
    ),
    "boss_mark": (
        "Silhouette: single bent blade diagonal upper-right to lower-left.\n"
        "Bent jian sword (剑); crossguard bears taotie (饕餮) bronze mask with horns; "
        "small house seal square on the blade flat — NOT an ox or bull head, NOT a "
        "straight vertical sword, NOT paper talismans. Taotie horns and blade tip at edge."
    ),
    "buff_saint": (
        "Silhouette: vertical antler fan upper-center, body lower (~1.15× tall).\n"
        "Deer in profile with branching antlers and lingzhi fungus clusters at the hooves "
        "— NOT mushrooms on a separate base platform, NOT a fox. Antler tips and snout "
        "break the upper contour; worn memorial grooves on the flank."
    ),
    "transformer": (
        "Silhouette: fox head upper-left, wide tail-fan lower-right (~1.3× width).\n"
        "Nine-tailed fox (九尾狐) mid-transformation: one fox head, fan of tail tips "
        "showing bushy, scaled, and feathered forms — NOT a single bushy tail, NOT a "
        "phoenix. S-curve body links head to tail fan."
    ),
    "tag_bearer": (
        "Silhouette: radial paper fan from center nail, five rectangular strips — "
        "no animal, no egg, no tortoise.\n"
        "Five paper ofuda strips fanning from a central nail head; cloud-shaped cutouts "
        "along each strip edge. Strip corners at the silhouette perimeter — NOT a sword, "
        "NOT a bird, NOT cracked eggshell."
    ),
    "meld_mason": (
        "Silhouette: horizontal shelf with nest cup under a tile eave lip.\n"
        "Pair of swallows building a mud nest under a tile-shaped eave lip; one bird "
        "carries a reed strand — NOT ofuda strips, NOT a single large bird. Nest cup, "
        "wing arcs, and eave corner break the contour."
    ),
    "deep_walker": (
        "Silhouette: tall stacked terraces (~1.45× height), shell dome left.\n"
        "Adult xuanwu (玄武): tortoise with snake entwined climbing three rock terraces "
        "— NOT a hatchling, NOT an egg, NOT flat on one plane. Snake head highest; "
        "shell dome breaks left; footprint pairs on lowest terrace."
    ),
    "dead_on_arrival": (
        "Silhouette: oval eggshell dome upper arc, hatchling reaching lower-right — "
        "no paper strips, no adult tortoise, no snake.\n"
        "Young tortoise hatchling half-emerged from cracked egg shell, one front flipper "
        "reaching forward — NOT ofuda strips, NOT xuanwu, NOT a coin or bowl. Jagged "
        "shell arc across the top; egg shards at the lower edge."
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
                "exaggerate": SHOP_EXAGGERATE,
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
                "exaggerate": MEMORIAL_EXAGGERATE,
            }
        )
    return out


def build_prompt(entry: dict) -> str:
    parts = [
        entry["style"],
        "",
        f"Subject — carve only this figurative scene:\n{entry['motif']}",
        "",
        CARVING_NEGATIVE,
    ]
    if entry["prefix"] == "memorial":
        parts.extend(["", MEMORIAL_NEGATIVE])
    parts.extend(
        [
            "",
            "This subject is unique among all talismans; follow the Subject line exactly.",
        ]
    )
    return "\n".join(parts)


def write_mask_for_entry(entry: dict, *, mask_method: str = "auto") -> Path:
    mpath = mask_path(entry["prefix"], entry["slug"])
    hp = height_path(entry["prefix"], entry["slug"])
    if not write_mask_from_height(hp, mpath, method=mask_method):
        raise SystemExit(f"cannot write mask — missing heightmap {hp}")
    return mpath


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate talisman carving heightmaps and masks")
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
        help="Write organic masks from existing heightmaps (no API)",
    )
    parser.add_argument("--dry-run", action="store_true", help="Print prompts only")
    parser.add_argument("--list", action="store_true", help="List slugs and exit")
    parser.add_argument("--model", type=str, default=DEFAULT_MODEL)
    parser.add_argument("--size", type=str, default="1:1@1K")
    parser.add_argument("--out-size", type=int, default=OUT_SIZE)
    parser.add_argument("--sleep", type=float, default=2.0, help="Seconds between API calls")
    parser.add_argument(
        "--exaggerate-shop",
        type=float,
        default=SHOP_EXAGGERATE,
        help="Relief exaggeration for shop heightmaps",
    )
    parser.add_argument(
        "--repostprocess-only",
        action="store_true",
        help="Strip AI matte/frame bands from existing heightmaps and rewrite masks (no API)",
    )
    parser.add_argument(
        "--exaggerate-memorial",
        type=float,
        default=MEMORIAL_EXAGGERATE,
        help="Relief exaggeration for memorial heightmaps",
    )
    parser.add_argument(
        "--mask-method",
        choices=("auto", "luma", "flood", "rembg"),
        default="auto",
        help=(
            "Silhouette extraction: auto (flat=luma, sculpted=flood, else rembg), "
            "luma (legacy threshold), flood (border-connected bg), rembg (local u2net)"
        ),
    )
    args = parser.parse_args()

    entries: list[dict] = []
    if args.set in ("shop", "all"):
        entries.extend(load_shop_entries())
    if args.set in ("memorial", "all"):
        entries.extend(load_memorial_entries())

    for e in entries:
        if e["prefix"] == "talisman":
            e["exaggerate"] = args.exaggerate_shop
        else:
            e["exaggerate"] = args.exaggerate_memorial

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
            mpath = write_mask_for_entry(e, mask_method=args.mask_method)
            print(f"  mask {mpath.name}")
        print("Done (masks only).")
        return

    if args.repostprocess_only:
        for e in entries:
            hp = height_path(e["prefix"], e["slug"])
            if not hp.is_file():
                print(f"  skip {hp.name} (missing)")
                continue
            with Image.open(hp) as im:
                cleaned = strip_border_matte_frame(im.convert("L"))
            cleaned.save(hp, "PNG", optimize=True)
            write_mask_for_entry(e, mask_method=args.mask_method)
            print(f"  strip border {hp.name} + mask")
        print("Done (repostprocess).")
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
            cleaned = postprocess_heightmap(raw, args.out_size, entry["exaggerate"])
            cleaned.save(hp, "PNG", optimize=True)
            print(f"  wrote {hp.name}")
            if i + 1 < len(entries) and args.sleep > 0:
                time.sleep(args.sleep)
        elif not need_mask:
            print(f"  skip {hp.name} (exists)")
        else:
            print(f"  skip {hp.name} (exists, mask pending)")

        if need_mask or need_height:
            write_mask_for_entry(entry, mask_method=args.mask_method)
            print(f"  wrote {mp.name}")

    print("Done.")


if __name__ == "__main__":
    main()
