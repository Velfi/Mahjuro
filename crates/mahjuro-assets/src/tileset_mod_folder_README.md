# Tileset mod folder

This folder is one custom tileset. Mahjuro loads it when **both** required files are present and valid:

## Required files

### `atlas.toml`

Metadata for the sprite sheet:

- `tile_width`, `tile_height` — pixel size of one tile cell
- `columns` — number of cells per row (usually `9`)
- `layout` — ordered list of tile codes, one per grid cell

Empty string `""` entries are intentional padding slots. Do not remove them unless you also resize the grid.

### `atlas.png`

RGBA PNG whose size matches the layout:

```
width  = columns × tile_width
height = rows × tile_height
rows   = ceil(number of layout codes / columns)
```

## Tile codes

| Code | Tile |
|------|------|
| `B1`–`B9` | Bamboo 1–9 |
| `C1`–`C9` | Characters 1–9 |
| `D1`–`D9` | Dots 1–9 |
| `EWind` / `SWind` / `WWind` / `NWind` | Winds |
| `DRed` / `DGreen` / `DWhite` | Dragons |
| `Flower1`–`Flower4` | Flowers |
| `Season1`–`Season4` | Seasons |

## In-game name

The **folder name** (parent directory name) is shown in Options as `<folder_name> (mod)`. Use only letters, numbers, underscores, or hyphens — no spaces or slashes.

## Getting started from `_template`

Copy the sibling `_template` folder, rename it, then edit `atlas.png`. The template `atlas.toml` already uses the standard 9-column mahjong layout with 128×192 pixel cells (1152×960 PNG).

## After editing

Restart the game or reopen **Options** to rescan. If you change `atlas.png`, delete the cached file under `../cache/tilesets/<folder_name>/` to refresh 3D tile faces.
