# Environment variables

Mahjuro reads `MAHJURO_*` variables from the process environment. For most toggles, any non-empty value means **on** (unless noted). Values `0` and `false` disable boolean flags where documented.

CLI flags and subcommands: [launch options](agents/launch-options.md).

## Main game (`mahjuro`)

Read during process startup, before or while the window and GPU come up.

### Logging

| Variable | Default | Description |
| --- | --- | --- |
| `RUST_LOG` | `info` | Standard `env_logger` filter (e.g. `mahjuro=debug,mahjuro_render=trace`). |
| `MAHJURO_LOG_FILE` | — | Append all `log` output to this path. Startup errors are mirrored here too. On **Windows release** builds without a console, logs go to the launching terminal when present, otherwise `%APPDATA%\Mahjuro\mahjuro.log`. Ignored on Mac App Store / Microsoft Store builds (container log only). |

### Assets

| Variable | Default | Description |
| --- | --- | --- |
| `MAHJURO_ASSETS` | — | Loose `assets/` tree root. Overrides ZIP packs when set. Disabled on store builds (`dist-mas`, `dist-msstore`). |
| `MAHJURO_ASSETS_PACK_DIR` | next to binary | Directory containing `pack_manifest.json` and pack zips. |
| `MAHJURO_STRICT_PACK_VERSION` | warn | When set, **panic** if manifest `game_version` ≠ binary version (default: log warning only). |
| `MAHJURO_ASSET_CACHE_MB` | `128` | Byte-weighted LRU cap for pack / loose asset reads. |

Resolution order: `MAHJURO_ASSETS` → `MAHJURO_ASSETS_PACK_DIR` → `pack_manifest.json` next to the executable → parent of `deps/` (tests). See [tools/bake_assets/README.md](../tools/bake_assets/README.md).

### Graphics preset

Applied when choosing the active [`GraphicsMode`](../../crates/mahjuro-gfx-types/src/graphics_mode.rs) at GPU init (overrides saved settings when set).

| Variable | Values | Description |
| --- | --- | --- |
| `MAHJURO_GRAPHICS_MODE` | `performance`, `low_memory`, `visuals` | Force a preset at startup. Aliases: `low-memory`, `lowmemory`. |
| `MAHJURO_AUTO_LOW_MEMORY` | any | Default preset to **Low memory** (same as choosing that preset on first launch). Useful for 4 GB soak tests without overriding saved settings semantics. |

### GPU, presentation, and WSI

| Variable | Description |
| --- | --- |
| `MAHJURO_PRESENT_MODE` | Swapchain override: `fifo`, `fifo_relaxed`, `mailbox`, `immediate`, `auto`, `auto_vsync`, `auto_no_vsync`. Falls back if the surface does not advertise the mode. |
| `MAHJURO_SKIP_VULKAN_WSI_PROBE` | On Windows, skip the parent-process Vulkan WSI smoke test and force Vulkan. |
| `MAHJURO_VULKAN_WIN_SURFACE_COPY` | Opt-in Windows Vulkan swapchain `COPY_SRC` usage (needed for some in-game screenshots; may crash on some stacks). |
| `MAHJURO_LIT_MESH_PROFILE` | Comma-separated shader cost toggles for `lit_mesh.wgsl`: `no_per_light_shadow`, `no_combined_shadow`, `no_shadow`, `no_pcf`, `no_spec`, `one_light`, `diffuse_only`. See [lit mesh shader](agents/lit-mesh-shader.md). |

Related **third-party** vars read at GPU init (not `MAHJURO_*`):

| Variable | Description |
| --- | --- |
| `WGPU_BACKEND` | Force wgpu backend (`vulkan`, `dx12`, `metal`, …). Mahjuro may clear `WGPU_BACKEND=vulkan` on Windows after a failed WSI probe unless `MAHJURO_SKIP_VULKAN_WSI_PROBE=1`. |
| `WGPU_DX12_COMPILER` | Set to `fxc` to force FXC instead of DXC on Windows (large shaders may fail on iGPUs). |

At startup Mahjuro also logs presence of Steam / gamescope / Wayland vars (`SteamDeck`, `GAMESCOPE_WAYLAND_DISPLAY`, `SDL_VIDEODRIVER`, …) for performance triage.

### Profiling and diagnostics

| Variable | Description |
| --- | --- |
| `MAHJURO_STARTUP_PROFILE` | Print startup timing tables and room GLB CPU/GPU upload metrics. |
| `MAHJURO_GPU_MEM_PROFILE` | Log adapter memory hint and wgpu allocator totals (also enabled when `MAHJURO_STARTUP_PROFILE=1`). |
| `MAHJURO_GPU_MEM_CONSTRAINED_MIB` | Override MiB threshold for `pressure=constrained` (startup + runtime). See [gpu memory](agents/gpu-memory.md). |
| `MAHJURO_GPU_MEM_CRITICAL_MIB` | Override MiB threshold for `pressure=critical` / eviction preflight. |
| `MAHJURO_SHADOW_PROBE` | Runtime punctual-shadow diagnostics (`1` = on; `0` or unset = off). Logs caster counts and depth readback every ~2 s. |
| `MAHJURO_VALIDATE_OFFLINE_BAKES` | Opt in to runtime validation of committed offline bakes (GI, shadow, decal, relic) at renderer init. |

Example profiling bundle:

```bash
export MAHJURO_STARTUP_PROFILE=1
export MAHJURO_GPU_MEM_PROFILE=1
export MAHJURO_GRAPHICS_MODE=low_memory
```

### Background loading

| Variable | Default | Description |
| --- | --- | --- |
| `MAHJURO_LOADER_THREADS` | `3` | Background loader pool worker count (clamped 1–4). |

## Build time (`cargo build`)

Set in the environment when invoking `cargo build` / `cargo test`. Not read by the running game unless noted.

| Variable | Description |
| --- | --- |
| `MAHJURO_SKIP_ASSET_BAKE` | Skip `tools/bake_assets` in `build.rs`. Supply packs, `MAHJURO_ASSETS_PACK_DIR`, or `MAHJURO_ASSETS` yourself. |
| `MAHJURO_SKIP_OFFLINE_BAKES` | Skip all committed offline bake freshness checks. |
| `MAHJURO_SKIP_COMMITTED_BAKE_CHECKS` | Skip every `.inputs_stamp` check. Auto-skipped when building `mahjuro-headless --features bake`. |
| `MAHJURO_SKIP_ROOM_GI_BAKE` | Skip room GI stamp check only. |
| `MAHJURO_SKIP_ROOM_SHADOW_BAKE` | Skip room shadow stamp check only. |
| `MAHJURO_SKIP_SHOWCASE_DECAL_BAKE` | Skip showcase decal atlas stamp check only. |
| `MAHJURO_SKIP_RELIC_BAKE` | Skip relic RLC2 stamp check only. |
| `MAHJURO_SKIP_TEXTURE_BAKE` | Skip static texture bake stamp check only. |
| `MAHJURO_DXC_REDIST` | Directory containing `dxcompiler.dll` + `dxil.dll` for Windows `build.rs` copy. See [windows build](agents/windows-build.md). |

Stale committed bakes panic at build with the exact `scripts/rebake-offline.sh <kind>` command. See [room shadows & baking](agents/room-shadows-and-baking.md).

**Debug menubar:** enable with Cargo feature `debug-menu` (always on in debug profile), not an environment variable.

## Headless tools

### Shared (`mahjuro-bake`, `mahjuro-screenshot`, bake bins)

Uses `MAHJURO_ASSETS`, `MAHJURO_LOG_FILE`, and build-time skip vars above. Bake bins also honor:

| Variable | Description |
| --- | --- |
| `MAHJURO_FORCE_RELIC_BAKE` | Rebake every relic even when sidecars match (`mahjuro-bake-relics --force`). |
| `MAHJURO_EXPECT_ROOM_GI_STAMP_HASH` | After GI bake, assert committed stamp hash (CI / regression). |
| `MAHJURO_EXPECT_ROOM_SHADOW_STAMP_HASH` | After shadow bake, assert committed stamp hash. |
| `MAHJURO_ROOM_SHADOW_DEBUG_DUMP` | Directory to write room shadow bake debug captures. |

### `mahjuro-screenshot` / headless render

| Variable | Description |
| --- | --- |
| `MAHJURO_HEADLESS_GPU_PROFILE_FRAMES` | Average GPU pass timings over N frames after warmup. |
| `MAHJURO_HEADLESS_SHADOW_QUALITY` | Shadow preset: `low`, `medium`, `high`. |

## Internal (do not set)

| Variable | Description |
| --- | --- |
| `MAHJURO_VULKAN_PROBE_CHILD` | Set by Mahjuro during the Windows Vulkan WSI child probe. |
