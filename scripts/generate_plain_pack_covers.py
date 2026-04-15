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
    "mahjong video game. FLAT VECTOR STYLE, PLAIN COLORS ONLY.\n\n"
    "HARD STYLE RULES — these are non-negotiable:\n"
    "- Pure solid color fills only. Every region is ONE flat RGB value "
    "with crisp hard edges between regions.\n"
    "- NO gradients. NO shading. NO highlights. NO shadows. NO ambient "
    "occlusion. NO volumetric lighting. NO glow. NO bloom.\n"
    "- NO textures of any kind: no paper, no foil, no metal, no fabric, "
    "no noise, no grain, no brushstrokes, no crinkles, no creases.\n"
    "- NO holographic shimmer, NO iridescence, NO reflections, NO specular.\n"
    "- NO text, NO letters, NO numerals, NO logos, NO glyphs, NO runes, "
    "NO script of any kind.\n"
    "- Simple flat-vector illustration is OK — characters, creatures, "
    "objects, and scenery are allowed but must be drawn as simple flat "
    "color shapes (think minimalist vector icon / flat-design poster), "
    "NOT painted, NOT rendered, NOT shaded.\n"
    "- Use only a small palette of flat fills per illustration element; "
    "do NOT blend colors.\n"
    "- Flat front-on view, orthographic, as if drawn with a vector tool "
    "using fill-only shapes (think Adobe Illustrator flat fills, or "
    "SVG <path>/<rect>/<circle> elements with NO filters applied).\n"
    "- The image fills the full frame edge-to-edge with the background "
    "color; no drop shadow, no mockup, no surface beneath.\n"
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
        "COMPOSITION: a single tall vertical ivory (#f2ead6) column "
        "running down the center of the frame, about one-third the "
        "image width, extending nearly top-to-bottom — a ceremonial "
        "totem. Stacked on this column are SEVEN flat-color symbols, "
        "evenly spaced vertically from top to bottom: "
        "(1) a flat ice-blue (#7ec8e3) arrow pointing up, "
        "(2) a flat ember-orange (#e07a3c) arrow pointing right, "
        "(3) a flat jade-green (#4ea87a) arrow pointing down, "
        "(4) a flat storm-violet (#7a4ea8) arrow pointing left, "
        "(5) a flat red (#e23b3b) solid circle, "
        "(6) a flat green (#3aa84e) solid circle, "
        "(7) a flat white (#f8f8f8) solid circle outlined thinly in "
        "navy. All symbols are plain flat shapes, no decoration.",
    ),
    (
        "polychrome",
        "pure black #000000",
        "COMPOSITION: a single large flat white (#f2f2f2) upright "
        "rounded-rectangle mahjong-tile shape in the dead center of "
        "the frame, about one-third the image width. Radiating outward "
        "from this tile in all directions, TEN flat triangular shards "
        "of pure color forming a starburst/explosion pattern that "
        "reaches toward the edges of the frame. Each shard is a "
        "different flat color, clockwise from top: red (#e23b3b), "
        "orange (#e88a2a), yellow (#e8d23a), lime (#8ae83a), green "
        "(#3aa84e), teal (#3ae8c4), blue (#3a6ee8), indigo (#5a3ae8), "
        "violet (#8a3ae8), magenta (#e83ac4). Hard triangular edges.",
    ),
    (
        "terminals",
        "warm obsidian #1a1412",
        "COMPOSITION: an architectural gateway. Two massive flat amber "
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
        "COMPOSITION: a 2x2 grid of four distinctly different flat "
        "flowers fills the frame, evenly spaced. "
        "Top-left: a pink (#f2a6c0) 5-petal plum blossom with a small "
        "gold (#e8c46a) center. "
        "Top-right: a violet (#b08ae0) 6-petal orchid with a magenta "
        "(#e06a8a) center. "
        "Bottom-left: a gold (#e8c46a) many-petal chrysanthemum, many "
        "thin flat petal shapes in a circle around an amber center. "
        "Bottom-right: a flat green (#4ea87a) bamboo leaf sprig — two "
        "or three simple pointed leaf shapes on a short stem. "
        "All flowers are plain flat-color shapes, no shading.",
    ),
    (
        "bamboo_grove",
        "deep forest-black #0a1a0e",
        "COMPOSITION: a horizontal forest scene. Eight flat emerald-"
        "green (#3aa84e) vertical bamboo stalks of VARYING heights "
        "stand side by side across the full width of the frame, "
        "uneven tops forming a jagged canopy silhouette. Each stalk "
        "has 2–3 thin darker green (#1a6a2e) horizontal segment "
        "bands. Hanging in the center-right area between stalks is "
        "one flat round gold (#e8c46a) lantern with a short dark "
        "string above it. Scattered among the stalks are 4 small "
        "flat yellow (#f8e87a) dot-fireflies. Reads as a scene.",
    ),
    (
        "coin_cache",
        "burgundy-black #1a0e12",
        "COMPOSITION: a diagonal cascade of coins. In the lower-left "
        "corner, a flat dark-red (#6a1a24) tipped-over rectangular "
        "chest shape with a flat gold (#c8882a) rim along its open "
        "top edge. From the chest, SEVEN flat gold (#e8c46a) ring-"
        "coins tumble in a diagonal arc toward the upper-right of "
        "the frame, each coin a solid circle with a smaller square "
        "hole punched out of its center (classic Chinese cash-coin "
        "shape). The coins grow progressively smaller as they reach "
        "the upper-right. Clear sense of motion along the diagonal.",
    ),
    (
        "scroll_library",
        "sepia-black #1a140a",
        "COMPOSITION: an interior wall of shelves. A 3-column by "
        "4-row grid of flat brown (#6a4a1a) rectangular cubbyhole "
        "openings fills most of the frame, evenly spaced with thin "
        "flat amber (#e8b86a) dividers between them. Two specific "
        "cubbies contain a flat cream (#f2ead6) rolled scroll "
        "partially pulled out — one in the top-row middle cubby and "
        "one in the bottom-row right cubby, each scroll shown as a "
        "small flat cream cylinder sticking out. In the lower-left "
        "corner in front of the shelves, one small flat amber "
        "(#e8c46a) round lantern shape. Reads as architecture.",
    ),
]


def build_prompt(bg_color: str, composition: str) -> str:
    return (
        f"{STYLE_PREFIX}\n"
        f"Background color (fills the full image edge-to-edge): {bg_color}.\n\n"
        f"Composition: {composition}\n\n"
        "Reminder: every color region must be a pure flat fill. No "
        "gradients. No shading. No textures. No text. No illustration."
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
    parser.add_argument("--model", type=str, default="gpt-image-1")
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
