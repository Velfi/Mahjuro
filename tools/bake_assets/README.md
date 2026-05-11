# Asset packs (release builds)

Shipped builds load game data from ZIP packs next to the executable (or under `Contents/Resources/` in a `.app`), driven by `pack_manifest.json`.

**Local `cargo build` / `cargo test`:** `build.rs` runs this bake into `target/<profile>/` (or `target/<triple>/<profile>/` when using `--target`), so the game and tests load packs without copying the repo `assets/` tree. Set **`MAHJURO_SKIP_ASSET_BAKE=1`** to skip that step (you must place `pack_manifest.json` + zips next to the binary, set **`MAHJURO_ASSETS_PACK_DIR`**, or point **`MAHJURO_ASSETS`** at a loose tree). Debug profile uses **`--no-lossy`** for faster iteration; release uses the default lossy profile when optimizers are on `PATH`.

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
- `MAHJURO_ASSETS` — loose `assets/` root (used when packs are absent or you want to override).
- `MAHJURO_SKIP_ASSET_BAKE` — if set (non-empty, not `0`/`false`), `build.rs` does not run the baker (see above).
- `MAHJURO_STRICT_PACK_VERSION` — if set, panic when manifest `game_version` ≠ binary (see above).
