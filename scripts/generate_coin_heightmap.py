#!/usr/bin/env python3
"""
Generate a heightmap texture for the coin face using Google's Nano
Banana 2 image API, then post-process it into a clean displacement map
(square, grayscale, mid-gray background, raised features in white,
recessed features in black).

Style direction: an authentic old Chinese cash coin (方孔錢, fāngkǒngqián) —
the round bronze coin with a square hole that was the standard currency
across most of imperial China from the Qin dynasty (~221 BCE) up through
the late Qing. The face carries four traditional Chinese seal-script
characters arranged top-bottom-right-left around the central square hole.
The relief is shallow and worn, as if struck in bronze and pocket-rubbed
for a few hundred years.

The model is asked to return a stone-rubbing-style grayscale image
(taku-hon / 拓本): pure tonal heightfield, no shading, no perspective,
no lighting. We then auto-level + center it so it drops straight into
a displacement / parallax map slot.

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python scripts/generate_coin_heightmap.py
    python scripts/generate_coin_heightmap.py --raw      # also keep the raw model output
    python scripts/generate_coin_heightmap.py --dry-run  # print the prompt only
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import (  # noqa: E402
    DEFAULT_MODEL,
    generate_image_bytes,
    init_client,
    parse_size,
)

try:
    from PIL import Image, ImageOps
except ImportError:
    print("Error: pillow package not installed. Run: pip install pillow")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "textures"
DEFAULT_NAME = "coin_heightmap"


PROMPT = (
    "A flat, top-down grayscale HEIGHTMAP texture of an authentic ancient "
    "Chinese bronze cash coin (fāngkǒngqián, 方孔錢) — the classic round "
    "coin with a square hole in the center, used across imperial China "
    "from the Qin through the Qing dynasty. "
    "Four traditional Chinese seal-script (篆書) characters sit on the coin "
    "face, one each above, below, right, and left of the central square hole, "
    "in the historical reading order of a real Tang or Song dynasty coin "
    "(for example 開元通寶 Kāiyuán Tōngbǎo or 乾隆通寶 Qiánlóng Tōngbǎo). "
    "These four characters are part of the coin relief itself — they are the "
    "only textual content in the image and they must be rendered as raised "
    "pure-white strokes on the coin face. "
    "Tonal key (strict, flat regions, no gradients):\n"
    "  - Pure white: the four seal-script characters and the raised inner "
    "and outer rims.\n"
    "  - Mid-gray (#808080): the flat coin field between the rims.\n"
    "  - Pure black: only the square center hole and the area outside the "
    "round coin silhouette.\n"
    "Render as a stone rubbing (拓本, taku-hon): flat orthographic tonal "
    "heightfield where lighter = higher and darker = lower. The whole image "
    "is the rubbing — no surrounding page, mount, caption, border, or "
    "annotation exists outside the coin silhouette. "
    "Slight age-worn surface variation in the field is fine, but the four "
    "characters must remain crisp and legible. "
    "Square 1:1 framing, coin centered and nearly filling the frame with a "
    "small uniform margin."
)


def build_prompt() -> str:
    return PROMPT


def fetch_image_bytes(client, prompt: str, model: str, size: str) -> bytes:
    aspect_ratio, image_size = parse_size(size)
    return generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio=aspect_ratio,
        image_size=image_size,
    )


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
        description="Generate a Chinese cash-coin heightmap via Google Nano Banana 2"
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
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1:1@1K",
        help=(
            "Generation size — Gemini ASPECT@TIER (default: 1:1@1K). "
            "Legacy WxH like '1024x1024' is auto-translated."
        ),
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

    client = init_client()

    print(f"Generating coin heightmap → {out_path}")
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
