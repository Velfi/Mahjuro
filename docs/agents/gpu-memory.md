# GPU memory (4 GB / 1080p)

## Presets

| Options → Graphics | Shadows / SSR | Internal resolution | Room GPU cap | Decal atlas cache |
| --- | --- | --- | --- | --- |
| **Low memory** | Off | 75% of window (min 1280×720) | 2 rooms | 1 tileset |
| **Performance** | Off | 100% | 6 | 4 |
| **Visuals** | High + SSR | 100% | 6 | 4 |

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
gates **optional** eager warm-up only — active-scene uploads always proceed after eviction preflight:

| Pressure | Trigger (allocator when available) | Eager behavior |
| --- | --- | --- |
| **Normal** | below ~2200 MiB allocated | warm shop → archive → hallway → gameplay → staircase |
| **Constrained** | ~2200–2800 MiB or at resident cap | hub only (shop + archive) |
| **Critical** | ≥ ~2800 MiB or over cap | evict unpinned LRU; pause all eager warm-up |

On Metal (no allocator report), pressure falls back to resident count vs
[`GraphicsMode::max_room_gpu_residents`](../../crates/mahjuro-gfx-types/src/graphics_mode.rs).

Watch for `gpu mem profile: pressure=` and `gpu mem profile: eager preload` lines when profiling.

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
