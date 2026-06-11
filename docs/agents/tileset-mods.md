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

Use **Local tileset mods** at the bottom of Options (or the pause-menu Options overlay) to reveal the install folder in Finder / Explorer / your file manager. On the Steam SKU this action lives under Options → **Steam** → **Local mod folder**.

### Steam Workshop (Steam build only)

Options includes a **Steam** section (Steam SKU only):

| Row | Action |
| --- | --- |
| **Browse Workshop** | Opens the Workshop hub in the Steam overlay to subscribe to tilesets. |
| **Publish mod** | Uploads the selected local mod folder via ISteamUGC (`atlas.toml` + `atlas.png`). A smaller preview PNG is generated automatically when `atlas.png` is ≥ 1 MiB (Steam's preview limit). Re-uploads update the same item when `.workshop_id` is present in the mod folder. |
| **Local mod folder** | Opens the install path above. |

Subscribed items download automatically and appear in **Tile set** as `<title> (Workshop)` with internal id `workshop:<published_file_id>`.

After the first publish, Steam may require accepting the Workshop legal agreement in the overlay before the item is visible to others. Set visibility to **Public** on the item page when ready.

Workshop packages must contain `atlas.toml` and `atlas.png` at the item root or in one immediate subfolder (same layout rules as local mods). Invalid subscriptions are skipped with a log warning.

Partner setup (one-time, [Steamworks app 4636490](https://partner.steamgames.com/apps/landings/4636490)):

1. **Steamworks → Edit Steamworks Settings → Workshop** — enable **ISteamUGC for file transfer**; use **Community items** (not game-managed).
2. **Steam Cloud** — set a non-zero byte quota and file count on [Steam Cloud Settings](https://partner.steamgames.com/apps/cloud/4636490) (e.g. **50 MB** per user). Workshop preview uploads count against the **player's** cloud quota for Mahjuro; if quota is **0** or full, uploads fail with `LimitExceeded`. The **1 MiB workshop preview image cap** is fixed by Steam and cannot be raised — the game auto-writes `.workshop_preview.png` when `atlas.png` is larger.
3. Publish a test item from in-game **Publish mod** (or copy `_template` into a named folder first) to verify download + validation.
4. Optional tag: `Tileset` (applied automatically on upload).

Code: subscribe sync in [`crates/mahjuro-distribution/src/steam/workshop.rs`](../../crates/mahjuro-distribution/src/steam/workshop.rs); publish in [`workshop_publish.rs`](../../crates/mahjuro-distribution/src/steam/workshop_publish.rs); registry in [`crates/mahjuro-assets/src/tileset_workshop.rs`](../../crates/mahjuro-assets/src/tileset_workshop.rs).

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
