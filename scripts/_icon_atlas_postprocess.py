"""Shared icon post-process + packed atlas helpers for Mahjuro asset scripts.

Used by `generate_skip_tag_icons.py` and `generate_boss_icons.py`: background
strip, contrast boost, content fit, downscale, then grid pack + `atlas.toml`.
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageEnhance, ImageOps

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
    """Turn corner-connected backdrop pixels transparent."""
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
    fill: float,
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
    cell_size: int = 128,
    work_px: int | None = None,
    content_fill: float = 0.82,
) -> Image.Image:
    """Background strip, contrast boost, content fit, smooth downscale."""
    if work_px is None:
        work_px = cell_size * 4
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


def pack_processed_icons(
    processed_dir: Path,
    output_dir: Path,
    *,
    layout: list[str],
    columns: int,
    cell_size: int = 128,
    file_prefix: str,
) -> Path:
    """Pack `{file_prefix}_{slug}.png` cells into atlas.png + atlas.toml."""
    icons: dict[str, Image.Image] = {}
    for slug in layout:
        if not slug:
            continue
        path = processed_dir / f"{file_prefix}_{slug}.png"
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
