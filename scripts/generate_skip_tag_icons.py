#!/usr/bin/env python3
"""
Generate skip-tag icons for Mahjuro and pack them into a sprite sheet.

Each skip tag (`TagKind` in `src/core/tag.rs`) gets a distinct faceted low-poly
inventory icon in the game's Walnut, Brass & Felt palette. The script calls
Google's Nano Banana 2 (`gemini-3.1-flash-image-preview`) for per-tag source
art, runs a light post-process (background strip, contrast boost, content fit,
smooth downscale), then packs a row-major atlas.

**Writes (under `assets/textures/skip_tags/` by default)**

  • `source/tag_{slug}.png` — raw API output (RGBA)
  • `processed/tag_{slug}.png` — cleaned icon, square `CELL_SIZE`
  • `atlas.png` — packed sprite sheet (3×3 grid, one cell per tag)
  • `atlas.toml` — cell size, columns, and layout ids (matches `TagKind::all()`)

Tag order and ids MUST stay aligned with `assets/data/tags.json` and
`TagKind::all()` in `src/core/tag.rs`.

Art direction: stylized low-poly vector icons (faceted planes, crisp edges,
thin outline stroke) readable at ~64 px on screen.

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python3 scripts/generate_skip_tag_icons.py                  # missing only
    python3 scripts/generate_skip_tag_icons.py --force          # regenerate all
    python3 scripts/generate_skip_tag_icons.py --name gold_ingot
    python3 scripts/generate_skip_tag_icons.py --tag 3          # 1-indexed
    python3 scripts/generate_skip_tag_icons.py --list
    python3 scripts/generate_skip_tag_icons.py --dry-run        # prompts only
    python3 scripts/generate_skip_tag_icons.py --pack-only      # repack atlas
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import (  # noqa: E402
    DEFAULT_MODEL,
    GEMINI_IMAGE_SIZES,
    generate_image_bytes,
    init_client,
)

try:
    from PIL import Image, ImageEnhance, ImageOps
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)


REPO_ROOT = Path(__file__).resolve().parent.parent
TAG_JSON_PATH = REPO_ROOT / "assets" / "data" / "tags.json"
TAG_RS_PATH = REPO_ROOT / "src" / "core" / "tag.rs"
OUTPUT_DIR = REPO_ROOT / "assets" / "textures" / "skip_tags"
SOURCE_DIR = OUTPUT_DIR / "source"
PROCESSED_DIR = OUTPUT_DIR / "processed"

# Atlas grid: 3 columns × 3 rows for the nine skip tags.
COLUMNS = 3
CELL_SIZE = 128
# Fraction of the atlas cell the subject should fill after auto-crop.
CONTENT_FILL = 0.82
# Working resolution before smooth downscale to CELL_SIZE.
WORK_PX = CELL_SIZE * 4

# Walnut, Brass & Felt — theme tokens from src/render/theme.rs (hex → RGB).
THEME_PALETTE: tuple[tuple[int, int, int], ...] = (
    (10, 8, 6),  # WALNUT_INK
    (18, 14, 11),  # WALNUT_DEEP
    (28, 22, 17),  # WALNUT_RAISED
    (42, 33, 26),  # WALNUT_SOFT
    (54, 42, 33),  # WALNUT_BRIGHT
    (245, 198, 116),  # CHAMPAGNE
    (232, 177, 74),  # GOLD
    (200, 144, 30),  # BRASS
    (138, 94, 20),  # ANTIQUE
    (244, 241, 232),  # PARCHMENT
    (184, 174, 162),  # STONE
    (99, 92, 82),  # UMBER
    (95, 212, 168),  # JADE
    (232, 90, 107),  # RUBY
    (240, 168, 72),  # AMBER
    (26, 42, 33),  # FELT_DEEP
    (74, 107, 82),  # FELT_LIT
    (14, 20, 34),  # TWILIGHT_INK
    (30, 42, 64),  # TWILIGHT
    (90, 110, 148),  # TWILIGHT_GLOW
)
PSX_PALETTE = THEME_PALETTE  # back-compat alias

# Per-rarity metal accent confined to the subject silhouette edges.
RARITY_ACCENTS: dict[str, str] = {
    "common": (
        "Rarity accent: cool gunmetal trim along the subject edges — "
        "subtle iron highlights."
    ),
    "uncommon": (
        "Rarity accent: warm rose-copper trim along the subject edges — "
        "soft amber glints."
    ),
    "rare": (
        "Rarity accent: polished silver trim along the subject edges — "
        "cool white speculars."
    ),
}

# One iconic, high-contrast subject per tag. Prompts are tuned for 64 px legibility.
TAG_VISUALS: dict[str, str] = {
    "gold_ingot": (
        "A single Chinese sycee gold ingot (boat-shaped yuanbao) seen from a "
        "slight three-quarter angle. One chunky low-poly ingot — wide boat "
        "silhouette, flat brass-gold faces, a single hard white specular "
        "stripe along the crest."
    ),
    "treasure_chest": (
        "A small lacquered treasure chest, lid cracked open just enough for a "
        "single horizontal band of bright gold light to escape. Simple box "
        "silhouette: dark walnut body, one brass hasp, one brass band. "
        "Readable as 'loot' from the wedge of light alone."
    ),
    "free_reroll": (
        "One antique price tag hanging straight, face-on to camera. The tag "
        "is parchment ivory with a single bold circular arrow loop (↻) "
        "embossed in dark umber — the arrow must be thick, occupying most of "
        "the tag face. One tag hanging alone."
    ),
    "patron_gift": (
        "A square gift box in isometric three-quarter view, wrapped in "
        "jade-green silk faceted planes with a large ruby-red bow on top. "
        "Thin champagne-gold outline stroke around the whole silhouette. "
        "Reads instantly as a present."
    ),
    "rich_stock": (
        "A walnut shop shelf seen straight-on, holding two large enamel-pin "
        "badges side by side (simple circle and diamond shapes). Two big "
        "badges filling the shelf width, evenly lit."
    ),
    "zodiac_blessing": (
        "A flat parchment scroll, mostly unrolled and facing the camera. "
        "One large stylised dragon medallion embroidered in bold gold thread "
        "at the scroll center — the dragon is a simple S-curve silhouette "
        "with chunky scale blocks. Dark walnut roller caps at each end."
    ),
    "bonus_play": (
        "The gameplay bronze play mirror: a flat polished circular bronze disc "
        "with a raised rim ring and a slightly recessed mirror face — the same "
        "prop players click to play a hand. Face-on from above. One ivory "
        "mahjong tile sits on the mirror face, and a faint second ghosted tile "
        "silhouette beside it implies an extra play. Chunky disc silhouette; "
        "plain ivory tile faces."
    ),
    "bonus_discard": (
        "The gameplay discard river: a miniature meandering stream with "
        "pebble-lined banks and twilight-blue water — the same prop players "
        "click to discard tiles. Seen from a slight three-quarter angle. One "
        "ivory mahjong tile rests on the water surface or splashes into the "
        "stream, and a second tile beside the bank implies an extra discard. "
        "Simple S-curve channel with bold geometric banks."
    ),
    "wide_hand": (
        "A stylized open palm facing the viewer with seven clearly separated "
        "fingers spread in a wide fan — an uncanny wide hand for expanded "
        "hand size. Chunky low-poly vector hand: parchment-ivory skin planes, "
        "walnut-brown shadow facets, thin champagne outline. Fingers are "
        "simple tapered blocks, evenly spaced, counting to seven at a glance. "
        "NOT a tile rack, NOT photorealistic."
    ),
}

STYLE_BASE = (
    "Stylized low-poly vector inventory icon for a mahjong roguelike set in "
    "a dark curio House. Faceted game UI art — flat color planes with hard "
    "edges between facets, a thin light outline stroke around the silhouette, "
    "and simple cel-shaded highlights. Clean vector-style polygon shading "
    "with crisp edges, readable at 64×64 pixels.\n\n"
    "Composition rules (strict):\n"
    "  • ONE subject only, centered, filling ~85% of the square frame.\n"
    "  • Mild isometric or gentle three-quarter view.\n"
    "  • Solid matte #000000 backdrop outside the subject.\n"
    "  • Subject alone on flat black with only a thin outline stroke.\n"
    "  • Limited palette: walnut browns, brass gold, emerald felt, parchment "
    "ivory, ruby red, jade green, twilight blue shadows.\n"
    "  • Chunky geometric shapes with 3–6 flat color regions per surface.\n"
    "  • Wordless pictogram icon — shapes and colors only.\n"
    "  • Flat cel-shaded color planes with hard facet boundaries.\n\n"
    "Style reference: a wrapped present with faceted green planes, a bold "
    "red bow, and a crisp gold outline — clean vector-like edges, readable "
    "when displayed at 64×64 pixels."
)


@dataclass(frozen=True)
class TagDef:
    slug: str
    name: str
    description: str
    rarity: str


def pascal_to_snake(name: str) -> str:
    s1 = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s1).lower()


def load_tag_kind_order() -> list[str]:
    """Parse `TagKind::all()` order from src/core/tag.rs."""
    if not TAG_RS_PATH.exists():
        raise SystemExit(f"Cannot read tag order: {TAG_RS_PATH} missing")
    text = TAG_RS_PATH.read_text(encoding="utf-8")
    m = re.search(
        r"pub fn all\(\)[^{]*\{[^[]*\[(.*?)\]\s*\}",
        text,
        re.DOTALL,
    )
    if not m:
        raise SystemExit(
            "Failed to parse TagKind::all() from src/core/tag.rs"
        )
    kinds = re.findall(r"TagKind::(\w+)", m.group(1))
    if not kinds:
        raise SystemExit("TagKind::all() contained no variants")
    return [pascal_to_snake(k) for k in kinds]


def load_tags() -> list[TagDef]:
    """Load tag rows from tags.json and validate order vs TagKind::all()."""
    if not TAG_JSON_PATH.exists():
        raise SystemExit(f"Cannot read tags: {TAG_JSON_PATH} missing")
    raw = json.loads(TAG_JSON_PATH.read_text(encoding="utf-8"))
    expected = load_tag_kind_order()
    slugs = [row["id"] for row in raw]
    if slugs != expected:
        raise SystemExit(
            "tags.json order does not match TagKind::all() in tag.rs.\n"
            f"  json:     {slugs}\n"
            f"  expected: {expected}"
        )
    missing_visuals = [s for s in slugs if s not in TAG_VISUALS]
    if missing_visuals:
        raise SystemExit(
            "TAG_VISUALS missing entries for: "
            + ", ".join(missing_visuals)
        )
    out: list[TagDef] = []
    for row in raw:
        slug = row["id"]
        out.append(
            TagDef(
                slug=slug,
                name=row["name"],
                description=row["description"],
                rarity=row["rarity"],
            )
        )
    return out


TAGS = load_tags()
LAYOUT = [t.slug for t in TAGS]


def layout_rows() -> list[list[str]]:
    rows: list[list[str]] = []
    for i in range(0, len(LAYOUT), COLUMNS):
        row = LAYOUT[i : i + COLUMNS]
        while len(row) < COLUMNS:
            row.append("")
        rows.append(row)
    return rows


def nearest_palette_color(
    rgb: tuple[int, int, int],
) -> tuple[int, int, int]:
    best = THEME_PALETTE[0]
    best_dist = float("inf")
    for color in THEME_PALETTE:
        dist = sum((a - b) ** 2 for a, b in zip(rgb, color))
        if dist < best_dist:
            best_dist = dist
            best = color
    return best


def _luminance(rgb: tuple[int, int, int]) -> float:
    r, g, b = rgb
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def _saturation(rgb: tuple[int, int, int]) -> int:
    r, g, b = rgb
    return max(r, g, b) - min(r, g, b)


def estimate_corner_background_rgb(img: Image.Image) -> tuple[int, int, int]:
    """Median RGB sampled from image edge midpoints and corners."""
    rgba = img.convert("RGBA")
    w, h = rgba.size
    px = rgba.load()
    samples: list[tuple[int, int, int]] = []
    for x, y in (
        (0, 0),
        (w - 1, 0),
        (0, h - 1),
        (w - 1, h - 1),
        (w // 2, 0),
        (w // 2, h - 1),
        (0, h // 2),
        (w - 1, h // 2),
    ):
        r, g, b, a = px[x, y]
        if a >= 128:
            samples.append((r, g, b))
    if not samples:
        return (0, 0, 0)
    rs = sorted(c[0] for c in samples)
    gs = sorted(c[1] for c in samples)
    bs = sorted(c[2] for c in samples)
    mid = len(rs) // 2
    return (rs[mid], gs[mid], bs[mid])


def is_background_pixel(
    rgb: tuple[int, int, int],
    alpha: int,
    bg_rgb: tuple[int, int, int],
    *,
    channel_tolerance: int = 10,
) -> bool:
    """True for transparent pixels or colors close to the sampled backdrop."""
    if alpha < 8:
        return True
    return all(abs(c - b) <= channel_tolerance for c, b in zip(rgb, bg_rgb))


def remove_corner_background(
    img: Image.Image,
    *,
    channel_tolerance: int = 10,
    bg_rgb: tuple[int, int, int] | None = None,
) -> Image.Image:
    """Turn corner-connected backdrop pixels transparent.

    Only removes pixels whose RGB is within ``channel_tolerance`` of the
    backdrop color sampled from the image edges — subject shadows and
    highlights are preserved even when dark or light.
    """
    rgba = img.convert("RGBA")
    bg = bg_rgb if bg_rgb is not None else estimate_corner_background_rgb(rgba)
    w, h = rgba.size
    pixels = rgba.load()
    visited = [[False] * w for _ in range(h)]
    stack: list[tuple[int, int]] = []

    def push_if_background(x: int, y: int) -> None:
        if x < 0 or y < 0 or x >= w or y >= h or visited[y][x]:
            return
        visited[y][x] = True
        r, g, b, a = pixels[x, y]
        if is_background_pixel(
            (r, g, b),
            a,
            bg,
            channel_tolerance=channel_tolerance,
        ):
            stack.append((x, y))

    for x in range(w):
        push_if_background(x, 0)
        push_if_background(x, h - 1)
    for y in range(h):
        push_if_background(0, y)
        push_if_background(w - 1, y)

    while stack:
        x, y = stack.pop()
        pixels[x, y] = (0, 0, 0, 0)
        for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
            if 0 <= nx < w and 0 <= ny < h and not visited[ny][nx]:
                push_if_background(nx, ny)

    return rgba


def strip_low_alpha(img: Image.Image, *, alpha_cutoff: int = 16) -> Image.Image:
    """Drop nearly-transparent resize halos without touching opaque subject pixels."""
    rgba = img.convert("RGBA")
    px = rgba.load()
    for y in range(rgba.height):
        for x in range(rgba.width):
            if px[x, y][3] < alpha_cutoff:
                px[x, y] = (0, 0, 0, 0)
    return rgba


# Back-compat alias.
remove_dark_background = remove_corner_background


def content_bbox(
    img: Image.Image,
    *,
    alpha_threshold: int = 24,
) -> tuple[int, int, int, int] | None:
    """Return (left, top, right, bottom) of non-transparent pixels."""
    rgba = img.convert("RGBA")
    w, h = rgba.size
    px = rgba.load()
    min_x, min_y = w, h
    max_x, max_y = -1, -1
    for y in range(h):
        for x in range(w):
            if px[x, y][3] >= alpha_threshold:
                min_x = min(min_x, x)
                min_y = min(min_y, y)
                max_x = max(max_x, x)
                max_y = max(max_y, y)
    if max_x < min_x or max_y < min_y:
        return None
    return (min_x, min_y, max_x + 1, max_y + 1)


def fit_content_to_square(
    img: Image.Image,
    *,
    side: int,
    fill: float = CONTENT_FILL,
    resample: Image.Resampling = Image.Resampling.LANCZOS,
) -> Image.Image:
    """Crop to subject bbox, scale to `fill` of `side`, center on transparent square."""
    bbox = content_bbox(img)
    if bbox is None:
        return Image.new("RGBA", (side, side), (0, 0, 0, 0))
    cropped = img.crop(bbox)
    cw, ch = cropped.size
    target = max(1, int(side * fill))
    scale = target / max(cw, ch)
    new_w = max(1, int(round(cw * scale)))
    new_h = max(1, int(round(ch * scale)))
    scaled = cropped.resize((new_w, new_h), resample)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    ox = (side - new_w) // 2
    oy = (side - new_h) // 2
    canvas.paste(scaled, (ox, oy), scaled)
    return canvas


def snap_to_palette(img: Image.Image) -> Image.Image:
    """Map opaque pixels to the nearest theme palette color."""
    rgba = img.convert("RGBA")
    out = Image.new("RGBA", rgba.size)
    qpx = out.load()
    spx = rgba.load()
    for y in range(rgba.height):
        for x in range(rgba.width):
            r, g, b, a = spx[x, y]
            if a < 16:
                qpx[x, y] = (0, 0, 0, 0)
                continue
            snapped = nearest_palette_color((r, g, b))
            qpx[x, y] = (*snapped, a)
    return out


def boost_subject_contrast(img: Image.Image) -> Image.Image:
    """Stretch RGB contrast on opaque pixels before palette snap."""
    rgba = img.convert("RGBA")
    alpha = rgba.split()[3]
    rgb = Image.merge("RGB", rgba.split()[:3])
    # Operate on a tight crop so autocontrast ignores empty margins.
    bbox = content_bbox(rgba)
    if bbox is None:
        return rgba
    subject = rgb.crop(bbox)
    subject = ImageOps.autocontrast(subject, cutoff=2)
    subject = ImageEnhance.Contrast(subject).enhance(1.25)
    boosted = rgb.copy()
    boosted.paste(subject, bbox)
    out = Image.merge("RGBA", (*boosted.split(), alpha))
    return out


def icon_postprocess(
    img: Image.Image,
    *,
    cell_size: int = CELL_SIZE,
    work_px: int = WORK_PX,
    content_fill: float = CONTENT_FILL,
) -> Image.Image:
    """Background strip, contrast boost, content fit, smooth downscale."""
    rgba = remove_corner_background(img)
    rgba = boost_subject_contrast(rgba)

    side = min(rgba.width, rgba.height)
    left = (rgba.width - side) // 2
    top = (rgba.height - side) // 2
    square = rgba.crop((left, top, left + side, top + side))

    fitted = fit_content_to_square(
        square,
        side=work_px,
        fill=content_fill,
    )
    out = fitted.resize((cell_size, cell_size), Image.Resampling.LANCZOS)
    return strip_low_alpha(out)


# Back-compat alias for tests and one-off scripts.
psx_postprocess = icon_postprocess


def build_prompt(tag: TagDef) -> str:
    visual = TAG_VISUALS[tag.slug]
    accent = RARITY_ACCENTS.get(
        tag.rarity,
        RARITY_ACCENTS["common"],
    )
    subject = (
        f'Icon subject for the skip reward "{tag.name}" ({tag.description}): '
        f"{visual}"
    )
    return "\n\n".join([STYLE_BASE, subject, accent])


def generate_image(
    client,
    prompt: str,
    output_path: Path,
    model: str,
    image_size: str | None,
) -> None:
    """Render one square icon via Nano Banana 2."""
    img_bytes = generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio="1:1",
        image_size=image_size,
    )
    output_path.write_bytes(img_bytes)


def process_source(source_path: Path, processed_path: Path) -> None:
    img = Image.open(source_path)
    out = icon_postprocess(img)
    processed_path.parent.mkdir(parents=True, exist_ok=True)
    out.save(processed_path)


def write_atlas_toml(
    path: Path,
    *,
    tile_w: int,
    tile_h: int,
    columns: int,
    layout: list[str],
) -> None:
    lines = [
        'image = "atlas.png"',
        f"tile_width = {tile_w}",
        f"tile_height = {tile_h}",
        f"columns = {columns}",
        "",
        "layout = [",
    ]
    for i in range(0, len(layout), columns):
        row = layout[i : i + columns]
        lines.append("    " + ",".join(f'"{c}"' for c in row) + ",")
    lines.append("]")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def pack_atlas(
    processed_dir: Path,
    output_dir: Path,
    *,
    cell_size: int = CELL_SIZE,
    columns: int = COLUMNS,
    layout: list[str] | None = None,
) -> Path:
    """Pack processed icons into atlas.png + atlas.toml."""
    layout = layout or LAYOUT
    icons: dict[str, Image.Image] = {}
    for slug in layout:
        if not slug:
            continue
        path = processed_dir / f"tag_{slug}.png"
        if not path.exists():
            raise FileNotFoundError(f"missing processed icon: {path}")
        img = Image.open(path).convert("RGBA")
        if img.size != (cell_size, cell_size):
            img = img.resize((cell_size, cell_size), Image.Resampling.NEAREST)
        icons[slug] = img

    rows = math.ceil(len(layout) / columns)
    atlas = Image.new(
        "RGBA",
        (columns * cell_size, rows * cell_size),
        (0, 0, 0, 0),
    )
    for i, slug in enumerate(layout):
        if not slug:
            continue
        col = i % columns
        row = i // columns
        atlas.paste(icons[slug], (col * cell_size, row * cell_size))

    output_dir.mkdir(parents=True, exist_ok=True)
    atlas_path = output_dir / "atlas.png"
    atlas.save(atlas_path)
    write_atlas_toml(
        output_dir / "atlas.toml",
        tile_w=cell_size,
        tile_h=cell_size,
        columns=columns,
        layout=layout,
    )
    print(
        f"packed {len(icons)} icons → {atlas_path} "
        f"({columns * cell_size}×{rows * cell_size})"
    )
    return atlas_path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate skip-tag icons and pack a sprite atlas"
    )
    parser.add_argument(
        "--tag",
        type=int,
        default=None,
        help="Generate only tag number N (1-indexed, TagKind::all order).",
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Generate only the tag with this slug (e.g. gold_ingot).",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all skip tags and exit.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if source PNG already exists.",
    )
    parser.add_argument(
        "--pack-only",
        action="store_true",
        help="Skip API calls; post-process existing sources and repack atlas.",
    )
    parser.add_argument(
        "--skip-post",
        action="store_true",
        help="Save raw API output only; do not post-process or pack.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}, aka Nano Banana 2).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="512px",
        choices=list(GEMINI_IMAGE_SIZES),
        help=(
            "API image size tier (default: 512px). Ignored on older "
            "google-genai SDKs that only accept aspect_ratio."
        ),
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help=f"Output root (default: {OUTPUT_DIR}).",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds between API calls (default: 2.0).",
    )
    args = parser.parse_args()

    if args.list:
        for i, tag in enumerate(TAGS, 1):
            print(
                f"  {i:2d}. {tag.name:<16s}  {tag.slug:<18s}  "
                f"[{tag.rarity}]  {tag.description}"
            )
        print(f"\nAtlas layout ({COLUMNS} columns):")
        for row in layout_rows():
            print("   ", "  ".join(f"{c or '—':<18s}" for c in row))
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    source_dir = out_dir / "source"
    processed_dir = out_dir / "processed"
    source_dir.mkdir(parents=True, exist_ok=True)
    processed_dir.mkdir(parents=True, exist_ok=True)

    if args.tag is not None and args.name is not None:
        print("Error: pass --tag OR --name, not both.")
        sys.exit(1)

    if args.tag is not None:
        if args.tag < 1 or args.tag > len(TAGS):
            print(f"Error: --tag must be between 1 and {len(TAGS)}")
            sys.exit(1)
        targets = [TAGS[args.tag - 1]]
    elif args.name is not None:
        match = next((t for t in TAGS if t.slug == args.name), None)
        if match is None:
            print(f"Error: no tag with slug '{args.name}'. Try --list.")
            sys.exit(1)
        targets = [match]
    else:
        targets = TAGS

    client = None
    if not args.dry_run and not args.pack_only:
        client = init_client()

    generated = 0
    skipped = 0
    failed = 0
    total_jobs = len(targets)

    for job, tag in enumerate(targets, 1):
        source_path = source_dir / f"tag_{tag.slug}.png"
        processed_path = processed_dir / f"tag_{tag.slug}.png"
        prompt = build_prompt(tag)

        print(f"\n[{job}/{total_jobs}] {tag.name} ({tag.slug})")

        if args.dry_run:
            print(f"  Source:      {source_path.name}")
            print(f"  Processed:   {processed_path.name}")
            print(f"  Prompt:\n    {prompt.replace(chr(10), chr(10) + '    ')}\n")
            continue

        need_api = not args.pack_only
        if need_api:
            if source_path.exists() and not args.force:
                print(f"  Skipping API (exists): {source_path.name}")
                skipped += 1
            else:
                try:
                    assert client is not None
                    generate_image(
                        client, prompt, source_path, args.model, args.size
                    )
                    print(f"  Saved source: {source_path.name}")
                    generated += 1
                except Exception as e:
                    print(f"  Error generating {tag.name}: {e}")
                    failed += 1
                    if job < total_jobs:
                        time.sleep(args.delay)
                    continue
                if job < total_jobs:
                    time.sleep(args.delay)

        if args.skip_post:
            continue

        if not source_path.exists():
            print(f"  Missing source, cannot post-process: {source_path.name}")
            failed += 1
            continue

        try:
            process_source(source_path, processed_path)
            print(f"  Post-processed: {processed_path.name}")
        except Exception as e:
            print(f"  Error post-processing {tag.name}: {e}")
            failed += 1

    if args.dry_run or args.skip_post:
        return

    try:
        pack_atlas(processed_dir, out_dir)
    except Exception as e:
        print(f"\nAtlas pack failed: {e}")
        sys.exit(1)

    print("\nDone.")
    if not args.dry_run:
        print(
            f"  generated={generated}  skipped={skipped}  failed={failed}"
            f"  → {out_dir}"
        )


if __name__ == "__main__":
    main()
