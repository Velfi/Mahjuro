#!/usr/bin/env python3
"""
Generate silk-ribbon textures for the 12 Chinese zodiac consumable cards
in Mahjuro using OpenAI's image generation API.

Each zodiac is sold/used as a long hanging silk ribbon (see
`build_ribbon_mesh` in src/render/ribbon_mesh.rs and `ZodiacRibbonPlacement`
in src/render/draw_cmd.rs). The mesh UVs run 0→1 across the width and 0→1
top→bottom along the length, so the texture is authored as a tall vertical
strip that fills the frame edge-to-edge — the silk fabric must reach all
four borders so there is no halo when the texture is sampled.

Style direction: "Midnight Gold" — woven silk banners hanging in a curio
shop. Each ribbon is its own per-zodiac silk color (matching the in-engine
`consumable_color` palette in src/scenes/shop.rs so the textured ribbons
read like richer versions of the existing flat tints) with the zodiac
animal embroidered in metallic gold thread, plus subtle gold trim along
the long edges. No background — the silk IS the background.

Filenames are `zodiac_<animal>.png`, written to assets/textures/, in
calendar order matching `ZodiacKind::all()` in src/core/zodiac.rs.

Usage:
    pip install openai pillow requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_zodiac_ribbons.py                  # all missing
    python scripts/generate_zodiac_ribbons.py --force          # regenerate all
    python scripts/generate_zodiac_ribbons.py --name dragon    # one by slug
    python scripts/generate_zodiac_ribbons.py --zodiac 5       # one by index
    python scripts/generate_zodiac_ribbons.py --list           # list all
    python scripts/generate_zodiac_ribbons.py --dry-run        # prompts only
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

# Shared style prefix injected into every prompt. The ribbon mesh samples
# this texture across its full width and length, so the silk fabric MUST
# fill the entire frame — any background bleed would show as a halo on
# the hanging ribbon in the shop scene.
STYLE_PREFIX = (
    "Texture for a hanging silk ribbon in a 'Midnight Gold' mahjong "
    "roguelite video game. The image is a TALL VERTICAL STRIP filled "
    "edge-to-edge with woven silk fabric — the silk reaches all four "
    "borders of the frame, with NO outer background, NO drop shadow, NO "
    "vignette, NO floor, NO wall, NO surrounding scene. The texture will "
    "be wrapped onto a 3D ribbon mesh, so the entire frame must be the "
    "ribbon itself. "
    "The silk has a subtle vertical weave grain and a soft satin sheen, "
    "with a thin metallic gold trim braid running down each long edge. "
    "Centered on the ribbon, the zodiac animal is rendered as embroidered "
    "metallic gold thread — clean readable silhouette, slight cartoon "
    "personality (animal is mildly self-aware, deadpan), bold flat shapes "
    "with thick darker outlines so it reads from across the room. "
    "Below the animal, a small abstract gold ornamental flourish (NOT a "
    "letter, NOT a Chinese character, NOT a number) acts as a tassel "
    "anchor at the bottom of the ribbon. "
    "Flat orthographic front-on view, no perspective, no foreshortening. "
    "No text, no letters, no numbers, no logos, no borders or frames "
    "outside the silk itself, no watermarks, no signatures."
)


# Each tuple: (slug, display_name, animal_visual, silk_palette).
# Order MUST match ZodiacKind::all() in src/core/zodiac.rs (calendar order:
# Rat, Ox, Tiger, Rabbit, Dragon, Snake, Horse, Goat, Monkey, Rooster,
# Dog, Pig). Silk colors echo the per-zodiac palette in `consumable_color`
# (src/scenes/shop.rs) so the textured ribbons feel like a richer version
# of the existing flat tint, not a different art set.
ZODIACS = [
    (
        "rat",
        "Rat",
        "A plump rat in profile, perked up on its hind legs with a long "
        "curling tail and bright button eyes. Faintly clever expression, "
        "as if it just spotted a tile on the floor.",
        "Warm peach silk (#f59e6b) with gold embroidery and gold edge trim.",
    ),
    (
        "ox",
        "Ox",
        "A broad-shouldered ox standing in three-quarter view with thick "
        "curved horns, a heavy yoke draped across its neck, and a calm "
        "stoic expression. Stout, dependable silhouette.",
        "Rich saffron-gold silk (#f2c752) with darker gold embroidery "
        "and gold edge trim.",
    ),
    (
        "tiger",
        "Tiger",
        "A crouching tiger mid-prowl, head low and tail flicking up "
        "behind. Bold stripe pattern stitched in slightly darker gold "
        "thread. Eyes narrowed, mouth set — not snarling, just focused.",
        "Brick-red silk (#c76b56) with gold embroidery and gold edge trim.",
    ),
    (
        "rabbit",
        "Rabbit",
        "A round-bodied rabbit sitting upright with long ears tilted "
        "slightly to one side, paws tucked together at the chest. Soft "
        "alert expression. A small crescent moon floats just above one ear.",
        "Pale jade-green silk (#80c78c) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "dragon",
        "Dragon",
        "A long sinuous Chinese dragon coiled into a vertical S-curve, "
        "with flowing whiskers, antler-like horns, and a cloud puff at "
        "its tail. Mouth slightly open in a knowing grin. The most "
        "ornate embroidery on the set.",
        "Cool sky-indigo silk (#8c9eeb) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "snake",
        "Snake",
        "A snake coiled twice into a tall vertical spiral with its head "
        "rising at the top, tongue flicked out. Diamond pattern stitched "
        "down its back in slightly darker gold. Half-lidded clever eyes.",
        "Mauve silk (#d98cd9) with gold embroidery and gold edge trim.",
    ),
    (
        "horse",
        "Horse",
        "A horse mid-gallop in profile with mane and tail streaming "
        "back, front legs lifted off the ground. Spirited, head held "
        "high. Slight wind-streak lines behind it stitched in gold.",
        "Rose silk (#eb759e) with gold embroidery and gold edge trim.",
    ),
    (
        "goat",
        "Goat",
        "A goat (or ram) standing in profile with curled spiraled horns, "
        "a tufted beard, and a placid sleepy expression. Small flowering "
        "sprig tucked behind one horn.",
        "Warm straw silk (#e0db8c) with gold embroidery and gold edge trim.",
    ),
    (
        "monkey",
        "Monkey",
        "A monkey perched in a crouch, one hand raised up holding a small "
        "round peach. Long curled tail swooping down behind it. Mischievous "
        "expression, eyebrows raised.",
        "Cool teal silk (#73b8c7) with gold embroidery and gold edge trim.",
    ),
    (
        "rooster",
        "Rooster",
        "A proud rooster in profile, chest puffed out, tall comb on its "
        "head and long flowing tail feathers arcing behind. Beak slightly "
        "open mid-crow. Confident strut.",
        "Bright ember-orange silk (#f28052) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "dog",
        "Dog",
        "A loyal dog sitting upright in three-quarter view with one ear "
        "perked, one slightly flopped, and a small bell on a thin collar. "
        "Tongue out in a friendly relaxed grin.",
        "Fresh moss-green silk (#9ed96b) with gold embroidery and gold "
        "edge trim.",
    ),
    (
        "pig",
        "Pig",
        "A round contented pig in profile with a curly tail, small "
        "upturned snout, and floppy ears. Eyes closed in a small blissful "
        "smile, as if dreaming about something good.",
        "Soft lavender silk (#c7a8eb) with gold embroidery and gold "
        "edge trim.",
    ),
]


def build_prompt(visual: str, palette: str) -> str:
    """Combine the shared style prefix with the per-zodiac description."""
    return f"{STYLE_PREFIX}\n\nSubject: {visual}\n\nSilk color: {palette}"


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


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro zodiac ribbon textures via the OpenAI image API"
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
        default="gpt-image-1",
        help="Image model to use (default: gpt-image-1).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1024x1536",
        help="Image size — must be a portrait aspect since the ribbon "
        "mesh is tall and narrow (default: 1024x1536).",
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
            print(f"  {i:2d}. {name:<10s}  (zodiac_{slug}.png)")
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
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    generated = 0
    skipped = 0
    failed = 0

    for idx, (slug, name, visual, palette) in targets:
        prompt = build_prompt(visual, palette)
        output_path = out_dir / f"zodiac_{slug}.png"

        print(f"\n[{idx + 1}/{len(ZODIACS)}] {name}")

        if args.dry_run:
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

        if len(targets) > 1:
            time.sleep(args.delay)

    print("\nDone.")
    if not args.dry_run:
        print(
            f"  generated={generated}  skipped={skipped}  failed={failed}"
            f"  → {out_dir}"
        )


if __name__ == "__main__":
    main()
