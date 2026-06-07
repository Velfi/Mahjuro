# Mahjuro tileset mods

Install custom mahjong tile face art here. Each mod is a **subfolder** with two required files:

| File | Purpose |
|------|---------|
| `atlas.toml` | Grid layout — tile size, column count, and tile codes |
| `atlas.png` | RGBA sprite sheet matching the grid in `atlas.toml` |

## Quick start

1. Copy the `_template` folder in this directory.
2. Rename the copy (e.g. `my_theme`). Do not use a name that starts with `_`.
3. Replace `atlas.png` with your artwork (keep the same dimensions, or update `atlas.toml` to match).
4. Launch Mahjuro → **Options** → **Tile set** and select `my_theme (mod)`.

The `_template` folder is a valid reference layout you can open in an image editor. It is not listed in Options. Mahjuro updates `_template` (and this README) when a new game version ships changed instructions — **copy** the folder before you edit it.

## Tile codes

Codes in `atlas.toml` map to mahjong tiles:

| Codes | Tiles |
|-------|-------|
| `B1`–`B9` | Bamboo (Souzu) 1–9 |
| `C1`–`C9` | Characters (Manzu) 1–9 |
| `D1`–`D9` | Dots (Pinzu) 1–9 |
| `EWind`, `SWind`, `WWind`, `NWind` | East / South / West / North winds |
| `DRed`, `DGreen`, `DWhite` | Dragons |
| `Flower1`–`Flower4`, `Season1`–`Season4` | Bonus tiles |
| `""` | Empty grid slot (padding) |

`atlas.png` width must equal `columns × tile_width`. Height must equal `rows × tile_height`, where `rows` is the number of layout rows (length of the layout list divided by `columns`, rounded up).

Full tile coverage is not required; missing codes fall back to built-in font art.

## Packing from per-tile PNGs

If you have the game repo, you can pack a folder of `B1.png`, `C1.png`, … files:

```bash
python3 scripts/pack_atlas.py /path/to/your/tiles/
cp atlas.png atlas.toml ~/Library/Application\ Support/Mahjuro/mods/tilesets/my_theme/
```

(macOS path; on Linux use `~/.config/Mahjuro/mods/tilesets/`, on Windows `%APPDATA%\Mahjuro\mods\tilesets\`.)

## Cache

3D tile faces are baked on first use and cached under:

`../cache/tilesets/<your_folder_name>/showcase_decal_atlas.png`

Delete that file to force a rebake after you change `atlas.png`.

## Troubleshooting

Invalid folders are skipped at startup. Check the game log for lines like `skipping tileset mod '…'`. Each mod folder also contains a `README.md` with the same requirements.
