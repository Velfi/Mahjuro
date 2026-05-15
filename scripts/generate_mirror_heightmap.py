#!/usr/bin/env python3
"""
Generate a heightmap texture for the bronze mirror face using Google's
Nano Banana 2 image API, then post-process it into a clean displacement
map (square, grayscale, mid-gray background, raised relief in white,
recessed cells in black).

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
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python scripts/generate_mirror_heightmap.py
    python scripts/generate_mirror_heightmap.py --raw      # also keep the raw model output
    python scripts/generate_mirror_heightmap.py --dry-run  # print the prompt only
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
DEFAULT_NAME = "mirror_heightmap"


PROMPT = (
    "A flat, top-down grayscale HEIGHTMAP texture of an authentic ancient "
    "Chinese cast-bronze mirror (銅鏡, tóngjìng) in the Han / Tang dynasty "
    "four-spirit (四神鏡, sìshén-jìng) tradition. This is a DISPLACEMENT "
    "MAP for 3D rendering, not a picture — tonal value is read as literal "
    "surface height, so the tonal range must be wide and the regions must "
    "be crisp. Think of it as a deeply-cast bronze relief, not a shallow "
    "rubbing: the raised elements sit boldly proud of the field, not just "
    "a hair above it.\n"
    "Composition, working from the center outward:\n"
    "(1) a tall round central boss / knob in the exact middle (the cord "
    "loop on a real mirror), the HIGHEST point on the whole mirror — pure "
    "white (#ffffff) with a thin sharp dark recess ring around its base.\n"
    "(2) a square frame around the central boss, with the so-called TLV "
    "pattern — short T, L, and V shaped marks placed at the cardinal and "
    "ordinal positions around the square — all crisply raised in pure "
    "white, each mark cleanly separated from the field.\n"
    "(3) the four directional guardian spirits cast in bold relief, each "
    "in their own quadrant around the square: Azure Dragon (long sinuous "
    "dragon, east), White Tiger (crouching tiger, west), Vermilion Bird "
    "(long-tailed phoenix-like bird, south), and Black Tortoise-Snake "
    "(turtle entwined with a snake, north). All four creatures in near-"
    "pure-white (#f0–#ff) with strong silhouettes and subtle internal "
    "tonal variation (slightly lower gray inside body/limb divisions so "
    "musculature, feathers, scales are legible as height variation, not "
    "flat cutouts). Each creature is surrounded by a thin dark recess "
    "moat (#3a3a3a) that separates it from the field — the casting groove "
    "around the figure — so they read as sculpted, not stickered on.\n"
    "(4) a ring of vigorous stylized cloud scrolls (雲氣紋) between the "
    "guardians and the rim, raised bright white, with small dark recess "
    "gaps between each scroll for definition.\n"
    "(5) a plain raised outer rim band in pure white, bounded on its "
    "inner edge by a thin dark recess groove that separates rim from "
    "cloud-scroll ring.\n"
    "Tonal key (STRICT — treat as discrete height plateaus, not gradients):\n"
    "  - #ffffff pure white: central boss, TLV marks, outer rim band, "
    "cloud-scroll ring highs, guardian silhouette peaks.\n"
    "  - #e0–#f0 near-white: secondary highs inside the guardian figures.\n"
    "  - #808080 flat mid-gray: the mirror field between elements — "
    "must be a clean uniform plateau with no blotches, no vignetting, "
    "no texture noise, no gradient.\n"
    "  - #3a3a3a dark gray: the thin recess grooves that outline every "
    "raised element (boss base, guardian moats, rim inner edge, cloud "
    "scroll gaps) — these recesses are what give the casting its depth.\n"
    "  - #000000 pure black: only the circular area outside the round "
    "mirror silhouette.\n"
    "Render as a flat orthographic tonal heightfield where lighter = "
    "higher and darker = lower. No shading, no cast shadows, no specular, "
    "no perspective, no paper texture, no rubbing grain. The whole image "
    "is the heightmap — no surrounding page, mount, caption, border, or "
    "annotation exists outside the mirror silhouette. The only imagery "
    "is the five pictorial elements listed above; the relief is purely "
    "pictorial with no inscribed writing of any kind on the face. "
    "Guardians and TLV marks are crisp and legible. Square 1:1 framing, "
    "mirror centered and nearly filling the frame with a small uniform "
    "margin."
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


def postprocess_heightmap(
    raw_bytes: bytes,
    out_size: int,
    exaggerate: float = 1.6,
) -> Image.Image:
    """Normalize the model's output into a clean heightmap.

    Steps: convert to grayscale, auto-stretch contrast so the darkest pixel
    is 0 and the brightest is 255, square-crop to the centered subject,
    apply an S-curve that pushes values away from mid-gray (so raised
    elements read as higher and recessed grooves read as deeper), and
    resize to the requested output resolution. The resulting image can be
    sampled directly as a displacement / parallax texture.

    `exaggerate` is the S-curve exponent applied to each pixel's signed
    distance from 128: 1.0 is the identity, >1 pushes values outward. 1.6
    roughly doubles the displacement amplitude of subjects vs. the field
    while leaving the field itself at ~128.
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

    if exaggerate != 1.0:
        img = _exaggerate_relief(img, exponent=exaggerate)

    if img.size != (out_size, out_size):
        img = img.resize((out_size, out_size), Image.LANCZOS)

    return img


def _exaggerate_relief(img: Image.Image, exponent: float) -> Image.Image:
    """Push each pixel further from mid-gray via `sign(d) * |d|**(1/exponent)`
    on the normalized signed distance `d = (v - 128) / 127`. Keeps 0, 128,
    and 255 fixed; expands values in between so the subject-vs-field
    contrast is larger but pure white and pure black still clip to the
    endpoints.
    """
    inv_exp = 1.0 / max(exponent, 1e-3)
    lut = []
    for v in range(256):
        d = (v - 128) / 127.0
        d = max(-1.0, min(1.0, d))
        sign = 1.0 if d >= 0.0 else -1.0
        pushed = sign * (abs(d) ** inv_exp)
        out = 128.0 + pushed * 127.0
        lut.append(max(0, min(255, int(round(out)))))
    return img.point(lut)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate a Chinese bronze mirror heightmap via Google Nano Banana 2"
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
    parser.add_argument(
        "--exaggerate",
        type=float,
        default=1.6,
        help=(
            "S-curve exponent applied around mid-gray to exaggerate the "
            "relief amplitude. 1.0 disables the push; 1.6 (default) ~doubles "
            "subject-vs-field contrast; 2.0+ is very bold."
        ),
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

    print(f"Generating bronze mirror heightmap → {out_path}")
    raw_bytes = fetch_image_bytes(client, prompt, args.model, args.size)

    if args.raw:
        raw_path = out_dir / f"{args.name}_raw.png"
        raw_path.write_bytes(raw_bytes)
        print(f"  raw model output → {raw_path}")

    heightmap = postprocess_heightmap(raw_bytes, args.out_size, exaggerate=args.exaggerate)
    heightmap.save(out_path, "PNG", optimize=True)
    print(f"  cleaned heightmap → {out_path}  ({args.out_size}x{args.out_size}, L)")


if __name__ == "__main__":
    main()
