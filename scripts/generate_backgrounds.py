#!/usr/bin/env python3
"""
Generate background art for Mahjuro scenes using OpenAI's image generation API.

Usage:
    pip install openai requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_backgrounds.py              # Generate all backgrounds
    python scripts/generate_backgrounds.py --bg 1       # Generate only background #1
    python scripts/generate_backgrounds.py --list       # List all backgrounds
    python scripts/generate_backgrounds.py --dry-run    # Print prompts without generating
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


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "backgrounds"

# Shared style prefix. The game uses a dark-mode palette with warm gold accents:
#   - Clear color: ~#0F1219 (dark navy-slate)
#   - Gold text:   ~#FFF2B3
#   - Panel blues:  ~#265A8C
#   - Tile face:   ivory ~#F2EBD9
# Backgrounds must stay dark enough (value < 0.25) so UI layers remain readable.
STYLE_PREFIX = (
    "Digital illustration, dark moody atmosphere, suitable as a video game background. "
    "Very dark overall (most pixels near-black or deep navy), so that bright UI text "
    "and game elements remain readable on top. "
    "Color palette: deep slate blues (#0A1020, #151D2E), warm gold accents (#FFD54F, #C9A84C), "
    "muted teal highlights (#2A5A5A). No text, no logos, no UI elements. "
    "Subtle grain/noise texture. Landscape orientation, 1536×1024."
)

# Each background: (filename, short_name, visual prompt, palette hint)
BACKGROUNDS = [
    (
        "menu_bg",
        "Main Menu",
        "An atmospheric top-down view of scattered mahjong tiles on a dark wooden table, "
        "partially obscured by shadow. Tiles are ivory-colored with faint suit markings "
        "(bamboo, circles, characters). A warm golden light spills from the upper left "
        "corner, casting long dramatic shadows. The tiles fade into deep darkness toward "
        "the edges. A few tiles are stacked, some face-down. The mood is mysterious and "
        "inviting — a game about to begin. Slight dust motes in the golden light beam. "
        "Center area should be relatively clear/dark (menu buttons will overlay there).",
        "Deep navy #0A0E1A, ivory tiles #F2EBD9, warm gold light #FFD54F, "
        "dark wood table #1A1410, shadow blacks.",
    ),
    (
        "gameplay_bg",
        "Gameplay Table",
        "A richly textured dark felt table surface seen from directly above, like a "
        "high-end mahjong table. The felt is deep navy-black with a very subtle woven "
        "texture visible in the weave. A faint circular vignette darkens the corners. "
        "Extremely subtle gold thread lines form a barely-visible geometric border "
        "pattern near the edges — angular, mahjong-inspired motifs. The center is the "
        "darkest and cleanest area (game tiles will be placed here). A very subtle warm "
        "ambient glow from above gives the felt slight depth. Minimalist and elegant — "
        "the table itself should feel premium but never compete with game elements.",
        "Deep navy felt #0C1018, subtle gold thread #3D3422, warm ambient #1A1510, "
        "vignette blacks #050508.",
    ),
    (
        "score_bg",
        "Score Screen",
        "A dramatic dark background with a radiant golden light source at center, "
        "emanating soft warm rays outward like a sunburst or divine light. The rays "
        "are subtle and painterly, not sharp — more like golden fog or bokeh. Faint "
        "silhouettes of mahjong tiles float in the golden haze, as if suspended in "
        "air and slowly rotating. Small sparkle particles drift through the scene. "
        "The edges are very dark (near black). The overall mood is triumphant and "
        "reverent — like opening a treasure chest. The golden center should be bright "
        "enough to feel warm but still dark enough for white/gold text overlay.",
        "Center gold glow #3D2E10 to #FFD54F gradient, surrounding darkness #08090E, "
        "floating tile silhouettes in dark gold #2A2010, sparkle particles #FFE082.",
    ),
]


def build_prompt(visual: str, palette: str) -> str:
    """Combine the shared style prefix with the background-specific description."""
    return f"{STYLE_PREFIX}\n\nScene: {visual}\n\nColor palette: {palette}"


def generate_image(client: OpenAI, prompt: str, output_path: Path, model: str, size: str) -> None:
    """Call the image generation API and save the resulting image."""
    response = client.images.generate(
        model=model,
        prompt=prompt,
        n=1,
        size=size,
        quality="high",
    )

    image_url = response.data[0].url
    if image_url is None:
        import base64
        b64 = response.data[0].b64_json
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
    print(f"  Saved: {output_path} ({len(img_bytes) / 1024:.0f} KB)")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Mahjuro background art via OpenAI")
    parser.add_argument(
        "--bg", type=int, default=None,
        help="Generate only background number N (1-indexed). Omit for all.",
    )
    parser.add_argument(
        "--list", action="store_true",
        help="List all backgrounds and exit.",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--model", type=str, default="gpt-image-1",
        help="Image model to use (default: gpt-image-1).",
    )
    parser.add_argument(
        "--size", type=str, default="1536x1024",
        help="Image size (default: 1536x1024). Options: 1024x1024, 1024x1536, 1536x1024.",
    )
    parser.add_argument(
        "--output-dir", type=str, default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (filename, name, _, _) in enumerate(BACKGROUNDS, 1):
            print(f"  {i}. {name:<20s}  ({filename}.png)")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.bg is not None:
        if args.bg < 1 or args.bg > len(BACKGROUNDS):
            print(f"Error: --bg must be between 1 and {len(BACKGROUNDS)}")
            sys.exit(1)
        targets = [(args.bg - 1, BACKGROUNDS[args.bg - 1])]
    else:
        targets = list(enumerate(BACKGROUNDS))

    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    for idx, (filename, name, visual, palette) in targets:
        prompt = build_prompt(visual, palette)
        output_path = out_dir / f"{filename}.png"

        print(f"\n[{idx + 1}/{len(BACKGROUNDS)}] {name}")

        if args.dry_run:
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists():
            print(f"  Skipping (already exists): {output_path}")
            print(f"  Delete the file to regenerate, or use --bg {idx + 1}")
            continue

        try:
            generate_image(client, prompt, output_path, args.model, args.size)
        except Exception as e:
            print(f"  Error generating {name}: {e}")
            continue

        # Rate-limit courtesy.
        if len(targets) > 1:
            time.sleep(2)

    print("\nDone!")
    if not args.dry_run:
        print(f"Images saved to: {out_dir}")


if __name__ == "__main__":
    main()
