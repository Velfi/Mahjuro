# Launch options

CLI flags and subcommands for `mahjuro` and headless binaries. **Environment variables:** [ENV.md](../ENV.md). For offline bakes and screenshots, see [room shadows & baking](room-shadows-and-baking.md).

Live help: `mahjuro --help`, `mahjuro bot --help`, `mahjuro-bake --help`, `mahjuro-screenshot --help`.

## Main binary (`mahjuro`)

| Flag / subcommand | Notes |
| --- | --- |
| `--no-steam` | Skip Steamworks init (local dev; no overlay / foreground grab). |
| `bot [N]` | Headless AI runs (default `N=100`). See [BOT.md](../../BOT.md). |
| `sweep` | Parameter grid sweep (`--runs`, `--export-json`). |
| `strategy-sweep <file>` | Sweep strategies from JSON. |
| `forced-relic-sweep` | Forced-relic balance sweep. |

Headless tools (`mahjuro-bake`, `mahjuro-screenshot`) and bot/sweep always skip Steam.

## Runtime environment

| Variable | When set |
| --- | --- |
| `MAHJURO_ASSETS` | Loose `assets/` tree (overrides packs when set). |
| `MAHJURO_ASSETS_PACK_DIR` | Directory with `pack_manifest.json` + zips. |
| `MAHJURO_STRICT_PACK_VERSION` | Panic if pack `game_version` ≠ binary. |
| `MAHJURO_LOG_FILE` | Append `log` output to this path (also mirrors startup errors). |
| `RUST_LOG` | Standard `env_logger` filter (e.g. `mahjuro=debug`). On **Windows release** builds, stderr has no console: logs go to the launching terminal when one exists, otherwise to `%APPDATA%\Mahjuro\mahjuro.log` (or set `MAHJURO_LOG_FILE`). See `scripts/run-release-startup-profile.bat`. |
| `MAHJURO_STARTUP_PROFILE` | Startup timing tables + room GLB CPU/GPU upload metrics. |
| `MAHJURO_GPU_MEM_PROFILE` | Log adapter hint + wgpu allocator totals (also on when `MAHJURO_STARTUP_PROFILE=1`). |
| `MAHJURO_GPU_MEM_CONSTRAINED_MIB` | Override allocator MiB threshold for `pressure=constrained` (startup + runtime profiling). |
| `MAHJURO_GPU_MEM_CRITICAL_MIB` | Override allocator MiB threshold for `pressure=critical` / eviction preflight. |
| `MAHJURO_VALIDATE_OFFLINE_BAKES` | Opt in to runtime validation of committed offline bakes (GI/shadow/decal/relic). |
| `MAHJURO_ASSET_CACHE_MB` | Byte-weighted LRU cap for pack/loose asset reads (default 128). |
| `MAHJURO_LOADER_THREADS` | Background loader pool worker count (default 3, max 4). |
| `MAHJURO_GRAPHICS_MODE` | Force preset at startup: `performance`, `low_memory`, or `visuals`. |
| `MAHJURO_AUTO_LOW_MEMORY` | Default graphics preset to **Low memory** (testing / 4 GB soak). |
| `MAHJURO_PRESENT_MODE` | WSI override: `fifo`, `mailbox`, `immediate`, `auto`, … |
| `MAHJURO_SKIP_VULKAN_WSI_PROBE` | Force Vulkan without parent WSI smoke test. |
| `MAHJURO_VULKAN_WIN_SURFACE_COPY` | Opt-in Windows Vulkan swapchain `COPY_SRC` (screenshots). |
| `MAHJURO_LIT_MESH_PROFILE` | Headless/dev only: comma-separated A/B toggles for `lit_mesh.wgsl` cost isolation (`no_per_light_shadow`, `no_combined_shadow`, `no_shadow`, `no_pcf`, `no_spec`, `one_light`, `diffuse_only`). See [lit mesh shader](lit-mesh-shader.md). |
| `MAHJURO_SHADOW_PROBE` | Runtime punctual-shadow diagnostics (`1` = on). |

Native debug menubar: Cargo feature `debug-menu` (always on in debug profile).

## Build-time environment

Full list: [ENV.md](../ENV.md#build-time-cargo-build).

| Variable | When set |
| --- | --- |
| `MAHJURO_SKIP_ASSET_BAKE` | Skip `tools/bake_assets` in `build.rs` (supply packs or `MAHJURO_ASSETS`). |
| `MAHJURO_SKIP_OFFLINE_BAKES` | Skip all committed offline bake freshness checks (GI, shadow, decal, relic). |
| `MAHJURO_SKIP_COMMITTED_BAKE_CHECKS` | Skip every `.inputs_stamp` check. `mahjuro-headless --features bake` skips via `mahjuro/offline-bake-support`; `headless-screenshot` skips via feature. |
| `MAHJURO_SKIP_ROOM_GI_BAKE` | Skip only room GI stamp check (use while rebaking GI alone). |
| `MAHJURO_SKIP_ROOM_SHADOW_BAKE` | Skip only room shadow stamp check. |
| `MAHJURO_SKIP_SHOWCASE_DECAL_BAKE` | Skip only showcase decal atlas stamp check. |
| `MAHJURO_SKIP_RELIC_BAKE` | Skip only relic RLC2 stamp check. |
| `MAHJURO_SKIP_TEXTURE_BAKE` | Skip only static texture bake stamp check. |
| `MAHJURO_DXC_REDIST` | Directory containing `dxcompiler.dll` + `dxil.dll` for `build.rs` to copy on Windows (see [windows-build.md](windows-build.md)). |

Stale committed bakes are not auto-rebaked by `build.rs`; it panics with the exact `scripts/rebake-offline.sh <kind>` command to run.

Asset-pack details: [tools/bake_assets/README.md](../../tools/bake_assets/README.md).

## Headless binaries

**`mahjuro-bake`** — offline room lightmap (`.lightmap.rlm`) / shadow (`.msh`) bakes. Requires `--features bake`.

**`mahjuro-screenshot`** — one offscreen PNG. Requires `--features screenshot`. Scene list in `crates/mahjuro-headless/src/screenshot_cli.rs`.

| Variable | When set |
| --- | --- |
| `MAHJURO_HEADLESS_GPU_PROFILE_FRAMES` | Average GPU pass timings over N frames after warmup (logs `main`, `shadow`, etc. via `mahjuro_render::gpu_profiler`). Used by `scripts/profile-lit-mesh-inspect.sh`. |
| `MAHJURO_HEADLESS_SHADOW_QUALITY` | Headless shadow preset: `low`, `medium`, `high` (PCF tap count in `projected_shadow.wgsl`). |

**`mahjuro-bake-relics`**, **`mahjuro-bake-decal-atlases`** — relic RLC2 and showcase decal atlases (`mahjuro-render` bins).
