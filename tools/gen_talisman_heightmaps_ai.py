#!/usr/bin/env python3
"""Generate talisman heightmap PNGs via OpenAI image generation.

Run:
    OPENAI_API_KEY=sk-... python3 tools/gen_talisman_heightmaps_ai.py

    # Or generate a single talisman:
    OPENAI_API_KEY=sk-... python3 tools/gen_talisman_heightmaps_ai.py jade

Outputs 256x256 grayscale PNGs into assets/textures/ with the same names
as the procedural generator:
    talisman_jade.png
    talisman_pearl.png
    talisman_gilded.png
    talisman_polychrome.png
    talisman_kiln.png

Each image is converted to grayscale, resized to 256x256, and masked to
an octagonal silhouette so the shader's normal-perturbation reads clean
edges. Mid-gray (128) is the neutral surface plane.

Requires:
    pip install openai Pillow
"""

import argparse
import base64
import io
import math
import os
import sys

from openai import OpenAI
from PIL import Image, ImageFilter

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

SIZE = 256
MID = 128  # neutral gray (uint8)
OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "textures")

# System-level style guidance prepended to every prompt.
STYLE_PREFIX = (
    "A top-down 256x256 grayscale heightmap texture on a pure mid-gray "
    "(#808080) background. The image represents a carved stone tablet in an "
    "octagonal shape. Bright pixels are raised surfaces; dark pixels are "
    "recessed/carved areas. The relief should be subtle and smooth, suitable "
    "for normal-map derivation. No text, no color, no perspective — pure "
    "flat orthographic heightfield. "
)

# Per-talisman prompts describing the motif to carve into the tablet.
TALISMAN_PROMPTS: dict[str, str] = {
    "jade": (
        "The motif is a Chinese jade bi-disc pendant with concentric rings "
        "and traditional cloud-scroll (ruyi) carvings. A central circular "
        "hole is recessed. Double raised rims frame the octagonal border. "
        "The surface has the soft, waxy luster of carved nephrite jade."
    ),
    "pearl": (
        "The motif is concentric luster ripples radiating from the center, "
        "like the nacre layers of a pearl cross-section. The ripples have "
        "slight organic irregularity and wobble. A gentle dome rises at the "
        "center. A single raised rim frames the octagonal border."
    ),
    "gilded": (
        "The motif is an ornate hammered-gold filigree lattice in a diamond "
        "pattern (rotated 45 degrees). Raised nodes sit at each lattice "
        "intersection. A double raised border frames the octagon. The surface "
        "has the bumpy, hand-hammered texture of beaten gold leaf."
    ),
    "polychrome": (
        "The motif is a prismatic starburst with 8 sharp radial rays "
        "emanating from a central faceted gemstone. A concentric ring pulse "
        "sits at mid-radius. A faint 5-pointed star overlays the rays. "
        "The surface is crisp and crystalline, like cut glass."
    ),
    "kiln": (
        "The motif is a cracked kiln-fired clay surface. Radial cracks "
        "emanate from the center like a shattered ceramic glaze. A central "
        "flame or fire symbol is raised above the cracked surface. The "
        "texture has rough, baked-clay grain. A single raised rim frames "
        "the octagonal border."
    ),
}

# ---------------------------------------------------------------------------
# Octagonal mask (matches the procedural generator exactly)
# ---------------------------------------------------------------------------


def _smoothstep(edge0: float, edge1: float, x: float) -> float:
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3.0 - 2.0 * t)


def _oct_dist(x: float, y: float) -> float:
    ax, ay = abs(x), abs(y)
    return ax * 0.9239 + ay * 0.3827 if ax > ay else ay * 0.9239 + ax * 0.3827


def make_octagonal_mask() -> Image.Image:
    """Return a SIZE x SIZE L-mode image: 255 inside the octagon, 0 outside."""
    mask = Image.new("L", (SIZE, SIZE), 0)
    px = mask.load()
    for y in range(SIZE):
        for x in range(SIZE):
            u = (x / (SIZE - 1)) * 2.0 - 1.0
            v = (y / (SIZE - 1)) * 2.0 - 1.0
            d = _oct_dist(u, v)
            a = _smoothstep(0.94, 0.88, d)
            px[x, y] = int(a * 255 + 0.5)
    return mask


# ---------------------------------------------------------------------------
# OpenAI image generation
# ---------------------------------------------------------------------------


def generate_heightmap(client: OpenAI, kind: str) -> Image.Image:
    """Call the OpenAI API and return a PIL Image (grayscale, SIZE x SIZE)."""
    prompt = STYLE_PREFIX + TALISMAN_PROMPTS[kind]

    print(f"  [{kind}] requesting image from OpenAI...")
    response = client.images.generate(
        model="gpt-image-1",
        prompt=prompt,
        n=1,
        size="1024x1024",
        quality="high",
    )

    # gpt-image-1 returns base64 by default
    b64 = response.data[0].b64_json
    img_bytes = base64.b64decode(b64)
    img = Image.open(io.BytesIO(img_bytes))

    # Convert to grayscale and resize to target dimensions.
    img = img.convert("L")
    img = img.resize((SIZE, SIZE), Image.LANCZOS)

    return img


def apply_mask(img: Image.Image, mask: Image.Image) -> Image.Image:
    """Blend the heightmap toward mid-gray outside the octagonal mask."""
    neutral = Image.new("L", (SIZE, SIZE), MID)
    return Image.composite(img, neutral, mask)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(
        description="Generate talisman heightmaps via OpenAI image generation."
    )
    parser.add_argument(
        "kinds",
        nargs="*",
        choices=list(TALISMAN_PROMPTS.keys()) + [[]],
        default=[],
        help="Which talisman(s) to generate. Omit for all.",
    )
    parser.add_argument(
        "--no-mask",
        action="store_true",
        help="Skip the octagonal mask (useful for previewing raw output).",
    )
    args = parser.parse_args()

    kinds = args.kinds if args.kinds else list(TALISMAN_PROMPTS.keys())

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        print("Error: set OPENAI_API_KEY environment variable.", file=sys.stderr)
        sys.exit(1)

    client = OpenAI(api_key=api_key)
    mask = make_octagonal_mask()

    os.makedirs(OUT_DIR, exist_ok=True)
    print(f"Generating talisman heightmaps for: {', '.join(kinds)}")

    for kind in kinds:
        img = generate_heightmap(client, kind)

        if not args.no_mask:
            img = apply_mask(img, mask)

        out_path = os.path.join(OUT_DIR, f"talisman_{kind}.png")
        img.save(out_path)
        print(f"  wrote {out_path}  ({os.path.getsize(out_path)} bytes)")

    print("Done.")


if __name__ == "__main__":
    main()
