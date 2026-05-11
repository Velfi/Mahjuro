# Asset packs (release builds)

Shipped builds load game data from ZIP packs next to the executable (or under `Contents/Resources/` in a `.app`), driven by `pack_manifest.json`. Debug / `cargo test` uses the repo `assets/` tree when no manifest is present.

## Bake

From the repo root:

```bash
python3 tools/bake_assets/bake_assets.py --out path/to/out
# or
scripts/bake-assets.sh --out path/to/out
```

Outputs `pack_manifest.json`, `mahjuro-pack-essential.zip`, `mahjuro-pack-gameplay.zip`, `mahjuro-pack-audio.zip`. Partition rules live in `pack_rules.json` (see top-level `_comment` for pack / prefix precedence).

- **Default** (`--lossy`): minifies JSON; runs `pngquant`/`oxipng` and `ffmpeg` when installed.
- **`--no-lossy`**: copies bytes except JSON minify (faster local checks).
- **ZIP**: already-compressed types (e.g. PNG, OGG, MP3, GLB) are stored uncompressed in the archive; JSON and similar use DEFLATE.

Release CI (Linux / Windows / macOS) installs `ffmpeg`, `pngquant`, and `oxipng` before baking so `--lossy` optimizers run on tagged builds.

## Boot loading (runtime)

At startup, **`essential` and `gameplay` packs are both mounted eagerly** — most of the asset tree is needed during renderer init, so gameplay is not lazy today. Only the **`audio` pack** stays lazy until first audio read (or until prefetch after the main menu). Plan pack contents accordingly.

The manifest includes **`game_version`** (from the root crate `[package].version` in `Cargo.toml`). At init, the game logs a warning if it does not match `CARGO_PKG_VERSION`. Set **`MAHJURO_STRICT_PACK_VERSION`** to any value to **panic** on mismatch instead (useful when debugging mismatched installs).

## Runtime overrides

- `MAHJURO_ASSETS_PACK_DIR` — directory containing `pack_manifest.json` and the zip files.
- `MAHJURO_ASSETS` — loose `assets/` root (overrides pack discovery when used with dev fallbacks).
- `MAHJURO_STRICT_PACK_VERSION` — if set, panic when manifest `game_version` ≠ binary (see above).
