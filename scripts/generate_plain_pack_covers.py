#!/usr/bin/env python3
"""Generate plain flat-color booster-pack covers via OpenAI image API.

These are deliberately effect-free: solid color regions only, no
gradients, no shading, no holographic sheen, no textures, no text, no
illustration detail. Art direction applies effects in a later pass.

Outputs `pack_<slug>.png` into assets/textures/packs/plain/.

Usage:
    pip install openai pillow
    export OPENAI_API_KEY="sk-..."
    python3 scripts/generate_plain_pack_covers.py                # all missing
    python3 scripts/generate_plain_pack_covers.py --force        # regenerate
    python3 scripts/generate_plain_pack_covers.py --name honors  # one slug
    python3 scripts/generate_plain_pack_covers.py --list         # list slugs
    python3 scripts/generate_plain_pack_covers.py --dry-run      # prompts only
"""

import argparse
import base64
import io
import os
import sys
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)


OUTPUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "assets" / "textures" / "packs" / "plain"
)
FINAL_SIZE = (256, 384)

STYLE_PREFIX = (
    "A tall vertical 2:3 portrait image of a booster-pack cover for a "
    "mahjong video game, drawn as a flat-design vector poster scene. "
    "The whole image IS the pack — the frame fills edge-to-edge with "
    "the background color, no tabletop, mockup, or drop shadow.\n\n"
    "Hard style rules — non-negotiable:\n"
    "- Every region is a single flat RGB value with crisp hard edges "
    "between regions. Surfaces must look like vector cutouts, like "
    "construction paper — not painted, not rendered, not photographed.\n"
    "- Use only a small palette of flat fills per element; colors are "
    "placed next to each other, never blended.\n"
    "- Characters, creatures, objects, and scenery are drawn as simple "
    "flat color shapes in the style of a minimalist vector icon or "
    "flat-design poster — as if built from SVG <path>, <rect>, and "
    "<circle> elements with no filters applied.\n"
    "- Flat front-on view, orthographic.\n"
    "- Purely pictorial decoration only. No written glyphs, numerals, "
    "or logos of any language anywhere in the image; any shape that "
    "resembles a letter or number must be replaced with a plain "
    "geometric mark.\n"
)


# Each pack has a deliberately distinct silhouette and composition so
# the seven are instantly recognizable at thumbnail size. All shapes
# are flat fills only — no gradients, no shading, no textures, no text.
#
# (slug, background_color_hex, composition)
PACKS = [
    (
        "honors",
        "deep navy #0e1838",
        "Composition: a single tall vertical ivory (#f2ead6) column "
        "running down the center of the frame, a narrow central stripe "
        "extending nearly top-to-bottom — a ceremonial totem. Stacked "
        "on this column is a column of flat-color symbols, evenly "
        "spaced vertically: in the upper half, four bold arrows "
        "pointing up, right, down, and left in ice-blue (#7ec8e3), "
        "ember-orange (#e07a3c), jade-green (#4ea87a), and "
        "storm-violet (#7a4ea8); in the lower half, three solid "
        "circles in red (#e23b3b), green (#3aa84e), and white "
        "(#f8f8f8) with a thin navy outline on the white one. All "
        "symbols are plain flat shapes.",
    ),
    (
        "polychrome",
        "pure black #000000",
        "Composition: a single large flat white (#f2f2f2) upright "
        "rounded-rectangle mahjong-tile shape in the dead center of "
        "the frame. Radiating outward from this tile in all "
        "directions, a starburst of flat triangular shards reaches "
        "toward the edges of the frame. The shards sweep through a "
        "full rainbow around the tile — warm reds and oranges on one "
        "side transitioning through yellow, green, teal, blue, and "
        "violet around to magenta — each shard a single saturated "
        "flat fill with hard triangular edges.",
    ),
    (
        "terminals",
        "warm obsidian #1a1412",
        "Composition: an architectural gateway. Two massive flat amber "
        "(#e8a84a) trapezoidal pillars stand on the left and right "
        "edges of the frame, widening slightly at the base, nearly "
        "full height. A flat gold (#c8882a) horizontal triangular peak "
        "bridges the tops of the pillars like a temple roof. Between "
        "the pillars in the center is a narrow dark vertical strip — "
        "a receding path — rendered as flat obsidian (#0a0706). At "
        "the base of each pillar sits a small flat gold square — "
        "offering stones. Strong bilateral symmetry.",
    ),
    (
        "flowers",
        "plum-black #1c0f1e",
        "Composition: a 2x2 grid of four distinctly different flat "
        "flowers fills the frame, evenly spaced. "
        "Top-left: a pink (#f2a6c0) five-petal plum blossom with a "
        "small gold (#e8c46a) round center. "
        "Top-right: a violet (#b08ae0) multi-petal orchid with a "
        "magenta (#e06a8a) center. "
        "Bottom-left: a gold (#e8c46a) chrysanthemum shown as many "
        "thin flat petal shapes arranged in a circle around an amber "
        "center. "
        "Bottom-right: a flat green (#4ea87a) bamboo leaf sprig — a "
        "few simple pointed leaf shapes on a short stem. "
        "All flowers are plain flat-color shapes.",
    ),
    (
        "bamboo_grove",
        "deep forest-black #0a1a0e",
        "Composition: a horizontal forest scene. A row of flat "
        "emerald-green (#3aa84e) vertical bamboo stalks of varying "
        "heights stand side by side across the full width of the "
        "frame, uneven tops forming a jagged canopy silhouette. Each "
        "stalk is broken by a couple of thin darker green (#1a6a2e) "
        "horizontal segment bands. Hanging in the center-right area "
        "between stalks is one flat round gold (#e8c46a) lantern with "
        "a short dark string above it. Scattered among the stalks are "
        "a handful of small flat yellow (#f8e87a) dot-fireflies.",
    ),
    (
        "coin_cache",
        "burgundy-black #1a0e12",
        "Composition: a diagonal cascade of coins. In the lower-left "
        "corner, a flat dark-red (#6a1a24) tipped-over rectangular "
        "chest shape with a flat gold (#c8882a) rim along its open "
        "top edge. From the chest, a stream of flat gold (#e8c46a) "
        "ring-coins tumbles in a diagonal arc toward the upper-right "
        "of the frame, each coin a solid circle with a smaller square "
        "hole punched out of its center (classic Chinese cash-coin "
        "shape). The coins grow progressively smaller as they reach "
        "the upper-right, giving a clear sense of motion along the "
        "diagonal.",
    ),
    (
        "scroll_library",
        "sepia-black #1a140a",
        "Composition: an interior wall of shelves. A grid of flat "
        "brown (#6a4a1a) rectangular cubbyhole openings — three "
        "columns wide and several rows tall — fills most of the "
        "frame, evenly spaced with thin flat amber (#e8b86a) dividers "
        "between them. A couple of cubbies contain a flat cream "
        "(#f2ead6) rolled scroll partially pulled out — one in the "
        "upper rows, one in the lower rows — each scroll shown as a "
        "small flat cream cylinder sticking out. In the lower-left "
        "corner in front of the shelves, one small flat amber "
        "(#e8c46a) round lantern shape.",
    ),
]


def build_prompt(bg_color: str, composition: str) -> str:
    return (
        f"{STYLE_PREFIX}\n"
        f"Background color (fills the full image edge-to-edge): {bg_color}.\n\n"
        f"{composition}"
    )


def generate_image(client: OpenAI, prompt: str, model: str, size: str) -> Image.Image:
    response = client.images.generate(
        model=model,
        prompt=prompt,
        n=1,
        size=size,
        quality="high",
    )
    data = response.data[0]
    b64 = getattr(data, "b64_json", None)
    if b64 is not None:
        img_bytes = base64.b64decode(b64)
    else:
        url = getattr(data, "url", None)
        if url is None:
            raise RuntimeError("No image URL or b64 data returned.")
        import requests
        img_bytes = requests.get(url, timeout=120).content
    return Image.open(io.BytesIO(img_bytes)).convert("RGBA")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate plain booster-pack covers via OpenAI image API."
    )
    parser.add_argument("--name", type=str, default=None, help="Single pack slug.")
    parser.add_argument("--list", action="store_true", help="List slugs and exit.")
    parser.add_argument("--dry-run", action="store_true", help="Print prompts only.")
    parser.add_argument("--force", action="store_true", help="Overwrite existing.")
    parser.add_argument("--model", type=str, default="gpt-image-2")
    parser.add_argument("--size", type=str, default="1024x1536")
    parser.add_argument(
        "--preview", action="store_true",
        help="Keep generation resolution instead of downscaling to 256x384.",
    )
    parser.add_argument("--output-dir", type=str, default=None)
    parser.add_argument("--delay", type=float, default=2.0)
    args = parser.parse_args()

    if args.list:
        for slug, _, _ in PACKS:
            print(f"  {slug}")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    targets = list(enumerate(PACKS))
    if args.name is not None:
        targets = [(i, p) for i, p in enumerate(PACKS) if p[0] == args.name]
        if not targets:
            print(f"Error: no pack with slug '{args.name}'. Try --list.")
            sys.exit(1)

    client = None
    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    generated = skipped = failed = 0

    for idx, (slug, bg, composition) in targets:
        prompt = build_prompt(bg, composition)
        output_path = out_dir / f"pack_{slug}.png"

        print(f"\n[{idx + 1}/{len(PACKS)}] {slug}")

        if args.dry_run:
            print(f"  Output: {output_path.name}")
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists() and not args.force:
            print(f"  Skipping (exists): {output_path.name} — use --force to regenerate")
            skipped += 1
            continue

        try:
            assert client is not None
            img = generate_image(client, prompt, args.model, args.size)
            if not args.preview:
                img = img.resize(FINAL_SIZE, Image.LANCZOS)
            img.save(str(output_path))
            print(f"  Saved: {output_path}  ({output_path.stat().st_size} bytes)")
            generated += 1
        except Exception as e:
            print(f"  Error generating {slug}: {e}")
            failed += 1
            continue

        if len(targets) > 1:
            time.sleep(args.delay)

    print("\nDone.")
    if not args.dry_run:
        print(f"  generated={generated}  skipped={skipped}  failed={failed}  → {out_dir}")


if __name__ == "__main__":
    main()
