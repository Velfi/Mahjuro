# Modularize wgpu init

**Priority:** P2 — large file but mostly linear setup code; lower payoff than runtime split.

## Files

- [src/render/wgpu_renderer/init.rs](../../src/render/wgpu_renderer/init.rs) — 2,765 lines

## Problem

`init.rs` does shader compilation, every render pipeline (tiles 3D, effects, UI, bloom, post), sampler setup, every bind-group layout, and all texture allocation in one linear function. Adding a new pipeline or adjusting a bind group requires navigating thousands of lines.

## Target shape

Carve into a `pipelines/` submodule and helper modules:

- `init/pipelines/tiles.rs` — tile 3D pipeline construction.
- `init/pipelines/effects.rs` — effects pipeline.
- `init/pipelines/ui.rs` — UI pipeline.
- `init/pipelines/bloom.rs` — bloom + post pipelines.
- `init/textures.rs` — render-target and asset texture allocation.
- `init/bind_groups.rs` — layout definitions, shared across pipelines.
- `init/shaders.rs` — shader module loading and compilation.

`init.rs` becomes a thin orchestrator that calls the constructors in order and assembles them into the renderer.

## Acceptance criteria

- No single file in `init/` over 800 lines.
- Each pipeline file constructs its `wgpu::RenderPipeline` and returns it; no shared mutable scratch state.
- Bind group layouts shared across pipelines live in `bind_groups.rs` and are referenced, not duplicated.

## Notes

- Mostly mechanical — these are independent constructions. Low risk, easy to split.
- Shader source paths and module names should stay stable so shader hot-reload (if any) keeps working.
- If you do #2 (runtime split) first, this becomes simpler since the runtime side will be clearer about which pipelines it consumes.
