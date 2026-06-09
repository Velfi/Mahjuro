# Launch options

Canonical reference for `mahjuro` CLI flags and `MAHJURO_*` environment variables. For offline bakes and screenshots, see [room shadows & baking](room-shadows-and-baking.md).

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
| `RUST_LOG` | Standard `env_logger` filter (e.g. `mahjuro=debug`). |
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

`MAHJURO_DEBUG_MENU=1` at **compile** time enables the native debug menubar in release builds (always on in debug profile). See `build.rs`.

## Build-time environment

| Variable | When set |
| --- | --- |
| `MAHJURO_SKIP_ASSET_BAKE` | Skip `tools/bake_assets` in `build.rs` (supply packs or `MAHJURO_ASSETS`). |
| `MAHJURO_SKIP_AUTO_OFFLINE_BAKE` | Disable `build.rs` auto-rebake when stamps are stale (panic instead). CI always skips auto-rebake via `CI=true`. |
| `MAHJURO_SKIP_OFFLINE_BAKES` | Skip all committed offline bake freshness checks (GI, shadow, decal, relic). |
| `MAHJURO_SKIP_COMMITTED_BAKE_CHECKS` | Skip every `.inputs_stamp` check. `mahjuro-headless --features bake` skips via `mahjuro/offline-bake-support`; `headless-screenshot` skips via feature. |
| `MAHJURO_SKIP_ROOM_GI_BAKE` | Skip only room GI stamp check (use while rebaking GI alone). |
| `MAHJURO_SKIP_ROOM_SHADOW_BAKE` | Skip only room shadow stamp check. |
| `MAHJURO_SKIP_SHOWCASE_DECAL_BAKE` | Skip only showcase decal atlas stamp check. |
| `MAHJURO_SKIP_RELIC_BAKE` | Skip only relic RLC1 stamp check. |
| `MAHJURO_DEBUG_MENU` | Compile release with debug menubar (`build.rs`). |
| `MAHJURO_DXC_REDIST` | Directory containing `dxcompiler.dll` + `dxil.dll` for `build.rs` to copy on Windows (see [windows-build.md](windows-build.md)). |

Asset-pack details: [tools/bake_assets/README.md](../../tools/bake_assets/README.md).

## Headless binaries

**`mahjuro-bake`** — offline `.mgi` / `.msh` room bakes. Requires `--features bake`.

**`mahjuro-screenshot`** — one offscreen PNG. Requires `--features screenshot`. Scene list in `crates/mahjuro-headless/src/screenshot_cli.rs`.

**`mahjuro-bake-relics`**, **`mahjuro-bake-decal-atlases`** — relic RLC1 and showcase decal atlases (`mahjuro-render` bins).
