# GPU memory (4 GB / 1080p)

Mahjuro's minimum supported graphics-memory budget is **4 GiB**. Adapters that look below this
floor (typically integrated GPUs outside Apple Silicon and known sub-4 GiB mobile SKUs) run in
best-effort mode and are not part of the supported matrix.

## Presets

| Options → Graphics | Shadows | Internal resolution | Room GPU cap | Showcase decal GPU | Relic GPU | 2D atlas CPU |
| --- | --- | --- | --- | --- | --- | --- |
| **Low memory** | Off | 100% | 2 rooms | 1 tileset | 24 LRU on-demand | 1 atlas, decode ≤4096 px |
| **Performance** | Off | 100% | 6 | 4 | eager batch (BC7 RLC2) | 2 atlases |
| **Visuals** | High | 100% | 6 | 4 | eager batch (BC7 RLC2) | 2 atlases |

Low memory also disables HDR swapchain output even if HDR is toggled in Options.

Room GLB GPU/CPU policy (bits, prefetch, eviction, upload retry) lives in one table:
[`RoomGpuResidentDesc`](../../crates/mahjuro-render/src/room_gpu_resident.rs). Add a row there when
introducing a new hub/run environment — do not scatter `match` arms for residency.

## Eager preload + pressure gating

At startup and each frame, Mahjuro tries to CPU-decode and GPU-warm every hub/run room GLB
(`kick_eager_all_room_cpu_prefetches` + frame-paced eager upload in
[`poll_room_prefetch_gpu_uploads`](../../crates/mahjuro-render/src/wgpu_renderer/room_gpu_load.rs)).
One room env upload runs per poll to avoid hitches.

Memory pressure ([`gpu_memory_pressure.rs`](../../crates/mahjuro-render/src/gpu_memory_pressure.rs))
gates **optional** eager warm-up only — active-scene uploads always proceed after eviction preflight.

On DX12/Vulkan, pressure uses **OS-reported process GPU usage** (DXGI `QueryVideoMemoryInfo` /
`VK_EXT_memory_budget`) when available, not just the wgpu allocator total. The allocator omits
swapchain images, pipeline caches, and driver reserve — on Windows that gap is often 1–2 GiB on a
4 GiB card and was the main cause of late OOM under Low memory.

Thresholds scale from the OS **budget** when present, else probed dedicated VRAM minus a 512 MiB
untracked overhead fudge:

| Pressure | Trigger (Low memory, ~4 GiB OS budget) | Eager behavior |
| --- | --- | --- |
| **Normal** | below ~55% of budget | warm shop → archive → hallway → gameplay → staircase |
| **Constrained** | 55–69% of budget or at resident cap | hub only (shop + archive) |
| **Critical** | ≥70% of budget or over cap | evict unpinned LRU; pause all eager warm-up |

Performance/Visuals use 75%/90% of budget (or fixed 6144/8192 MiB when budget is unavailable).

On Metal (no allocator report), pressure falls back to resident count vs
[`GraphicsMode::max_room_gpu_residents`](../../crates/mahjuro-gfx-types/src/graphics_mode.rs).

Watch for `gpu mem profile: pressure=` (includes `relic_gpu=`, `decal_atlas_cpu=`, `music_pcm=` suffixes) and `gpu mem profile: eager preload` lines when profiling.

## Relic textures (RLC2)

Offline bakes (`data/relic_baked/*.rlc`, format **RLC2**) store BC7 mip chains at source albedo/relief resolution plus full-size RGBA fallbacks. Runtime uploads BC7 when `TEXTURE_COMPRESSION_BC` is available.

- **Low memory:** on-demand decode + GPU LRU cap of **24** (one archive relic page is 3×7 = **21** slots) ([`impl_relic_residency.rs`](../../crates/mahjuro-render/src/wgpu_renderer/impl_relic_residency.rs)); no startup batch.
- **Performance / Visuals:** eager batch after hub CPU prefetch completes and pressure is Normal; batch reads bypass the asset byte LRU.

Rebake: `cargo run -p mahjuro-render --bin mahjuro-bake-relics --features relic_bc7_bake --release`

Thresholds can be overridden with `MAHJURO_GPU_MEM_CONSTRAINED_MIB` and
`MAHJURO_GPU_MEM_CRITICAL_MIB` (see [launch options](launch-options.md)).

## Profiling soak (GTX 1050 / RX 550 class)

```bash
export MAHJURO_STARTUP_PROFILE=1
export MAHJURO_GPU_MEM_PROFILE=1
# Optional force:
export MAHJURO_GRAPHICS_MODE=low_memory   # or visuals / performance

RUST_LOG=mahjuro=info cargo run --release
```

1. Borderless 1920×1080.
2. Hub tour: main menu → shop → hallway → archive → gameplay → back.
3. Options: switch tileset once.
4. Play 30+ minutes; watch logs for `gpu mem profile:` and `room gpu profile:` lines; confirm no device lost / OOM.

First launch without a saved graphics choice applies [`GraphicsMode::suggest_for_adapter`](../../crates/mahjuro-gfx-types/src/graphics_mode.rs) from the adapter name (and integrated-GPU class). **Apple Silicon (M-series) defaults to Visuals**, not Low memory — unified memory is not treated as a 4 GB discrete target.

If you previously auto-picked **Low memory** on a Mac, switch to **Visuals** or **Performance** in Options → Graphics to raise the room GPU cap from 2 → 6 and allow full eager preload.

See also [launch options](launch-options.md).
