"""Generate a classic-style mahjong tile atlas.

Draws traditional tile faces procedurally with Pillow: transparent face (mesh supplies
the body), red/green/black ink, circle pips for dots, stylized stalks for bamboo, CJK
glyphs for characters/winds/dragons, and simple floral motifs for flowers. Corner rank hints
(flowers, seasons, suits: CJK-sized Arabic digits; winds: Latin letter) share the
same upper-area placement and styling family.

Outputs:
  <set_dir>/<code>.png        individual 256x384 RGBA tiles (game-loadable)
  <set_dir>/atlas.png         packed 9-column preview atlas
  <set_dir>/atlas.toml        atlas descriptor

Season tiles (Season1–4) start empty (transparent); unless ``--skip-season-motifs``,
motifs are painted into the atlas in the same run.

To repaint only the season row on an existing atlas:

  python3 scripts/generate_classic_atlas.py paint-seasons --atlas path/to/atlas.png
"""

from __future__ import annotations

import argparse
import math
import random
import sys
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

TILE_W = 256
TILE_H = 384
COLUMNS = 9

INK_BLACK = (30, 28, 24, 255)
INK_RED = (180, 30, 30, 255)
INK_GREEN = (30, 110, 55, 255)
INK_BLUE = (35, 70, 140, 255)
PIP_WHITE = (248, 244, 230, 255)

LAYOUT = [
    "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9",
    "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9",
    "D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9",
    "EWind", "SWind", "WWind", "NWind", "DRed", "DGreen", "DWhite", "", "",
    "Flower1", "Flower2", "Flower3", "Flower4",
    "Season1", "Season2", "Season3", "Season4", "",
]

CHAR_NUMERALS = {
    1: "一", 2: "二", 3: "三", 4: "四", 5: "五",
    6: "六", 7: "七", 8: "八", 9: "九",
}
WAN = "萬"

# Character tiles: C1–C6 use a smaller numeral + lower band (see draw_char); C7–C9 larger.
# Dots: cluster centered; D8 gets an extra nudge (steepest UL diagonal in _dot_layout).
# Spacing + pip radius pull the field away from draw_corner_rank_marker (~22%, 15%).
DOT_CLUSTER_CX = TILE_W / 2
DOT_CLUSTER_CY = TILE_H / 2
DOT_SX = TILE_W * 0.30
DOT_SY = TILE_H * 0.32
DOT_PIP_R = 24

WIND_GLYPHS = {"EWind": "東", "SWind": "南", "WWind": "西", "NWind": "北"}
WIND_LATIN = {"EWind": "E", "SWind": "S", "WWind": "W", "NWind": "N"}
DRAGON_GLYPHS = {"DRed": "中", "DGreen": "發"}  # DWhite handled specially

FLOWER_NAMES = {
    "Flower1": ("梅", "plum"),
    "Flower2": ("蘭", "orchid"),
    "Flower3": ("菊", "chrysanthemum"),
    "Flower4": ("竹", "bamboo"),
}

CJK_FONT_CANDIDATES = [
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/PingFang.ttc",
]

LATIN_FONT_CANDIDATES = [
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
]


def load_cjk_font(size: int) -> ImageFont.FreeTypeFont:
    for path in CJK_FONT_CANDIDATES:
        if Path(path).exists():
            try:
                return ImageFont.truetype(path, size=size)
            except OSError:
                continue
    raise RuntimeError(
        "No CJK font found. Install a CJK font or add its path to CJK_FONT_CANDIDATES."
    )


def load_latin_font(size: int) -> ImageFont.FreeTypeFont:
    for path in LATIN_FONT_CANDIDATES:
        if Path(path).exists():
            try:
                return ImageFont.truetype(path, size=size)
            except OSError:
                continue
    return ImageFont.load_default()


def new_tile() -> Image.Image:
    """Fully transparent base; mesh supplies the tile body. Only ink/decals are drawn."""
    return Image.new("RGBA", (TILE_W, TILE_H), (0, 0, 0, 0))


def draw_centered_text(
    img: Image.Image,
    text: str,
    font: ImageFont.FreeTypeFont,
    fill,
    center: tuple[int, int],
) -> None:
    draw = ImageDraw.Draw(img)
    bbox = draw.textbbox((0, 0), text, font=font)
    w = bbox[2] - bbox[0]
    h = bbox[3] - bbox[1]
    x = center[0] - w // 2 - bbox[0]
    y = center[1] - h // 2 - bbox[1]
    draw.text((x, y), text, font=font, fill=fill)


def draw_corner_rank_marker(
    img: Image.Image,
    text: str,
    font: ImageFont.FreeTypeFont,
    fill,
    *,
    tile_w: int = TILE_W,
    tile_h: int = TILE_H,
) -> None:
    """Shared upper-area anchor for rank / wind letter (flowers, seasons, suits, winds)."""
    draw_centered_text(
        img,
        text,
        font,
        fill,
        (int(tile_w * 0.22), int(tile_h * 0.15)),
    )


def draw_bottom_tile_glyph(
    img: Image.Image,
    glyph: str,
    font: ImageFont.FreeTypeFont,
    fill,
    *,
    tile_w: int = TILE_W,
    tile_h: int = TILE_H,
) -> None:
    """梅蘭菊竹 / 春夏秋冬 — same bottom band placement for flower and season tiles."""
    draw_centered_text(
        img,
        glyph,
        font,
        fill,
        (tile_w // 2, int(tile_h * 0.88)),
    )


# ---------------------------------------------------------------- circle pips
# Traditional HK/Cantonese color scheme:
#   1-dot center: red
#   5-dot center: red
#   7-dot top diagonal of 3: red; bottom 2x2: blue (with one green accent)
#   9-dot top + bottom rows: red; middle row: green
#   2, 3, 4, 6, 8: all blue

def _shade(color, factor: float) -> tuple:
    """Darken (factor<1) or lighten (factor>1) an RGBA color, clamped to [0,255]."""
    r, g, b, a = color
    if factor <= 1.0:
        return (int(r * factor), int(g * factor), int(b * factor), a)
    return (
        min(255, int(r + (255 - r) * (factor - 1))),
        min(255, int(g + (255 - g) * (factor - 1))),
        min(255, int(b + (255 - b) * (factor - 1))),
        a,
    )


def _pip(img: Image.Image, cx: float, cy: float, r: float, color) -> None:
    """Draw a shaded dot pip: engraved cup with a gold bead nucleus."""
    # cast shadow under pip (soft, offset down-right)
    shadow_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow_layer)
    sd.ellipse(
        (cx - r + 2, cy - r + 3, cx + r + 2, cy + r + 3),
        fill=(0, 0, 0, 55),
    )
    shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(2.0))
    img.alpha_composite(shadow_layer)

    draw = ImageDraw.Draw(img)
    dark = _shade(color, 0.65)
    light = _shade(color, 1.25)
    # outer ring: dark rim, transitioning to the ink color
    draw.ellipse((cx - r, cy - r, cx + r, cy + r), fill=dark)
    draw.ellipse(
        (cx - r * 0.92, cy - r * 0.92, cx + r * 0.92, cy + r * 0.92),
        fill=color,
    )
    # highlight crescent on upper-left of the ring
    draw.pieslice(
        (cx - r * 0.96, cy - r * 0.96, cx + r * 0.96, cy + r * 0.96),
        start=190, end=280, fill=light,
    )
    # inner well (recessed face) -- ivory, slightly darker at edge
    draw.ellipse(
        (cx - r * 0.62, cy - r * 0.62, cx + r * 0.62, cy + r * 0.62),
        fill=_shade(PIP_WHITE, 0.92),
    )
    draw.ellipse(
        (cx - r * 0.55, cy - r * 0.55, cx + r * 0.55, cy + r * 0.55),
        fill=PIP_WHITE,
    )
    # gold bead nucleus with tiny highlight
    gold = (215, 170, 60, 255)
    draw.ellipse(
        (cx - r * 0.32, cy - r * 0.32, cx + r * 0.32, cy + r * 0.32),
        fill=color,
    )
    draw.ellipse(
        (cx - r * 0.26, cy - r * 0.26, cx + r * 0.26, cy + r * 0.26),
        fill=gold,
    )
    draw.ellipse(
        (cx - r * 0.12, cy - r * 0.18, cx + r * 0.02, cy - r * 0.04),
        fill=(255, 240, 180, 220),
    )


def _dot_layout(n: int) -> list[tuple[float, float, tuple]]:
    """Return a list of (x_rel, y_rel, color) in normalized [-1, 1] tile-face coords.
    y is downward in screen space, so positive y means lower on tile."""
    B = INK_BLUE
    R = INK_RED
    G = INK_GREEN
    if n == 1:
        return [(0.0, 0.0, R)]
    if n == 2:
        # diagonal, top-right to bottom-left
        return [(0.45, -0.55, B), (-0.45, 0.55, B)]
    if n == 3:
        return [(0.55, -0.65, B), (0.0, 0.0, B), (-0.55, 0.65, B)]
    if n == 4:
        return [(-0.45, -0.55, B), (0.45, -0.55, B),
                (-0.45, 0.55, B),  (0.45, 0.55, B)]
    if n == 5:
        return [(-0.45, -0.60, B), (0.45, -0.60, B),
                (0.0, 0.0, R),
                (-0.45, 0.60, B),  (0.45, 0.60, B)]
    if n == 6:
        return [(-0.45, -0.65, B), (0.45, -0.65, B),
                (-0.45, 0.0, B),   (0.45, 0.0, B),
                (-0.45, 0.65, B),  (0.45, 0.65, B)]
    if n == 7:
        # top diag-3 (top-left to middle-right), bottom 2x2 block
        return [(-0.55, -0.80, R), (0.0, -0.55, R), (0.55, -0.30, R),
                (-0.40, 0.30, B),  (0.40, 0.30, G),
                (-0.40, 0.75, B),  (0.40, 0.75, B)]
    if n == 8:
        # top diag-3 + middle pair + bottom diag-3 (parallel diagonals)
        return [(-0.55, -0.85, B), (0.0, -0.65, B), (0.55, -0.45, B),
                (-0.35, 0.0, B),   (0.35, 0.0, B),
                (-0.55, 0.45, B),  (0.0, 0.65, B),  (0.55, 0.85, B)]
    if n == 9:
        return [(-0.50, -0.70, R), (0.0, -0.70, R), (0.50, -0.70, R),
                (-0.50, 0.0, G),   (0.0, 0.0, G),   (0.50, 0.0, G),
                (-0.50, 0.70, R),  (0.0, 0.70, R),  (0.50, 0.70, R)]
    raise ValueError(f"dots count {n} out of range")


def draw_dots(img: Image.Image, n: int, font_rank_corner: ImageFont.FreeTypeFont) -> None:
    cx, cy = DOT_CLUSTER_CX, DOT_CLUSTER_CY
    sx, sy = DOT_SX, DOT_SY
    r = DOT_PIP_R
    # 8-dot hourglass reaches highest / leftmost; tuck it down slightly and tighten.
    if n == 8:
        cy += TILE_H * 0.045
        sx *= 0.90
        sy *= 0.90
        r = max(20, DOT_PIP_R - 2)
    for xr, yr, color in _dot_layout(n):
        _pip(img, cx + xr * sx, cy + yr * sy, r, color)
    draw_corner_rank_marker(img, str(n), font_rank_corner, INK_BLUE)


# ---------------------------------------------------------------- bamboo stalks

def _bamboo_stalk(
    img: Image.Image,
    cx: float,
    cy: float,
    length: float = 110,
    width: float = 14,
    color=INK_GREEN,
    tilt_deg: float = 0.0,
) -> None:
    """Slender bamboo stalk: nearly-straight cylinder divided into tall flat
    segments by raised knuckle rings. Left highlight + right shadow give a
    cylindrical shade. A small curved leaf sits at the apex.

    When tilt_deg != 0, the stalk is drawn to an offscreen layer and rotated
    around (cx, cy) before compositing."""
    # when tilted, render onto a temporary layer and rotate; recursion with
    # tilt=0 keeps the main drawing path simple
    if tilt_deg != 0.0:
        layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
        _bamboo_stalk(layer, cx, cy, length=length, width=width, color=color, tilt_deg=0.0)
        layer = layer.rotate(-tilt_deg, center=(cx, cy), resample=Image.BICUBIC)
        img.alpha_composite(layer)
        return

    half = length / 2
    top = cy - half
    bot = cy + half

    # cast shadow (narrow, soft, down-right)
    shadow_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow_layer)
    sd.rounded_rectangle(
        (cx - width / 2 + 2, top + 4, cx + width / 2 + 3, bot + 4),
        radius=width / 2,
        fill=(0, 0, 0, 50),
    )
    shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(2.5))
    img.alpha_composite(shadow_layer)

    draw = ImageDraw.Draw(img)
    dark = _shade(color, 0.50)
    mid = _shade(color, 0.78)
    light = _shade(color, 1.20)
    hi = _shade(color, 1.35)

    # number of segments scales with length: shorter stalks get 3, longer 4
    n_seg = 4 if length >= 100 else 3
    # knuckles are raised rings, not gaps; allocate a small portion of each
    # segment boundary to the ring itself
    ring_h = max(3, int(width * 0.35))

    # draw the main cylindrical body first (one rounded rect, not per-segment
    # ellipses — this is what kills the soybean look)
    body_rect = (cx - width / 2, top, cx + width / 2, bot)
    draw.rounded_rectangle(body_rect, radius=width / 2, fill=color)

    # vertical cylinder shading: left highlight + right shadow as thin strips
    draw.rectangle(
        (cx - width / 2 + 1, top + 4, cx - width / 2 + 3, bot - 4),
        fill=light,
    )
    draw.rectangle(
        (cx + width / 2 - 3, top + 4, cx + width / 2 - 1, bot - 4),
        fill=mid,
    )
    # tiny specular sheen, 1px
    draw.line(
        (cx - width / 2 + 2, top + 6, cx - width / 2 + 2, bot - 6),
        fill=hi, width=1,
    )

    # knuckle rings: horizontal bands that visibly protrude (darker shadow band
    # + a brighter highlight line just above) at each segment boundary
    seg_h = length / n_seg
    for s in range(1, n_seg):
        ky = top + s * seg_h
        # the ring extends slightly beyond the stalk width on both sides
        ring_w = width + 3
        ring_rect = (
            cx - ring_w / 2,
            ky - ring_h / 2,
            cx + ring_w / 2,
            ky + ring_h / 2,
        )
        # dark underside of the ring
        draw.rounded_rectangle(ring_rect, radius=ring_h / 2, fill=dark)
        # mid-tone top of the ring (gives the raised feel)
        draw.rounded_rectangle(
            (ring_rect[0] + 1, ring_rect[1], ring_rect[2] - 1, ring_rect[1] + ring_h / 2),
            radius=ring_h / 2,
            fill=mid,
        )
        # bright hairline on top edge
        draw.line(
            (cx - ring_w / 2 + 2, ky - ring_h / 2 + 1,
             cx + ring_w / 2 - 2, ky - ring_h / 2 + 1),
            fill=hi, width=1,
        )

    # top cap: a small darker ellipse so the stalk reads as cut/hollow
    draw.ellipse(
        (cx - width / 2 + 1, top - 1, cx + width / 2 - 1, top + 4),
        fill=dark,
    )

    # single slender leaf angled up-right from the top — narrow sickle shape
    leaf_color = _shade(color, 0.78)
    leaf_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ld = ImageDraw.Draw(leaf_layer)
    lx = cx + width * 0.3
    ly = top - width * 0.2
    lw = width * 1.4
    lh = width * 0.4
    ld.chord(
        (lx - lw / 2, ly - lh / 2, lx + lw / 2, ly + lh / 2),
        start=200, end=20, fill=leaf_color,
    )
    # leaf vein
    ld.line(
        (lx - lw * 0.4, ly, lx + lw * 0.45, ly - 1),
        fill=_shade(leaf_color, 0.7), width=1,
    )
    leaf_layer = leaf_layer.rotate(-35, center=(cx, top), resample=Image.BICUBIC)
    img.alpha_composite(leaf_layer)


def _bamboo_1_bird(img: Image.Image) -> None:
    """1-bamboo: a classic mahjong peacock-sparrow, tight silhouette with a
    sweeping tail, red throat/crest accents, and a perched pose. Rendered on
    an offscreen layer and then pasted so rotations / shading composite cleanly."""
    cx, cy = TILE_W / 2, TILE_H * 0.48
    body = INK_GREEN
    dark = _shade(body, 0.65)
    accent = INK_RED
    accent_dark = _shade(accent, 0.65)
    gold = (215, 170, 60, 255)

    # cast shadow
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    sd.ellipse((cx - 70, cy + 45, cx + 70, cy + 65), fill=(0, 0, 0, 70))
    shadow = shadow.filter(ImageFilter.GaussianBlur(4))
    img.alpha_composite(shadow)

    draw = ImageDraw.Draw(img)

    # sweeping tail: three overlapping feathers fanning down-left
    # each feather is a thin tapered polygon anchored at the body
    anchor_x, anchor_y = cx - 10, cy + 18
    for fi, (angle_deg, length, feather_w, shade) in enumerate([
        (195, 105, 10, dark),
        (205, 120, 12, body),
        (215, 100, 10, body),
    ]):
        ang = math.radians(angle_deg)
        tip_x = anchor_x + math.cos(ang) * length
        tip_y = anchor_y + math.sin(ang) * length
        perp = ang + math.pi / 2
        px = math.cos(perp) * feather_w / 2
        py = math.sin(perp) * feather_w / 2
        draw.polygon(
            [(anchor_x + px, anchor_y + py),
             (tip_x + px * 0.25, tip_y + py * 0.25),
             (tip_x - px * 0.25, tip_y - py * 0.25),
             (anchor_x - px, anchor_y - py)],
            fill=shade,
        )
        # feather rib
        draw.line((anchor_x, anchor_y, tip_x, tip_y),
                  fill=_shade(shade, 0.7), width=1)

    # main body: teardrop pointing up-right, achieved via two overlapping ellipses
    draw.ellipse((cx - 35, cy - 25, cx + 35, cy + 40), fill=body)
    # belly highlight
    draw.ellipse((cx - 22, cy - 5, cx + 18, cy + 35), fill=_shade(body, 1.15))
    # red throat/breast patch
    draw.chord((cx - 25, cy - 10, cx + 20, cy + 30),
               start=20, end=160, fill=accent)

    # wing: folded across the back, as a dark curved shape
    draw.chord((cx - 28, cy - 28, cx + 30, cy + 10),
               start=200, end=355, fill=dark)
    # wing feather lines
    for off in (0, 6, 12):
        draw.line(
            (cx - 20 + off, cy - 15 + off // 2,
             cx + 18 - off // 2, cy - 5 + off // 2),
            fill=_shade(body, 0.45), width=1,
        )

    # head: sits atop the body at a slight forward angle
    head_cx, head_cy = cx + 18, cy - 32
    draw.ellipse(
        (head_cx - 22, head_cy - 22, head_cx + 22, head_cy + 22),
        fill=body,
    )
    # head highlight
    draw.ellipse(
        (head_cx - 16, head_cy - 16, head_cx + 6, head_cy + 4),
        fill=_shade(body, 1.12),
    )

    # crest: one subtle red feather tuft above the head (not a crown)
    crest_pts = [
        (head_cx - 6, head_cy - 18),
        (head_cx - 2, head_cy - 30),
        (head_cx + 6, head_cy - 24),
        (head_cx + 4, head_cy - 15),
    ]
    draw.polygon(crest_pts, fill=accent)

    # beak: two small triangles meeting at a point
    beak_tip = (head_cx + 40, head_cy + 2)
    draw.polygon(
        [(head_cx + 18, head_cy - 4), beak_tip, (head_cx + 18, head_cy + 2)],
        fill=gold,
    )
    draw.polygon(
        [(head_cx + 18, head_cy + 2), beak_tip, (head_cx + 18, head_cy + 8)],
        fill=_shade(gold, 0.75),
    )

    # eye with highlight
    eye_cx, eye_cy = head_cx + 8, head_cy - 4
    draw.ellipse((eye_cx - 5, eye_cy - 5, eye_cx + 5, eye_cy + 5),
                 fill=(250, 245, 230, 255))
    draw.ellipse((eye_cx - 3, eye_cy - 3, eye_cx + 3, eye_cy + 3),
                 fill=INK_BLACK)
    draw.ellipse((eye_cx - 1, eye_cy - 2, eye_cx + 1, eye_cy), fill=(255, 255, 255, 255))

    # perch line (classic sets often show the bird on a twig)
    draw.line(
        (cx - 45, cy + 48, cx + 45, cy + 50),
        fill=BROWN, width=4,
    )
    # feet
    for foot_x in (cx - 8, cx + 14):
        draw.line((foot_x, cy + 40, foot_x, cy + 49), fill=gold, width=3)
        # toes
        draw.line((foot_x, cy + 49, foot_x - 4, cy + 52), fill=gold, width=2)
        draw.line((foot_x, cy + 49, foot_x + 4, cy + 52), fill=gold, width=2)


def _bamboo_layout(n: int) -> list[tuple[float, float, tuple, float]]:
    """Traditional HK bamboo groupings. (x_rel, y_rel, color, tilt_deg).
    Red stalks: 3-top, 5-center, 7-top, 9-middle-row.
    8-bam uses an hourglass layout with top stalks tilted inward and bottom
    stalks tilted outward."""
    G = INK_GREEN
    R = INK_RED
    if n == 2:
        return [(-0.30, 0.0, G, 0.0), (0.30, 0.0, G, 0.0)]
    if n == 3:
        return [(0.0, -0.50, R, 0.0),
                (-0.30, 0.45, G, 0.0), (0.30, 0.45, G, 0.0)]
    if n == 4:
        return [(-0.30, -0.50, G, 0.0), (0.30, -0.50, G, 0.0),
                (-0.30, 0.50, G, 0.0),  (0.30, 0.50, G, 0.0)]
    if n == 5:
        return [(-0.35, -0.55, G, 0.0), (0.35, -0.55, G, 0.0),
                (0.0, 0.0, R, 0.0),
                (-0.35, 0.55, G, 0.0),  (0.35, 0.55, G, 0.0)]
    if n == 6:
        return [(-0.45, -0.45, G, 0.0), (0.0, -0.45, G, 0.0), (0.45, -0.45, G, 0.0),
                (-0.45, 0.45, G, 0.0),  (0.0, 0.45, G, 0.0),  (0.45, 0.45, G, 0.0)]
    if n == 7:
        return [(0.0, -0.80, R, 0.0),
                (-0.45, -0.05, G, 0.0), (0.0, -0.05, G, 0.0), (0.45, -0.05, G, 0.0),
                (-0.45, 0.65, G, 0.0),  (0.0, 0.65, G, 0.0),  (0.45, 0.65, G, 0.0)]
    if n == 8:
        # Hourglass / bowtie: top 4 tilt inward toward center, bottom 4 tilt
        # outward. Two stacked Vs forming an X silhouette.
        return [(-0.50, -0.48, G,  18), (-0.18, -0.48, G,  18),
                (0.18, -0.48, G, -18), (0.50, -0.48, G, -18),
                (-0.50, 0.48, G, -18), (-0.18, 0.48, G, -18),
                (0.18, 0.48, G,  18), (0.50, 0.48, G,  18)]
    if n == 9:
        return [(-0.45, -0.65, G, 0.0), (0.0, -0.65, G, 0.0), (0.45, -0.65, G, 0.0),
                (-0.45, 0.0, R, 0.0),   (0.0, 0.0, R, 0.0),   (0.45, 0.0, R, 0.0),
                (-0.45, 0.65, G, 0.0),  (0.0, 0.65, G, 0.0),  (0.45, 0.65, G, 0.0)]
    raise ValueError(f"bamboo count {n} out of range")


def draw_bamboo(img: Image.Image, n: int, font_rank_corner: ImageFont.FreeTypeFont) -> None:
    if n == 1:
        _bamboo_1_bird(img)
        draw_corner_rank_marker(img, str(n), font_rank_corner, INK_GREEN)
        return
    cx, cy = TILE_W / 2, TILE_H / 2
    sx = TILE_W * 0.34
    sy = TILE_H * 0.38
    # slender stalks: longer + thinner than before. Shrink for dense counts.
    if n <= 4:
        length, width = 120, 15
    elif n <= 6:
        length, width = 100, 13
    else:
        length, width = 82, 11
    for xr, yr, color, tilt in _bamboo_layout(n):
        _bamboo_stalk(img, cx + xr * sx, cy + yr * sy,
                      length=length, width=width, color=color, tilt_deg=tilt)
    draw_corner_rank_marker(img, str(n), font_rank_corner, INK_GREEN)


# ---------------------------------------------------------------- characters (wan)

def draw_char(img: Image.Image, n: int, font_num: ImageFont.FreeTypeFont,
              font_wan: ImageFont.FreeTypeFont, font_rank_corner: ImageFont.FreeTypeFont) -> None:
    """Numeral dominates the upper-mid (centered); 萬 below — C1–6 sit lower + smaller font."""
    numeral = CHAR_NUMERALS[n]
    color = INK_RED if n == 1 else INK_BLACK
    if n <= 6:
        numeral_c = (TILE_W // 2, int(TILE_H * 0.39))
        wan_c = (TILE_W // 2, int(TILE_H * 0.78))
    else:
        numeral_c = (TILE_W // 2, int(TILE_H * 0.34))
        wan_c = (TILE_W // 2, int(TILE_H * 0.75))
    draw_centered_text(img, numeral, font_num, color, numeral_c)
    draw_centered_text(img, WAN, font_wan, INK_RED, wan_c)
    draw_corner_rank_marker(img, str(n), font_rank_corner, color)


# ---------------------------------------------------------------- winds & dragons

def draw_wind(img: Image.Image, code: str, font: ImageFont.FreeTypeFont,
              font_wind_corner: ImageFont.FreeTypeFont) -> None:
    draw_centered_text(img, WIND_GLYPHS[code], font, INK_BLACK, (TILE_W // 2, TILE_H // 2))
    draw_corner_rank_marker(img, WIND_LATIN[code], font_wind_corner, INK_BLACK)


def draw_dragon(img: Image.Image, code: str, font: ImageFont.FreeTypeFont,
                font_watermark: ImageFont.FreeTypeFont | None = None) -> None:
    if code == "DWhite":
        # White dragon: blue double frame on the tile face (same inset as the
        # inner rounded recess from new_tile), not a tight box around the 白.
        draw = ImageDraw.Draw(img)
        pad_face = 14
        draw.rounded_rectangle(
            (pad_face, pad_face, TILE_W - pad_face, TILE_H - pad_face),
            radius=18,
            outline=INK_BLUE,
            width=3,
        )
        pad_inner = 22
        draw.rounded_rectangle(
            (pad_inner, pad_inner, TILE_W - pad_inner, TILE_H - pad_inner),
            radius=14,
            outline=INK_BLUE,
            width=1,
        )
        # faint 白 watermark
        if font_watermark is not None:
            wm = Image.new("RGBA", img.size, (0, 0, 0, 0))
            draw_centered_text(
                wm, "白", font_watermark,
                (INK_BLUE[0], INK_BLUE[1], INK_BLUE[2], 38),
                (TILE_W // 2, TILE_H // 2),
            )
            img.alpha_composite(wm)
        return
    color = INK_RED if code == "DRed" else INK_GREEN
    draw_centered_text(img, DRAGON_GLYPHS[code], font, color, (TILE_W // 2, TILE_H // 2))


# ---------------------------------------------------------------- flowers

BROWN = (105, 70, 40, 255)
LEAF_GREEN = (60, 120, 55, 255)
LEAF_GREEN_DARK = (35, 85, 35, 255)


def _small_blossom(draw: ImageDraw.ImageDraw, cx: float, cy: float,
                   r: float, petal_color, center_color=(245, 220, 90, 255)) -> None:
    """5-petal blossom with a yellow center. Used for plum + chrysanthemum accents."""
    petal_r = r * 0.55
    ring_r = r * 0.55
    for k in range(5):
        theta = -math.pi / 2 + k * (2 * math.pi / 5)
        px = cx + math.cos(theta) * ring_r
        py = cy + math.sin(theta) * ring_r
        draw.ellipse(
            (px - petal_r, py - petal_r, px + petal_r, py + petal_r),
            fill=petal_color,
        )
    # stamen
    sr = r * 0.22
    draw.ellipse((cx - sr, cy - sr, cx + sr, cy + sr), fill=center_color)


def _plum_motif(img: Image.Image, color) -> None:
    """Plum blossom: three blossoms clustered on a dark twig."""
    draw = ImageDraw.Draw(img)
    # twig
    draw.line((TILE_W * 0.25, TILE_H * 0.55, TILE_W * 0.75, TILE_H * 0.30),
              fill=BROWN, width=5)
    draw.line((TILE_W * 0.50, TILE_H * 0.42, TILE_W * 0.60, TILE_H * 0.55),
              fill=BROWN, width=4)
    draw.line((TILE_W * 0.38, TILE_H * 0.47, TILE_W * 0.30, TILE_H * 0.40),
              fill=BROWN, width=3)
    # three blossoms at branch tips
    _small_blossom(draw, TILE_W * 0.28, TILE_H * 0.38, 24, color)
    _small_blossom(draw, TILE_W * 0.72, TILE_H * 0.30, 28, color)
    _small_blossom(draw, TILE_W * 0.60, TILE_H * 0.55, 22, color)
    # a bud
    draw.ellipse(
        (TILE_W * 0.48 - 8, TILE_H * 0.40 - 8, TILE_W * 0.48 + 8, TILE_H * 0.40 + 8),
        fill=color,
    )


def _orchid_motif(img: Image.Image, color) -> None:
    """Orchid: long arching leaves sweeping outward, plus a small bloom."""
    draw = ImageDraw.Draw(img)
    cx = TILE_W / 2
    base_y = TILE_H * 0.60
    # draw several leaves as thick bezier-approximation polylines
    # each leaf: list of (x, y) points from base tapering outward
    leaves = [
        # long left arch
        [(cx - 4, base_y), (cx - 30, base_y - 30), (cx - 60, base_y - 65),
         (cx - 85, base_y - 85), (cx - 95, base_y - 75)],
        # long right arch
        [(cx + 4, base_y), (cx + 30, base_y - 30), (cx + 60, base_y - 65),
         (cx + 85, base_y - 85), (cx + 95, base_y - 75)],
        # medium upward curl left
        [(cx - 2, base_y), (cx - 15, base_y - 40), (cx - 25, base_y - 80),
         (cx - 20, base_y - 115)],
        # medium upward curl right
        [(cx + 2, base_y), (cx + 15, base_y - 40), (cx + 25, base_y - 80),
         (cx + 20, base_y - 115)],
        # tall center
        [(cx, base_y), (cx, base_y - 60), (cx + 5, base_y - 110), (cx + 15, base_y - 140)],
    ]
    for pts in leaves:
        for i in range(len(pts) - 1):
            w = max(2, 7 - i * 2)
            draw.line((pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1]),
                      fill=LEAF_GREEN_DARK, width=w + 2)
            draw.line((pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1]),
                      fill=LEAF_GREEN, width=w)
    # a small orchid bloom in the accent color
    _small_blossom(draw, cx - 8, base_y - 125, 20, color)


def _chrysanthemum_motif(img: Image.Image, color) -> None:
    """Chrysanthemum: petals placed on a phyllotactic spiral (golden angle).

    r(n) = c * sqrt(n), theta(n) = n * golden_angle.
    Petals are drawn smallest-first (innermost) so outer petals overlap on top,
    giving the natural layered look."""
    cx, cy = TILE_W / 2, int(TILE_H * 0.42)
    dark = _shade(color, 0.62)
    light = _shade(color, 1.22)

    golden_angle = math.pi * (3 - math.sqrt(5))  # ~2.39996 rad ≈ 137.5°
    c = 6.8            # radial scale
    n_petals = 75      # density of the bloom
    max_r = 68         # outer clip

    petals: list[tuple[float, float, float, float]] = []
    for n in range(n_petals):
        r = c * math.sqrt(n)
        if r > max_r:
            break
        theta = n * golden_angle
        px = cx + math.cos(theta) * r
        py = cy + math.sin(theta) * r
        petals.append((px, py, theta, r))

    # draw from center outward so outer petals paint on top
    for px, py, theta, r in petals:
        # petal grows with distance from center, then shrinks at the rim
        t = r / max_r
        length = 18 + 22 * (1 - abs(t - 0.65) * 1.3)
        length = max(10, length)
        petal_w = length * 0.45

        # shade ring: inner paler, middle saturated, outer darker
        if r < max_r * 0.35:
            outer = color
            inner = light
        elif r < max_r * 0.75:
            outer = dark
            inner = color
        else:
            outer = _shade(color, 0.55)
            inner = dark

        layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
        ld = ImageDraw.Draw(layer)
        # petal is an elongated ellipse centered offset outward along the ray,
        # so its base meets the previous layer and tip extends outward
        shift = length * 0.30
        ex = px + math.cos(theta) * shift
        ey = py + math.sin(theta) * shift
        ld.ellipse(
            (ex - length / 2, ey - petal_w / 2,
             ex + length / 2, ey + petal_w / 2),
            fill=outer,
        )
        ld.ellipse(
            (ex - length / 2 + 2, ey - petal_w / 2 + 1,
             ex + length / 2 - 4, ey + petal_w / 2 - 1),
            fill=inner,
        )
        rotated = layer.rotate(
            -math.degrees(theta), center=(ex, ey), resample=Image.BICUBIC,
        )
        img.alpha_composite(rotated)

    # tight yellow disc florets at center
    draw = ImageDraw.Draw(img)
    draw.ellipse((cx - 10, cy - 10, cx + 10, cy + 10), fill=(235, 190, 70, 255))
    draw.ellipse((cx - 5, cy - 5, cx + 5, cy + 5), fill=(250, 225, 120, 255))


def _bamboo_plant_motif(img: Image.Image, color) -> None:
    """Bamboo flower tile: three slender stalks with leaf clusters (no blossom)."""
    # draw three thin stalks at different heights
    stalks = [
        (TILE_W * 0.30, TILE_H * 0.70, 140),
        (TILE_W * 0.50, TILE_H * 0.68, 170),
        (TILE_W * 0.70, TILE_H * 0.72, 130),
    ]
    draw = ImageDraw.Draw(img)
    for sx, sy, slen in stalks:
        # stalk itself (thin + shaded)
        draw.line((sx, sy, sx, sy - slen), fill=LEAF_GREEN_DARK, width=5)
        draw.line((sx - 1, sy, sx - 1, sy - slen), fill=LEAF_GREEN, width=2)
        # node marks
        for frac in (0.3, 0.6, 0.85):
            ny = sy - slen * frac
            draw.line((sx - 5, ny, sx + 5, ny), fill=LEAF_GREEN_DARK, width=2)
        # leaves — a few narrow triangles angled up-and-out near the top
        leaf_tip = sy - slen
        for dx_sign in (-1, 1):
            for leaf_y_off, leaf_len, leaf_angle in [(0, 30, 0.5), (18, 24, 0.8)]:
                lx = sx
                ly = leaf_tip + leaf_y_off
                ex = lx + dx_sign * leaf_len * math.cos(leaf_angle)
                ey = ly - leaf_len * math.sin(leaf_angle)
                # leaf as an elongated triangle
                draw.polygon(
                    [(lx, ly),
                     (ex + dx_sign * 3, ey + 2),
                     (ex - dx_sign * 2, ey + 8)],
                    fill=LEAF_GREEN,
                )
                # leaf vein
                draw.line((lx, ly, ex, ey), fill=LEAF_GREEN_DARK, width=1)


FLOWER_MOTIFS = {
    "Flower1": _plum_motif,
    "Flower2": _orchid_motif,
    "Flower3": _chrysanthemum_motif,
    "Flower4": _bamboo_plant_motif,
}


def draw_flower(img: Image.Image, code: str, font_num: ImageFont.FreeTypeFont,
                font_glyph: ImageFont.FreeTypeFont) -> None:
    idx = int(code[-1])
    color = (INK_RED, (140, 50, 140, 255), INK_BLUE, INK_GREEN)[idx - 1]
    draw_corner_rank_marker(img, str(idx), font_num, color)
    FLOWER_MOTIFS[code](img, color)
    glyph, _ = FLOWER_NAMES[code]
    draw_bottom_tile_glyph(img, glyph, font_glyph, color)


# ---------------------------------------------------------------- dispatch

def render_tile(code: str, fonts: dict[str, ImageFont.FreeTypeFont]) -> Image.Image:
    img = new_tile()
    rc = fonts["rank_corner_cjk"]
    if code.startswith("B"):
        draw_bamboo(img, int(code[1:]), rc)
    elif code.startswith("C"):
        cn = int(code[1:])
        num_font = fonts["num_cjk_lo"] if cn <= 6 else fonts["num_cjk"]
        draw_char(img, cn, num_font, fonts["wan"], rc)
    elif code.startswith("D") and code not in DRAGON_GLYPHS and code != "DWhite":
        # "D1".."D9" = dots
        draw_dots(img, int(code[1:]), rc)
    elif code.endswith("Wind"):
        draw_wind(img, code, fonts["big_cjk"], fonts["wind_corner_latin"])
    elif code in ("DRed", "DGreen", "DWhite"):
        draw_dragon(img, code, fonts["big_cjk"], fonts.get("watermark"))
    elif code.startswith("Flower"):
        draw_flower(img, code, fonts["rank_corner_cjk"], fonts["flower_glyph"])
    elif code.startswith("Season"):
        # Ivory face only; season artwork is composited after the atlas is built.
        pass
    else:
        raise ValueError(f"unknown tile code {code}")
    return img


def build_atlas(tiles: dict[str, Image.Image]) -> Image.Image:
    rows = math.ceil(len(LAYOUT) / COLUMNS)
    atlas = Image.new("RGBA", (COLUMNS * TILE_W, rows * TILE_H), (0, 0, 0, 0))
    for i, code in enumerate(LAYOUT):
        if not code:
            continue
        col = i % COLUMNS
        row = i // COLUMNS
        atlas.paste(tiles[code], (col * TILE_W, row * TILE_H))
    return atlas


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


# --- season tile painting (same grid as LAYOUT / pack_atlas) ---

_SEASON_FLOWER_ROW = 4
SEASON_RENDER_SCALE = 2

SeasonSpec = tuple[int, str, tuple[int, int, int], str]

CLASSIC_SPECS: list[SeasonSpec] = [
    (1, "春", (218, 72, 92), "spring"),
    (2, "夏", (228, 138, 48), "summer"),
    (3, "秋", (188, 72, 38), "autumn"),
    (4, "冬", (72, 128, 198), "winter"),
]


@dataclass(frozen=True)
class TileSetProfile:
    """Season cell content (rank, glyphs, motif ids)."""

    season_specs: list[SeasonSpec]


DEFAULT_PROFILE = TileSetProfile(season_specs=CLASSIC_SPECS)


def quad_bezier(p0: tuple[float, float], p1: tuple[float, float], p2: tuple[float, float], n: int) -> list[tuple[float, float]]:
    out: list[tuple[float, float]] = []
    for i in range(n):
        t = i / max(1, n - 1)
        omt = 1.0 - t
        x = omt * omt * p0[0] + 2 * omt * t * p1[0] + t * t * p2[0]
        y = omt * omt * p0[1] + 2 * omt * t * p1[1] + t * t * p2[1]
        out.append((x, y))
    return out


def cubic_point(
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    t: float,
) -> tuple[float, float]:
    omt = 1.0 - t
    x = omt**3 * p0[0] + 3 * omt**2 * t * p1[0] + 3 * omt * t**2 * p2[0] + t**3 * p3[0]
    y = omt**3 * p0[1] + 3 * omt**2 * t * p1[1] + 3 * omt * t**2 * p2[1] + t**3 * p3[1]
    return (x, y)


def cubic_deriv(
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    t: float,
) -> tuple[float, float]:
    omt = 1.0 - t
    x = 3 * omt**2 * (p1[0] - p0[0]) + 6 * omt * t * (p2[0] - p1[0]) + 3 * t**2 * (p3[0] - p2[0])
    y = 3 * omt**2 * (p1[1] - p0[1]) + 6 * omt * t * (p2[1] - p1[1]) + 3 * t**2 * (p3[1] - p2[1])
    return (x, y)


def cubic_bezier(
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    n: int,
) -> list[tuple[float, float]]:
    return [cubic_point(p0, p1, p2, p3, i / max(1, n - 1)) for i in range(n)]


def polyline_length(pts: list[tuple[float, float]]) -> float:
    return sum(math.hypot(pts[i + 1][0] - pts[i][0], pts[i + 1][1] - pts[i][1]) for i in range(len(pts) - 1))


def point_along_polyline(pts: list[tuple[float, float]], frac: float) -> tuple[float, float, tuple[float, float]]:
    """Point at fraction of arc length + unit tangent (toward increasing arc)."""
    total = polyline_length(pts)
    if total < 1e-6:
        return (pts[0][0], pts[0][1], (1.0, 0.0))
    target = max(0.0, min(1.0, frac)) * total
    acc = 0.0
    for i in range(len(pts) - 1):
        a, b = pts[i], pts[i + 1]
        seg = math.hypot(b[0] - a[0], b[1] - a[1])
        if acc + seg >= target - 1e-4:
            t = (target - acc) / seg if seg > 1e-6 else 0.0
            x = a[0] + (b[0] - a[0]) * t
            y = a[1] + (b[1] - a[1]) * t
            tx, ty = (b[0] - a[0]) / seg, (b[1] - a[1]) / seg
            return (x, y, (tx, ty))
        acc += seg
    tx = pts[-1][0] - pts[-2][0]
    ty = pts[-1][1] - pts[-2][1]
    ln = math.hypot(tx, ty) or 1.0
    return (pts[-1][0], pts[-1][1], (tx / ln, ty / ln))


def unit_perp(tangent: tuple[float, float]) -> tuple[float, float]:
    tx, ty = tangent
    nx, ny = -ty, tx
    ln = math.hypot(nx, ny) or 1.0
    return (nx / ln, ny / ln)


def draw_plum_blossom_cluster(
    d: ImageDraw.ImageDraw,
    x: float,
    y: float,
    br: float,
    pink: tuple[int, int, int, int],
    rim_p: tuple[int, int, int, int],
) -> None:
    # Soft base so petals read larger against the cream tile.
    glow_r = br * 1.22
    d.ellipse([x - glow_r, y - glow_r, x + glow_r, y + glow_r], fill=(255, 205, 218, 115))
    pr = br * 0.58
    ph = br * 0.44
    ring_r = br * 0.50
    for k in range(5):
        ang = k * (2 * math.pi / 5) + 0.2
        ox2 = math.cos(ang) * ring_r
        oy2 = math.sin(ang) * ring_r
        d.ellipse(
            [x + ox2 - pr, y + oy2 - ph, x + ox2 + pr, y + oy2 + ph],
            fill=pink,
            outline=rim_p,
            width=3,
        )
    cr = br * 0.34
    d.ellipse([x - cr, y - cr, x + cr, y + cr], fill=(255, 248, 252, 255))
    sr = 5.5
    d.ellipse([x - sr, y - sr, x + sr, y + sr], fill=(255, 185, 205, 255))
    d.ellipse([x - 2.5, y - 2.5, x + 2.5, y + 2.5], fill=(255, 235, 240, 255))


def draw_leaf_bud(
    d: ImageDraw.ImageDraw,
    x: float,
    y: float,
    tangent: tuple[float, float],
    scale: float = 1.0,
) -> None:
    """Small sprouting leaf pair along branch direction."""
    tx, ty = tangent
    ln = math.hypot(tx, ty) or 1.0
    tx, ty = tx / ln, ty / ln
    px, py = -ty, tx
    stem = (88, 118, 72, 255)
    dark = (48, 82, 48, 255)
    for sign in (-1, 1):
        bx = x + sign * px * 6 * scale
        by = y + sign * py * 6 * scale
        d.ellipse(
            [bx - 10 * scale, by - 14 * scale, bx + 12 * scale, by + 8 * scale],
            fill=(86, 138, 78, 255),
            outline=dark,
            width=2,
        )
        d.ellipse(
            [bx - 4 * scale, by - 10 * scale, bx + 6 * scale, by + 2 * scale],
            fill=(118, 178, 102, 255),
        )
    stroke_thick_line(d, (x, y), (x + tx * 8 * scale, y + ty * 8 * scale), 5, stem)


def stroke_thick_line(
    draw: ImageDraw.ImageDraw,
    a: tuple[float, float],
    b: tuple[float, float],
    width: float,
    fill: tuple[int, int, int, int],
) -> None:
    """Soft-cap stroke using overlapping discs (PIL lines are harsh at 2×)."""
    n = max(8, int(math.hypot(b[0] - a[0], b[1] - a[1]) / 3))
    for i in range(n + 1):
        t = i / max(1, n)
        x = a[0] + (b[0] - a[0]) * t
        y = a[1] + (b[1] - a[1]) * t
        hw = width / 2
        draw.ellipse([x - hw, y - hw, x + hw, y + hw], fill=fill)


def draw_motif_layer(size: tuple[int, int], kind: str, rgb: tuple[int, int, int], rank: int) -> Image.Image:
    """Bold vector-style motifs (no blur); 2× canvas + LANCZOS downscale only."""
    w, h = size
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer, "RGBA")
    cx, cy = w / 2, h * 0.42

    if kind == "spring":
        # One continuous flowering branch: main limb + side twig, blossoms on pedicels.
        p0 = (cx - 148, cy + 118)
        p1 = (cx - 55, cy + 35)
        p2 = (cx + 55, cy - 35)
        p3 = (cx + 132, cy - 138)
        main_pts = cubic_bezier(p0, p1, p2, p3, 72)

        t_tw = 0.46
        pos_tw = cubic_point(p0, p1, p2, p3, t_tw)
        dv_tw = cubic_deriv(p0, p1, p2, p3, t_tw)
        t_len = math.hypot(dv_tw[0], dv_tw[1]) or 1.0
        tan_m = (dv_tw[0] / t_len, dv_tw[1] / t_len)
        perp_m = unit_perp(tan_m)
        twig_hi = (
            pos_tw[0] + perp_m[0] * 38 + tan_m[0] * 8,
            pos_tw[1] + perp_m[1] * 38 + tan_m[1] * 8,
        )
        twig_tip = (
            pos_tw[0] + perp_m[0] * 78 + tan_m[0] * 52,
            pos_tw[1] + perp_m[1] * 78 + tan_m[1] * 52,
        )
        twig_pts = quad_bezier(pos_tw, twig_hi, twig_tip, 28)

        bark_outer = (68, 44, 28, 255)
        bark_inner = (118, 82, 54, 255)
        for path, w_outer, w_inner in ((main_pts, 15, 9), (twig_pts, 11, 6)):
            for w, col in ((w_outer, bark_outer), (w_inner, bark_inner)):
                for i in range(len(path) - 1):
                    stroke_thick_line(d, path[i], path[i + 1], w, col)

        # Foliage under flowers so blossoms stay visually dominant.
        lf_x, lf_y, lf_t = point_along_polyline(main_pts, 0.20)
        draw_leaf_bud(d, lf_x, lf_y, lf_t, scale=1.0)
        if len(twig_pts) >= 4:
            wx, wy, wt = twig_pts[-4][0], twig_pts[-4][1], (
                twig_pts[-3][0] - twig_pts[-5][0],
                twig_pts[-3][1] - twig_pts[-5][1],
            )
            wl = math.hypot(wt[0], wt[1]) or 1.0
            draw_leaf_bud(d, wx, wy, (wt[0] / wl, wt[1] / wl), scale=0.85)

        pink = (255, 138, 168, 255)
        rim_p = (188, 42, 72, 255)
        ped = (96, 64, 42, 255)

        bloom_fracs = [0.11, 0.26, 0.40, 0.54, 0.67, 0.80]
        brs = [24.0, 26.0, 29.0, 25.0, 28.0, 23.0]
        for frac, br, alt in zip(bloom_fracs, brs, [1, -1, 1, -1, 1, -1]):
            bx, by, tng = point_along_polyline(main_pts, frac)
            nx, ny = unit_perp(tng)
            side = alt
            off = 15.0 + br * 0.16
            fx = bx + nx * side * off
            fy = by + ny * side * off
            stroke_thick_line(d, (bx, by), (fx, fy), 5, ped)
            draw_plum_blossom_cluster(d, fx, fy, br, pink, rim_p)

        # One cluster at twig tip (still “on” the branch system)
        if len(twig_pts) >= 3:
            tx0, ty0 = twig_pts[-2]
            tx1, ty1 = twig_pts[-1]
            tlx = tx1 - tx0
            tly = ty1 - ty0
            tl = math.hypot(tlx, tly) or 1.0
            tt = (tlx / tl, tly / tl)
            pn = unit_perp(tt)
            fx = tx1 + pn[0] * 20
            fy = ty1 + pn[1] * 20
            stroke_thick_line(d, (tx1, ty1), (fx, fy), 4, ped)
            draw_plum_blossom_cluster(d, fx, fy, 24.0, pink, rim_p)

    elif kind == "summer":
        r_disk = 46
        d.ellipse([cx - r_disk, cy - r_disk, cx + r_disk, cy + r_disk], fill=(255, 210, 60, 255), outline=(235, 150, 30, 255), width=4)
        d.ellipse([cx - 22, cy - 24, cx + 18, cy + 14], fill=(255, 236, 140, 255))
        # Chunky rays (filled trapezoids)
        for i in range(12):
            a0 = (i / 12.0) * 2 * math.pi - 0.08
            a1 = (i / 12.0) * 2 * math.pi + 0.08
            r_in, r_out = r_disk + 4, r_disk + 62
            p0 = (cx + math.cos(a0) * r_in, cy + math.sin(a0) * r_in)
            p1 = (cx + math.cos(a1) * r_in, cy + math.sin(a1) * r_in)
            p2 = (cx + math.cos(a1) * r_out, cy + math.sin(a1) * r_out)
            p3 = (cx + math.cos(a0) * r_out, cy + math.sin(a0) * r_out)
            d.polygon([p0, p1, p2, p3], fill=(255, 195, 70, 255), outline=(240, 160, 40, 255))

    elif kind == "autumn":
        def maple_outline(lcx: float, lcy: float, ang: float, scale: float) -> list[tuple[float, float]]:
            poly: list[tuple[float, float]] = []
            for i in range(38):
                t = -math.pi / 2 + (i / 38) * 2 * math.pi
                lobe = 0.5 * math.cos(5 * t) + 0.1 * math.sin(9 * t)
                w = 0.04 * math.sin(15 * t)
                r = scale * (0.46 + 0.34 * lobe + w)
                poly.append((lcx + r * math.cos(t + ang), lcy + r * math.sin(t + ang)))
            return poly

        def polygon_centroid(poly: list[tuple[float, float]]) -> tuple[float, float]:
            a = 0.0
            cx_, cy_ = 0.0, 0.0
            for i in range(len(poly)):
                x0, y0 = poly[i]
                x1, y1 = poly[(i + 1) % len(poly)]
                c = x0 * y1 - x1 * y0
                a += c
                cx_ += (x0 + x1) * c
                cy_ += (y0 + y1) * c
            if abs(a) < 1e-6:
                return (sum(p[0] for p in poly) / len(poly), sum(p[1] for p in poly) / len(poly))
            a *= 0.5
            return (cx_ / (6 * a), cy_ / (6 * a))

        def draw_maple_leaf(
            poly: list[tuple[float, float]],
            fill_rgba: tuple[int, int, int, int],
            rim: tuple[int, int, int, int],
        ) -> None:
            d.polygon(poly, fill=fill_rgba, outline=rim, width=3)
            tip_i = min(range(len(poly)), key=lambda i: poly[i][1])
            tip = poly[tip_i]
            cen = polygon_centroid(poly)
            mx, my = (cen[0] + tip[0]) * 0.5, (cen[1] + tip[1]) * 0.5
            stroke_thick_line(d, (mx, my), (tip[0], tip[1]), 4, (72, 34, 18, 240))
            # Side veins toward lobes
            for j in (-9, 9):
                i2 = (tip_i + j) % len(poly)
                stroke_thick_line(d, (cen[0] * 0.55 + tip[0] * 0.45, cen[1] * 0.55 + tip[1] * 0.45), poly[i2], 3, (120, 58, 32, 160))

        bark_o = (62, 40, 26, 255)
        bark_i = (98, 68, 44, 255)
        rim = (72, 28, 14, 255)

        tw0 = (cx - 125, cy + 105)
        tw1 = (cx - 20, cy + 15)
        tw2 = (cx + 95, cy - 95)
        twig = quad_bezier(tw0, tw1, tw2, 56)

        for w, col in ((13, bark_o), (7, bark_i)):
            for i in range(len(twig) - 1):
                stroke_thick_line(d, twig[i], twig[i + 1], w, col)

        # Leaves along twig: hue variety, alternating sides, slight tilt
        placements: list[tuple[float, float, float, float, tuple[int, int, int]]] = [
            (0.22, 1.0, 44.0, 0.12, (210, 92, 42)),
            (0.38, -1.0, 48.0, -0.2, (225, 125, 48)),
            (0.54, 1.05, 46.0, 0.28, (188, 68, 38)),
            (0.70, -1.08, 42.0, -0.35, (238, 145, 58)),
            (0.86, 1.0, 36.0, 0.08, (198, 78, 44)),
        ]
        decorated: list[tuple[list[tuple[float, float]], tuple[int, int, int]]] = []
        for frac, side, sc, tilt, fill_rgb in placements:
            bx, by, tng = point_along_polyline(twig, frac)
            nx, ny = unit_perp(tng)
            base_ang = math.atan2(tng[1], tng[0])
            lcx = bx + nx * side * 26
            lcy = by + ny * side * 26
            ang = base_ang + math.pi / 2 + tilt * (1 if side > 0 else -1)
            poly = maple_outline(lcx, lcy, ang, sc)
            decorated.append((poly, fill_rgb))

        # Back-to-front by centroid Y so lower leaves paint over upper twig cleanly
        decorated.sort(key=lambda item: polygon_centroid(item[0])[1], reverse=True)
        for poly, fill_rgb in decorated:
            draw_maple_leaf(poly, (*fill_rgb, 255), rim)

        # Two smaller “falling” leaves (no stem) for depth
        for (fx, fy, fa, fs, fcol) in [
            (cx - 88, cy + 62, 0.9, 26.0, (200, 82, 40)),
            (cx + 102, cy + 48, -0.55, 22.0, (218, 118, 52)),
        ]:
            fp = maple_outline(fx, fy, fa, fs)
            d.polygon(fp, fill=(*fcol, 255), outline=rim, width=2)
            tip_i = min(range(len(fp)), key=lambda i: fp[i][1])
            tip = fp[tip_i]
            cen = polygon_centroid(fp)
            mx, my = (cen[0] + tip[0]) * 0.52, (cen[1] + tip[1]) * 0.52
            stroke_thick_line(d, (mx, my), (tip[0], tip[1]), 3, (72, 34, 18, 220))

    else:  # winter
        random.seed(31 + rank * 17)
        ice = (*rgb, 255)
        ice_dark = (max(0, rgb[0] - 35), max(0, rgb[1] - 40), min(255, rgb[2] + 15), 255)
        r0, r1 = 18.0, 84.0
        for arm in range(6):
            a = (arm / 6.0) * 2 * math.pi
            stroke_thick_line(
                d,
                (cx + math.cos(a) * r0, cy + math.sin(a) * r0),
                (cx + math.cos(a) * r1, cy + math.sin(a) * r1),
                12,
                ice_dark,
            )
        for arm in range(6):
            a = (arm / 6.0) * 2 * math.pi
            stroke_thick_line(
                d,
                (cx + math.cos(a) * (r0 + 3), cy + math.sin(a) * (r0 + 3)),
                (cx + math.cos(a) * (r1 - 4), cy + math.sin(a) * (r1 - 4)),
                6,
                ice,
            )
            b = a + math.pi / 2
            stroke_thick_line(
                d,
                (cx + math.cos(b) * r0, cy + math.sin(b) * r0),
                (cx + math.cos(b) * (r1 * 0.52), cy + math.sin(b) * (r1 * 0.52)),
                8,
                (200, 230, 255, 255),
            )

        d.ellipse([cx - 10, cy - 10, cx + 10, cy + 10], fill=(255, 255, 255, 255), outline=ice_dark, width=2)
        d.ellipse([cx - 4, cy - 4, cx + 4, cy + 4], fill=(*rgb, 255))

        for _ in range(22):
            sx = random.uniform(cx - 100, cx + 100)
            sy = random.uniform(cy + 38, cy + 122)
            sr = random.uniform(1.5, 3.0)
            d.ellipse([sx - sr, sy - sr, sx + sr, sy + sr], fill=(255, 255, 255, 230))

    return layer


def build_tile(
    rank: int,
    glyph: str,
    accent: tuple[int, int, int],
    motif: str,
    corner_rank_font_2x: ImageFont.FreeTypeFont,
    bottom_glyph_font_2x: ImageFont.FreeTypeFont,
) -> Image.Image:
    tw, th = TILE_W * SEASON_RENDER_SCALE, TILE_H * SEASON_RENDER_SCALE
    tile = Image.new("RGBA", (tw, th), (0, 0, 0, 0))

    draw_corner_rank_marker(
        tile,
        str(rank),
        corner_rank_font_2x,
        (*accent, 255),
        tile_w=tw,
        tile_h=th,
    )

    motif_layer = draw_motif_layer((tw, th), motif, accent, rank)
    tile = Image.alpha_composite(tile, motif_layer)

    draw_bottom_tile_glyph(
        tile,
        glyph,
        bottom_glyph_font_2x,
        (*accent, 255),
        tile_w=tw,
        tile_h=th,
    )

    return tile.resize((TILE_W, TILE_H), Image.Resampling.LANCZOS)


def paint_season_row(atlas_png: Path, profile: TileSetProfile = DEFAULT_PROFILE) -> None:
    im = Image.open(atlas_png).convert("RGBA")
    corner_rank_font_2x = load_cjk_font(62 * SEASON_RENDER_SCALE)
    bottom_glyph_font_2x = load_cjk_font(74 * SEASON_RENDER_SCALE)

    for col_offset, (rank, glyph, accent, motif) in enumerate(profile.season_specs):
        col = 4 + col_offset
        tile = build_tile(
            rank,
            glyph,
            accent,
            motif,
            corner_rank_font_2x,
            bottom_glyph_font_2x,
        )
        im.paste(tile, (col * TILE_W, _SEASON_FLOWER_ROW * TILE_H))

    im.save(atlas_png)
    print(f"Updated season cells in {atlas_png}")



def _paint_season_motifs_if_applicable(out_dir: Path, enabled: bool) -> None:
    if not enabled:
        return
    paint_season_row((out_dir / "atlas.png").resolve())


def _main_paint_seasons_only() -> None:
    parser = argparse.ArgumentParser(
        description="Paint Season1–4 into an existing atlas.png (9×5 pack_atlas layout).",
    )
    parser.add_argument(
        "--atlas",
        type=Path,
        required=True,
        help="Path to atlas.png to modify in place.",
    )
    args = parser.parse_args()
    paint_season_row(args.atlas.resolve())


def main() -> None:
    if len(sys.argv) >= 2 and sys.argv[1] == "paint-seasons":
        sys.argv.pop(1)
        _main_paint_seasons_only()
        return

    parser = argparse.ArgumentParser(description="Generate classic-style mahjong tile atlas.")
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("assets/textures/tile_sets/classic"),
        help="Output set directory (default: assets/textures/tile_sets/classic)",
    )
    parser.add_argument(
        "--with-tiles",
        action="store_true",
        help="Also emit per-tile PNGs alongside the atlas (default: atlas only)",
    )
    parser.add_argument(
        "--skip-season-motifs",
        action="store_true",
        help="Skip painting Season1–4 motifs after writing the atlas",
    )
    parser.add_argument(
        "--annotate-indices",
        action="store_true",
        help="After writing atlas, draw 1…N Arabic indices on each tile (see scripts/annotate_atlas_indices.py)",
    )
    args = parser.parse_args()

    out_dir: Path = args.out
    out_dir.mkdir(parents=True, exist_ok=True)

    fonts = {
        "num_cjk": load_cjk_font(186),      # C7–C9 numeral
        "num_cjk_lo": load_cjk_font(156),    # C1–C6 (clear corner rank)
        "wan": load_cjk_font(88),           # 萬, more delicate
        "big_cjk": load_cjk_font(210),      # winds + red/green dragons
        "rank_corner_cjk": load_cjk_font(62),  # flowers, seasons, suit Arabic ranks
        "wind_corner_latin": load_latin_font(34),  # E S W N (same corner as rank)
        "flower_glyph": load_cjk_font(74),  # 梅蘭菊竹
        "watermark": load_cjk_font(170),    # DWhite 白 watermark
    }

    tiles: dict[str, Image.Image] = {}
    for code in LAYOUT:
        if not code:
            continue
        tiles[code] = render_tile(code, fonts)

    atlas = build_atlas(tiles)
    atlas.save(out_dir / "atlas.png")
    write_toml(out_dir / "atlas.toml")
    print(f"wrote atlas.png + atlas.toml to {out_dir}")

    _paint_season_motifs_if_applicable(out_dir, not args.skip_season_motifs)

    if args.annotate_indices:
        _scripts = Path(__file__).resolve().parent
        if str(_scripts) not in sys.path:
            sys.path.insert(0, str(_scripts))
        import annotate_atlas_indices as _atlas_lbl

        n = _atlas_lbl.annotate_atlas_png(out_dir / "atlas.png")
        print(f"annotated atlas with {n} corner indices")

    if args.with_tiles:
        for code, img in tiles.items():
            img.save(out_dir / f"{code}.png")
        print(f"wrote {len(tiles)} tile PNGs to {out_dir}")


if __name__ == "__main__":
    main()
