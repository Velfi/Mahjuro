#!/usr/bin/env python3
"""
Generate relic icon art for Mahjuro using OpenAI's DALL-E image generation API.

Usage:
    pip install openai requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_relic_art.py              # Generate all relics
    python scripts/generate_relic_art.py --relic 6    # Generate only relic #6
    python scripts/generate_relic_art.py --list       # List all relics
    python scripts/generate_relic_art.py --dry-run    # Print prompts without generating
"""

import argparse
import os
import sys
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "relics"

# Shared style prefix injected into every prompt.
STYLE_PREFIX = (
    "Sardonic vector art icon for a mahjong roguelite video game. "
    "Bold flat colors, thick black outlines, 2-3 accent colors, transparent background. "
    "Deadpan humor — objects look slightly too self-aware. "
    "Clean silhouette that reads at small sizes. "
    "512×512, centered composition, no text or letters in the image."
)

# Each relic: (filename, short_name, detailed visual prompt, palette hint)
RELICS = [
    (
        "triplet_boost",
        "Triplet Boost",
        "Three identical mahjong tiles stuffed into a single oversized tan trenchcoat, "
        "the top tile peering out nervously over the collar. "
        "A bold multiplication symbol floats above them.",
        "Ivory tiles, tan trenchcoat, red accents.",
    ),
    (
        "sequence_surge",
        "Sequence Surge",
        "Three mahjong tiles numbered 1-2-3 riding a lightning bolt like a surfboard. "
        "The middle tile looks bored and unimpressed while the other two are screaming.",
        "Blue lightning, ivory tiles, yellow sparks.",
    ),
    (
        "pair_power",
        "Pair Power",
        "Two mahjong tiles aggressively fist-bumping each other, tiny impact lines "
        "radiating from the bump. Both tiles wear matching red sweatbands.",
        "Red sweatbands, ivory tiles, orange impact lines.",
    ),
    (
        "honor_fury",
        "Honor Fury",
        "A mahjong honor tile with an angry expression, veins popping out, and steam "
        "rising from the top. Tiny cracked floor beneath it.",
        "Gold tile face, dark red veins, grey steam.",
    ),
    (
        "bamboo_charm",
        "Bamboo Charm",
        "A single bamboo stalk wearing a tiny black top hat and monocle, leaning on "
        "a gentleman's cane with a smug expression.",
        "Green bamboo, black top hat, gold monocle rim.",
    ),
    (
        "red_dragon_rage",
        "Red Dragon Rage",
        "A red mahjong dragon tile that has literally caught fire. The tile looks only "
        "mildly inconvenienced, sipping tea while engulfed in flames. "
        "'This is fine' energy.",
        "Deep red base, orange and yellow flames.",
    ),
    (
        "green_luck",
        "Green Luck",
        "A four-leaf clover where one leaf is clearly wilting and held up with a piece "
        "of beige tape. A small pink heart icon floats above with a plus sign.",
        "Green clover, beige tape, pink heart.",
    ),
    (
        "white_silence",
        "White Silence",
        "A white mahjong dragon tile wearing oversized noise-canceling headphones, "
        "eyes closed in bliss. A clock icon in the corner covered in icicles, frozen.",
        "White tile, matte black headphones, icy blue clock.",
    ),
    (
        "joker_tile",
        "Joker Tile",
        "A mahjong tile wearing a comically oversized fake black mustache, googly eyes, "
        "and a tiny red hat that is falling off. A question mark on its face.",
        "Ivory tile, black mustache, red hat, yellow question mark.",
    ),
    (
        "overflow",
        "Overflow",
        "A wooden bucket tipping over sideways, mahjong tiles spilling out in a cascade. "
        "The bucket has a simple exasperated face drawn on it.",
        "Brown bucket, ivory tiles, blue splash lines.",
    ),
    (
        "quick_draw",
        "Quick Draw",
        "A mahjong tile dressed as a cowboy — tiny tan hat, leather holster belt — "
        "mid-draw with two tiles in each hand. Dust cloud at its feet.",
        "Tan cowboy hat, brown belt, ivory tiles, sandy dust.",
    ),
    (
        "chain_reaction",
        "Chain Reaction",
        "A line of mahjong tiles set up like dominoes, mid-topple. The first tile is "
        "smugly leaning back with arms crossed watching the chaos unfold. "
        "Small explosion stars at each impact point.",
        "Ivory tiles, orange and yellow impact stars, grey shadow.",
    ),
    (
        "multiplier_master",
        "Multiplier Master",
        "A mahjong tile wearing a black graduation cap and tiny round glasses, holding "
        "a small green chalkboard covered in multiplication symbols. "
        "The tile looks exhausted but proud.",
        "Black grad cap, green chalkboard, white chalk marks, ivory tile.",
    ),
    (
        "set_magnet",
        "Set Magnet",
        "A classic red and silver horseshoe magnet crackling with blue energy arcs, "
        "pulling a startled mahjong tile through the air toward it. Motion lines trail "
        "behind the tile.",
        "Red and silver magnet, blue energy arcs, ivory tile, motion lines.",
    ),
    (
        "wild_winds",
        "Wild Winds",
        "Four mahjong wind tiles spinning in a mini teal tornado, their directional "
        "symbols scrambled and swapping between them. All four tiles look dizzy with "
        "spiral eyes.",
        "Teal tornado, ivory tiles, mixed colored symbols.",
    ),
    (
        "dragon_echo",
        "Dragon Echo",
        "A red mahjong dragon tile shouting into a grey canyon, with smaller and "
        "progressively more faded copies of itself bouncing back as echoes. "
        "Small musical note symbols float around.",
        "Red dragon, fading red echoes, grey canyon walls, yellow notes.",
    ),
    (
        "reverse_tile",
        "Reverse Tile",
        "Two mahjong tiles mid-swap connected by swirling purple arrows. One tile is "
        "upside down with confused yellow stars around it. The other is smugly "
        "right-side up.",
        "Ivory tiles, purple swap arrows, yellow confusion stars.",
    ),
    (
        "stealth_tile",
        "Stealth Tile",
        "A mahjong tile wearing a black ninja outfit, crouched in a stealth pose, "
        "holding a finger to where its lips would be. Only its eyes are visible. "
        "A dark shadow pools behind it.",
        "Black ninja suit, ivory eyes, dark grey shadow.",
    ),
    (
        "locked_set",
        "Locked Set",
        "Three mahjong tiles chained together with a heavy chain and a gold padlock. "
        "The tiles look resigned and tired. The padlock has a tiny smug face.",
        "Ivory tiles, grey chain, gold padlock.",
    ),
    (
        "lucky_pair",
        "Lucky Pair",
        "Two mahjong tiles on a tiny romantic date — one offers the other a small red "
        "flower, both are blushing pink. A heart with a multiplication symbol inside "
        "floats above them.",
        "Ivory tiles, pink blush, red flower, pink heart with gold accents.",
    ),
]


def build_prompt(visual: str, palette: str) -> str:
    """Combine the shared style prefix with the relic-specific description."""
    return f"{STYLE_PREFIX}\n\nSubject: {visual}\n\nColor palette: {palette}"


def generate_image(client: OpenAI, prompt: str, output_path: Path, model: str, size: str) -> None:
    """Call the DALL-E API and save the resulting image."""
    response = client.images.generate(
        model=model,
        prompt=prompt,
        n=1,
        size=size,
        quality="high",
    )

    image_url = response.data[0].url
    if image_url is None:
        # For gpt-image-1 or models that return b64_json instead of a URL
        import base64
        b64 = response.data[0].b64_json
        if b64 is None:
            print(f"  Error: No image URL or b64 data returned.")
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
    parser = argparse.ArgumentParser(description="Generate Mahjuro relic art via DALL-E")
    parser.add_argument(
        "--relic", type=int, default=None,
        help="Generate only relic number N (1-indexed). Omit for all.",
    )
    parser.add_argument(
        "--list", action="store_true",
        help="List all relics and exit.",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--model", type=str, default="gpt-image-1",
        help="DALL-E model to use (default: gpt-image-1).",
    )
    parser.add_argument(
        "--size", type=str, default="1024x1024",
        help="Image size (default: 1024x1024). Options: 1024x1024, 1024x1536, 1536x1024.",
    )
    parser.add_argument(
        "--output-dir", type=str, default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (filename, name, _, _) in enumerate(RELICS, 1):
            print(f"  {i:2d}. {name:<20s}  ({filename}.png)")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    # Select which relics to generate.
    if args.relic is not None:
        if args.relic < 1 or args.relic > len(RELICS):
            print(f"Error: --relic must be between 1 and {len(RELICS)}")
            sys.exit(1)
        targets = [(args.relic - 1, RELICS[args.relic - 1])]
    else:
        targets = list(enumerate(RELICS))

    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    for idx, (filename, name, visual, palette) in targets:
        prompt = build_prompt(visual, palette)
        output_path = out_dir / f"{filename}.png"

        print(f"\n[{idx + 1}/{len(RELICS)}] {name}")

        if args.dry_run:
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists():
            print(f"  Skipping (already exists): {output_path}")
            print(f"  Delete the file to regenerate, or use --relic {idx + 1}")
            continue

        try:
            generate_image(client, prompt, output_path, args.model, args.size)
        except Exception as e:
            print(f"  Error generating {name}: {e}")
            continue

        # Rate-limit courtesy: small delay between API calls.
        if len(targets) > 1:
            time.sleep(2)

    print("\nDone!")
    if not args.dry_run:
        print(f"Images saved to: {out_dir}")


if __name__ == "__main__":
    main()
