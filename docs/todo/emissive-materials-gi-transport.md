---
name: Emissive materials — global illumination transport
description: Let glTF emissive (texture × factor × strength) contribute incident light to other surfaces via GI-style passes, not only self-emission + bloom
type: project
---

# Emissive materials — global illumination transport

## Why
Emissive is applied correctly on the **emitting** fragment (`room_glb.wgsl`, `tile_3d.wgsl`, `lit_mesh.wgsl`), but that energy does not **transport** to neighbors: receivers only see punctual lights, ambient, and SSR where applicable. Artists must duplicate lamps with `KHR_lights_punctual` if the room should be lit by fixtures. Real “emissive drives the scene” needs an explicit global-illumination or area-light path, not shader-only add.

**Product constraint:** Gameplay uses a **fixed camera** and **scenes change slowly**. That favors **offline bakes**, **temporal accumulation** of a cheap screen-space or probe pass across frames, or **amortized** updates (recompute indirect only when the room or major props change), instead of paying full dynamic GI cost every frame at full resolution.

## Scope
1. **Emissive visibility buffer:** Add an **emissive-only** output (MRT on room draws, or a second depth-tested pass) so indirect passes can key off emission without treating bright diffuse as a light source.
2. **Static / semi-static indirect (high ROI given fixed view):** Second-UV **lightmaps** or **irradiance volumes** baked offline for shop/hallway/table backdrops; invalidate or rebake only on asset or layout changes. Consider **temporal box-filter or history blend** on any screen-space bounce so noise and missing samples matter less when the camera does not move.
3. **Dynamic one-bounce (when bake is not enough):** Half- or quarter-res **screen-space** gather (short rays or horizon-based) using **depth** and, if available, **normals** (extra RT or reconstruction). Composite in **linear HDR** before tonemap; scope v1 to **shop / hallway** GLB paths if gameplay `lit_mesh` stays more dynamic.
4. **Stretch goals:** Analytic **LTC rect** (or similar) lights fitted to large emissive panels; **probe / DDGI** only if SSGI + bake prove insufficient.

Out of scope for this doc: automatic **punctual proxies** derived from emissive (cheap but not area/GI transport). Out of scope for v1: full path tracing or changing glTF import semantics.

## Touchpoints
- [shaders/room_glb.wgsl](../../shaders/room_glb.wgsl) — emissive term today; MRT output or separate pass emission; composite input for bounce.
- [shaders/tile_3d.wgsl](../../shaders/tile_3d.wgsl) — same for tiled GLB / shared env path; `gltf_emissive_hdr` split from final color is already a hook.
- [shaders/scene_pbr_lights.wgsl](../../shaders/scene_pbr_lights.wgsl) — shared attenuation; GI pass may stay separate but lives alongside punctual math.
- [src/render/wgpu_renderer/runtime/render.rs](../../src/render/wgpu_renderer/runtime/render.rs) — Pass A / composite order; where bloom and tonemap run; new fullscreen or compute pass before `post_bloom` / tonemap.
- [src/render/wgpu_renderer/init.rs](../../src/render/wgpu_renderer/init.rs) — pipelines, bind groups, extra color targets for MRT or emissive prepass.
- [src/render/wgpu_renderer/resources.rs](../../src/render/wgpu_renderer/resources.rs) — textures matching `scene_color` size or half-res GI buffer.
- [src/render/gltf_helpers.rs](../../src/render/gltf_helpers.rs) — `effective_gltf_emissive_rgb`; bake/import paths if lightmaps arrive as assets.
- [src/render/shop_glb.rs](../../src/render/shop_glb.rs) — material summary / validation; doc for artists if bake workflow is added.

## Open questions
- **Bake-first vs screen-space first.** Fixed camera makes **lightmaps / probes** the likely win; use SSGI only for props or regions that cannot bake cleanly.
- **MRT vs prepass.** MRT avoids a second scene draw but increases bandwidth and pipeline permutations; emissive-only prepass is simpler to gate per scene.
- **Normals for SSGI.** Extra G-buffer target vs depth-only reconstruction (artifacts on shallow angles).
- **Gameplay table (`lit_mesh`).** Tiles and candles move; decide whether indirect is **baked for static room only** with punctual for the table, or a separate low-cost pass with temporal accumulation.
