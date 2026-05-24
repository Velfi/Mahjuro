#!/usr/bin/env python3
"""
Generate single-image silk ribbon textures for the 16 zodiac consumable
cards in Mahjuro using Google's Nano Banana 2 (`gemini-3.1-flash-image-
preview`) API (Mouse + the 12 standard animals + Qilin, Phoenix, Crane).

Each zodiac is one tall portrait image rather than a 3-piece tile set.
The 3D ribbon mesh maps the texture full-bleed across its length, so the
visible finial and animal proportions are baked into the image itself.

    zodiac_<slug>.png        — full ribbon portrait, default 1:4@2K

Nano Banana 2 supports a 1:4 aspect ratio (the closest to the original
1:3 layout); the prompt describes a finial band plus embroidered animal
on plain silk, mapped full-bleed.

Style direction: full-bleed game texture of a silk ribbon — only the
ribbon surface appears in frame, with realistic traditional Chinese
court embroidery (Suzhou satin stitch, couched gold thread).

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python3 scripts/generate_zodiac_ribbons.py                  # all missing
    python3 scripts/generate_zodiac_ribbons.py --force          # regenerate all
    python3 scripts/generate_zodiac_ribbons.py --name dragon    # one by slug
    python3 scripts/generate_zodiac_ribbons.py --zodiac 5       # one by index
    python3 scripts/generate_zodiac_ribbons.py --list           # list all
    python3 scripts/generate_zodiac_ribbons.py --dry-run        # prompts only
"""

import argparse
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import (  # noqa: E402
    DEFAULT_MODEL,
    generate_image_bytes,
    init_client,
    parse_size,
)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "textures" / "zodiacs"

# ---------------------------------------------------------------------------
# Shared style
# ---------------------------------------------------------------------------

# Shared layout and embroidery craft for every zodiac. The renderer maps
# this full-bleed onto a tall ribbon mesh as a game texture.
STYLE_BASE = (
    "Video-game texture map for a tall portrait silk ribbon (~1:3). The "
    "entire image is the ribbon surface alone — every pixel is dyed silk "
    "or embroidery on silk, mapped full-bleed edge to edge. The banner is "
    "a solid silk rectangle with straight cut edges; all decoration is "
    "flat stitchwork on that surface.\n\n"
    "Embroidery: realistic traditional Chinese court needlework in the "
    "Suzhou tradition — hand-stitched on silk with visible thread "
    "strands and slight relief. Padded satin stitch raises the figures; "
    "long-and-short stitch follows anatomy and feather or fur direction; "
    "split-stitch outlines define edges; couched Japanese gold thread "
    "supplies metallic highlights; dense stitch areas gently pucker the "
    "silk ground. Straight-on surface detail with soft raking daylight "
    "from upper-left so gold reads as metal and individual stitches "
    "throw fine shadows.\n\n"
    "Layout (top → bottom):\n"
    "Top ~15%: goldwork rosette finial stitched flat into the silk, "
    "starting at the top cut edge of the banner.\n"
    "Below: broad dyed silk ground with one embroidered animal as the "
    "centerpiece, plain silk continuing to the bottom cut edge.\n\n"
    "Purely pictorial decoration on silk."
)


# ---------------------------------------------------------------------------
# Zodiac definitions
# ---------------------------------------------------------------------------

# Each tuple: (slug, display_name, animal_visual, silk_palette).
# Order MUST match ZodiacKind::all() in src/core/zodiac.rs (calendar order:
# Mouse … Pig, then Qilin, Phoenix, Crane — 16 total). Silk colors are
# creature-appropriate.
ZODIACS = [
    (
        "mouse",
        "Mouse",
        "Small field mouse in three-quarter view, seated on haunches with "
        "forepaws together, oversized rounded ears, slim curved tail, "
        "delicate miniature scale occupying the middle third of the panel "
        "with open silk above and below.",
        "Warm dusty-grey silk (#b0a89e) with gold embroidery.",
    ),
    (
        "rat",
        "Rat",
        "Heavy stocky rat in strict profile on all fours, arched back, "
        "blunt muzzle, long thick rope-like tail, dominant figure filling "
        "most of the panel width.",
        "Dark charcoal silk (#4a4a50) with gold embroidery.",
    ),
    (
        "ox",
        "Ox",
        "Broad-shouldered ox in formal three-quarter pose, head lowered, "
        "thick curved horns, heavy dewlap, patterned yoke across the neck.",
        "Deep earthen-brown silk (#7a5c3a) with gold embroidery.",
    ),
    (
        "tiger",
        "Tiger",
        "Tiger in a crouched stalking pose, body low and elongated, head "
        "forward, long tail curving up behind, bold gold stripes.",
        "Burnt-orange silk (#d4792a) with gold embroidery.",
    ),
    (
        "rabbit",
        "Rabbit",
        "Rabbit seated upright in profile, long erect ears, forepaws tucked "
        "at the chest, small couched-gold crescent moon above the head.",
        "Soft white silk (#f0ece4) with gold embroidery.",
    ),
    (
        "dragon",
        "Dragon",
        "Four-clawed Chinese dragon in a vertical S-curve, head turned "
        "three-quarter forward, open mouth with forked tongue, swept-back "
        "horns, mane along the spine, auspicious cloud curl at the tail, "
        "elaborate individual scales in goldwork.",
        "Imperial crimson silk (#b5262e) with gold embroidery.",
    ),
    (
        "snake",
        "Snake",
        "Snake coiled twice into a tall vertical spiral, head rising at "
        "the top with tongue flicked out, diamond-scale lattice along the "
        "body.",
        "Deep jade-green silk (#2e7d4f) with gold embroidery.",
    ),
    (
        "horse",
        "Horse",
        "Horse in mid-gallop, strict profile, all four legs lifted in the "
        "traditional flying-gallop pose, mane and tail streaming behind, "
        "couched-gold harness and bridle.",
        "Rich chestnut silk (#8b4513) with gold embroidery.",
    ),
    (
        "goat",
        "Goat",
        "Long-haired ram standing in profile, heavy spiral-curled horns, "
        "tufted beard, small ling-zhi sprig beside the body.",
        "Creamy wool-white silk (#ede5d0) with gold embroidery.",
    ),
    (
        "monkey",
        "Monkey",
        "Monkey crouched seated, one hand raised holding a small round "
        "peach, long tail curving behind the body.",
        "Warm tawny-gold silk (#c8a04a) with gold embroidery.",
    ),
    (
        "rooster",
        "Rooster",
        "Standing rooster in profile, chest forward, tall serrated comb "
        "and wattle, long arching tail of layered sickle feathers.",
        "Scarlet-red silk (#c23028) with gold embroidery.",
    ),
    (
        "dog",
        "Dog",
        "Dog seated upright in three-quarter view, one ear erect and one "
        "folded, thin collar with a small spherical bell at the throat.",
        "Warm sandy-tan silk (#c4a672) with gold embroidery.",
    ),
    (
        "pig",
        "Pig",
        "Pig in strict profile, rounded broad body, short legs, upturned "
        "snout, floppy ears, curled tail.",
        "Rosy pink silk (#e8a0b4) with gold embroidery.",
    ),
    (
        "qilin",
        "Qilin",
        "Qilin in formal three-quarter pose: cloven deer legs, dragon-scaled "
        "flanks, flowing leonine mane and tufted tail, paired antlers, "
        "small auspicious cloud curls around the body, elaborate goldwork.",
        "Deep twilight-violet silk (#3a2f55) with gold embroidery.",
    ),
    (
        "phoenix",
        "Phoenix",
        "Fenghuang in a rising pose, elongated crest and neck, wings "
        "half-spread, long tail feathers fanning downward in layered arcs, "
        "small flaming pearl motif near the breast.",
        "Crimson-gold silk (#c45a2a) with gold embroidery.",
    ),
    (
        "crane",
        "Crane",
        "Red-crowned crane standing on one leg in profile, neck curved "
        "in an S, wings folded, red crown patch, gold legs and beak.",
        "Pale sky-blue silk (#a8c8e0) with gold embroidery.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(visual: str, palette: str) -> str:
    return "\n\n".join(
        [
            STYLE_BASE,
            f"Embroidered subject: {visual}",
            f"Silk ground: {palette}",
        ]
    )


# ---------------------------------------------------------------------------
# Image generation
# ---------------------------------------------------------------------------

def generate_image(
    client, prompt: str, output_path: Path, model: str, size: str
) -> None:
    """Call Gemini Nano Banana 2 and save the resulting PNG."""
    aspect_ratio, image_size = parse_size(size)
    img_bytes = generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio=aspect_ratio,
        image_size=image_size,
    )
    output_path.write_bytes(img_bytes)
    print(f"  Saved: {output_path}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro zodiac ribbon textures (single tall "
        "portrait per zodiac) via Google Nano Banana 2"
    )
    parser.add_argument(
        "--zodiac",
        type=int,
        default=None,
        help="Generate only zodiac number N (1-indexed, calendar order).",
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Generate only the zodiac with this filename slug (e.g. dragon).",
    )
    parser.add_argument(
        "--list", action="store_true", help="List all zodiacs and exit."
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if the output file already exists.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1:4@2K",
        # Nano Banana 2 supports 1:4 (closest to the legacy 1024x3072 ≈ 1:3
        # portrait). 2K keeps the long edge sharp without wasting tokens.
        help="Generation size — Gemini ASPECT@TIER (default: 1:4@2K).",
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds to sleep between API calls (default: 2.0).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(ZODIACS, 1):
            print(f"  {i:2d}. {name:<10s}  zodiac_{slug}.png")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.zodiac is not None and args.name is not None:
        print("Error: pass --zodiac OR --name, not both.")
        sys.exit(1)

    if args.zodiac is not None:
        if args.zodiac < 1 or args.zodiac > len(ZODIACS):
            print(f"Error: --zodiac must be between 1 and {len(ZODIACS)}")
            sys.exit(1)
        targets = [(args.zodiac - 1, ZODIACS[args.zodiac - 1])]
    elif args.name is not None:
        match = next(
            ((i, z) for i, z in enumerate(ZODIACS) if z[0] == args.name), None
        )
        if match is None:
            print(f"Error: no zodiac with slug '{args.name}'. Try --list.")
            sys.exit(1)
        targets = [match]
    else:
        targets = list(enumerate(ZODIACS))

    client = None
    if not args.dry_run:
        client = init_client()

    generated = 0
    skipped = 0
    failed = 0
    total_jobs = len(targets)

    for job, (_idx, (slug, name, visual, palette)) in enumerate(targets, 1):
        output_path = out_dir / f"zodiac_{slug}.png"
        prompt = build_prompt(visual, palette)

        print(f"\n[{job}/{total_jobs}] {name}")

        if args.dry_run:
            print(f"  Output: {output_path.name}")
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists() and not args.force:
            print(
                f"  Skipping (exists): {output_path.name}"
                "  — use --force to regenerate"
            )
            skipped += 1
            continue

        try:
            assert client is not None
            generate_image(client, prompt, output_path, args.model, args.size)
            generated += 1
        except Exception as e:
            print(f"  Error generating {name}: {e}")
            failed += 1
            continue

        if job < total_jobs:
            time.sleep(args.delay)

    print("\nDone.")
    if not args.dry_run:
        print(
            f"  generated={generated}  skipped={skipped}  failed={failed}"
            f"  → {out_dir}"
        )


if __name__ == "__main__":
    main()
