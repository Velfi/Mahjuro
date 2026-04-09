#!/usr/bin/env python3
"""
Generate a heightmap texture for the bronze mirror face using OpenAI's
image API, then post-process it into a clean displacement map (square,
grayscale, mid-gray background, raised relief in white, recessed cells
in black).

Style direction: an authentic ancient Chinese cast-bronze mirror (銅鏡,
tóngjìng) in the tradition of Han and Tang dynasty work — specifically
a four-spirit mirror (四神鏡, sìshén-jìng) with the four directional
guardians cast in low relief around a central knob:

  * Azure Dragon  (青龍, East)
  * White Tiger   (白虎, West)
  * Vermilion Bird (朱雀, South)
  * Black Tortoise-Snake (玄武, North)

A small round central boss (the nipple/knob the cord would have passed
through) sits at the middle, ringed by the TLV pattern and a band of
auspicious cloud scrolls. The outer rim is a plain raised band.

The relief is shallow and worn, as if cast in bronze and buried for
fifteen hundred years. The model is asked to return a stone-rubbing
(拓本, taku-hon) grayscale image: pure tonal heightfield, no shading,
no perspective, no lighting. We then auto-level + center it so it
drops straight into a displacement / parallax map slot on the
`build_mirror_mesh` face plate (src/render/mirror_mesh.rs).

Usage:
    pip install openai pillow requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_mirror_heightmap.py
    python scripts/generate_mirror_heightmap.py --raw      # also keep the raw model output
    python scripts/generate_mirror_heightmap.py --dry-run  # print the prompt only
"""

import argparse
import base64
import os
import sys
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai pillow")
    sys.exit(1)

try:
    from PIL import Image, ImageOps
except ImportError:
    print("Error: pillow package not installed. Run: pip install pillow")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "textures"
DEFAULT_NAME = "mirror_heightmap"


PROMPT = (
    "A flat, top-down grayscale HEIGHTMAP texture of an authentic ancient "
    "Chinese cast-bronze mirror (銅鏡, tóngjìng) in the Han / Tang dynasty "
    "four-spirit (四神鏡, sìshén-jìng) tradition. "
    "Composition, working from the center outward: "
    "(1) a small round central boss / knob in the exact middle (the cord "
    "loop on a real mirror), pure white. "
    "(2) a square frame around the central boss, with the so-called TLV "
    "pattern — short T, L, and V shaped marks placed at the cardinal and "
    "ordinal positions around the square — all raised in pure white. "
    "(3) the four directional guardian spirits cast in low relief, each "
    "in their own quadrant around the square: Azure Dragon (long sinuous "
    "dragon, east), White Tiger (crouching tiger, west), Vermilion Bird "
    "(long-tailed phoenix-like bird, south), and Black Tortoise-Snake "
    "(turtle entwined with a snake, north). All four creatures pure white "
    "low-relief silhouettes, facing inward toward the center, evenly "
    "spaced. "
    "(4) a thin ring of stylized cloud scrolls between the guardians and "
    "the rim, raised in white. "
    "(5) a plain raised outer rim band, pure white. "
    "The flat mirror field between these elements is mid-gray (#808080). "
    "The area outside the round mirror is pure black. "
    "Render this as a STONE RUBBING (拓本, taku-hon) — flat orthographic, "
    "no perspective, no lighting, no shadows, no specular highlights, no "
    "color tint, no metallic sheen, no patina color. Just a clean tonal "
    "heightfield where lighter = higher and darker = lower. "
    "Slight age-worn surface variation in the field (very subtle) is fine, "
    "but the four guardians and the TLV marks must remain crisp and "
    "legible. "
    "Square 1:1 framing with the mirror centered and filling ~95% of the "
    "frame. "
    "No text labels, no Chinese characters, no captions, no borders, no "
    "watermarks, no signatures."
)


def build_prompt() -> str:
    return PROMPT


def fetch_image_bytes(client: OpenAI, prompt: str, model: str, size: str) -> bytes:
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
            raise RuntimeError("API returned neither url nor b64_json")
        return base64.b64decode(b64)
    import requests

    r = requests.get(image_url, timeout=120)
    r.raise_for_status()
    return r.content


def postprocess_heightmap(raw_bytes: bytes, out_size: int) -> Image.Image:
    """Normalize the model's output into a clean heightmap.

    Steps: convert to grayscale, auto-stretch contrast so the darkest pixel
    is 0 and the brightest is 255, square-crop to the centered subject, and
    resize to the requested output resolution. The resulting image can be
    sampled directly as a displacement / parallax texture.
    """
    from io import BytesIO

    img = Image.open(BytesIO(raw_bytes)).convert("L")

    # Square-center crop in case the model returned a non-square image.
    w, h = img.size
    side = min(w, h)
    left = (w - side) // 2
    top = (h - side) // 2
    img = img.crop((left, top, left + side, top + side))

    # Stretch contrast so the histogram covers the full 0..255 range.
    # 1% cutoff on each end suppresses any stray near-black/near-white
    # noise pixels that would otherwise pin the auto-level.
    img = ImageOps.autocontrast(img, cutoff=1)

    if img.size != (out_size, out_size):
        img = img.resize((out_size, out_size), Image.LANCZOS)

    return img


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate a Chinese bronze mirror heightmap via the OpenAI image API"
    )
    parser.add_argument(
        "--name",
        type=str,
        default=DEFAULT_NAME,
        help=f"Output filename stem (default: {DEFAULT_NAME}).",
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
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
        default="1024x1024",
        help="Generation size passed to the API (default: 1024x1024).",
    )
    parser.add_argument(
        "--out-size",
        type=int,
        default=1024,
        help="Final post-processed texture size in pixels (default: 1024).",
    )
    parser.add_argument(
        "--raw",
        action="store_true",
        help="Also save the unprocessed model output next to the cleaned heightmap.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the prompt without calling the API.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite the output file if it already exists.",
    )
    args = parser.parse_args()

    prompt = build_prompt()

    if args.dry_run:
        print("Prompt:\n")
        print(prompt)
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{args.name}.png"

    if out_path.exists() and not args.force:
        print(f"Refusing to overwrite {out_path} — pass --force to regenerate.")
        sys.exit(1)

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        print("Error: OPENAI_API_KEY environment variable not set.")
        sys.exit(1)

    client = OpenAI(api_key=api_key)

    print(f"Generating bronze mirror heightmap → {out_path}")
    raw_bytes = fetch_image_bytes(client, prompt, args.model, args.size)

    if args.raw:
        raw_path = out_dir / f"{args.name}_raw.png"
        raw_path.write_bytes(raw_bytes)
        print(f"  raw model output → {raw_path}")

    heightmap = postprocess_heightmap(raw_bytes, args.out_size)
    heightmap.save(out_path, "PNG", optimize=True)
    print(f"  cleaned heightmap → {out_path}  ({args.out_size}x{args.out_size}, L)")


if __name__ == "__main__":
    main()
