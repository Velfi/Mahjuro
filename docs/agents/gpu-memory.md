# GPU memory (4 GB / 1080p)

## Presets

| Options → Graphics | Shadows / SSR | Internal resolution | Room GPU cap | Decal atlas cache |
| --- | --- | --- | --- | --- |
| **Low memory** | Off | 75% of window (min 1280×720) | 2 rooms | 1 tileset |
| **Performance** | Off | 100% | 6 | 4 |
| **Visuals** | High + SSR | 100% | 6 | 4 |

Low memory also disables HDR swapchain output even if HDR is toggled in Options.

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

First launch without a saved graphics choice applies [`GraphicsMode::suggest_for_adapter`](../../crates/mahjuro-gfx-types/src/graphics_mode.rs) from the adapter name (and integrated-GPU class).

See also [launch options](launch-options.md).
