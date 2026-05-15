#!/usr/bin/env python3
"""Render a Mahjuro palette preview PNG.

Pulls canonical color tokens out of `src/render/theme.rs` (the source of truth
in code) and suit colors out of `src/core/tile.rs`. Adds the material concepts
documented in `COLOR_THEME.md` that aren't real consts yet (Felt, Twilight,
Lacquer, Cinnabar, Tallow). Renders everything as a swatch sheet plus a button
state matrix.

Usage:
    python3 tools/palette_preview.py
    python3 tools/palette_preview.py --out path/to/preview.png --open

Requires Pillow (`pip install pillow` if not already available).
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

REPO_ROOT = Path(__file__).resolve().parents[1]
THEME_RS = REPO_ROOT / "src" / "render" / "theme.rs"
TILE_RS = REPO_ROOT / "src" / "core" / "tile.rs"
FONT_REGULAR = (
    REPO_ROOT / "assets" / "fonts" / "Instrument_Serif" / "InstrumentSerif-Regular.ttf"
)
FONT_ITALIC = (
    REPO_ROOT / "assets" / "fonts" / "Instrument_Serif" / "InstrumentSerif-Italic.ttf"
)


# ───────────────────────────── data model ──────────────────────────────


@dataclass
class Swatch:
    name: str
    rgba: tuple[float, float, float, float]
    note: str = ""

    @property
    def hex(self) -> str:
        r, g, b, _a = self.rgba
        return "#{:02X}{:02X}{:02X}".format(
            round(r * 255), round(g * 255), round(b * 255)
        )

    @property
    def rgb255(self) -> tuple[int, int, int]:
        r, g, b, _a = self.rgba
        return (round(r * 255), round(g * 255), round(b * 255))

    @property
    def luminance(self) -> float:
        r, g, b, _a = self.rgba
        return 0.2126 * r + 0.7152 * g + 0.0722 * b


@dataclass
class Section:
    title: str
    blurb: str
    swatches: list[Swatch]


# ─────────────────────── parsing theme.rs / tile.rs ────────────────────


CONST_RE = re.compile(
    r"pub const (?P<name>[A-Z_]+): \[f32; 4\] = "
    r"\[\s*(?P<r>[\d.]+)\s*,\s*(?P<g>[\d.]+)\s*,"
    r"\s*(?P<b>[\d.]+)\s*,\s*(?P<a>[\d.]+)\s*\]\s*;",
)
DOC_RE = re.compile(r"^\s*///\s?(.*)$")


def parse_theme_consts() -> dict[str, Swatch]:
    """Walk theme.rs collecting every `pub const NAME: [f32;4]` with its
    immediately preceding `///` doc line as the note."""
    consts: dict[str, Swatch] = {}
    pending_doc: list[str] = []
    for line in THEME_RS.read_text(encoding="utf-8").splitlines():
        m = DOC_RE.match(line)
        if m:
            pending_doc.append(m.group(1).strip())
            continue
        m = CONST_RE.search(line)
        if m:
            note = " ".join(pending_doc).strip()
            note = re.sub(r"^`#[0-9A-Fa-f]+`\s*[—-]\s*", "", note)
            consts[m.group("name")] = Swatch(
                name=m.group("name"),
                rgba=(
                    float(m.group("r")),
                    float(m.group("g")),
                    float(m.group("b")),
                    float(m.group("a")),
                ),
                note=note,
            )
        if not line.lstrip().startswith("///"):
            pending_doc.clear()
    return consts


SUIT_RE = re.compile(
    r"Suit::(?P<name>\w+)\s*=>\s*\[\s*(?P<r>[\d.]+)\s*,\s*"
    r"(?P<g>[\d.]+)\s*,\s*(?P<b>[\d.]+)\s*,\s*(?P<a>[\d.]+)\s*\]"
)


DRAGON_BLOCK_RE = re.compile(
    r"Suit::Dragon\s*=>\s*match\s+self\.rank\s*\{(?P<body>.*?)^\s*\}",
    re.DOTALL | re.MULTILINE,
)
RANK_RE = re.compile(
    r"(?P<rank>[123])\s*=>\s*\[\s*(?P<r>[\d.]+)\s*,\s*"
    r"(?P<g>[\d.]+)\s*,\s*(?P<b>[\d.]+)\s*,\s*(?P<a>[\d.]+)\s*\]"
)


def parse_suit_colors() -> list[Swatch]:
    """Grab `Suit::Foo => [...]` for the simple suits, then dive into the
    `Suit::Dragon => match self.rank { ... }` block to expand the three ranks."""
    text = TILE_RS.read_text(encoding="utf-8")
    out: list[Swatch] = []
    seen: set[str] = set()

    def rank_label(rank: str) -> str:
        return {
            "1": "Dragon Chun (中)",
            "2": "Dragon Hatsu (發)",
            "3": "Dragon Haku (白)",
        }[rank]

    for m in SUIT_RE.finditer(text):
        name = m.group("name")
        if name in seen:
            continue
        seen.add(name)
        out.append(
            Swatch(
                name=name,
                rgba=(
                    float(m.group("r")),
                    float(m.group("g")),
                    float(m.group("b")),
                    float(m.group("a")),
                ),
            )
        )

    for dm in DRAGON_BLOCK_RE.finditer(text):
        rank_matches = list(RANK_RE.finditer(dm.group("body")))
        if not rank_matches:
            continue
        for rm in rank_matches:
            out.append(
                Swatch(
                    name=rank_label(rm.group("rank")),
                    rgba=(
                        float(rm.group("r")),
                        float(rm.group("g")),
                        float(rm.group("b")),
                        float(rm.group("a")),
                    ),
                )
            )
        break

    suit_order = [
        "Characters",
        "Bamboos",
        "Dots",
        "Wind",
        "Dragon Chun (中)",
        "Dragon Hatsu (發)",
        "Dragon Haku (白)",
        "Flower",
        "Season",
    ]
    by_name = {s.name: s for s in out}
    return [by_name[n] for n in suit_order if n in by_name] + [
        s for s in out if s.name not in suit_order
    ]


# ────────────────────── tile surface (still hardcoded) ────────────────
# Tile face/edge colors live in the tile *mesh*, not in `theme.rs`. They're
# documented in `COLOR_THEME.md`; if they ever get promoted to consts,
# delete this block and they'll come from `parse_theme_consts()`.


def tile_surface_swatches() -> list[Swatch]:
    return [
        Swatch("TILE_FACE", (0.95, 0.92, 0.85, 1.0), "warm ivory"),
        Swatch("TILE_EDGE_LIGHT", (0.60, 0.48, 0.28, 1.0), "bamboo tan bevel"),
        Swatch("TILE_EDGE_DARK", (0.45, 0.35, 0.20, 1.0), "walnut bevel"),
    ]


# ───────────────── grouping consts into named sections ─────────────────


WALNUT_NAMES = ["WALNUT_INK", "WALNUT_DEEP", "WALNUT_RAISED", "WALNUT_SOFT", "WALNUT_BRIGHT"]
BRASS_NAMES = ["CHAMPAGNE", "GOLD", "BRASS", "ANTIQUE"]
NEUTRAL_NAMES = ["PARCHMENT", "STONE", "UMBER"]
SEMANTIC_NAMES = ["JADE", "RUBY", "AMBER"]
FELT_NAMES = ["FELT_DEEP", "FELT", "FELT_LIT"]
TWILIGHT_NAMES = ["TWILIGHT_INK", "TWILIGHT", "TWILIGHT_GLOW"]
LACQUER_NAMES = ["LACQUER", "CINNABAR"]
TALLOW_NAMES = ["TALLOW"]
RARITY_NAMES = ["STONE", "JADE", "WALNUT_BRIGHT", "CHAMPAGNE"]
RARITY_LABELS = ["common", "uncommon", "rare", "legendary"]


def assemble_sections(consts: dict[str, Swatch]) -> list[Section]:
    def grab(names: list[str]) -> list[Swatch]:
        return [consts[n] for n in names if n in consts]

    sections: list[Section] = [
        Section(
            "Walnut ladder",
            "the room itself — backgrounds, panels, button rests",
            grab(WALNUT_NAMES),
        ),
        Section(
            "Brass",
            "fixtures, headers, currency, selected-tile rims — sparing",
            grab(BRASS_NAMES),
        ),
        Section(
            "Neutrals",
            "paper and stone — text and dividers",
            grab(NEUTRAL_NAMES),
        ),
        Section(
            "Semantic accents",
            "success / danger / warning — desaturated for warm wood",
            grab(SEMANTIC_NAMES),
        ),
        Section(
            "Felt — cabinet linings & tabletops",
            "material color; reach for FELT before JADE on a panel",
            grab(FELT_NAMES),
        ),
        Section(
            "Twilight — the world outside",
            "the cool counterpoint; reach here before any saturated UI blue",
            grab(TWILIGHT_NAMES),
        ),
        Section(
            "Lacquer — black & ceremonial red",
            "deep contrast and ritual — distinct from WALNUT_INK and RUBY",
            grab(LACQUER_NAMES),
        ),
        Section(
            "Tallow — candle bloom highlight",
            "highlights pull toward TALLOW, not toward white",
            grab(TALLOW_NAMES),
        ),
        Section(
            "Tile surface (from tile mesh)",
            "documented in COLOR_THEME.md, not yet promoted to theme.rs consts",
            tile_surface_swatches(),
        ),
    ]
    rarity = []
    for name, label in zip(RARITY_NAMES, RARITY_LABELS):
        if name in consts:
            s = consts[name]
            rarity.append(Swatch(f"{label.upper()} ({s.name})", s.rgba, ""))
    sections.append(
        Section(
            "Rarity spectrum",
            "from theme::color::rarity(tier) — echoes existing tokens",
            rarity,
        )
    )
    return sections


def suit_section() -> Section:
    return Section(
        "Suit colors (src/core/tile.rs)",
        "spread across the wheel for instant readability",
        parse_suit_colors(),
    )


# ───────────────────────── button matrix ───────────────────────────────


def lighten(c: Swatch, t: float) -> Swatch:
    r, g, b, a = c.rgba
    k = max(0.0, min(1.0, t))
    return Swatch(c.name, (r + (1 - r) * k, g + (1 - g) * k, b + (1 - b) * k, a))


def darken(c: Swatch, t: float) -> Swatch:
    r, g, b, a = c.rgba
    k = 1.0 - max(0.0, min(1.0, t))
    return Swatch(c.name, (r * k, g * k, b * k, a))


def alpha(c: Swatch, a: float) -> Swatch:
    r, g, b, _ = c.rgba
    return Swatch(c.name, (r, g, b, a))


@dataclass
class ButtonColors:
    bg: Swatch
    border: Swatch
    text: Swatch


def button_colors(consts: dict[str, Swatch], variant: str, state: str) -> ButtonColors:
    """Mirror of `theme::button_colors` in Rust so the preview reflects the
    real UI surface each button will paint."""
    table = {
        "Default": (consts["WALNUT_SOFT"], consts["BRASS"], consts["PARCHMENT"]),
        "Primary": (consts["WALNUT_BRIGHT"], consts["GOLD"], consts["CHAMPAGNE"]),
        "Danger": (consts["WALNUT_RAISED"], consts["RUBY"], consts["RUBY"]),
        "Subtle": (consts["WALNUT_DEEP"], consts["UMBER"], consts["STONE"]),
    }
    bg_rest, border_rest, text_rest = table[variant]
    if state == "Rest":
        return ButtonColors(bg_rest, border_rest, text_rest)
    if state == "Hover":
        return ButtonColors(lighten(bg_rest, 0.15), consts["GOLD"], consts["CHAMPAGNE"])
    if state == "Press":
        return ButtonColors(darken(bg_rest, 0.18), consts["BRASS"], text_rest)
    if state == "Disabled":
        return ButtonColors(
            darken(bg_rest, 0.35),
            darken(border_rest, 0.40),
            alpha(consts["UMBER"], 0.60),
        )
    raise ValueError(state)


# ───────────────────────────── rendering ───────────────────────────────


W = 1600
PAD = 56
SECTION_GAP = 38
SWATCH_W = 220
SWATCH_H = 130
SWATCH_GAP = 16
LABEL_GAP = 8


def font(path: Path, size: int) -> ImageFont.FreeTypeFont:
    try:
        return ImageFont.truetype(str(path), size)
    except OSError:
        return ImageFont.load_default()


def text_color_on(bg: Swatch) -> tuple[int, int, int]:
    return (15, 12, 9) if bg.luminance > 0.55 else (244, 241, 232)


def draw_swatch(
    draw: ImageDraw.ImageDraw,
    x: int,
    y: int,
    sw: Swatch,
    f_name: ImageFont.FreeTypeFont,
    f_hex: ImageFont.FreeTypeFont,
    f_note: ImageFont.FreeTypeFont,
    parchment: tuple[int, int, int],
    stone: tuple[int, int, int],
) -> None:
    rect = [x, y, x + SWATCH_W, y + SWATCH_H]
    draw.rounded_rectangle(rect, radius=10, fill=sw.rgb255)
    inner = text_color_on(sw)
    draw.text((x + 14, y + 12), sw.name, fill=inner, font=f_name)
    draw.text((x + 14, y + 14 + f_name.size + 4), sw.hex, fill=inner, font=f_hex)
    rgb = sw.rgb255
    rgb_str = f"rgb({rgb[0]}, {rgb[1]}, {rgb[2]})"
    draw.text(
        (x + 14, y + SWATCH_H - 22),
        rgb_str,
        fill=inner,
        font=f_note,
    )
    if sw.note:
        wrap_at = 38
        words = sw.note.split()
        lines, cur = [], ""
        for w in words:
            test = (cur + " " + w).strip()
            if len(test) <= wrap_at:
                cur = test
            else:
                if cur:
                    lines.append(cur)
                cur = w
        if cur:
            lines.append(cur)
        ny = y + SWATCH_H + LABEL_GAP
        for line in lines[:2]:
            draw.text((x + 2, ny), line, fill=stone, font=f_note)
            ny += f_note.size + 2


def draw_button_matrix(
    draw: ImageDraw.ImageDraw,
    consts: dict[str, Swatch],
    x0: int,
    y0: int,
    f_label: ImageFont.FreeTypeFont,
    f_btn: ImageFont.FreeTypeFont,
    f_note: ImageFont.FreeTypeFont,
    parchment: tuple[int, int, int],
    stone: tuple[int, int, int],
) -> int:
    variants = ["Default", "Primary", "Danger", "Subtle"]
    states = ["Rest", "Hover", "Press", "Disabled"]
    cell_w, cell_h = 280, 92
    gap_x, gap_y = 14, 14
    header_h = 30
    col_label_w = 110

    for i, st in enumerate(states):
        cx = x0 + col_label_w + i * (cell_w + gap_x)
        draw.text((cx, y0), st, fill=parchment, font=f_label)
    for j, var in enumerate(variants):
        cy = y0 + header_h + j * (cell_h + gap_y)
        draw.text((x0, cy + cell_h // 2 - f_label.size // 2), var, fill=parchment, font=f_label)
        for i, st in enumerate(states):
            cx = x0 + col_label_w + i * (cell_w + gap_x)
            bc = button_colors(consts, var, st)
            r, g, b, _ = bc.bg.rgba
            bg_rgb = (round(r * 255), round(g * 255), round(b * 255))
            r, g, b, _ = bc.border.rgba
            border_rgb = (round(r * 255), round(g * 255), round(b * 255))
            r, g, b, a = bc.text.rgba
            text_rgb = (round(r * 255), round(g * 255), round(b * 255))
            text_a = max(0.35, a)
            rect = [cx, cy, cx + cell_w, cy + cell_h]
            draw.rounded_rectangle(rect, radius=12, fill=bg_rgb, outline=border_rgb, width=3)
            label = f"{var}"
            tw = draw.textlength(label, font=f_btn)
            draw.text(
                (cx + (cell_w - tw) / 2, cy + cell_h / 2 - f_btn.size / 2),
                label,
                fill=tuple(int(round(c * text_a + bg * (1 - text_a))) for c, bg in zip(text_rgb, bg_rgb)),
                font=f_btn,
            )
    return y0 + header_h + len(variants) * (cell_h + gap_y)


def render(out_path: Path) -> None:
    consts = parse_theme_consts()
    sections = assemble_sections(consts) + [suit_section()]

    f_title = font(FONT_REGULAR, 56)
    f_subtitle = font(FONT_ITALIC, 24)
    f_section = font(FONT_REGULAR, 32)
    f_section_blurb = font(FONT_ITALIC, 18)
    f_swatch_name = font(FONT_REGULAR, 18)
    f_swatch_hex = font(FONT_REGULAR, 16)
    f_swatch_note = font(FONT_REGULAR, 13)
    f_btn_label = font(FONT_REGULAR, 18)
    f_btn = font(FONT_REGULAR, 22)

    cols = (W - 2 * PAD + SWATCH_GAP) // (SWATCH_W + SWATCH_GAP)
    cols = max(4, int(cols))

    walnut_ink = consts["WALNUT_INK"].rgb255
    parchment = consts["PARCHMENT"].rgb255
    champagne = consts["CHAMPAGNE"].rgb255
    stone = consts["STONE"].rgb255
    brass = consts["BRASS"].rgb255

    y = PAD + 80 + 40 + SECTION_GAP
    for sec in sections:
        n = len(sec.swatches)
        rows = (n + cols - 1) // cols
        y += 24 + 22
        y += rows * (SWATCH_H + 30 + LABEL_GAP)
        y += SECTION_GAP
    y += 40 + 30 + 4 * (92 + 14)
    y += PAD

    H = y
    img = Image.new("RGB", (W, H), walnut_ink)
    draw = ImageDraw.Draw(img, "RGBA")

    draw.text((PAD, PAD), "Mahjuro — Walnut, Brass & Felt", fill=champagne, font=f_title)
    draw.text(
        (PAD, PAD + f_title.size + 6),
        "palette preview · generated from src/render/theme.rs + COLOR_THEME.md",
        fill=stone,
        font=f_subtitle,
    )
    rule_y = PAD + f_title.size + f_subtitle.size + 22
    draw.line(
        [(PAD, rule_y), (W - PAD, rule_y)],
        fill=brass + (255,) if len(brass) == 3 else brass,
        width=2,
    )

    y = rule_y + SECTION_GAP
    for sec in sections:
        draw.text((PAD, y), sec.title, fill=champagne, font=f_section)
        if sec.blurb:
            draw.text(
                (PAD, y + f_section.size + 2),
                sec.blurb,
                fill=stone,
                font=f_section_blurb,
            )
        y += f_section.size + 22
        for i, sw in enumerate(sec.swatches):
            col = i % cols
            row = i // cols
            sx = PAD + col * (SWATCH_W + SWATCH_GAP)
            sy = y + row * (SWATCH_H + 30 + LABEL_GAP)
            draw_swatch(
                draw,
                sx,
                sy,
                sw,
                f_swatch_name,
                f_swatch_hex,
                f_swatch_note,
                parchment,
                stone,
            )
        rows = (len(sec.swatches) + cols - 1) // cols
        y += rows * (SWATCH_H + 30 + LABEL_GAP) + SECTION_GAP

    draw.text((PAD, y), "Button matrix", fill=champagne, font=f_section)
    draw.text(
        (PAD, y + f_section.size + 2),
        "from theme::button_colors(variant, state) — every button funnels through here",
        fill=stone,
        font=f_section_blurb,
    )
    y += f_section.size + 30
    draw_button_matrix(
        draw, consts, PAD, y, f_btn_label, f_btn, f_swatch_note, parchment, stone
    )

    out_path.parent.mkdir(parents=True, exist_ok=True)
    img.save(out_path, optimize=True)
    print(f"wrote {out_path} ({W}×{H})")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--out",
        type=Path,
        default=REPO_ROOT / "target" / "palette_preview.png",
        help="output PNG path (default: target/palette_preview.png)",
    )
    ap.add_argument(
        "--open",
        action="store_true",
        help="open the rendered PNG when done (macOS `open`, Linux `xdg-open`)",
    )
    args = ap.parse_args()
    render(args.out)
    if args.open:
        opener = "open" if sys.platform == "darwin" else "xdg-open"
        subprocess.run([opener, str(args.out)], check=False)


if __name__ == "__main__":
    main()
