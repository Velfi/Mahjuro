"""Per-tile content specifications — what should be drawn on each tile face.

Used by atlas generators (whole-set stylings) and the single-tile fixer
(spot-repair of one cell within an existing styled atlas). The specs describe
*content and composition only* — material/palette/ornament are supplied by the
caller's theme or inferred from the reference image.

Arrangements and color conventions are the traditional HK/Cantonese scheme.
"""

from __future__ import annotations


CHAR_NUMERAL = {1: "一", 2: "二", 3: "三", 4: "四", 5: "五",
                6: "六", 7: "七", 8: "八", 9: "九"}

WIND_GLYPH = {"EWind": ("東", "East"), "SWind": ("南", "South"),
              "WWind": ("西", "West"), "NWind": ("北", "North")}

FLOWER_MOTIF = {
    "Flower1": ("plum blossom branch (梅) with 3–5 five-petaled blooms on a "
                "gnarled twig", "梅", 1),
    "Flower2": ("wild orchid (蘭) with long arching leaves and a small "
                "spray of blooms", "蘭", 2),
    "Flower3": ("chrysanthemum (菊) — a single densely-layered flower head "
                "viewed from above", "菊", 3),
    "Flower4": ("bamboo sprig (竹) with 2–3 nodes and several narrow leaf "
                "clusters, NO flower", "竹", 4),
}

DOT_ARRANGEMENT = {
    1: "a single large red pip centered (one concentric-ring dot)",
    2: "two pips on a top-right to bottom-left diagonal, both blue",
    3: "three pips on a top-right to bottom-left diagonal, all blue",
    4: "four pips in a 2x2 square, all blue",
    5: ("five pips in a quincunx: four blue pips in the corners plus one red "
        "pip in the very center"),
    6: "six pips in two columns of three, all blue",
    7: ("seven pips: three red pips on a diagonal across the top half, and a "
        "2x2 block of four blue pips in the bottom half (one of the four may "
        "be green in some traditional variants)"),
    8: ("eight pips: a diagonal of three on top, a horizontal pair in the "
        "middle, a diagonal of three on the bottom — both diagonals slant "
        "the same direction. All blue."),
    9: ("nine pips in a 3x3 grid: top row red, middle row green, bottom row "
        "red"),
}

BAMBOO_ARRANGEMENT = {
    1: ("a single ornate bird perched on a branch — traditional 1-bamboo "
        "sparrow/peacock. Green body with a small red crest and red tail/"
        "breast accents"),
    2: "two vertical green bamboo stalks side by side",
    3: ("three bamboo stalks in a pyramid: one RED stalk on top, two green "
        "stalks below"),
    4: "four green bamboo stalks in a 2x2 arrangement",
    5: ("five bamboo stalks in a quincunx: four GREEN stalks in the corners, "
        "one RED stalk in the center"),
    6: "six green bamboo stalks in two rows of three",
    7: ("seven bamboo stalks: one RED stalk centered on top, then two rows "
        "of three green stalks below"),
    8: ("eight green bamboo stalks in an hourglass: top four stalks tilt "
        "INWARD toward center (like \\\\ //), bottom four tilt OUTWARD "
        "(like // \\\\), forming an X / bowtie silhouette"),
    9: ("nine bamboo stalks in a 3x3 grid: top row GREEN, middle row all "
        "RED, bottom row GREEN"),
}


def tile_content_spec(code: str) -> str:
    """Return a single-sentence description of what the given tile depicts.

    Intended to be fed into a style-agnostic image-generation prompt; describes
    composition and color-accent positions but not materials."""
    if code.startswith("B") and code[1:].isdigit():
        return BAMBOO_ARRANGEMENT[int(code[1:])]
    if code.startswith("C") and code[1:].isdigit():
        n = int(code[1:])
        numeral = CHAR_NUMERAL[n]
        numeral_color = "red" if n == 1 else "black"
        return (f"the Chinese numeral {numeral} ({numeral_color}) occupying "
                f"the top two-thirds of the tile, and a smaller red 萬 "
                f"character centered below")
    if code.startswith("D") and code[1:].isdigit():
        return DOT_ARRANGEMENT[int(code[1:])]
    if code in WIND_GLYPH:
        glyph, _ = WIND_GLYPH[code]
        return f"the single large black Chinese character {glyph}, centered"
    if code == "DRed":
        return "the single large red Chinese character 中, centered"
    if code == "DGreen":
        return "the single large green Chinese character 發, centered"
    if code == "DWhite":
        return ("a thin blue double-line rectangular frame, centered, "
                "with no character inside (the blank white-dragon tile)")
    if code in FLOWER_MOTIF:
        motif, glyph, idx = FLOWER_MOTIF[code]
        return (f"a {motif}; the digit {idx} in the top-left corner and the "
                f"character {glyph} along the bottom, all in the suit accent "
                f"color")
    raise ValueError(f"unknown tile code: {code!r}")
