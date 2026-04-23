"""Generate a classic-style mahjong tile atlas.

Draws traditional tile faces procedurally with Pillow: ivory background, red/green/
black ink, circle pips for dots, stylized stalks for bamboo, CJK glyphs for
characters/winds/dragons, and simple floral motifs for flowers.

Outputs:
  <set_dir>/<code>.png        individual 256x384 RGBA tiles (game-loadable)
  <set_dir>/atlas.png         packed 8-column preview atlas
  <set_dir>/atlas.toml        atlas descriptor
"""

from __future__ import annotations

import argparse
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

TILE_W = 256
TILE_H = 384
COLUMNS = 9

IVORY = (245, 238, 220, 255)
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
]

CHAR_NUMERALS = {
    1: "一", 2: "二", 3: "三", 4: "四", 5: "五",
    6: "六", 7: "七", 8: "八", 9: "九",
}
WAN = "萬"

WIND_GLYPHS = {"EWind": "東", "SWind": "南", "WWind": "西", "NWind": "北"}
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


def new_tile() -> Image.Image:
    """Ivory tile face with a gentle bevel: slight vertical gradient, thin outer
    bezel rim, soft inner border. No aggressive inset shadow — real tiles read
    as flat cream with only a hint of recession."""
    img = Image.new("RGBA", (TILE_W, TILE_H), IVORY)

    # gentle vertical gradient (top 2% brighter, bottom 2% dimmer) for subtle
    # light-from-above feel. 1-pixel-wide strip stretched horizontally.
    grad = Image.new("RGBA", (1, TILE_H), IVORY)
    for y in range(TILE_H):
        t = y / (TILE_H - 1)
        delta = int((t - 0.5) * -8)  # ±4 across the tile
        px = (
            max(0, min(255, IVORY[0] + delta)),
            max(0, min(255, IVORY[1] + delta)),
            max(0, min(255, IVORY[2] + delta - 1)),
            255,
        )
        grad.putpixel((0, y), px)
    img.alpha_composite(grad.resize((TILE_W, TILE_H)))

    draw = ImageDraw.Draw(img)
    # outer bezel rim — warm darker line, 2px
    pad_outer = 6
    draw.rounded_rectangle(
        (pad_outer, pad_outer, TILE_W - pad_outer, TILE_H - pad_outer),
        radius=22,
        outline=(180, 165, 135, 200),
        width=2,
    )
    # inner recess border — very thin, hint of a step-down to the face
    pad = 14
    draw.rounded_rectangle(
        (pad, pad, TILE_W - pad, TILE_H - pad),
        radius=18,
        outline=(205, 190, 160, 180),
        width=1,
    )
    return img


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


def draw_dots(img: Image.Image, n: int) -> None:
    cx, cy = TILE_W / 2, TILE_H / 2
    # scale normalized coords into tile face, leaving margin for border
    sx = TILE_W * 0.36
    sy = TILE_H * 0.40
    r = 28
    for xr, yr, color in _dot_layout(n):
        _pip(img, cx + xr * sx, cy + yr * sy, r, color)


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


def draw_bamboo(img: Image.Image, n: int) -> None:
    if n == 1:
        _bamboo_1_bird(img)
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


# ---------------------------------------------------------------- characters (wan)

def draw_char(img: Image.Image, n: int, font_num: ImageFont.FreeTypeFont,
              font_wan: ImageFont.FreeTypeFont) -> None:
    """Numeral dominates the top 2/3; 萬 sits smaller + lighter below."""
    numeral = CHAR_NUMERALS[n]
    color = INK_RED if n == 1 else INK_BLACK
    draw_centered_text(img, numeral, font_num, color, (TILE_W // 2, int(TILE_H * 0.32)))
    draw_centered_text(img, WAN, font_wan, INK_RED, (TILE_W // 2, int(TILE_H * 0.75)))


# ---------------------------------------------------------------- winds & dragons

def draw_wind(img: Image.Image, code: str, font: ImageFont.FreeTypeFont) -> None:
    draw_centered_text(img, WIND_GLYPHS[code], font, INK_BLACK, (TILE_W // 2, TILE_H // 2))


def draw_dragon(img: Image.Image, code: str, font: ImageFont.FreeTypeFont,
                font_watermark: ImageFont.FreeTypeFont | None = None) -> None:
    if code == "DWhite":
        # White dragon: thin double-line rectangle + faint 白 watermark.
        # Traditional tiles leave the face mostly blank but indicate intent
        # with a border; the watermark disambiguates it from a bad tile face.
        draw = ImageDraw.Draw(img)
        pad = 54
        rect = (pad, pad + 50, TILE_W - pad, TILE_H - pad - 50)
        draw.rectangle(rect, outline=INK_BLUE, width=3)
        inner = (rect[0] + 6, rect[1] + 6, rect[2] - 6, rect[3] - 6)
        draw.rectangle(inner, outline=INK_BLUE, width=1)
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
    # index numeral in the top-left corner
    draw_centered_text(img, str(idx), font_num, color,
                       (int(TILE_W * 0.22), int(TILE_H * 0.15)))
    # motif
    FLOWER_MOTIFS[code](img, color)
    # glyph label at bottom
    glyph, _ = FLOWER_NAMES[code]
    draw_centered_text(img, glyph, font_glyph, color,
                       (TILE_W // 2, int(TILE_H * 0.88)))


# ---------------------------------------------------------------- dispatch

def render_tile(code: str, fonts: dict[str, ImageFont.FreeTypeFont]) -> Image.Image:
    img = new_tile()
    if code.startswith("B"):
        draw_bamboo(img, int(code[1:]))
    elif code.startswith("C"):
        draw_char(img, int(code[1:]), fonts["num_cjk"], fonts["wan"])
    elif code.startswith("D") and code not in DRAGON_GLYPHS and code != "DWhite":
        # "D1".."D9" = dots
        draw_dots(img, int(code[1:]))
    elif code.endswith("Wind"):
        draw_wind(img, code, fonts["big_cjk"])
    elif code in ("DRed", "DGreen", "DWhite"):
        draw_dragon(img, code, fonts["big_cjk"], fonts.get("watermark"))
    elif code.startswith("Flower"):
        draw_flower(img, code, fonts["flower_num"], fonts["flower_glyph"])
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


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate classic-style mahjong tile atlas.")
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("assets/sets/classic"),
        help="Output set directory (default: assets/sets/classic)",
    )
    parser.add_argument(
        "--with-tiles",
        action="store_true",
        help="Also emit per-tile PNGs alongside the atlas (default: atlas only)",
    )
    args = parser.parse_args()

    out_dir: Path = args.out
    out_dir.mkdir(parents=True, exist_ok=True)

    fonts = {
        "num_cjk": load_cjk_font(195),      # C1..C9 numeral
        "wan": load_cjk_font(88),           # 萬, more delicate
        "big_cjk": load_cjk_font(210),      # winds + red/green dragons
        "flower_num": load_cjk_font(62),    # flower index digit
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

    if args.with_tiles:
        for code, img in tiles.items():
            img.save(out_dir / f"{code}.png")
        print(f"wrote {len(tiles)} tile PNGs to {out_dir}")


if __name__ == "__main__":
    main()
