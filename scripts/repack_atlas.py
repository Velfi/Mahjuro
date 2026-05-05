"""Repack an existing tile atlas into the current COLUMNS/LAYOUT.

Reads `<set_dir>/atlas.png` + `atlas.toml`, slices the image into tile-sized
cells according to the toml's own layout, then re-emits them at the current
`pack_atlas.COLUMNS` / `pack_atlas.LAYOUT` geometry. Use this when the atlas
grid shape changes (e.g. 8 → 9 columns) and you no longer have per-tile PNGs.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

from PIL import Image

import pack_atlas


def _parse_toml(src: str) -> tuple[int, int, int, list[str]]:
    tw = int(re.search(r"tile_width\s*=\s*(\d+)", src).group(1))
    th = int(re.search(r"tile_height\s*=\s*(\d+)", src).group(1))
    cols = int(re.search(r"columns\s*=\s*(\d+)", src).group(1))
    layout_block = re.search(r"layout\s*=\s*\[(.*?)\]", src, re.S).group(1)
    codes = re.findall(r'"([^"]+)"', layout_block)
    return tw, th, cols, codes


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("set_dir", type=Path)
    args = ap.parse_args()

    src_toml = (args.set_dir / "atlas.toml").read_text()
    tw, th, cols, codes = _parse_toml(src_toml)
    atlas = Image.open(args.set_dir / "atlas.png").convert("RGBA")

    # Slice the old atlas into (code -> tile image). Skip empty layout slots.
    tiles: dict[str, Image.Image] = {}
    for i, code in enumerate(codes):
        if not code:
            continue
        col = i % cols
        row = i // cols
        x, y = col * tw, row * th
        tiles[code] = atlas.crop((x, y, x + tw, y + th))

    # Re-emit using the current LAYOUT / COLUMNS from pack_atlas.
    missing = [c for c in pack_atlas.LAYOUT if c and c not in tiles]
    if missing:
        raise SystemExit(f"source atlas is missing codes: {missing}")

    new_cols = pack_atlas.COLUMNS
    rows = (len(pack_atlas.LAYOUT) + new_cols - 1) // new_cols
    new_atlas = Image.new("RGBA", (new_cols * tw, rows * th), (0, 0, 0, 0))
    for i, code in enumerate(pack_atlas.LAYOUT):
        if not code:
            continue
        col = i % new_cols
        row = i // new_cols
        new_atlas.paste(tiles[code], (col * tw, row * th))
    new_atlas.save(args.set_dir / "atlas.png")

    # Rewrite atlas.toml — reuse pack_atlas's format by hand here to avoid
    # importing a private helper.
    lines = [
        'image = "atlas.png"',
        f"tile_width = {tw}",
        f"tile_height = {th}",
        f"columns = {new_cols}",
        "",
        "layout = [",
    ]
    for i in range(0, len(pack_atlas.LAYOUT), new_cols):
        chunk = pack_atlas.LAYOUT[i:i + new_cols]
        lines.append("    " + ",".join(f'"{c}"' for c in chunk) + ",")
    lines.append("]")
    (args.set_dir / "atlas.toml").write_text("\n".join(lines) + "\n")

    print(f"repacked {args.set_dir} from {cols}→{new_cols} cols "
          f"({new_cols * tw}x{rows * th})")


if __name__ == "__main__":
    main()
