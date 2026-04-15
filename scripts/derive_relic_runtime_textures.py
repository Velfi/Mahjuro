#!/usr/bin/env python3
"""
Derive **runtime** relic PNGs from OpenAI source renders.

**Inputs**

  - `assets/textures/relics/source/<slug>_object.png` (required)
  - `assets/textures/relics/source/<slug>_height.png` (optional; preferred relief source)

**Outputs**

  - `assets/textures/relics/<slug>.png` — **preferred in-game albedo** (`RelicId::render_texture_path`).
    Alpha channel: silhouette × derived enamel-height encoding (UI / mip-friendly).
    The 3D **shader** reads separate linear relief from `source/<slug>_height.png`
    when present (`src/render/relic_pipeline.rs`); it does not rely on this alpha
    for normal/height on the enamel path.
  - With `--emit-masks`: `source/<slug>_mask.png` — binary silhouette for mesh
    extrusion (`RelicId::source_mask_path`). Pixels are opaque white on black; the
    engine may use luminance if alpha is flat.

**Relief data:** The on-disk heightmap the game uploads as `relief_tex` is
`source/<slug>_height.png` from the art pipeline (or generated offline). This
script does not replace that file; it only uses height to **composite** the
runtime albedo’s alpha when deriving from `_object.png`.

Usage:
    pip install pillow
    python scripts/derive_relic_runtime_textures.py
    python scripts/derive_relic_runtime_textures.py --name kan_drum
    python scripts/derive_relic_runtime_textures.py --emit-masks
    python scripts/derive_relic_runtime_textures.py --force
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image, ImageChops, ImageFilter, ImageOps
except ImportError:
    print("Error: pillow package not installed. Run: pip install pillow")
    sys.exit(1)


ROOT = Path(__file__).resolve().parent.parent
SOURCE_DIR = ROOT / "assets" / "textures" / "relics" / "source"
OUTPUT_DIR = ROOT / "assets" / "textures" / "relics"


def alpha_from_image(img: Image.Image, alpha_threshold: int) -> Image.Image:
    rgba = img.convert("RGBA")
    alpha = rgba.getchannel("A")
    if alpha.getbbox():
        mask = alpha.point(lambda v: 255 if v >= alpha_threshold else 0)
    else:
        # Fallback for non-transparent generations: treat bright subject on
        # relatively plain background as foreground.
        rgb = ImageOps.grayscale(rgba.convert("RGB"))
        mask = rgb.point(lambda v: 255 if v > 24 else 0)
    return mask.filter(ImageFilter.MedianFilter(size=3))


def crop_to_mask(img: Image.Image, mask: Image.Image, padding: int) -> tuple[Image.Image, Image.Image]:
    bbox = mask.getbbox()
    if bbox is None:
        return img, mask
    left = max(0, bbox[0] - padding)
    top = max(0, bbox[1] - padding)
    right = min(img.width, bbox[2] + padding)
    bottom = min(img.height, bbox[3] + padding)
    box = (left, top, right, bottom)
    return img.crop(box), mask.crop(box)


def fit_center(img: Image.Image, mask: Image.Image, out_size: int, fill_ratio: float) -> tuple[Image.Image, Image.Image]:
    src_w, src_h = img.size
    if src_w == 0 or src_h == 0:
        blank = Image.new("RGBA", (out_size, out_size), (0, 0, 0, 0))
        blank_mask = Image.new("L", (out_size, out_size), 0)
        return blank, blank_mask

    max_dim = max(src_w, src_h)
    target_dim = max(1, int(out_size * fill_ratio))
    scale = target_dim / max_dim
    new_w = max(1, int(round(src_w * scale)))
    new_h = max(1, int(round(src_h * scale)))

    img = img.resize((new_w, new_h), Image.LANCZOS)
    mask = mask.resize((new_w, new_h), Image.LANCZOS)

    canvas = Image.new("RGBA", (out_size, out_size), (0, 0, 0, 0))
    canvas_mask = Image.new("L", (out_size, out_size), 0)
    ox = (out_size - new_w) // 2
    oy = (out_size - new_h) // 2
    canvas.paste(img, (ox, oy), img.getchannel("A"))
    canvas_mask.paste(mask, (ox, oy))
    return canvas, canvas_mask


def prepare_object_image(
    source_path: Path,
    alpha_threshold: int,
    out_size: int,
    fill_ratio: float,
    padding: int,
) -> tuple[Image.Image, Image.Image]:
    src = Image.open(source_path).convert("RGBA")
    mask = alpha_from_image(src, alpha_threshold)
    src.putalpha(mask)
    cropped_img, cropped_mask = crop_to_mask(src, mask, padding)
    fitted_img, fitted_mask = fit_center(cropped_img, cropped_mask, out_size, fill_ratio)
    return fitted_img, fitted_mask


def derive_enamel_height(
    fitted_img: Image.Image,
    fitted_mask: Image.Image,
) -> Image.Image:
    binary_mask = fitted_mask.point(lambda v: 255 if v >= 96 else 0)
    fill = binary_mask.point(lambda v: 184 if v > 0 else 0)

    rgb = fitted_img.convert("RGB")
    quantized = ImageOps.posterize(rgb, 3)
    edges = quantized.filter(ImageFilter.FIND_EDGES).convert("L")
    edges = ImageOps.autocontrast(edges, cutoff=2)
    edges = edges.point(lambda v: 255 if v >= 22 else 0)
    edges = edges.filter(ImageFilter.MaxFilter(size=3))
    edges = ImageChops.multiply(edges, binary_mask)
    edges = edges.point(lambda v: 228 if v > 0 else 0)

    outer_rim = ImageChops.subtract(
        binary_mask,
        binary_mask.filter(ImageFilter.MinFilter(size=9)),
    )
    outer_rim = outer_rim.point(lambda v: 255 if v > 0 else 0)

    inner_rim_seed = binary_mask.filter(ImageFilter.MinFilter(size=15))
    inner_rim = ImageChops.subtract(
        inner_rim_seed.filter(ImageFilter.MaxFilter(size=7)),
        inner_rim_seed,
    )
    inner_rim = inner_rim.point(lambda v: 220 if v > 0 else 0)

    shape_relief = ImageOps.grayscale(quantized)
    shape_relief = ImageOps.autocontrast(shape_relief, cutoff=4)
    shape_relief = shape_relief.point(
        lambda v: 0 if v < 32 else min(210, 160 + int(v * 0.18))
    )
    shape_relief = ImageChops.multiply(shape_relief, binary_mask)

    height = ImageChops.lighter(fill, shape_relief)
    height = ImageChops.lighter(height, inner_rim)
    height = ImageChops.lighter(height, edges)
    height = ImageChops.lighter(height, outer_rim)
    return height.filter(ImageFilter.GaussianBlur(radius=0.75))


def load_source_height(
    source_path: Path,
    out_size: int,
    binary_mask: Image.Image,
) -> Image.Image | None:
    if not source_path.exists():
        return None
    guide = Image.open(source_path).convert("L")
    guide = ImageOps.autocontrast(guide, cutoff=1)
    if guide.size != (out_size, out_size):
        guide = guide.resize((out_size, out_size), Image.LANCZOS)
    guide = guide.point(lambda v: min(235, 128 + int(v * 0.42)))
    guide = ImageChops.multiply(guide, binary_mask)
    return guide


def derive_runtime_texture(
    fitted_img: Image.Image,
    fitted_mask: Image.Image,
    out_path: Path,
    mask_path: Path | None,
    height_alpha: Image.Image,
    force: bool,
) -> None:
    final_alpha = fitted_mask.point(lambda v: 255 if v >= 96 else 0)
    encoded_alpha = ImageChops.multiply(height_alpha, final_alpha.point(lambda v: 255 if v > 0 else 0))

    if out_path.exists() and not force:
        print(f"Skipping existing runtime texture: {out_path.name}")
    else:
        fitted_img.putalpha(encoded_alpha)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        fitted_img.save(out_path, "PNG", optimize=True)
        print(f"Derived runtime texture → {out_path}")

    if mask_path is not None:
        if mask_path.exists() and not force:
            print(f"Skipping existing mask: {mask_path.name}")
        else:
            mask_path.parent.mkdir(parents=True, exist_ok=True)
            final_alpha.save(mask_path, "PNG", optimize=True)
            print(f"Derived mask           → {mask_path}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Derive runtime relic textures from generated source art."
    )
    parser.add_argument("--name", type=str, default=None, help="Only process a single relic slug.")
    parser.add_argument("--source-dir", type=str, default=str(SOURCE_DIR))
    parser.add_argument("--output-dir", type=str, default=str(OUTPUT_DIR))
    parser.add_argument("--out-size", type=int, default=1024)
    parser.add_argument("--fill-ratio", type=float, default=0.82)
    parser.add_argument("--padding", type=int, default=24)
    parser.add_argument("--alpha-threshold", type=int, default=16)
    parser.add_argument("--emit-masks", action="store_true")
    parser.add_argument(
        "--blend-source-height",
        action="store_true",
        help="Blend the object-derived enamel relief on top of the primary generated height guide.",
    )
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    source_dir = Path(args.source_dir)
    output_dir = Path(args.output_dir)
    if not source_dir.exists():
        print(f"Error: source directory does not exist: {source_dir}")
        sys.exit(1)

    object_files = sorted(source_dir.glob("*_object.png"))
    if args.name is not None:
        object_files = [p for p in object_files if p.stem == f"{args.name}_object"]

    if not object_files:
        print("No object textures found to derive.")
        return

    for object_path in object_files:
        slug = object_path.stem.removesuffix("_object")
        runtime_out = output_dir / f"{slug}.png"
        # Loader checks `textures/relics/source/<slug>_mask.png` before `relics/<slug>_mask.png`.
        mask_out = (source_dir / f"{slug}_mask.png") if args.emit_masks else None
        fitted_img, fitted_mask = prepare_object_image(
            source_path=object_path,
            alpha_threshold=args.alpha_threshold,
            out_size=args.out_size,
            fill_ratio=args.fill_ratio,
            padding=args.padding,
        )
        binary_mask = fitted_mask.point(lambda v: 255 if v >= 96 else 0)
        enamel_height = derive_enamel_height(fitted_img, fitted_mask)
        source_height = load_source_height(
            source_path=source_dir / f"{slug}_height.png",
            out_size=args.out_size,
            binary_mask=binary_mask,
        )
        height = source_height if source_height is not None else enamel_height
        if args.blend_source_height and source_height is not None:
            height = ImageChops.lighter(height, enamel_height)
        derive_runtime_texture(
            fitted_img=fitted_img,
            fitted_mask=fitted_mask,
            out_path=runtime_out,
            mask_path=mask_out,
            height_alpha=height,
            force=args.force,
        )


if __name__ == "__main__":
    main()
