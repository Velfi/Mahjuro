# Asset packs (release builds)

Shipped builds load game data from ZIP packs next to the executable (or under `Contents/Resources/` in a `.app`), driven by `pack_manifest.json`.

**Local `cargo build` / `cargo test`:** `build.rs` runs this bake into `target/<profile>/` (or `target/<triple>/<profile>/` when using `--target`), so the game and tests load packs without copying the repo `assets/` tree. The bake is skipped when an FNV digest of pack-eligible files (same exclusions as `bake_assets.py`, so `.DS_Store` and authoring trees do not count) matches `target/<profile>/.pack_inputs_stamp` and the zip outputs exist. Set **`MAHJURO_SKIP_ASSET_BAKE=1`** to skip that step entirely (you must place `pack_manifest.json` + zips next to the binary, set **`MAHJURO_ASSETS_PACK_DIR`**, or point **`MAHJURO_ASSETS`** at a loose tree). Debug profile uses **`--no-lossy`** for faster iteration; release uses the default lossy profile when optimizers are on `PATH`.

## Bake

From the repo root:

```bash
python3 tools/bake_assets/bake_assets.py --out path/to/out
# or
scripts/bake-assets.sh --out path/to/out
```

Outputs `pack_manifest.json`, `mahjuro-pack-shared.zip`, `mahjuro-pack-rooms.zip`, `mahjuro-pack-gameplay-bulk.zip`, `mahjuro-pack-music.zip`. Partition rules live in `pack_rules.json` (see top-level `_comment` for pack / prefix precedence).

- **`music` (lazy):** background music + win/loss jingles under `audio/music/` (decoded on first play, not at `AudioManager::new`). Listed first so the `audio/music/` prefix wins over the `shared` pack's broader `audio/` prefix.
- **`shared` (eager):** `fonts/`, `textures/tile_sets/` (tile atlases), and the rest of `audio/` (sound effects) — needed across scenes.
- **`rooms` (lazy, splash-mounted):** `3d/` GLBs and offline `data/room_shadow/` + `data/room_gi/` bakes.
- **`gameplay_bulk` (lazy, chain-mounted):** remaining `data/` (relic RLC2, JSON, …), `textures/` (non-tileset), and `steam_assets/`.

### Bake options

- **Default** (`--lossy`): minifies JSON; runs `pngquant`/`oxipng` and `ffmpeg` when installed.
- **`--no-lossy`**: copies bytes except JSON minify (faster local checks).
- **ZIP**: already-compressed types (e.g. PNG, OGG, MP3, GLB) are stored uncompressed in the archive; JSON and similar use DEFLATE.
- **Atomic outputs**: each zip and `pack_manifest.json` is built to a per-pid `.<name>.<pid>.tmp` file in the output dir, then `os.replace`d into place (same filesystem → atomic). The manifest is written last. Readers — the running game, or a second `cargo` process running `build.rs` concurrently — therefore only ever see complete packs, never a half-written zip mid-rebake.

Release CI (Windows / macOS) installs `ffmpeg`, `pngquant`, and `oxipng` before baking so `--lossy` optimizers run on tagged builds.

### Room GLB textures (`shop`, `hallway`, `archive`, `main_menu`, `staircase`, `gameplay`)

At runtime the game caps room textures to **1024px** and builds mip chains on the CPU when uploading to the GPU. Large source GLBs (e.g. multi‑hundred‑MB `Shop.glb`) dominate startup.

When **`gltf-transform`** is on `PATH` (`npm i -g @gltf-transform/cli`), the baker runs:

`gltf-transform resize <input.glb> <output> --width 1024 --height 1024`

for those room files before they go into `mahjuro-pack-rooms.zip`. Without the tool, GLBs are copied as-is and a warning is printed for files over 32 MB.

Authoring tip: export room textures at ≤1024 in Blender so decode + pack size stay small even without `gltf-transform`.

Loose `baseColor_*` / `normal_*` files sometimes land in `assets/3d/` after `gltf-transform`; move them to `assets/3d/_gltf_sidecars/` (gitignored, excluded from packs). The game only reads embedded textures inside the `.glb` files.

## Boot loading (runtime)

At startup, only **`shared`** mounts **eagerly**. **`rooms`** mounts at splash (before hub GLB decode). **`gameplay_bulk`** mounts when entering the shop or resuming a run. **`music/`** uses a **lazy** zip (BGM when a track first starts). The splash screen does not wait on music or bulk texture decodes — the hub may briefly show a solid fallback until the façade texture uploads.

The manifest includes **`game_version`** (from the root crate `[package].version` in `Cargo.toml`). At init, the game logs a warning if it does not match `CARGO_PKG_VERSION`. Set **`MAHJURO_STRICT_PACK_VERSION`** to any value to **panic** on mismatch instead (useful when debugging mismatched installs).

## Runtime overrides

- `MAHJURO_ASSETS_PACK_DIR` — directory containing `pack_manifest.json` and the zip files.
- `MAHJURO_ASSETS` — loose `assets/` root (used when packs are absent or you want to override).
- `MAHJURO_SKIP_ASSET_BAKE` — if set (non-empty, not `0`/`false`), `build.rs` does not run the baker (see above).
- `MAHJURO_STRICT_PACK_VERSION` — if set, panic when manifest `game_version` ≠ binary (see above).

Full env/CLI list: [docs/agents/launch-options.md](../../docs/agents/launch-options.md).
