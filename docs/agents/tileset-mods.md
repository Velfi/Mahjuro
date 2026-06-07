# Tileset mods

Players can install custom mahjong tile face atlases without repacking game assets or running dev bake tools.

## Install location

Drop a folder here (created automatically on first launch):

| OS | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Mahjuro/mods/tilesets/<name>/` |
| Linux | `~/.config/Mahjuro/mods/tilesets/<name>/` (or `$XDG_CONFIG_HOME/Mahjuro/...`) |
| Windows | `%APPDATA%\Mahjuro\mods\tilesets\<name>\` |

On each mod scan the game checks shipped scaffold files (FNV-1a hash) and writes only when content changed:

- `README.md` — overview in the tilesets root
- `_template/` — copyable starter with `README.md`, `atlas.toml`, and a transparent placeholder `atlas.png` (128×192 cells, 9×5 grid)

Copy `_template` to a new folder name before editing — in-place edits to `_template` are replaced on the next game update that changes the embedded files.

For incomplete player mod folders (missing or invalid `atlas.toml` / `atlas.png`), a `README.md` is written only when absent (your edits are not overwritten).

Folders whose names start with `_` or `.` (including `_template`) are never listed in Options.

## Required files

Each mod folder must contain:

- `atlas.toml` — grid metadata (`tile_width`, `tile_height`, `columns`, `layout = ["B1", ...]`)
- `atlas.png` — RGBA sprite sheet matching the layout dimensions

Tile codes follow the same convention as shipped sets (`B1`–`B9` bamboo, `C1`–`C9` characters, `D1`–`D9` dots, `EWind`/`SWind`/`WWind`/`NWind`, `DRed`/`DGreen`/`DWhite`, `Flower1`–`4`, `Season1`–`4`). See [`scripts/pack_atlas.py`](../../scripts/pack_atlas.py) for the canonical 9-column layout.

Pack per-tile PNGs into an atlas:

```bash
python3 scripts/pack_atlas.py /path/to/atlas/sources/
# then copy atlas.png + atlas.toml into the mod folder above
```

## In-game selection

Options → **Tile set** cycles built-in sets and validated mods. Mods appear as `<folder_name> (mod)` and are stored internally as `mod:<folder_name>` so they never collide with shipped set names.

Use **Open tileset mods** at the bottom of Options (or the pause-menu Options overlay) to reveal the install folder in Finder / Explorer / your file manager.

## Showcase decal cache

3D tile faces use a pre-rasterized decal atlas. Built-in sets ship this PNG; mods are **runtime-baked** on first use and cached at:

```
{config_dir}/Mahjuro/mods/cache/tilesets/<name>/showcase_decal_atlas.png
```

To force a rebake after editing `atlas.png`, delete that cache file and restart (or re-select the mod in Options).

## Validation

Invalid mod folders are skipped with a `warn!` log entry. Common failures:

- missing `atlas.toml` or `atlas.png`
- `atlas.toml` parse error or zero dimensions
- `atlas.png` size does not match `columns * tile_width` by `rows * tile_height`

Full mahjong tile coverage is not required; missing codes fall back to font rasterization.

## Code touchpoints

- Discovery / validation: [`crates/mahjuro-assets/src/tileset_mod.rs`](../../crates/mahjuro-assets/src/tileset_mod.rs)
- Merged player list: [`list_player_tilesets()`](../../crates/mahjuro-assets/src/asset_path.rs)
- Atlas load: [`crates/mahjuro-render/src/decal.rs`](../../crates/mahjuro-render/src/decal.rs)
- Runtime bake: [`load_or_bake_showcase_decal_atlas()`](../../crates/mahjuro-render/src/showcase_decal_atlas.rs)
