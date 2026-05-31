"""Shared postprocess and organic silhouette mask helpers for talisman carving art."""

from __future__ import annotations

import statistics
from collections import deque
from io import BytesIO
from pathlib import Path

from PIL import Image

# Match `HEIGHT_ALPHA_LO` in generate_relic_art.py / chitin discard in lit_mesh.wgsl.
HEIGHT_ALPHA_LO = 8

# Default relief exaggeration (plan: 2.4 shop / 2.0 memorial).
SHOP_EXAGGERATE = 2.4
MEMORIAL_EXAGGERATE = 2.0

# rembg alpha → binary mask (anti-aliased edge preserved in extrusion threshold).
MASK_ALPHA_THRESHOLD = 16

# Sculpted heightmaps: studio plate tone matches border gray; internal piercings
# share that tone but are enclosed by jade — punch if |luma − bg| <= this.
PLATE_VOID_TOL = 8
# Ignore speckle this small when punching interior plate voids.
MIN_INTERIOR_VOID_AREA = 20
# Fill only tiny enclosed void flecks (carving shadow), not design piercings.
MAX_CARVING_PINHOLE_AREA = 16

# Postprocess: strip AI matte / inset frame bands connected to the image border.
EDGE_MATTE_LO = 88
EDGE_MATTE_HI = 118
EDGE_FRAME_LO = 65
EDGE_FRAME_HI = 92
EDGE_STRIP_MAX_DEPTH = 18
PLATE_GRAY = 128

_REMBG_SESSION = None


def _flat_heightfield_style(height: Image.Image) -> bool:
    """True when the plate uses black void outside the silhouette (legacy heightmaps)."""
    w, h = height.size
    px = height.load()
    border: list[int] = []
    for x in range(w):
        border.append(px[x, 0])
        border.append(px[x, h - 1])
    for y in range(1, h - 1):
        border.append(px[0, y])
        border.append(px[w - 1, y])
    return statistics.median(border) <= 24


def _mask_luma_threshold(height: Image.Image, *, min_luma: int = HEIGHT_ALPHA_LO) -> Image.Image:
    return height.point(lambda v: 255 if v >= min_luma else 0, mode="L")


def _mask_border_flood(height: Image.Image, *, tol: int = 20) -> Image.Image:
    """Mark foreground as pixels not connected to the border through similar gray."""
    w, h = height.size
    data = list(height.getdata())
    if len(data) != w * h:
        data = list(height.convert("L").getdata())

    def idx(x: int, y: int) -> int:
        return y * w + x

    border_vals: list[int] = []
    for x in range(w):
        border_vals.append(data[idx(x, 0)])
        border_vals.append(data[idx(x, h - 1)])
    for y in range(1, h - 1):
        border_vals.append(data[idx(0, y)])
        border_vals.append(data[idx(w - 1, y)])
    bg = int(statistics.median(border_vals))

    is_bg = [False] * (w * h)
    q: deque[tuple[int, int]] = deque()

    def try_seed(x: int, y: int) -> None:
        i = idx(x, y)
        if not is_bg[i] and abs(data[i] - bg) <= tol:
            is_bg[i] = True
            q.append((x, y))

    for x in range(w):
        try_seed(x, 0)
        try_seed(x, h - 1)
    for y in range(h):
        try_seed(0, y)
        try_seed(w - 1, y)

    while q:
        x, y = q.popleft()
        for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
            if 0 <= nx < w and 0 <= ny < h:
                i = idx(nx, ny)
                if not is_bg[i] and abs(data[i] - bg) <= tol:
                    is_bg[i] = True
                    q.append((nx, ny))

    out = [0 if is_bg[i] else 255 for i in range(w * h)]
    mask = Image.new("L", (w, h))
    mask.putdata(out)
    return mask


def _punch_interior_plate_voids(
    height: Image.Image,
    *,
    plate_void_tol: int = PLATE_VOID_TOL,
    min_area: int = MIN_INTERIOR_VOID_AREA,
) -> Image.Image:
    """Punch enclosed studio-plate tone voids (wave gates, coin squares, piercings).

    Border flood alone leaves these as foreground when they match the exterior
    plate gray but are walled off by brighter jade relief.
    """
    w, h = height.size
    data = list(height.getdata())
    if len(data) != w * h:
        data = list(height.convert("L").getdata())
    n = w * h

    def idx(x: int, y: int) -> int:
        return y * w + x

    border_vals: list[int] = []
    for x in range(w):
        border_vals.extend((data[idx(x, 0)], data[idx(x, h - 1)]))
    for y in range(1, h - 1):
        border_vals.extend((data[idx(0, y)], data[idx(w - 1, y)]))
    bg = int(statistics.median(border_vals))

    plate_void = [abs(data[i] - bg) <= plate_void_tol for i in range(n)]
    exterior_void = [False] * n
    q: deque[tuple[int, int]] = deque()

    def seed(x: int, y: int) -> None:
        i = idx(x, y)
        if plate_void[i] and not exterior_void[i]:
            exterior_void[i] = True
            q.append((x, y))

    for x in range(w):
        seed(x, 0)
        seed(x, h - 1)
    for y in range(h):
        seed(0, y)
        seed(w - 1, y)

    while q:
        x, y = q.popleft()
        for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
            if 0 <= nx < w and 0 <= ny < h:
                i = idx(nx, ny)
                if plate_void[i] and not exterior_void[i]:
                    exterior_void[i] = True
                    q.append((nx, ny))

    punch = [False] * n
    seen = [False] * n
    for y in range(h):
        for x in range(w):
            i = idx(x, y)
            if seen[i] or exterior_void[i] or not plate_void[i]:
                continue
            comp: list[int] = []
            cq: deque[tuple[int, int]] = deque([(x, y)])
            seen[i] = True
            while cq:
                cx, cy = cq.popleft()
                ci = idx(cx, cy)
                comp.append(ci)
                for nx, ny in ((cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)):
                    if 0 <= nx < w and 0 <= ny < h:
                        j = idx(nx, ny)
                        if not seen[j] and plate_void[j] and not exterior_void[j]:
                            seen[j] = True
                            cq.append((nx, ny))
            if len(comp) >= min_area:
                for j in comp:
                    punch[j] = True

    out = [0 if (exterior_void[i] or punch[i]) else 255 for i in range(n)]
    mask = Image.new("L", (w, h))
    mask.putdata(out)
    return mask


def _mask_sculpted_plate(height: Image.Image) -> Image.Image:
    """Silhouette for gray-studio sculpted heightmaps with internal piercings."""
    return _punch_interior_plate_voids(height)


def _fill_tiny_enclosed_voids(mask: Image.Image, *, max_area: int = MAX_CARVING_PINHOLE_AREA) -> Image.Image:
    """Fill only pinhole-sized void flecks inside the silhouette (not design piercings)."""
    if max_area <= 0:
        return mask
    w, h = mask.size
    data = list(mask.getdata())
    if len(data) != w * h:
        data = list(mask.convert("L").getdata())
    n = w * h

    def idx(x: int, y: int) -> int:
        return y * w + x

    out = data[:]
    seen = [False] * n
    for y in range(h):
        for x in range(w):
            i = idx(x, y)
            if seen[i] or data[i] >= 128:
                continue
            comp: list[int] = []
            touches_border = False
            cq: deque[tuple[int, int]] = deque([(x, y)])
            seen[i] = True
            while cq:
                cx, cy = cq.popleft()
                if cx == 0 or cy == 0 or cx == w - 1 or cy == h - 1:
                    touches_border = True
                ci = idx(cx, cy)
                comp.append(ci)
                for nx, ny in ((cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)):
                    if 0 <= nx < w and 0 <= ny < h:
                        j = idx(nx, ny)
                        if not seen[j] and data[j] < 128:
                            seen[j] = True
                            cq.append((nx, ny))
            if not touches_border and len(comp) <= max_area:
                for j in comp:
                    out[j] = 255
    filled = Image.new("L", (w, h))
    filled.putdata(out)
    return filled


def _foreground_ratio(mask: Image.Image) -> float:
    data = list(mask.getdata())
    if not data:
        return 0.0
    return sum(1 for v in data if v >= 128) / len(data)


def _mask_rembg(height: Image.Image) -> Image.Image:
    """Local u2net segmentation via rembg (same stack as generate_relic_art.py)."""
    global _REMBG_SESSION
    try:
        from rembg import new_session, remove
    except ImportError as e:
        raise RuntimeError(
            "rembg not installed. Run: pip install rembg pillow onnxruntime"
        ) from e

    if _REMBG_SESSION is None:
        _REMBG_SESSION = new_session("u2net")

    rgb = Image.merge("RGB", (height, height, height))
    buf = BytesIO()
    rgb.save(buf, format="PNG")
    out_bytes = remove(buf.getvalue(), session=_REMBG_SESSION)
    rgba = Image.open(BytesIO(out_bytes)).convert("RGBA")
    alpha = rgba.split()[-1]
    return alpha.point(lambda v: 255 if v >= MASK_ALPHA_THRESHOLD else 0, mode="L")


def mask_from_height_image(
    height: Image.Image,
    *,
    method: str = "auto",
    min_luma: int = HEIGHT_ALPHA_LO,
) -> Image.Image:
    """Build a binary carving silhouette mask from a grayscale heightmap."""
    gray = height.convert("L")

    if method == "luma":
        return _mask_luma_threshold(gray, min_luma=min_luma)

    if method == "flood":
        mask = _mask_sculpted_plate(gray)
        return _fill_tiny_enclosed_voids(mask)

    if method == "rembg":
        return _mask_rembg(gray)

    # auto
    if _flat_heightfield_style(gray):
        return _mask_luma_threshold(gray, min_luma=min_luma)

    mask = _fill_tiny_enclosed_voids(_mask_sculpted_plate(gray))
    ratio = _foreground_ratio(mask)
    if 0.06 <= ratio <= 0.88:
        return mask

    try:
        rembg_mask = _mask_rembg(gray)
        if 0.04 <= _foreground_ratio(rembg_mask) <= 0.92:
            return rembg_mask
    except RuntimeError:
        pass

    return mask


def write_mask_from_height(
    height_path: Path,
    mask_path: Path,
    *,
    min_luma: int = HEIGHT_ALPHA_LO,
    method: str = "auto",
) -> bool:
    """Silhouette mask for mesh extrusion + chitin discard."""
    if not height_path.exists():
        return False
    with Image.open(height_path) as im:
        mask = mask_from_height_image(im, method=method, min_luma=min_luma)
    mask_path.parent.mkdir(parents=True, exist_ok=True)
    mask.save(mask_path, format="PNG", optimize=True)
    return True


def _is_edge_strip_luma(v: int) -> bool:
    return (EDGE_FRAME_LO <= v <= EDGE_FRAME_HI) or (EDGE_MATTE_LO <= v <= EDGE_MATTE_HI)


def strip_border_matte_frame(img: Image.Image, *, plate: int = PLATE_GRAY) -> Image.Image:
    """Remove common generated matte + inset rectangular frame bands at the canvas edge.

    Only pixels connected to the border through strip-like tones within
    ``EDGE_STRIP_MAX_DEPTH`` are flattened to plate gray — interior carving is untouched.
    """
    w, h = img.size
    src = list(img.convert("L").getdata())
    if len(src) != w * h:
        return img
    n = w * h

    def idx(x: int, y: int) -> int:
        return y * w + x

    strip = [False] * n
    q: deque[tuple[int, int, int]] = deque()

    def seed(x: int, y: int) -> None:
        i = idx(x, y)
        if _is_edge_strip_luma(src[i]) and not strip[i]:
            strip[i] = True
            q.append((x, y, 0))

    for x in range(w):
        seed(x, 0)
        seed(x, h - 1)
    for y in range(h):
        seed(0, y)
        seed(w - 1, y)

    while q:
        x, y, depth = q.popleft()
        if depth >= EDGE_STRIP_MAX_DEPTH:
            continue
        for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
            if 0 <= nx < w and 0 <= ny < h:
                i = idx(nx, ny)
                if not strip[i] and _is_edge_strip_luma(src[i]):
                    strip[i] = True
                    q.append((nx, ny, depth + 1))

    if not any(strip):
        return img
    out = [plate if strip[i] else src[i] for i in range(n)]
    cleaned = Image.new("L", (w, h))
    cleaned.putdata(out)
    return cleaned


def postprocess_heightmap(raw_bytes: bytes, out_size: int, exaggerate: float = SHOP_EXAGGERATE) -> Image.Image:
    from PIL import ImageOps

    img = Image.open(BytesIO(raw_bytes)).convert("L")
    w, h = img.size
    side = min(w, h)
    left = (w - side) // 2
    top = (h - side) // 2
    img = img.crop((left, top, left + side, top + side))
    img = strip_border_matte_frame(img)
    img = ImageOps.autocontrast(img, cutoff=1)
    if exaggerate != 1.0:
        inv_exp = 1.0 / max(exaggerate, 1e-3)
        lut = []
        for v in range(256):
            d = max(-1.0, min(1.0, (v - 128) / 127.0))
            sign = 1.0 if d >= 0.0 else -1.0
            pushed = sign * (abs(d) ** inv_exp)
            lut.append(max(0, min(255, int(round(128.0 + pushed * 127.0)))))
        img = img.point(lut)
    if img.size != (out_size, out_size):
        img = img.resize((out_size, out_size), Image.LANCZOS)
    return img
