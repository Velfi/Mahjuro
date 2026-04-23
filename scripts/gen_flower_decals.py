#!/usr/bin/env python3
"""Generate flower tile face decal PNGs via OpenAI image generation.

Run:
    OPENAI_API_KEY=sk-... python3 tools/gen_flower_decals.py

    # Or generate a single flower:
    OPENAI_API_KEY=sk-... python3 tools/gen_flower_decals.py plum

Outputs 192x256 RGBA PNGs into assets/textures/:
    flower_1_plum.png
    flower_2_orchid.png
    flower_3_chrysanthemum.png
    flower_4_bamboo.png

Each image is an engraved motif on a transparent background, tinted in the
flower suit's warm pink accent. The tile_3d shader composites these over
the wood albedo the same way it handles the rasterised Unicode decals for
regular suits.

Requires:
    pip install openai Pillow
"""

import argparse
import base64
import io
import os
import sys

from openai import OpenAI
from PIL import Image

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

# Match the tile face decal dimensions used in wgpu_renderer.rs:
# DECAL_W=192, DECAL_H=256 (0.734:1 aspect ratio of the tile face).
WIDTH = 192
HEIGHT = 256

OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "textures")

# Flower suit accent colour — warm pink, matching Suit::Flower::suit_color().
TINT_COLOR = (230, 115, 140)  # roughly [0.90, 0.45, 0.55] in u8

# System-level style guidance prepended to every prompt.
#
# Output is engraved white-on-black — the downstream `tint_image` pass converts
# luminance to alpha, so anything gray or brighter becomes visible decal, and
# the pure black becomes transparent. "Transparent" is intentionally NOT
# requested from the model; we want a flat black fill and let the pipeline
# derive alpha.
STYLE_PREFIX = (
    "A tall vertical illustration of a single botanical motif, drawn in a "
    "traditional East Asian woodblock print / ink-wash style. The motif is "
    "rendered as a monochrome engraved relief: solid white lines and shapes "
    "on a pure flat black background. Centered, front-facing, orthographic, "
    "high contrast, clean carved-tile feel. Fill only with plain black outside "
    "the motif — treat the background as a single flat black region with no "
    "decoration, no lettering, no framing border. "
)

# Per-flower prompts describing the specific botanical motif.
#
# Hanzi are intentionally omitted from the prompt body: including the literal
# characters (梅, 蘭, 菊, 竹) alongside "no lettering" creates a strong summon
# signal that tends to paste the character into the image.
FLOWER_PROMPTS: dict[str, str] = {
    "plum": (
        "A plum blossom branch with 3-5 five-petaled blossoms and "
        "a few gnarled twigs. The flowers are open and facing the viewer. "
        "Sparse, elegant composition typical of Chinese scholar painting."
    ),
    "orchid": (
        "A wild orchid with slender arching leaves and a single "
        "spray of small delicate blooms. The leaves are long, graceful, and "
        "grass-like. Minimalist composition in the Four Gentlemen tradition."
    ),
    "chrysanthemum": (
        "A chrysanthemum flower head with layered radiating petals, "
        "viewed from above. A few serrated leaves frame the bloom below. "
        "Dense but orderly petal arrangement, bold and round."
    ),
    "bamboo": (
        "A bamboo stalk segment with 2-3 nodes and several leaf "
        "clusters. The leaves are narrow, pointed, and arranged in fan-like "
        "sprays. Clean, geometric, calligraphic brushstroke style."
    ),
}

# Map rank to name for filenames.
RANK_NAME = {1: "plum", 2: "orchid", 3: "chrysanthemum", 4: "bamboo"}


# ---------------------------------------------------------------------------
# OpenAI image generation
# ---------------------------------------------------------------------------


def generate_decal(client: OpenAI, kind: str) -> Image.Image:
    """Call the OpenAI API and return a PIL Image (RGBA, WIDTH x HEIGHT)."""
    prompt = STYLE_PREFIX + FLOWER_PROMPTS[kind]

    print(f"  [{kind}] requesting image from OpenAI...")
    response = client.images.generate(
        model="gpt-image-2",
        prompt=prompt,
        n=1,
        size="1024x1024",
        quality="high",
    )

    b64 = response.data[0].b64_json
    img_bytes = base64.b64decode(b64)
    img = Image.open(io.BytesIO(img_bytes))

    # Resize to target dimensions (tall rectangle).
    img = img.resize((WIDTH, HEIGHT), Image.LANCZOS)

    return img


def tint_image(img: Image.Image, color: tuple[int, int, int]) -> Image.Image:
    """Convert to grayscale alpha mask, then tint with the given RGB color.

    The result is an RGBA image where:
    - RGB channels are the tint color everywhere
    - Alpha channel is derived from the original image's luminance
    This matches how the other suit decals work (single-color tinted glyphs
    composited over the wood albedo).
    """
    # Convert to grayscale to get luminance.
    gray = img.convert("L")

    # Create the tinted output.
    r, g, b = color
    out = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    pixels = out.load()
    gray_px = gray.load()

    for y in range(HEIGHT):
        for x in range(WIDTH):
            a = gray_px[x, y]
            pixels[x, y] = (r, g, b, a)

    return out


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(
        description="Generate flower tile decals via OpenAI image generation."
    )
    parser.add_argument(
        "kinds",
        nargs="*",
        choices=list(FLOWER_PROMPTS.keys()) + [[]],
        default=[],
        help="Which flower(s) to generate. Omit for all.",
    )
    parser.add_argument(
        "--no-tint",
        action="store_true",
        help="Skip the pink tint (output raw AI image).",
    )
    args = parser.parse_args()

    kinds = args.kinds if args.kinds else list(FLOWER_PROMPTS.keys())

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        print("Error: set OPENAI_API_KEY environment variable.", file=sys.stderr)
        sys.exit(1)

    client = OpenAI(api_key=api_key)

    os.makedirs(OUT_DIR, exist_ok=True)
    print(f"Generating flower decals for: {', '.join(kinds)}")

    for kind in kinds:
        img = generate_decal(client, kind)

        if not args.no_tint:
            img = tint_image(img, TINT_COLOR)

        rank = [r for r, n in RANK_NAME.items() if n == kind][0]
        out_path = os.path.join(OUT_DIR, f"flower_{rank}_{kind}.png")
        img.save(out_path)
        print(f"  wrote {out_path}  ({os.path.getsize(out_path)} bytes)")

    print("Done.")


if __name__ == "__main__":
    main()
