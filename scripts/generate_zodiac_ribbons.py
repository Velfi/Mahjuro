#!/usr/bin/env python3
"""
Generate three-piece silk-ribbon textures for the 13 zodiac consumable
cards in Mahjuro using OpenAI's image generation API (Mouse + the 12
standard).

Each zodiac ribbon is split into three square tiles so that the ribbon
mesh can stretch to any length by tiling the middle piece:

    zodiac_<slug>_top.png    — decorative top cap (tassel knot / finial)
    zodiac_<slug>_mid.png    — repeating middle with the zodiac animal
    zodiac_<slug>_bot.png    — decorative bottom cap (tassel fringe)

The top and bottom edges of _mid tile seamlessly with themselves (and
with the caps), so the renderer can repeat _mid N times between the two
caps for ribbons of any length.

Style direction: "Midnight Gold" — woven silk banners hanging in a curio
shop. Each ribbon is its own per-zodiac silk color with the zodiac
animal embroidered in metallic gold thread, plus subtle gold trim along
the long edges. No background — the silk IS the background.

Usage:
    pip install openai pillow requests
    export OPENAI_API_KEY="sk-..."
    python3 scripts/generate_zodiac_ribbons.py                  # all missing
    python3 scripts/generate_zodiac_ribbons.py --force          # regenerate all
    python3 scripts/generate_zodiac_ribbons.py --name dragon    # one by slug
    python3 scripts/generate_zodiac_ribbons.py --zodiac 5       # one by index
    python3 scripts/generate_zodiac_ribbons.py --piece mid      # only middles
    python3 scripts/generate_zodiac_ribbons.py --list           # list all
    python3 scripts/generate_zodiac_ribbons.py --dry-run        # prompts only
"""

import argparse
import base64
import os
import sys
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "textures"

# ---------------------------------------------------------------------------
# Shared style constants
# ---------------------------------------------------------------------------

# Base style shared by all three pieces. The silk fabric fills the entire
# frame edge-to-edge; any background bleed would show as a halo on the 3D
# ribbon mesh. The three pieces are generated in independent API calls, so
# the per-piece styles below cannot reference "the other pieces" — they
# must state absolute constraints (e.g. exact silk color and weave
# direction at the joining edge) that each piece can satisfy on its own.
STYLE_BASE = (
    "Texture tile for a hanging silk ribbon in a 'Midnight Gold' mahjong "
    "roguelite video game. The image is a square filled edge-to-edge with "
    "woven silk fabric — the silk reaches all four borders of the frame, "
    "and the silk IS the whole image (no surrounding scene, no mount, no "
    "shadow, no vignette). The texture will be wrapped onto a 3D ribbon "
    "mesh, so the entire frame must be the ribbon itself. "
    "The silk has a subtle vertical weave grain (weave threads run top-to-"
    "bottom) and a soft satin sheen, with a thin metallic gold trim braid "
    "running down each long (left and right) edge. "
    "Flat orthographic front-on view, purely pictorial decoration only — "
    "no written glyphs, numerals, or logos of any language on the silk."
)

# Per-piece style suffixes.
#
# IMPORTANT: each piece is generated independently, so "tile seamlessly with
# the other piece" is not an instruction the model can verify. Instead we
# describe the absolute state of the joining edge: a band of plain silk of
# the specified `Silk color`, vertical weave direction, no decoration.
STYLE_TOP = (
    "This is the TOP CAP of the ribbon. The top edge has a decorative "
    "gold tassel knot or finial — an ornate gathered fabric rosette with "
    "a hanging loop, as if the ribbon is pinned to a wall. "
    "The bottom 15% of the tile is an unbroken band of plain silk in the "
    "exact silk color specified below, with the vertical weave running "
    "top-to-bottom and no decoration, embroidery, or gradient crossing "
    "into that band. This bottom band is the joining edge; the silk color "
    "and weave must be identical there to the rest of the silk."
)

STYLE_MID = (
    "This is the MIDDLE piece of the ribbon. Centered in the tile, the "
    "zodiac animal is rendered as embroidered metallic gold thread — "
    "clean readable silhouette, slight cartoon personality (animal is "
    "mildly self-aware, deadpan), bold flat shapes with thick darker "
    "outlines so it reads from across the room. "
    "The top 15% and bottom 15% of the tile are unbroken bands of plain "
    "silk in the exact silk color specified below, with the vertical weave "
    "running top-to-bottom and no embroidery or decoration crossing into "
    "those bands. These bands are the joining edges — the silk color, "
    "weave direction, and brightness must be identical at the very top "
    "pixel row and the very bottom pixel row so the tile can repeat "
    "vertically without a visible seam."
)

STYLE_BOT = (
    "This is the BOTTOM CAP of the ribbon. The bottom edge tapers to a "
    "decorative point or V-notch cut with delicate gold fringe threads "
    "hanging from the tip, plus one small purely-pictorial gold flourish "
    "near the point — shaped as a dot, curl, teardrop, or small geometric "
    "motif, with no resemblance to any script or numeral. "
    "The top 15% of the tile is an unbroken band of plain silk in the "
    "exact silk color specified below, with the vertical weave running "
    "top-to-bottom and no decoration, embroidery, or gradient crossing "
    "into that band. This top band is the joining edge; the silk color "
    "and weave must be identical there to the rest of the silk."
)

PIECE_STYLES = {
    "top": STYLE_TOP,
    "mid": STYLE_MID,
    "bot": STYLE_BOT,
}

ALL_PIECES = ["top", "mid", "bot"]


# ---------------------------------------------------------------------------
# Zodiac definitions
# ---------------------------------------------------------------------------

# Each tuple: (slug, display_name, animal_visual, silk_palette).
# Order MUST match ZodiacKind::all() in src/core/zodiac.rs (calendar order:
# Mouse, Rat, Ox, Tiger, Rabbit, Dragon, Snake, Horse, Goat, Monkey,
# Rooster, Dog, Pig — 13 total). Silk colors are creature-appropriate.
ZODIACS = [
    (
        "mouse",
        "Mouse",
        "A tiny field mouse seen from above at a slight angle, hunched "
        "over a single mahjong tile it is clutching in both forepaws. "
        "Round ears, long thin tail curling into a spiral beneath it, "
        "delicate whiskers fanning out. Watchful, secretive expression — "
        "it knows something you don't.",
        "Warm dusty-grey silk (#b0a89e) with gold embroidery and gold edge trim.",
    ),
    (
        "rat",
        "Rat",
        "A plump rat in profile, perked up on its hind legs with a long "
        "curling tail and bright button eyes. Faintly clever expression, "
        "as if it just spotted a tile on the floor.",
        "Dark charcoal silk (#4a4a50) with gold embroidery and gold edge trim.",
    ),
    (
        "ox",
        "Ox",
        "A broad-shouldered ox standing in three-quarter view with thick "
        "curved horns, a heavy yoke draped across its neck, and a calm "
        "stoic expression. Stout, dependable silhouette.",
        "Deep earthen-brown silk (#7a5c3a) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "tiger",
        "Tiger",
        "A crouching tiger mid-prowl, head low and tail flicking up "
        "behind. Bold stripe pattern stitched in slightly darker gold "
        "thread. Eyes narrowed, mouth set — not snarling, just focused.",
        "Burnt-orange silk (#d4792a) with gold embroidery and gold edge trim.",
    ),
    (
        "rabbit",
        "Rabbit",
        "A round-bodied rabbit sitting upright with long ears tilted "
        "slightly to one side, paws tucked together at the chest. Soft "
        "alert expression. A small crescent moon floats just above one ear.",
        "Soft white silk (#f0ece4) with gold embroidery and gold edge trim.",
    ),
    (
        "dragon",
        "Dragon",
        "A long sinuous Chinese dragon coiled into a vertical S-curve, "
        "with flowing whiskers, antler-like horns, and a cloud puff at "
        "its tail. Mouth slightly open in a knowing grin. The most "
        "ornate embroidery on the set.",
        "Imperial crimson silk (#b5262e) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "snake",
        "Snake",
        "A snake coiled twice into a tall vertical spiral with its head "
        "rising at the top, tongue flicked out. Diamond pattern stitched "
        "down its back in slightly darker gold. Half-lidded clever eyes.",
        "Deep jade-green silk (#2e7d4f) with gold embroidery and gold edge trim.",
    ),
    (
        "horse",
        "Horse",
        "A horse mid-gallop in profile with mane and tail streaming "
        "back, front legs lifted off the ground. Spirited, head held "
        "high. Slight wind-streak lines behind it stitched in gold.",
        "Rich chestnut silk (#8b4513) with gold embroidery and gold edge trim.",
    ),
    (
        "goat",
        "Goat",
        "A goat (or ram) standing in profile with curled spiraled horns, "
        "a tufted beard, and a placid sleepy expression. Small flowering "
        "sprig tucked behind one horn.",
        "Creamy wool-white silk (#ede5d0) with gold embroidery and gold edge trim.",
    ),
    (
        "monkey",
        "Monkey",
        "A monkey perched in a crouch, one hand raised up holding a small "
        "round peach. Long curled tail swooping down behind it. Mischievous "
        "expression, eyebrows raised.",
        "Warm tawny-gold silk (#c8a04a) with gold embroidery and gold edge trim.",
    ),
    (
        "rooster",
        "Rooster",
        "A proud rooster in profile, chest puffed out, tall comb on its "
        "head and long flowing tail feathers arcing behind. Beak slightly "
        "open mid-crow. Confident strut.",
        "Scarlet-red silk (#c23028) with gold embroidery and gold edge trim.",
    ),
    (
        "dog",
        "Dog",
        "A loyal dog sitting upright in three-quarter view with one ear "
        "perked, one slightly flopped, and a small bell on a thin collar. "
        "Tongue out in a friendly relaxed grin.",
        "Warm sandy-tan silk (#c4a672) with gold embroidery and gold edge trim.",
    ),
    (
        "pig",
        "Pig",
        "A round contented pig in profile with a curly tail, small "
        "upturned snout, and floppy ears. Eyes closed in a small blissful "
        "smile, as if dreaming about something good.",
        "Rosy pink silk (#e8a0b4) with gold embroidery and gold edge trim.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(piece: str, visual: str, palette: str) -> str:
    """Combine style base + piece suffix + per-zodiac description."""
    piece_style = PIECE_STYLES[piece]
    parts = [STYLE_BASE, piece_style]
    if piece == "mid":
        parts.append(f"Subject: {visual}")
    parts.append(f"Silk color: {palette}")
    return "\n\n".join(parts)


# ---------------------------------------------------------------------------
# Image generation
# ---------------------------------------------------------------------------

def generate_image(
    client: OpenAI, prompt: str, output_path: Path, model: str, size: str
) -> None:
    """Call the image API and save the resulting PNG."""
    response = client.images.generate(
        model=model,
        prompt=prompt,
        n=1,
        size=size,
        quality="high",
    )

    data = response.data[0]
    image_url = getattr(data, "url", None)
    if image_url is None:
        b64 = getattr(data, "b64_json", None)
        if b64 is None:
            print("  Error: No image URL or b64 data returned.")
            return
        img_bytes = base64.b64decode(b64)
    else:
        import requests

        img_response = requests.get(image_url, timeout=120)
        img_response.raise_for_status()
        img_bytes = img_response.content

    output_path.write_bytes(img_bytes)
    print(f"  Saved: {output_path}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro zodiac ribbon textures (3-piece) "
        "via the OpenAI image API"
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
        "--piece",
        type=str,
        default=None,
        choices=ALL_PIECES,
        help="Generate only one piece type: top, mid, or bot.",
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
        default="gpt-image-2",
        help="Image model to use (default: gpt-image-2).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1024x1024",
        help="Image size — square tiles (default: 1024x1024).",
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
            print(f"  {i:2d}. {name:<10s}  zodiac_{slug}_{{top,mid,bot}}.png")
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

    pieces = [args.piece] if args.piece else ALL_PIECES

    client = None
    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    generated = 0
    skipped = 0
    failed = 0
    total_jobs = len(targets) * len(pieces)

    job = 0
    for idx, (slug, name, visual, palette) in targets:
        for piece in pieces:
            job += 1
            output_path = out_dir / f"zodiac_{slug}_{piece}.png"
            prompt = build_prompt(piece, visual, palette)

            print(f"\n[{job}/{total_jobs}] {name} ({piece})")

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
                print(f"  Error generating {name} ({piece}): {e}")
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
