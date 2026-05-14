#!/usr/bin/env python3
"""Generate a new mahjong tile-set atlas via a single OpenAI image edit call.

**Default workflow:** pass a **reference atlas** image (default: the repo’s
procedural layout at `assets/textures/tile_sets/classic/atlas.png`) plus style
instructions; the model restyles the full 9×5 grid in one shot while preserving
layout and tile identity. Output goes to
`assets/textures/tile_sets/<name>/atlas.png` + `atlas.toml`.

Why one call: generating dozens of tiles individually is expensive; one
whole-atlas edit is typically ~$0.04–0.10 and keeps the grid aligned.

Usage:
    pip install openai pillow
    export OPENAI_API_KEY="sk-..."

    # list built-in themes
    python3 scripts/generate_tile_atlas.py --list

    # reference atlas + built-in theme
    python3 scripts/generate_tile_atlas.py --theme jade --name jade

    # default reference + your own prompt (new or restyled set name)
    python3 scripts/generate_tile_atlas.py --name my_theme --force \\
        --prompt "soft watercolor, spring wildflowers, pastel..."

    # use another reference image (same grid geometry)
    python3 scripts/generate_tile_atlas.py --theme lacquer --name lacquer \\
        --template assets/textures/tile_sets/original/atlas.png

    # preview the prompt, don't call the API
    python3 scripts/generate_tile_atlas.py --theme jade --dry-run
"""

from __future__ import annotations

import argparse
import base64
import io
import os
import sys
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent

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


# ---------------------------------------------------------------------------
# Atlas geometry — matches scripts/pack_atlas.py and the loader in decal.rs
# ---------------------------------------------------------------------------

TILE_W = 256
TILE_H = 384
COLUMNS = 9

# One suit per row — the model's grid reasoning is much better when it can
# think "row = suit" rather than tracking mid-row suit boundaries.
LAYOUT = [
    "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9",
    "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9",
    "D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9",
    "EWind", "SWind", "WWind", "NWind", "DRed", "DGreen", "DWhite", "", "",
    "Flower1", "Flower2", "Flower3", "Flower4",
    "Season1", "Season2", "Season3", "Season4", "",
]

ATLAS_W = TILE_W * COLUMNS                            # 2304
ATLAS_H = TILE_H * ((len(LAYOUT) + COLUMNS - 1) // COLUMNS)  # 1920

REPO_ROOT = Path(__file__).resolve().parent.parent
SETS_DIR = REPO_ROOT / "assets" / "textures" / "tile_sets"
# Default reference: procedural atlas shipped in-repo (9×5 layout). Override with --template.
DEFAULT_TEMPLATE = SETS_DIR / "classic" / "atlas.png"


# ---------------------------------------------------------------------------
# Themes — visual treatment applied across the whole set.
# ---------------------------------------------------------------------------

THEMES: dict[str, dict[str, str]] = {
    "jade": {
        "blurb": "carved jade stone tiles",
        "style": (
            "polished translucent green jade stone tile faces with subtle "
            "mineral veining. All design elements are carved in relief, "
            "rendered in cream-white ivory or pale gold against the jade. "
            "Museum-quality imperial Chinese carved jade."
        ),
    },
    "porcelain": {
        "blurb": "Ming blue-and-white porcelain tiles",
        "style": (
            "glossy white porcelain tile faces with deep cobalt blue painted "
            "decoration. All design elements are rendered in traditional Ming "
            "dynasty blue-and-white porcelain style — hand-painted cobalt "
            "lines, slight bleeding, thin brush feel, soft glaze sheen."
        ),
    },
    "brass": {
        "blurb": "art deco brass and black enamel tiles",
        "style": (
            "polished brushed brass tile faces with deep black enamel inlay. "
            "All design elements are rendered in 1920s Art Deco style: "
            "strong geometric lines, symmetry, streamlined ornament, hairline "
            "brass dividers, warm metallic sheen with faint brush marks."
        ),
    },
    "lacquer": {
        "blurb": "black lacquer with gold leaf tiles",
        "style": (
            "deep glossy black lacquer tile faces with fine gold leaf (maki-e "
            "style) decoration. Delicate gilt lines, subtle sparkle, "
            "traditional Japanese lacquerware craftsmanship. Mirror-glossy "
            "surface with a soft highlight."
        ),
    },
    "paper": {
        "blurb": "sumi ink on washi paper tiles",
        "style": (
            "aged cream washi paper tile faces with visible fiber texture. "
            "All design elements are rendered in sumi ink brush strokes — "
            "slight ink bleed, variable line weight, calligraphic feel. "
            "Reds are cinnabar, greens are pine-ink, faint tea stains."
        ),
    },
    "neon": {
        "blurb": "neon glass on black tiles",
        "style": (
            "matte black tile faces with all design elements rendered as "
            "glowing neon-glass tubes — saturated magenta, cyan, and "
            "electric-green, with soft outward glow and a bright white core "
            "inside each stroke. 1980s arcade cabinet aesthetic."
        ),
    },
    "bone": {
        "blurb": "aged bone-and-bamboo tiles",
        "style": (
            "aged ivory bone tile faces with warm patina, hairline crazing, "
            "and softly polished corners. All design elements are deeply "
            "incised and filled with vivid hand-painted pigment — cinnabar "
            "red, pine green, cobalt blue. Looks like a century-old heirloom "
            "mahjong set."
        ),
    },
}


# ---------------------------------------------------------------------------
# Geometry — keep model output on a uniform grid (no aspect-ratio squash)
# ---------------------------------------------------------------------------

def fit_atlas_cover(img: Image.Image, target_w: int, target_h: int) -> Image.Image:
    """Scale uniformly (cover) then center-crop to exact atlas size.

    A naive ``resize`` to the target distorts when the API aspect ratio differs,
    which makes the 9×5 tile grid look uneven or stretched.
    """
    iw, ih = img.size
    if iw == target_w and ih == target_h:
        return img
    scale = max(target_w / iw, target_h / ih)
    nw = max(1, int(round(iw * scale)))
    nh = max(1, int(round(ih * scale)))
    img = img.resize((nw, nh), Image.Resampling.LANCZOS)
    left = (nw - target_w) // 2
    top = (nh - target_h) // 2
    return img.crop((left, top, left + target_w, top + target_h))


# ---------------------------------------------------------------------------
# Prompt construction
# ---------------------------------------------------------------------------

PROMPT_TEMPLATE = """\
Restyle this mahjong tile atlas into a new visual theme, keeping the \
9-column grid layout, every tile's position, boundaries, and depicted \
content EXACTLY the same as the reference. Only the visual treatment \
(material, palette, ornament, linework) changes.

Theme: {blurb}.
Style: {style}

Hard requirements — follow these verbatim:
- Output is a single atlas image at {atlas_w}x{atlas_h} with 9 tiles per \
row and {rows} rows total.
- Each illustrated tile face occupies one {tile_w}x{tile_h} pixel cell; \
all cells are identical in width and height; the 9×5 grid is perfectly \
rectilinear (no curved rows, no drifting seams, no variable gutters).
- Column boundaries are at x = 0, {tile_w}, {tile_w}*2, …; row boundaries \
at y = 0, {tile_h}, {tile_h}*2, … — match the reference atlas exactly.
- The set of tiles depicted and their grid positions must match the \
reference atlas precisely — one suit per row:
  * Row 1: bamboos B1 through B9 left to right
  * Row 2: characters 一萬 through 九萬 left to right (C1–C9)
  * Row 3: circles/dots D1 through D9 left to right
  * Row 4: winds 東 南 西 北 then dragons 中 發 白 (7 tiles), then 2 empty cells
  * Row 5: flowers 梅 蘭 菊 竹 (4 tiles), then four season tiles 春 夏 秋 冬 \
with recognizable spring/summer/autumn/winter motifs, then 1 empty cell
- Preserve the traditional color conventions: 1-dot red center; 5-dot red \
center; 7-dot red top-diagonal; 9-dot red top+bottom rows with green \
middle; 3-bamboo, 5-bamboo, 7-bamboo, 9-bamboo have red accents in the \
traditional positions; 一萬 is red; 中 is red; 發 is green; 白 is blank \
with a thin blue frame.
- No text, numbers, watermarks, or labels outside the tile faces. No \
tabletop, no drop shadows beneath tiles, no perspective, no 3D tile \
thickness — each cell is a flat orthographic tile face.
- Tiles do not overlap or bleed into neighbors; a small consistent gap \
or clean tile edge between neighbors is fine.
- Empty cells (row 4 columns 8–9 and row 5 column 9) should be left as \
transparent or neutral background, matching the reference.
"""


def build_prompt(theme_key: str | None, custom_style: str | None) -> str:
    if custom_style:
        blurb = f"custom: {custom_style[:60]}"
        style = custom_style
    else:
        theme = THEMES[theme_key]
        blurb = theme["blurb"]
        style = theme["style"]
    return PROMPT_TEMPLATE.format(
        blurb=blurb,
        style=style,
        atlas_w=ATLAS_W,
        atlas_h=ATLAS_H,
        tile_w=TILE_W,
        tile_h=TILE_H,
        rows=(len(LAYOUT) + COLUMNS - 1) // COLUMNS,
    )


# ---------------------------------------------------------------------------
# atlas.toml writer (must match scripts/pack_atlas.py output)
# ---------------------------------------------------------------------------

def write_toml(out_path: Path) -> None:
    lines = [
        'image = "atlas.png"',
        f"tile_width = {TILE_W}",
        f"tile_height = {TILE_H}",
        f"columns = {COLUMNS}",
        "",
        "layout = [",
    ]
    for i in range(0, len(LAYOUT), COLUMNS):
        row = LAYOUT[i:i + COLUMNS]
        lines.append("    " + ",".join(f'"{c}"' for c in row) + ",")
    lines.append("]")
    out_path.write_text("\n".join(lines) + "\n")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--theme", choices=sorted(THEMES.keys()),
                    help="Built-in theme to apply.")
    ap.add_argument(
        "--custom-style",
        "--prompt",
        dest="custom_style",
        metavar="TEXT",
        help="Full style / art-direction prompt (instead of --theme). "
             "Alias: --prompt",
    )
    ap.add_argument("--name",
                    help="Output set directory name under assets/textures/tile_sets/. "
                         "Defaults to the theme name.")
    ap.add_argument("--template", type=Path, default=DEFAULT_TEMPLATE,
                    help=f"Reference atlas image passed to the model as the "
                         f"layout template. Default: "
                         f"{DEFAULT_TEMPLATE.relative_to(REPO_ROOT)}")
    ap.add_argument("--list", action="store_true",
                    help="List available themes and exit.")
    ap.add_argument("--dry-run", action="store_true",
                    help="Print the prompt, don't call the API.")
    ap.add_argument("--model", default="gpt-image-2",
                    help="OpenAI image model.")
    ap.add_argument("--size", default=f"{ATLAS_W}x{ATLAS_H}",
                    help="API output size. Default matches the atlas "
                         f"({ATLAS_W}x{ATLAS_H}); the model may clamp to a "
                         "supported size and Pillow resizes back.")
    ap.add_argument("--force", action="store_true",
                    help="Overwrite existing atlas.png/atlas.toml.")
    ap.add_argument(
        "--annotate-indices",
        action="store_true",
        help="After generation, draw 1…N Arabic indices on each tile cell",
    )
    args = ap.parse_args()

    if args.list:
        print("Available themes:")
        for key, t in sorted(THEMES.items()):
            print(f"  {key:12s} {t['blurb']}")
        return

    if not args.theme and not args.custom_style:
        ap.error("pass --theme, --custom-style, or --list")
    if args.theme and args.custom_style:
        ap.error("pass only one of --theme or --custom-style")

    set_name = args.name or args.theme or "custom"
    set_dir = SETS_DIR / set_name
    atlas_out = set_dir / "atlas.png"
    toml_out = set_dir / "atlas.toml"

    if not args.template.exists():
        ap.error(f"template not found: {args.template}")

    prompt = build_prompt(args.theme, args.custom_style)
    if args.dry_run:
        print(f"Would call {args.model} with template={args.template}")
        print(f"Output set: {set_dir}")
        print(f"--- prompt ---\n{prompt}")
        return

    if atlas_out.exists() and not args.force:
        ap.error(f"{atlas_out} exists; pass --force to overwrite")

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        print("Error: OPENAI_API_KEY not set.", file=sys.stderr)
        sys.exit(1)
    client = OpenAI(api_key=api_key)

    set_dir.mkdir(parents=True, exist_ok=True)
    print(f"Restyling {args.template.relative_to(REPO_ROOT)} → {atlas_out}")
    print(f"Theme: {args.theme or 'custom'}")

    with args.template.open("rb") as image_file:
        response = client.images.edit(
            model=args.model,
            image=image_file,
            prompt=prompt,
            size=args.size,
            n=1,
        )

    b64 = response.data[0].b64_json
    img_bytes = base64.b64decode(b64)
    img = Image.open(io.BytesIO(img_bytes)).convert("RGBA")
    # Model size may differ; uniform scale + crop preserves square tile aspect.
    if img.size != (ATLAS_W, ATLAS_H):
        print(f"  fitting model output {img.size} → {(ATLAS_W, ATLAS_H)} "
              f"(uniform scale + center crop)")
        img = fit_atlas_cover(img, ATLAS_W, ATLAS_H)
    img.save(atlas_out)
    write_toml(toml_out)
    print(f"Wrote {atlas_out} ({atlas_out.stat().st_size} bytes) + "
          f"{toml_out.name}")

    if args.annotate_indices:
        if str(SCRIPTS_DIR) not in sys.path:
            sys.path.insert(0, str(SCRIPTS_DIR))
        import annotate_atlas_indices as _atlas_lbl

        n = _atlas_lbl.annotate_atlas_png(atlas_out)
        print(f"Annotated atlas with {n} corner indices")

    print("Run the game and switch to this set in Options → Visual → "
          "Tile Set.")


if __name__ == "__main__":
    main()
