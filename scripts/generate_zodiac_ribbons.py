#!/usr/bin/env python3
"""
Generate single-image washi-paper ribbon textures for the 16 zodiac
consumable cards in Mahjuro using Google's Nano Banana 2
(`gemini-3.1-flash-image-preview`) API (the 12 standard animals
+ Mouse, Qilin, Phoenix, Crane).

The 3D ribbon mesh maps the texture full-bleed across its length, so the
visible finial and animal proportions are baked into the image itself.

    zodiac_<slug>.png        — full ribbon portrait, default 1:4@2K

Nano Banana 2 supports a 1:4 aspect ratio (the closest to the original
1:3 layout); the prompt describes a finial band plus hair-embroidered
animal on plain colored washi, mapped full-bleed.

Style direction: full-bleed game texture of a plain colored washi-paper
ribbon — only the ribbon surface appears in frame, with fine traditional
Chinese hair embroidery (发绣 faxiu).

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
    "UV texture map for a tall portrait washi-paper ribbon (~1:4) — not a "
    "photograph of a ribbon on a background. The image IS the flat ribbon "
    "surface: colored washi paper must fill every pixel from edge to edge. "
    "No white border, no margin, no empty canvas, no deckled-edge halo, no "
    "centered ribbon inset on a larger field. The left, right, top, and "
    "bottom image borders are the ribbon's straight cut sides.\n\n"
    "Plain dyed washi with visible fiber texture; traditional Chinese hair "
    "embroidery (发绣 faxiu) in glossy human-hair thread — hand-stitched "
    "needlework on washi, NOT a photograph, NOT digital painting, NOT "
    "photorealistic fur or feathers. Visible parallel hair-thread rows with "
    "slight relief; long-and-short stitch follows fur or feather direction; "
    "split-stitch outlines edges; flat color regions filled by dense thread "
    "(no airbrush gradients, no depth-of-field, no glossy wet eyes). Soft "
    "raking daylight from upper-left so individual stitches throw fine "
    "shadows.\n\n"
    "Layout: small embroidered rosette finial at the top cut edge; one "
    "embroidered animal below on plain washi to the bottom cut edge."
)


STYLE_REF_SUFFIX = (
    "\n\nStyle reference: match the visible hair-thread stitch craft, "
    "relief, and folk-art pictorial quality of the attached reference "
    "ribbon — NOT the reference animal subject."
)



# Each tuple: (slug, display_name, animal_visual, ribbon_color, hair_threads).
# Order MUST match ZodiacKind::all() in src/core/zodiac.rs (calendar order:
# Mouse … Pig, then Qilin, Phoenix, Crane — 16 total). Every zodiac gets a
# unique washi ribbon color and a distinct hair-thread palette.
ZODIACS = [
    (
        "mouse",
        "Mouse",
        "Small field mouse in three-quarter view, seated on haunches with "
        "forepaws together, oversized rounded ears, slim curved tail, "
        "delicate miniature scale occupying the middle third of the panel "
        "with open washi above and below.",
        "Warm dusty-grey washi paper (#b0a89e).",
        "Mouse embroidered in soft grey-brown and charcoal human hair.",
    ),
    (
        "rat",
        "Rat",
        "Heavy stocky rat in strict profile on all fours, arched back, "
        "blunt muzzle, long thick rope-like tail, dominant figure filling "
        "most of the panel width.",
        "Dark charcoal washi paper (#4a4a50).",
        "Rat embroidered in glossy black and deep brown human hair.",
    ),
    (
        "ox",
        "Ox",
        "Broad-shouldered ox in formal three-quarter pose, head lowered, "
        "thick curved horns, heavy dewlap, patterned yoke across the neck.",
        "Deep earthen-brown washi paper (#7a5c3a).",
        "Ox embroidered in dark umber and warm chestnut human hair.",
    ),
    (
        "tiger",
        "Tiger",
        "Tiger in a crouched stalking pose, body low and elongated, head "
        "forward, long tail curving up behind, bold dark stripes over a "
        "lighter tawny coat.",
        "Burnt-orange washi paper (#d4792a).",
        "Tiger embroidered in black-stripe and golden-tawny human hair.",
    ),
    (
        "rabbit",
        "Rabbit",
        "Rabbit seated upright in profile, long erect ears, forepaws tucked "
        "at the chest, small pale crescent moon above the head.",
        "Soft ivory washi paper (#f0ece4).",
        "Rabbit embroidered in warm brown and soft grey human hair.",
    ),
    (
        "dragon",
        "Dragon",
        "Four-clawed Chinese dragon in a vertical S-curve, head turned "
        "three-quarter forward, open mouth with forked tongue, swept-back "
        "horns, mane along the spine, auspicious cloud curl at the tail, "
        "elaborate individual scales in fine hair stitch.",
        "Imperial crimson washi paper (#b5262e).",
        "Dragon embroidered in jet black, deep auburn, and bronze-brown "
        "human hair.",
    ),
    (
        "snake",
        "Snake",
        "Snake coiled twice into a tall vertical spiral, head rising at "
        "the top with tongue flicked out, diamond-scale lattice along the "
        "body.",
        "Deep jade-green washi paper (#2e7d4f).",
        "Snake embroidered in black, olive-brown, and pale grey-green "
        "human hair.",
    ),
    (
        "horse",
        "Horse",
        "Horse rearing rampant in strict profile for a tall vertical ribbon — "
        "front legs lifted high, hind legs planted, neck arched upward, mane "
        "and tail streaming vertically; simple hair-stitched harness and "
        "bridle. Dominant figure spanning most of the panel height from just "
        "below the finial to the lower washi edge, filling the narrow width.",
        "Rich chestnut washi paper (#8b4513).",
        "Horse embroidered in black mane, dark brown body, and warm "
        "mahogany human hair.",
    ),
    (
        "goat",
        "Goat",
        "Mountain goat climbing a steep rocky ledge in strict profile — "
        "compact stocky body, short pale coat, backward-curving black horns, "
        "small beard, sure-footed on jagged stone; natural caprine proportions "
        "(not llama-like). Enlarged dominant figure spanning most of the panel "
        "height from just below the finial to the lower washi edge.",
        "Creamy wool-white washi paper (#ede5d0).",
        "Mountain goat embroidered in pale cream fleece, black horns and "
        "hooves, and warm grey-brown rock human hair.",
    ),
    (
        "monkey",
        "Monkey",
        "Monkey crouched seated, one hand raised holding a small round "
        "peach, long tail curving behind the body.",
        "Warm tawny-gold washi paper (#c8a04a).",
        "Monkey embroidered in chocolate brown, russet, and black human "
        "hair.",
    ),
    (
        "rooster",
        "Rooster",
        "Standing rooster in profile, chest forward, tall serrated comb "
        "and wattle, long arching tail of layered sickle feathers.",
        "Scarlet-red washi paper (#c23028).",
        "Rooster embroidered in glossy black tail, warm brown body, and "
        "deep red-brown comb human hair.",
    ),
    (
        "dog",
        "Dog",
        "Folk-art dog seated upright in three-quarter view, one ear "
        "erect and one folded, thin cord collar with a small bell — stylized "
        "silhouette built from visible faxiu hair-thread rows and "
        "split-stitch outlines, long-and-short stitch bands for the coat "
        "(no photographic fur, no breed-portrait realism, no wet-nose shine, "
        "matte split-stitch eyes). Modest figure occupying the lower half of "
        "the panel with open washi above.",
        "Warm sandy-tan washi paper (#c4a672).",
        "Dog embroidered in dark brown, black, and warm tan human hair only.",
    ),
    (
        "pig",
        "Pig",
        "Pig in strict profile, rounded broad body, short legs, upturned "
        "snout, floppy ears, curled tail.",
        "Rosy pink washi paper (#e8a0b4).",
        "Pig embroidered in deep brown outline and warm pink-brown human "
        "hair.",
    ),
    (
        "qilin",
        "Qilin",
        "Qilin in formal three-quarter pose: cloven deer legs, dragon-scaled "
        "flanks, flowing leonine mane and tufted tail, paired antlers, "
        "small auspicious cloud curls around the body, fine hair-stitched "
        "scale detail.",
        "Deep twilight-violet washi paper (#3a2f55).",
        "Qilin embroidered in silver-grey, black, and pale lavender human "
        "hair.",
    ),
    (
        "phoenix",
        "Phoenix",
        "Fenghuang in a rising pose, elongated crest and neck, wings "
        "half-spread, long tail feathers fanning downward in layered arcs, "
        "small flaming pearl motif near the breast.",
        "Crimson-gold washi paper (#c45a2a).",
        "Phoenix embroidered in black, vermilion-red, and golden-brown "
        "human hair.",
    ),
    (
        "crane",
        "Crane",
        "Red-crowned crane standing on one leg in profile, neck curved "
        "in an S, wings folded, red crown patch, dark legs and beak.",
        "Pale sky-blue washi paper (#a8c8e0).",
        "Crane embroidered in white body, black wing tips, and vermilion "
        "crown human hair.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(visual: str, ribbon_color: str, hair_threads: str) -> str:
    return "\n\n".join(
        [
            STYLE_BASE,
            f"Ribbon color (plain washi ground, edge to edge): {ribbon_color}",
            f"Hair embroidery threads: {hair_threads}",
            f"Embroidered subject: {visual}",
        ]
    )


# ---------------------------------------------------------------------------
# Image generation
# ---------------------------------------------------------------------------

def generate_image(
    client,
    prompt: str,
    output_path: Path,
    model: str,
    size: str,
    style_ref: Path | None = None,
) -> None:
    """Call Gemini Nano Banana 2 and save the resulting PNG."""
    aspect_ratio, image_size = parse_size(size)
    refs: list[Path] = []
    full_prompt = prompt
    if style_ref is not None:
        refs = [style_ref]
        full_prompt = prompt + STYLE_REF_SUFFIX
    img_bytes = generate_image_bytes(
        client,
        full_prompt,
        model=model,
        aspect_ratio=aspect_ratio,
        image_size=image_size,
        refs=refs,
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
        "--style-ref",
        type=str,
        default=None,
        help="Optional reference ribbon PNG whose embroidery craft to match.",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds to sleep between API calls (default: 2.0).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, ribbon_color, _) in enumerate(ZODIACS, 1):
            print(f"  {i:2d}. {name:<10s}  zodiac_{slug}.png  {ribbon_color}")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)
    style_ref = Path(args.style_ref) if args.style_ref else None
    if style_ref is not None and not style_ref.is_file():
        print(f"Error: style reference not found: {style_ref}")
        sys.exit(1)

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

    for job, (_idx, (slug, name, visual, ribbon_color, hair_threads)) in enumerate(
        targets, 1
    ):
        output_path = out_dir / f"zodiac_{slug}.png"
        prompt = build_prompt(visual, ribbon_color, hair_threads)

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
            generate_image(
                client, prompt, output_path, args.model, args.size, style_ref
            )
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
