#!/usr/bin/env python3
"""
Generate single-image silk ribbon textures for the 14 zodiac consumable
cards in Mahjuro using Google's Nano Banana 2 (`gemini-3.1-flash-image-
preview`) API (Mouse + the 12 standard animals + Qilin for Kokushi Musō).

Each zodiac is one tall portrait image rather than a 3-piece tile set.
The 3D ribbon mesh maps the texture full-bleed across its length, so the
visible animal/finial/tassel proportions are baked into the image itself.

    zodiac_<slug>.png        — full ribbon portrait, default 1:4@2K

Nano Banana 2 supports a 1:4 aspect ratio (the closest to the original
1:3 layout); the prompt still describes a 3-band finial / silk / tassel
composition that the renderer maps full-bleed.

Style direction: "Walnut, Brass & Felt" — woven silk banners hanging in
a curio shop. Each ribbon is its own per-zodiac silk color with the
zodiac animal embroidered in metallic brass thread, plus subtle brass
trim along the long edges.

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

# Single combined prompt template. Framed as a museum-archive photograph
# of a real antique silk banner, with named embroidery techniques, so the
# model treats it as photoreal documentation rather than game art. The
# renderer maps this image full-bleed onto a tall ribbon mesh, so the
# silk fills the entire frame and the decorative finial / tasselled tip
# are stitched *into* the silk rectangle (no transparency, no real cuts).
STYLE_BASE = (
    "Sharp macro photograph of an antique Chinese imperial-court silk "
    "banner laid flat in a museum archive, photographed straight-on for "
    "documentation. Tall portrait frame (~1:3) filled edge-to-edge with "
    "the silk banner — the silk reaches all four borders and the banner "
    "IS the entire image, no surrounding mount, no shadow, no vignette.\n\n"
    "Banner construction (top → bottom along the length):\n"
    "1. Top ~15% — embroidered finial. A stylised gold-thread rosette "
    "knot stitched directly into the silk, with a small fabric loop above "
    "it where the banner would hang from a wooden rod. The rosette is "
    "rendered in goldwork: parallel rows of laid Japanese gold-wrapped "
    "thread couched down with fine red silk stitches at regular intervals, "
    "highlights running along each gold strand.\n"
    "2. Middle ~70% — broad expanse of dyed silk ground in the per-zodiac "
    "color, with a single embroidered animal as the centerpiece, occupying "
    "the central two-thirds of this band. The animal is worked in the "
    "Suzhou tradition: the body is built from padded satin stitch raised "
    "slightly off the silk; long-and-short stitch fills the form with "
    "directional silk and metallic floss following the anatomy (so light "
    "catches the thread differently along the curves of muscle and "
    "feather/scale/fur); a finer split-stitch line defines the outer "
    "edge; tiny French knots and short bullion knots pick out the eye and "
    "smallest features. Real distinguishable thread strands are visible "
    "throughout, with subtle dimpling of the silk ground around the "
    "densest stitch areas. Pose and proportions follow traditional Chinese "
    "decorative-arts convention for that creature; the creature occupies "
    "its space the way it does on an imperial robe panel — formal, "
    "front-facing or three-quarter, treated as ornament rather than "
    "portrait. Plain silk breathes above and below the figure. Up to a "
    "few small auspicious motifs (couched-gold cloud curls, dotted bats, "
    "or geometric flourishes) may sit sparingly in the surrounding silk.\n"
    "3. Bottom ~15% — embroidered tapered tip. The banner's lower border "
    "is shaped as a downward V-notch or scalloped point indicated by a "
    "couched-gold edging that frames a slightly darker hem panel; below "
    "the tip, fine individually-rendered gold fringe threads hang straight "
    "down. One small stitched motif (a knot, curl, teardrop, or "
    "auspicious cloud) sits centered just above the fringe.\n\n"
    "Material and surface: dyed mulberry silk with a tight visible weave "
    "grain running top-to-bottom and a soft satin sheen. A narrow couched "
    "gold-thread border, two strands wide, runs down each long edge of "
    "the banner from top to bottom. Where embroidery is dense, the silk "
    "ground shows the characteristic puckering and shadow of real "
    "stitchwork.\n\n"
    "Lighting: even soft museum daylight from upper-left, raking gently "
    "across the surface so the metallic gold thread reads as actual metal "
    "and individual stitches throw fine micro-shadows. Photoreal, "
    "documentary museum-archive style. Decoration is purely pictorial — "
    "the silk carries no glyphs, numerals, or logos of any language."
)


# ---------------------------------------------------------------------------
# Zodiac definitions
# ---------------------------------------------------------------------------

# Each tuple: (slug, display_name, animal_visual, silk_palette).
# Order MUST match ZodiacKind::all() in src/core/zodiac.rs (calendar order:
# Mouse … Pig, then Qilin — 14 total). Silk colors are creature-appropriate.
ZODIACS = [
    (
        "mouse",
        "Mouse",
        "A small field mouse rendered in three-quarter view, body "
        "compact and rounded, seated on its haunches with both forepaws "
        "together as if holding a small object close to its chest. Large "
        "disproportionately rounded ears (mouse-sized, oversized relative "
        "to the head), a slim hair-thin tail curving in a single arc "
        "behind the body, individually stitched whiskers radiating from "
        "a tiny pointed muzzle. The fur is built up in long-and-short "
        "stitch with directional flow from spine to belly; the ears use "
        "padded satin in two tones of gold; the eye is a single small "
        "French knot. SCALE: the embroidered figure is intentionally "
        "small in the central panel, occupying only the middle third of "
        "the panel's width and a little less than half its height, "
        "surrounded by a generous expanse of unworked silk. Read as a "
        "delicate miniature, not a heraldic centerpiece.",
        "Warm dusty-grey silk (#b0a89e) with gold embroidery and gold edge trim.",
    ),
    (
        "rat",
        "Rat",
        "A heavy-bodied rat in strict profile, on all fours with the "
        "back arched and shoulders raised, body thick and stocky, head "
        "broad with a blunt muzzle. A long thick rope-like tapering tail "
        "extends behind in a single curve, drawn with the heft of a real "
        "rat tail rather than a thread. Small rounded ears set close to "
        "the skull, eye a single bullion knot. The figure is "
        "anatomically rendered in long-and-short stitch with directional "
        "flow following the musculature across the flanks, padded satin "
        "raises the shoulder hump and haunch, split-stitch outline runs "
        "along the back and tail, whiskers as individual couched gold "
        "filaments. SCALE: the embroidered figure is the dominant element "
        "of the central panel, deliberately rendered roughly twice the "
        "linear scale of the Mouse banner — the body alone fills most of "
        "the panel's width and the tail extends nearly to the panel edge. "
        "Read as a substantial, weighty creature, clearly a different "
        "and larger animal than a mouse.",
        "Dark charcoal silk (#4a4a50) with gold embroidery and gold edge trim.",
    ),
    (
        "ox",
        "Ox",
        "A broad-shouldered ox in formal three-quarter pose, head lowered, "
        "thick curved horns, heavy dewlap, a patterned yoke draped across "
        "the neck. Body filled with dense padded satin stitch raising the "
        "musculature off the silk; horns worked in laid-and-couched gold "
        "with parallel ridges; the yoke detail in alternating couched gold "
        "and split stitch.",
        "Deep earthen-brown silk (#7a5c3a) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "tiger",
        "Tiger",
        "A tiger in a crouched stalking pose, body low and elongated, "
        "head forward, long tail curving up behind. Stripes worked in a "
        "darker tone of gold thread laid perpendicular to the body's "
        "long-and-short stitching, so the stripes catch light against "
        "the surrounding fill. Padded satin builds the shoulder and "
        "haunch volume; split-stitch outlines define the silhouette and "
        "stripe edges; small French knots at the eye and nostril.",
        "Burnt-orange silk (#d4792a) with gold embroidery and gold edge trim.",
    ),
    (
        "rabbit",
        "Rabbit",
        "A rabbit seated upright in profile, long ears erect with one "
        "leaning slightly forward, forepaws tucked at the chest, hind "
        "legs folded. A small crescent moon motif worked in couched gold "
        "floats just above the head. Body in long-and-short stitch with "
        "fine directional flow; ears in padded satin with a paler silk "
        "interior; eye a single French knot.",
        "Soft white silk (#f0ece4) with gold embroidery and gold edge trim.",
    ),
    (
        "dragon",
        "Dragon",
        "A four-clawed Chinese dragon in a vertical S-curve along the "
        "length of the silk, head turned three-quarter facing slightly "
        "forward, mouth open showing fangs and a flicked forked tongue, "
        "long whiskers trailing, antler-like horns swept back, mane and "
        "fin-fringe along the spine. A single auspicious cloud curl at "
        "the tail. The most elaborate piece in the set: scales worked "
        "individually in alternating tones of gold and the silk's own "
        "color, body built up with heavy padded goldwork (laid Japanese "
        "gold thread couched in red silk), claws and teeth picked out in "
        "split stitch, eyes accented with bullion-knot pupils.",
        "Imperial crimson silk (#b5262e) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "snake",
        "Snake",
        "A snake coiled twice into a tall vertical spiral, head rising "
        "at the top of the spiral with the tongue flicked out. The dorsal "
        "scales form a regular diamond lattice worked in a darker shade "
        "of gold, set against long-and-short stitched body fill that "
        "follows the curl of each coil; padded satin raises the head; a "
        "small bullion knot marks the eye.",
        "Deep jade-green silk (#2e7d4f) with gold embroidery and gold edge trim.",
    ),
    (
        "horse",
        "Horse",
        "A horse in mid-gallop in strict profile, all four legs lifted "
        "from the ground in the traditional flying-gallop pose, mane and "
        "tail streaming horizontally behind. Body in long-and-short stitch "
        "following the musculature; mane and tail in long laid silk "
        "strands; harness and bridle indicated in couched gold; hooves "
        "in padded satin.",
        "Rich chestnut silk (#8b4513) with gold embroidery and gold edge trim.",
    ),
    (
        "goat",
        "Goat",
        "A long-haired ram (yang) standing in profile, body squared, "
        "with heavy spiral-curled horns, a tufted beard hanging from the "
        "chin, and a short tail. A small ling-zhi sprig or flowering "
        "branch tucked beside the body. Coat worked in long-and-short "
        "stitch with the wool texture suggested by short overlapping "
        "stitches; horns in laid-and-couched gold with fine concentric "
        "ridges; hooves in split-stitch outline.",
        "Creamy wool-white silk (#ede5d0) with gold embroidery and gold edge trim.",
    ),
    (
        "monkey",
        "Monkey",
        "A monkey in a crouched seated pose, one hand raised holding a "
        "small round peach worked in a contrasting tone, the other hand "
        "resting on the knee, long tail curving down and around behind "
        "the body. Anatomy rendered in long-and-short stitch; face and "
        "ears in padded satin with split-stitch outline; the peach in "
        "padded satin with a stitched leaf in green silk; eyes as small "
        "French knots.",
        "Warm tawny-gold silk (#c8a04a) with gold embroidery and gold edge trim.",
    ),
    (
        "rooster",
        "Rooster",
        "A standing rooster in profile, chest forward, tall serrated comb "
        "and wattle, long arching tail of layered sickle feathers. Each "
        "tail feather worked individually in laid-and-couched gold, "
        "graduating in length; body plumage in long-and-short stitch with "
        "directional flow; comb and wattle in dense padded satin; legs "
        "and spurs in couched gold.",
        "Scarlet-red silk (#c23028) with gold embroidery and gold edge trim.",
    ),
    (
        "dog",
        "Dog",
        "A dog seated upright in three-quarter view, one ear erect and "
        "one folded, a thin collar with a small spherical bell at the "
        "throat. Coat in long-and-short stitch with fur direction "
        "fanning out from the spine; muzzle in padded satin; collar and "
        "bell in couched gold; eyes as small French knots; tongue not "
        "shown.",
        "Warm sandy-tan silk (#c4a672) with gold embroidery and gold edge trim.",
    ),
    (
        "pig",
        "Pig",
        "A pig in strict profile, body rounded and broad, short legs, "
        "small upturned snout, floppy ears, a curled tail at the rear. "
        "Body filled with long-and-short stitch in a single tone of "
        "gold, ears and snout in padded satin with split-stitch outline; "
        "the curled tail and trotters in couched gold.",
        "Rosy pink silk (#e8a0b4) with gold embroidery and gold edge trim.",
    ),
    (
        "qilin",
        "Qilin",
        "A qilin (Chinese kirin) standing in formal three-quarter pose: "
        "cloven-hoofed deer legs, dragon-scaled flanks, a flowing leonine "
        "mane and tufted tail, delicate paired antlers, small auspicious "
        "cloud curls drifting around the body. The most elaborate piece "
        "in the set, treated as imperial-rank goldwork: scales worked "
        "individually in alternating tones, mane and tail in long laid "
        "silk floss, antlers in laid-and-couched gold, eyes accented "
        "with bullion knots, supporting cloud curls couched in fine "
        "filigree-like gold.",
        "Deep twilight-violet silk (#3a2f55) with brilliant gold embroidery "
        "and gold edge trim.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(visual: str, palette: str) -> str:
    subject = (
        "The embroidered subject in the middle panel of the banner is "
        "rendered as follows. " + visual
    )
    return "\n\n".join([STYLE_BASE, subject, f"Silk ground color: {palette}"])


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
