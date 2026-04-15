#!/usr/bin/env python3
"""Generate booster-pack cover art for TilePackKind booster packs.

Each tile pack is a booster pack the player can buy in the shop to
permanently add extra tiles to their wall. The cover art must be
instantly recognizable at thumbnail size — the player identifies a
pack by its silhouette and its one signature color, not by reading it.

Style: "Flat Icon Sigil". One bold centered sigil per pack on a
pack-unique flat color background, wrapped in a shared frame. Modern
trading-card minimalism (MTG mana symbols, Slay the Spire relics,
Balatro booster packs) rather than '90s foil wrappers. Shiny effects,
if wanted, are applied later as an in-engine shader pass — not baked
into the PNG.

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
    "A booster-pack cover for a mahjong roguelite video game. TALL "
    "VERTICAL PORTRAIT, 2:3 aspect. 'FLAT ICON SIGIL' style: one bold "
    "centered sigil on a flat pack-color background, wrapped in a "
    "shared frame. Flat vector look, like a modern trading-card pack "
    "icon (think MTG mana symbols, Slay the Spire relics, Balatro "
    "booster packs). Orthographic front-on view, no perspective.\n\n"
    "SHARED FRAME (identical across every pack — this is the 'brand'):\n"
    "- The full image is a rounded-corner rectangle, 2:3 portrait, "
    "filling the frame edge-to-edge with the pack-specific background "
    "color.\n"
    "- A thin gold (#d9b35a) hairline border sits just inside the "
    "rounded edge, 2–3 px thick, evenly offset from the outer edge.\n"
    "- A small V-shaped tear notch is cut into the top edge, centered "
    "horizontally — a subtle visual cue that this is a pack to open.\n"
    "- A short gold (#d9b35a) hairline bar spans the bottom quarter "
    "of the frame, dividing the sigil area above from a blank title "
    "bar region below. The title bar region is left EMPTY (no text, "
    "no glyphs, no script — the engine overlays a real title later).\n"
    "- The sigil sits in the upper three-quarters of the frame, "
    "centered horizontally, comfortably inset from the gold border.\n\n"
    "HARD STYLE RULES — non-negotiable:\n"
    "- Pure flat color fills. Every region is ONE flat RGB value with "
    "crisp hard edges between regions.\n"
    "- NO gradients. NO shading. NO highlights. NO shadows. NO ambient "
    "occlusion. NO volumetric lighting. NO glow. NO bloom.\n"
    "- NO textures of any kind: no foil crinkle, no paper, no grain, "
    "no noise, no brushstrokes, no creases.\n"
    "- NO holographic shimmer, NO iridescence, NO reflections.\n"
    "- NO text, NO letters, NO numerals, NO logos, NO runes, NO script "
    "of any kind anywhere in the image.\n"
    "- The sigil must be silhouette-first — identifiable from its "
    "shape alone at 32×32. Bold, simple, iconic. Not a scene, not a "
    "painting, not an illustration with depth. One symbol.\n"
    "- Small unifying flourish allowed: 2–3 tiny flat gold (#d9b35a) "
    "dots drifting in negative space near the sigil. That's it.\n"
    "- No watermarks, no mockup, no surface beneath the pack.\n"
)

# ---------------------------------------------------------------------------
# Per-pack definitions
# ---------------------------------------------------------------------------

# Each pack owns ONE background color and ONE sigil. Silhouette-first:
# the sigil must be identifiable at 32×32. No scenes, no painterly detail.
#
# (slug, display_name, sigil_prompt, background_color)
PACKS = [
    (
        "honors",
        "Honors Pack",
        "SIGIL: a single bold 8-pointed compass rose, flat ivory "
        "(#f2ead6), centered. Four long cardinal points (N/E/S/W) and "
        "four shorter diagonal points, all crisp straight triangles "
        "meeting at a small ivory circle at the center. The compass "
        "occupies about 55% of the sigil area. Nothing else.",
        "Pack background: deep navy #0e1838, one flat fill edge-to-edge.",
    ),
    (
        "polychrome",
        "Polychrome Pack",
        "SIGIL: a single upright rounded-rectangle mahjong-tile "
        "silhouette, flat white (#f8f8f8), centered, occupying about "
        "45% of the sigil-area height. From behind the tile, 8 flat "
        "triangular shards radiate outward in a symmetric starburst, "
        "each a different pure flat color, clockwise from top: red "
        "(#e23b3b), orange (#e88a2a), yellow (#e8d23a), green "
        "(#3aa84e), teal (#3ae8c4), blue (#3a6ee8), violet (#8a3ae8), "
        "magenta (#e83ac4). Hard triangular edges, no blur.",
        "Pack background: pure black #000000, one flat fill edge-to-edge.",
    ),
    (
        "terminals",
        "Terminals Pack",
        "SIGIL: a simple symmetric gateway silhouette in flat amber "
        "(#e8a84a). Two thick vertical trapezoidal pillars (wider at "
        "base) flank a narrow dark gap; a flat horizontal amber lintel "
        "bridges the tops. The overall shape reads as a Π / torii / "
        "gate glyph. Strong bilateral symmetry. One color (amber) "
        "against the background. Nothing else.",
        "Pack background: warm obsidian #1a1412, one flat fill "
        "edge-to-edge.",
    ),
    (
        "flowers",
        "Flowers Pack",
        "SIGIL: a single bold stylized 5-petal plum blossom, flat "
        "pink (#f2a6c0), centered, with a small flat gold (#e8c46a) "
        "round center. Five rounded petals arranged symmetrically "
        "around the center, each petal a simple convex flat shape. "
        "The blossom occupies about 55% of the sigil area. One "
        "blossom only — no grid, no scene, no scatter.",
        "Pack background: plum-black #1c0f1e, one flat fill edge-to-edge.",
    ),
    (
        "bamboo_grove",
        "Bamboo Grove",
        "SIGIL: a single upright bamboo stalk silhouette, flat jade-"
        "green (#3aa84e), centered, running most of the sigil area's "
        "height. The stalk is segmented by 4–5 thin darker green "
        "(#1a6a2e) horizontal bands at even intervals. Two small flat "
        "jade-green pointed leaf shapes angle outward from near the "
        "top. One stalk only — no grove, no scene.",
        "Pack background: deep forest-black #0a1a0e, one flat fill "
        "edge-to-edge.",
    ),
    (
        "coin_cache",
        "Coin Cache",
        "SIGIL: a single bold round Chinese cash-coin silhouette, "
        "flat gold (#e8c46a), centered, occupying about 55% of the "
        "sigil area. A square hole is punched out of the center, "
        "revealing the burgundy-black pack background through it. "
        "The coin's outer edge is a clean circle; the inner square "
        "is axis-aligned. One coin only — no cascade, no chest.",
        "Pack background: burgundy-black #1a0e12, one flat fill "
        "edge-to-edge.",
    ),
    (
        "scroll_library",
        "Scroll Library",
        "SIGIL: a single partly-unrolled scroll silhouette, flat "
        "cream (#f2ead6), centered, horizontally oriented. Two flat "
        "cream cylindrical rolls sit at the left and right ends; "
        "between them a flat cream rectangular sheet spans across, "
        "slightly taller than the rolls. Thin amber (#e8b86a) end-caps "
        "on each roll. The sheet is blank — NO text, NO script, NO "
        "glyphs. One scroll only.",
        "Pack background: sepia-black #1a140a, one flat fill "
        "edge-to-edge.",
    ),
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(sigil: str, bg_color: str) -> str:
    return (
        f"{STYLE_PREFIX}\n"
        f"{bg_color}\n\n"
        f"{sigil}\n\n"
        "Reminder: shared frame (rounded 2:3 rect, thin gold hairline "
        "border, top center tear-notch, bottom gold hairline bar with "
        "empty title region below) must be present. Flat fills only. "
        "No gradients, no shading, no textures, no text, no script."
    )


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

    for idx, (slug, name, sigil, bg_color) in targets:
        prompt = build_prompt(sigil, bg_color)
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
