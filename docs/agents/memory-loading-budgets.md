# Memory and loading budgets (baseline)

Baseline numbers for the [memory loading strategy](../../.cursor/plans/memory_loading_strategy_d77710eb.plan.md) (Phase 0–2). Update this doc after each phase with before/after deltas.

Related: [GPU memory / 4 GB soak](gpu-memory.md), [launch options](launch-options.md).

## Capture procedure

### Automated wrapper

```bash
# Boot-only baseline (auto-quits after sync + async startup profiles):
./scripts/memory-loading-baseline.sh --startup

# Full soak — run hub tour + 30 min play, then quit normally:
./scripts/memory-loading-baseline.sh

# Re-summarize an existing capture:
./scripts/memory-loading-baseline.sh --summarize baseline-captures/YYYYMMDD-HHMMSS
```

Outputs land in `baseline-captures/<timestamp>/`:

| File | Contents |
| --- | --- |
| `game.log` | Full `RUST_LOG=mahjuro=info` output with profiling lines |
| `rss-samples.tsv` | Process RSS (KiB) every 5 s |
| `time.log` | macOS `/usr/bin/time -l` stderr (peak RSS) |
| `summary.txt` | Parsed key metrics |

### Environment (set by script)

```bash
export MAHJURO_STARTUP_PROFILE=1
export MAHJURO_GPU_MEM_PROFILE=1
export MAHJURO_GRAPHICS_MODE=low_memory
RUST_LOG=mahjuro=info cargo run --release -- --no-steam
```

Use `MAHJURO_AUTO_LOW_MEMORY=1` instead of `MAHJURO_GRAPHICS_MODE` when testing the saved-settings path on a machine that would otherwise pick Performance/Visuals.

### Manual soak checklist

1. Borderless 1920×1080 (default when borderless fullscreen is on in settings).
2. **Hub tour:** main menu → shop → hallway → archive → gameplay → back.
3. **Options:** switch tileset once.
4. **Play 30+ minutes** — watch for `gpu mem profile:` drift and device lost.
5. Quit normally (save on exit).

Peak RSS during the hub tour is the critical RAM number — note the Activity Monitor peak if sampling misses a spike.

---

## Metrics matrix

| Metric | Low memory target | Pre-change soak | Post-change soak (`20260604-210323`) | Notes |
| --- | --- | --- | --- | --- |
| Sync boot wall time | TBD | **3036 ms** | **4736 ms** | Cold-run variance (shader scope +780 ms this session) |
| `assets.init` (pack mount) | TBD | **2.5 ms** | **1.2 ms** | Pack split: only eager `shared` at init |
| `wgpu.renderer_new` | TBD | **2344 ms** | **3444 ms** | Session variance on M4 Max |
| Peak process RSS (sampled) | TBD | **2381 MiB** (~3 min) | **2645 MiB** (~6 min) | Longer soak → higher peak |
| Peak process RSS (time -l) | TBD | **~3245 MiB** | **~3524 MiB** | |
| Max frame hitch at scene fade | < 50 ms | **321 ms** (hallway) | **321 ms** (hallway), **102 ms** (staircase) | **10 ms** (hallway) after hitch fix — talisman GPU warm moved off shop upload path |
| Relic GPU (profiled, Low memory hub) | ≤ 35 MiB LRU | — | — | on-demand cap 24 (archive page = 21); RLC2 BC7 pack ~243 MiB on disk |
| Decal atlas CPU (profiled) | ≤ 60 MiB | — | — | LRU 1 atlas; original downscaled to 4096 px long side on Low memory |
| Music PCM (profiled) | ≤ 40 MiB live set | — | — | evict non-active tracks on BGM start |
| Device lost / OOM | none | **none** | **none** | |
| Concurrent CPU decodes at menu | ≤ 1 | shop + archive early | shop + archive early | Throttle gates *starting* new work; in-flight chain still overlaps on this path |

---

## Baseline capture log

Record machine context once:

| Field | Value |
| --- | --- |
| Date | 2026-06-04 |
| Machine / RAM | Apple M4 Max / 36 GiB |
| OS | Darwin 25.5.0 arm64 |
| GPU / adapter | Apple M4 Max (Metal, LowMemory forced) |
| Pre soak | `baseline-captures/20260604-204159` (~3 min) |
| Post soak | `baseline-captures/20260604-210323` (~6 min) |
| Post hitch fix | `baseline-captures/20260604-212040` (`--startup`, hallway prev dt **10.0 ms**) |

### Startup scope table (paste from `--startup` summary)

```
wgpu.renderer_new                          2533.0 ms
wgpu.offline_bakes                         1238.4 ms
wgpu.lit_meshes_and_pools                   663.3 ms
app.new                                     561.3 ms
wgpu.tile_mesh                              485.0 ms
sdl.window                                  101.1 ms
wgpu.room.main_menu                          59.0 ms
wgpu.pack_textures                           51.0 ms
wgpu.shaders_and_pipelines                   32.4 ms
wgpu.fonts                                   28.6 ms
assets.init                                  21.4 ms
```

### Room upload hitches (from `--startup` auto-prefetch)

Pre-fix (hallway hitch — shop frame paid talisman texture upload):

```
hallway.glb GPU upload — 15.2 ms upload | prev frame dt 326.3 ms (HITCH)
shop.glb CPU decode — 847.5 ms (worker)
archive.glb CPU decode — 298.8 ms (worker, concurrent with shop)
gameplay.glb CPU decode — 420.9 ms (worker)
```

Post-fix (2026-06-05, `--startup`):

```
shop.glb GPU upload — 72.7 ms | prev frame dt 10.1 ms (ok)
hallway.glb GPU upload — 19.2 ms | prev frame dt 10.0 ms (ok)
```

Talisman textures/meshes now warm on the main menu poll (one frame after main-menu env upload), not inside `ensure_shop_room_gpu`. One room env upload per poll via `maybe_upload_one_room_env`.

Full manual soak (hub tour + 30 min) still required for RSS drift and archive visit hitches.

---

## What each plan phase should move

| Phase | Primary metrics |
| --- | --- |
| 1a pack split | ↓ sync boot wall, ↓ `assets.init`, faster cold start to interactive hub |
| 1b asset cache | ↓ repeat-read RSS spikes (qualitative until instrumented) |
| 1c prefetch throttle | ↓ peak RSS during hub chain |
| 1d CPU evict clear | ↓ RSS after room GPU eviction (low memory) |
| 2a mmap bakes | ↓ decode-time copies (dev/loose tree) |
| 2b bumpalo | ↓ decode allocator pressure under parallel prefetch |
| 2c compressed bakes | ↓ pack size + I/O; slight ↑ decode CPU |
| 2d loader pool | ↓ thread count; smoother priority scheduling |
