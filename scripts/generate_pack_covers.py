#!/usr/bin/env python3
"""Generate trading-card-pack cover art for TilePackKind booster packs.

Each tile pack is a booster pack the player can buy in the shop to
permanently add extra tiles to their wall. The cover art evokes the
feeling of tearing open a collectible trading-card foil pack — glossy,
vivid, and slightly mysterious.

Style: retro-TCG foil packs from the late 1990s / early 2000s. Think
Pokémon booster packs, Yu-Gi-Oh!, or Magic: The Gathering — a narrow
vertical portrait with a dramatic illustration, a stylized logo area,
and holographic / metallic sheen. The pack art lives in the "Midnight
Gold" universe of Mahjuro: deep indigo-black backgrounds, metallic gold
accents, and silk-ribbon energy.

Outputs `pack_<slug>.png` into assets/textures/packs/.

Usage:
    pip install openai pillow
    export OPENAI_API_KEY="sk-..."
    python3 scripts/generate_pack_covers.py                # all missing
    python3 scripts/generate_pack_covers.py --force        # regenerate all
    python3 scripts/generate_pack_covers.py --name honors  # one by slug
    python3 scripts/generate_pack_covers.py --list         # list all
    python3 scripts/generate_pack_covers.py --dry-run      # prompts only
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
    Path(__file__).resolve().parent.parent / "assets" / "textures" / "packs"
)
FINAL_SIZE = (256, 384)  # tall portrait, matches a booster-pack aspect ratio

# ---------------------------------------------------------------------------
# Shared style
# ---------------------------------------------------------------------------

STYLE_PREFIX = (
    "Cover art for a collectible trading-card booster pack in a 'Midnight "
    "Gold' mahjong roguelite video game. The image is a TALL VERTICAL "
    "PORTRAIT (2:3 aspect) depicting the front face of a sealed foil pack. "
    "\n\n"
    "STYLE RULES:\n"
    "- Late-1990s / early-2000s TCG booster pack aesthetic: glossy "
    "metallic wrapper with holographic shimmer, dramatic central "
    "illustration, stylized title area across the top.\n"
    "- The pack wrapper has a faint foil-crinkle texture and catches "
    "light along diagonal creases.\n"
    "- Background color is deep midnight indigo (#0e1225) fading to "
    "black at the edges.\n"
    "- Gold metallic accents: a thin decorative border, corner "
    "flourishes, and the title lettering.\n"
    "- The central illustration floats inside an ornate gold frame "
    "or medallion shape on the pack face.\n"
    "- Any visible text MUST be in a made-up nonsense language (NOT "
    "real words in any real language). The pack title area should have "
    "one or two lines of stylized nonsense-script that evoke the pack "
    "name without being readable.\n"
    "- A small nonsense-text tagline at the bottom of the pack in "
    "smaller font (like a card count: 'Karthun ×7' style).\n"
    "- The wrapper has a subtle tear-notch at the top edge.\n"
    "- No real brand names, no real language, no watermarks.\n"
    "- Flat front-on view, no perspective, as if the pack is laid on "
    "a scanner.\n"
)

# ---------------------------------------------------------------------------
# Per-pack definitions
# ---------------------------------------------------------------------------

# (slug, display_name, illustration_prompt, pack_color_accent)
PACKS = [
    (
        "honors",
        "Honors Pack",
        "The central illustration shows four wind-compass roses stacked "
        "vertically, each glowing a different elemental color (ice-blue, "
        "ember-orange, jade-green, storm-violet), with three dragon "
        "silhouettes coiling around them — one red, one green, one white. "
        "Ethereal mist swirls between the symbols. The overall mood is "
        "ceremonial and ancient.",
        "Pack wrapper is deep navy (#0e1838) with silver-white holographic "
        "shimmer and gold border accents.",
    ),
    (
        "polychrome",
        "Polychrome Pack",
        "The central illustration shows a single mahjong tile exploding "
        "into prismatic shards of rainbow light, each shard refracting "
        "into a different saturated color. The tile itself is cracking "
        "open like an egg to reveal iridescent crystal inside. Scattered "
        "smaller tiles orbit the burst, each trailing a different-colored "
        "comet tail.",
        "Pack wrapper shifts between magenta, violet, and teal in a "
        "holographic oil-slick rainbow effect. Gold border accents.",
    ),
    (
        "terminals",
        "Terminals Pack",
        "The central illustration shows two massive stone pillars — one "
        "carved with the numeral 1 and one with 9 (in stylized nonsense "
        "script, not real numbers) — flanking a narrow gate. Between "
        "the pillars, a beam of golden light shoots upward into a "
        "star-filled sky. Tiles are stacked at the base of each pillar "
        "like offerings. Ancient, monumental.",
        "Pack wrapper is warm obsidian-black (#1a1412) with amber-gold "
        "holographic shimmer and gold border accents.",
    ),
    (
        "flowers",
        "Flowers Pack",
        "The central illustration shows four flowers arranged in a "
        "diamond pattern — plum blossom, orchid, chrysanthemum, and "
        "bamboo stalk — each rendered in luminous white-and-pink ink "
        "wash. Petals drift between them, caught in an invisible breeze. "
        "A faint full moon hangs behind the arrangement. Delicate, "
        "serene, East Asian woodblock energy.",
        "Pack wrapper is soft plum-black (#1c0f1e) with pearlescent "
        "pink holographic shimmer and rose-gold border accents.",
    ),
    (
        "bamboo_grove",
        "Bamboo Grove",
        "The central illustration shows a dense grove of bamboo stalks "
        "rising from misty ground, receding into darkness. Each stalk "
        "has faintly glowing segments etched with tile-rank symbols. "
        "A narrow path winds between the stalks toward a distant golden "
        "lantern. Fireflies dot the scene. Lush, mysterious, green-on-"
        "black.",
        "Pack wrapper is deep forest-black (#0a1a0e) with emerald-green "
        "holographic shimmer and gold border accents.",
    ),
    (
        "coin_cache",
        "Coin Cache",
        "The central illustration shows a cascade of ancient coins "
        "tumbling from an overturned lacquered chest. Each coin has a "
        "square hole at its center and faintly glowing concentric-circle "
        "tile symbols on its face. The coins catch golden light as they "
        "fall. A velvet cloth beneath. Opulent, weighty.",
        "Pack wrapper is rich burgundy-black (#1a0e12) with warm gold "
        "holographic shimmer and gold border accents.",
    ),
    (
        "scroll_library",
        "Scroll Library",
        "The central illustration shows a towering wall of wooden "
        "cubbyholes, each holding a rolled scroll. Several scrolls are "
        "pulled halfway out, unfurling to reveal brushstroke tile-rank "
        "characters (nonsense script) in red and black ink. A scholar's "
        "hand reaches for one from the bottom of the frame. Candlelit, "
        "scholarly, warm amber atmosphere.",
        "Pack wrapper is deep sepia-black (#1a140a) with warm amber "
        "holographic shimmer and gold border accents.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(illustration: str, color_accent: str) -> str:
    return f"{STYLE_PREFIX}\n\nCentral illustration: {illustration}\n\nWrapper color: {color_accent}"


# ---------------------------------------------------------------------------
# Image generation + post-processing
# ---------------------------------------------------------------------------

def generate_image(client: OpenAI, prompt: str, model: str, size: str) -> Image.Image:
    """Call the OpenAI API and return a PIL Image (RGBA)."""
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


def add_foil_sheen(img: Image.Image) -> Image.Image:
    """Overlay a subtle diagonal holographic sheen."""
    import math

    w, h = img.size
    overlay = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    op = overlay.load()

    for y in range(h):
        for x in range(w):
            # Diagonal wave.
            t = math.sin((x + y) * math.pi / 120) * 0.5 + 0.5
            # Subtle iridescent tint cycling R/G/B.
            r = int(200 + 55 * math.sin(t * math.pi * 2))
            g = int(200 + 55 * math.sin(t * math.pi * 2 + 2.1))
            b = int(200 + 55 * math.sin(t * math.pi * 2 + 4.2))
            op[x, y] = (r, g, b, 18)

    return Image.alpha_composite(img, overlay)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro tile-pack cover art via OpenAI image API"
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Generate only the pack with this slug (e.g. honors).",
    )
    parser.add_argument(
        "--list", action="store_true", help="List all packs and exit."
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
        help="Generation size (default: 1024x1536, portrait).",
    )
    parser.add_argument(
        "--no-sheen",
        action="store_true",
        help="Skip the holographic sheen post-processing.",
    )
    parser.add_argument(
        "--preview",
        action="store_true",
        help="Keep at generation resolution instead of downscaling.",
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
        for slug, name, _, _ in PACKS:
            print(f"  {slug:<20s}  {name}")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.name is not None:
        match = next(((i, p) for i, p in enumerate(PACKS) if p[0] == args.name), None)
        if match is None:
            print(f"Error: no pack with slug '{args.name}'. Try --list.")
            sys.exit(1)
        targets = [match]
    else:
        targets = list(enumerate(PACKS))

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

    for idx, (slug, name, illustration, color_accent) in targets:
        prompt = build_prompt(illustration, color_accent)
        output_path = out_dir / f"pack_{slug}.png"

        print(f"\n[{idx + 1}/{len(PACKS)}] {name}")

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
            img = generate_image(client, prompt, args.model, args.size)

            if not args.no_sheen:
                img = add_foil_sheen(img)

            if not args.preview:
                img = img.resize(FINAL_SIZE, Image.LANCZOS)

            img.save(str(output_path))
            print(f"  Saved: {output_path}  ({output_path.stat().st_size} bytes)")
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
