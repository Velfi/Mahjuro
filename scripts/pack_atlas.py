"""Pack a directory of per-tile mahjong PNGs into a single atlas.png + atlas.toml.

Expects files named by tile code (B1.png, EWind.png, DRed.png, Flower1.png, …)
with uniform dimensions across the set. Writes:
  <dir>/atlas.png    RGBA grid, row-major using the LAYOUT constant below
                     (shared with ``generate_classic_atlas.py`` and the game)
  <dir>/atlas.toml   image / tile_width / tile_height / columns / layout

Does NOT delete the source PNGs — that's a manual follow-up after verifying
the atlas loader works.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image

COLUMNS = 9

# Layout organised so each row is a coherent group — one whole suit per row
# keeps the atlas readable as a sprite sheet and lets image-gen prompts talk
# about "row of bamboos" without crossing suit boundaries.
# Honors row has 2 padding slots so flowers start fresh on row 5.
LAYOUT = [
    "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9",
    "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9",
    "D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9",
    "EWind", "SWind", "WWind", "NWind", "DRed", "DGreen", "DWhite", "", "",
    "Flower1", "Flower2", "Flower3", "Flower4",
    "Season1", "Season2", "Season3", "Season4", "",
]

# Optional alternate stems (no `.png`) when sources use descriptive names.
CODE_ALTERNATES: dict[str, tuple[str, ...]] = {
    "EWind": ("EastWind",),
    "SWind": ("SouthWind",),
    "WWind": ("WestWind",),
    "NWind": ("NorthWind",),
    "DRed": ("RedDragon",),
    "DGreen": ("GreenDragon",),
    "DWhite": ("WhiteDragon",),
    "Flower1": ("F1",),
    "Flower2": ("F2",),
    "Flower3": ("F3",),
    "Flower4": ("F4",),
    "Season1": ("S1",),
    "Season2": ("S2",),
    "Season3": ("S3",),
    "Season4": ("S4",),
}


def _resolve_tile_path(set_dir: Path, code: str) -> Path | None:
    candidates = (code,) + CODE_ALTERNATES.get(code, ())
    for stem in candidates:
        path = set_dir / f"{stem}.png"
        if path.exists():
            return path
    return None


def pack(set_dir: Path) -> None:
    tiles: dict[str, Image.Image] = {}
    tile_w = tile_h = None
    for code in LAYOUT:
        if not code:
            continue  # empty layout slot: no source PNG expected
        path = _resolve_tile_path(set_dir, code)
        if path is None:
            if (code.startswith("Flower") or code.startswith("Season")) and tile_w is not None:
                print(
                    f"warning: missing {code} (no {code}.png / alternates); "
                    f"using transparent {tile_w}x{tile_h} placeholder"
                )
                tiles[code] = Image.new("RGBA", (tile_w, tile_h), (0, 0, 0, 0))
                continue
            tried = (code,) + CODE_ALTERNATES.get(code, ())
            raise FileNotFoundError(
                f"missing tile for {code}: tried "
                + ", ".join(f"{s}.png" for s in tried)
            )
        img = Image.open(path).convert("RGBA")
        if tile_w is None:
            tile_w, tile_h = img.size
        elif img.size != (tile_w, tile_h):
            raise ValueError(
                f"{path.name} is {img.size}, expected {(tile_w, tile_h)} "
                f"(all tiles must share dimensions)"
            )
        tiles[code] = img

    rows = (len(LAYOUT) + COLUMNS - 1) // COLUMNS
    atlas = Image.new("RGBA", (COLUMNS * tile_w, rows * tile_h), (0, 0, 0, 0))
    for i, code in enumerate(LAYOUT):
        if not code:
            continue
        col = i % COLUMNS
        row = i // COLUMNS
        atlas.paste(tiles[code], (col * tile_w, row * tile_h))
    atlas.save(set_dir / "atlas.png")

    lines = [
        'image = "atlas.png"',
        f"tile_width = {tile_w}",
        f"tile_height = {tile_h}",
        f"columns = {COLUMNS}",
        "",
        "layout = [",
    ]
    for i in range(0, len(LAYOUT), COLUMNS):
        row = LAYOUT[i:i + COLUMNS]
        lines.append("    " + ",".join(f'"{c}"' for c in row) + ",")
    lines.append("]")
    (set_dir / "atlas.toml").write_text("\n".join(lines) + "\n")

    print(f"packed {len(tiles)} tiles → {set_dir / 'atlas.png'} "
          f"({COLUMNS * tile_w}x{rows * tile_h})")


def main() -> None:
    ap = argparse.ArgumentParser(description="Pack a tile-set directory into an atlas.")
    ap.add_argument("set_dir", type=Path, help="Path to assets/sets/<name>/")
    args = ap.parse_args()
    pack(args.set_dir)


if __name__ == "__main__":
    main()
