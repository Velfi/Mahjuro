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

Outputs `pack_<slug>.png` into assets/textures/tile_packs/.

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python3 scripts/generate_pack_covers.py                # all missing
    python3 scripts/generate_pack_covers.py --force        # regenerate all
    python3 scripts/generate_pack_covers.py --name honors  # one by slug
    python3 scripts/generate_pack_covers.py --list         # list all
    python3 scripts/generate_pack_covers.py --dry-run      # prompts only
"""

import argparse
import io
import json
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

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)


REPO_ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = REPO_ROOT / "assets" / "textures" / "tile_packs"
PALETTE_JSON = REPO_ROOT / "tools" / "pack_palette.json"
FINAL_SIZE = (256, 384)  # tall portrait, matches a booster-pack aspect ratio


def _rgba_to_hex(rgba: list[float]) -> str:
    """`[0..1, 0..1, 0..1, _]` → `"#rrggbb"` for prompt interpolation."""
    r, g, b = (max(0, min(255, round(c * 255))) for c in rgba[:3])
    return f"#{r:02x}{g:02x}{b:02x}"


# Loaded once at module import; the runtime palette and the prompt
# strings can't drift because they share this single source of truth.
_PALETTE = json.loads(PALETTE_JSON.read_text(encoding="utf-8"))


def _bg_phrase_for(slug: str) -> str:
    """Synthesize the `Pack background:` sentence from the canonical
    palette JSON. The descriptive name ("deep navy", "warm obsidian",
    …) is kept alongside the hex so the prompt stays semantically rich.
    """
    entry = _PALETTE["packs"][slug]
    return (
        f"Pack background: {entry['bg_name']} {_rgba_to_hex(entry['bg'])}, "
        "one flat fill edge-to-edge."
    )

# ---------------------------------------------------------------------------
# Shared style
# ---------------------------------------------------------------------------

STYLE_PREFIX = (
    "A booster-pack cover for a mahjong roguelite video game. Tall "
    "vertical portrait, 2:3 aspect. 'Flat Icon Sigil' style: one bold "
    "centered sigil on a flat pack-color background, wrapped in a "
    "shared frame. Flat vector look, like a modern trading-card pack "
    "icon (think MTG mana symbols, Slay the Spire relics, Balatro "
    "booster packs). Orthographic front-on view, no perspective. The "
    "image IS the pack — the whole frame is the pack surface, no "
    "tabletop, mockup, or drop shadow beneath it.\n\n"
    "Shared frame (identical across every pack — this is the 'brand'):\n"
    "- The full image is a rounded-corner rectangle, 2:3 portrait, "
    "filling the frame edge-to-edge with the pack-specific background "
    "color.\n"
    "- A thin gold (#d9b35a) hairline border sits just inside the "
    "rounded edge, a couple of pixels thick, evenly offset from the "
    "outer edge.\n"
    "- A small V-shaped tear notch is cut into the top edge, centered "
    "horizontally — a subtle visual cue that this is a pack to open.\n"
    "- A short gold (#d9b35a) hairline bar spans the bottom quarter "
    "of the frame, dividing the sigil area above from a blank title "
    "bar region below. The title-bar region is left as pure background "
    "color — the engine overlays a real title later, so the image "
    "itself shows no writing there.\n"
    "- The sigil sits in the upper three-quarters of the frame, "
    "centered horizontally, comfortably inset from the gold border.\n\n"
    "Hard style rules — non-negotiable:\n"
    "- Every region is a single flat RGB value with crisp hard edges "
    "between regions. Solid color fills only — surfaces must look like "
    "vector cutouts, like construction paper, not like anything painted, "
    "rendered, or photographed.\n"
    "- The sigil is silhouette-first — identifiable from its shape "
    "alone at 32×32. Bold, simple, iconic. One symbol, not a scene or "
    "a detailed illustration.\n"
    "- Purely pictorial decoration only. No written glyphs, numerals, "
    "or logos of any language anywhere in the image; any shape that "
    "resembles a letter or number must be replaced with a plain "
    "geometric mark.\n"
    "- Small unifying flourish allowed: a few tiny flat gold (#d9b35a) "
    "dots drifting in negative space near the sigil. That's it.\n"
)

# ---------------------------------------------------------------------------
# Per-pack definitions
# ---------------------------------------------------------------------------

# Each pack owns ONE background color and ONE sigil. Silhouette-first:
# the sigil must be identifiable at 32×32. No scenes, no painterly detail.
#
# Background colors come from `tools/pack_palette.json` via
# `_bg_phrase_for(slug)`; only the per-pack sigil description lives here.
#
# (slug, display_name, sigil_prompt)
_SIGILS = [
    (
        "honors",
        "Honors Pack",
        "Sigil: a single bold 8-pointed compass rose, flat ivory "
        "(#f2ead6), centered. Four long cardinal points and four "
        "shorter diagonal points, all crisp straight triangles meeting "
        "at a small ivory circle at the center. The compass is large "
        "and centered, filling most of the sigil area.",
    ),
    (
        "terminals",
        "Terminals Pack",
        "Sigil: a simple symmetric gateway silhouette in flat amber "
        "(#e8a84a). Two thick vertical trapezoidal pillars (wider at "
        "base) flank a narrow dark gap; a flat horizontal amber lintel "
        "bridges the tops. The overall shape reads as a torii gate. "
        "Strong bilateral symmetry. One amber color against the "
        "background, nothing else.",
    ),
    (
        "flowers",
        "Flowers Pack",
        "Sigil: a single bold stylized five-petal plum blossom, flat "
        "pink (#f2a6c0), centered, with a small flat gold (#e8c46a) "
        "round center. Five rounded petals arranged symmetrically "
        "around the center, each petal a simple convex flat shape. "
        "The blossom is large and centered, filling most of the sigil "
        "area — a single blossom, not a spray or pattern.",
    ),
    (
        "bamboo_grove",
        "Bamboo Grove",
        "Sigil: a single upright bamboo stalk silhouette, flat "
        "jade-green (#3aa84e), centered, running the full height of "
        "the sigil area. The stalk is segmented by a handful of thin "
        "darker green (#1a6a2e) horizontal bands at even intervals. A "
        "pair of small flat jade-green pointed leaf shapes angle "
        "outward from near the top. A single stalk, not a grove.",
    ),
    (
        "coin_cache",
        "Coin Cache",
        "Sigil: a single bold round Chinese cash-coin silhouette, "
        "flat gold (#e8c46a), centered, large and filling most of the "
        "sigil area. A square hole is punched out of the center, "
        "revealing the burgundy-black pack background through it. The "
        "coin's outer edge is a clean circle; the inner square is "
        "axis-aligned. A single coin, not a pile.",
    ),
    (
        "scroll_library",
        "Scroll Library",
        "Sigil: a single partly-unrolled scroll silhouette, flat "
        "cream (#f2ead6), centered, horizontally oriented. Two flat "
        "cream cylindrical rolls sit at the left and right ends; "
        "between them a flat cream rectangular sheet spans across, "
        "slightly taller than the rolls. Thin amber (#e8b86a) end-caps "
        "on each roll. The sheet surface is a single uniform flat "
        "cream fill with nothing on it. A single scroll, not a stack.",
    ),
]


# (slug, display_name, sigil_prompt, background_phrase) — the bg phrase
# is synthesized from the canonical palette JSON so it can't drift.
PACKS = [
    (slug, name, sigil, _bg_phrase_for(slug))
    for slug, name, sigil in _SIGILS
]


# ---------------------------------------------------------------------------
# Prompt building
# ---------------------------------------------------------------------------

def build_prompt(sigil: str, bg_color: str) -> str:
    return (
        f"{STYLE_PREFIX}\n"
        f"{bg_color}\n\n"
        f"{sigil}"
    )


# ---------------------------------------------------------------------------
# Image generation + post-processing
# ---------------------------------------------------------------------------

def generate_image(client, prompt: str, model: str, size: str) -> Image.Image:
    """Call Gemini and return a PIL Image (RGBA)."""
    aspect_ratio, image_size = parse_size(size)
    img_bytes = generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio=aspect_ratio,
        image_size=image_size,
    )
    return Image.open(io.BytesIO(img_bytes)).convert("RGBA")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro tile-pack cover art via Google Nano Banana 2"
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
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="2:3@2K",
        help=(
            "Generation size — Gemini ASPECT@TIER (default: 2:3@2K). "
            "Legacy WxH like '1024x1536' is auto-translated to the closest "
            "Gemini aspect/size."
        ),
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
        client = init_client()

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
