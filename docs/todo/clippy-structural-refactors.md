# Clippy — structural refactors (too many arguments, type complexity, large enums)

## Why
After landing mechanical Clippy fixes (`cargo clippy --fix`, collapsible `if`s, digit grouping, etc.), the library still emits ~56 warnings in three categories that need real design work—not one-liners. A crate-level `#![allow(...)]` in [`src/lib.rs`](../../src/lib.rs) was tried and removed; the right fix is to shrink hot types and bundle long parameter lists so `cargo clippy` stays clean without blanket suppression.

## Scope
Work in three tracks; each can land incrementally, but the goal is zero warnings in these lints without new crate-wide allows.

1. **`clippy::too_many_arguments` (~55 call sites).** Introduce small context/params structs where functions carry 8+ mostly-related arguments. Prefer existing patterns (`ChronicleEmit`, `DecalUploadCtx`, `DrawCtx`) over ad-hoc tuples. Highest-leverage clusters:
   - Chart/dashboard emitters — [`push_chart_y_axis`](../../src/ui/chart_primitives.rs), [`push_yaku_hbar_row`](../../src/ui/chart_primitives.rs), [`push_chronicle_dashboard`](../../src/ui/chronicle_dashboard.rs), [`push_kpi_card`](../../src/ui/chronicle_charts.rs) and siblings.
   - Render upload/bind helpers — [`TextureSlotPool::upload`](../../src/render/wgpu_renderer/resources.rs), [`create_shadow_sample_bind_group`](../../src/render/lit_mesh.rs), [`upload_room_baked_shadow_gpu`](../../src/render/wgpu_renderer/impl_room_shadow.rs).
   - Scene layout/draw — [`push_archive_cubby_new_badges`](../../src/scenes/collection.rs), [`archive_page_step`](../../src/scenes/collection.rs), pack celebration helpers in [`tile_pack.rs`](../../src/scenes/showcase/tile_pack.rs), discard animation placement fns in [`discard_animation.rs`](../../src/scenes/gameplay/discard_animation.rs).
   - Particles/rain — [`ParticleSystem::update_world`](../../src/render/particles.rs), [`RainField::update`](../../src/render/rain_field.rs), debug overlay row drawers.
   - Headless harness — [`HeadlessApp::with_run`](../../src/main/headless/app.rs).

   **Out of scope:** deleting parameters just to satisfy the lint; keep debug-only knobs on debug overlays unless they move to a shared `DebugOverlayCtx`.

2. **`clippy::type_complexity` (4 fields).** Add a type alias (e.g. `GltfMipChain = Arc<Vec<(Vec<u8>, u32, u32)>>`) in [`tile_glb.rs`](../../src/render/tile_glb.rs) and use it on [`LoadedPrimitive`](../../src/render/tile_glb.rs) albedo/normal/MR/emissive mip-chain fields. Reuse the alias anywhere else the same triple repeats.

3. **`clippy::large_enum_variant` (1 enum).** [`Scene::Shop(ShopScene)`](../../src/scenes/mod.rs) is the outlier (~640 B vs ~360 B for the next-largest variant). Box heavy scene payloads the same way [`Scene::Gameplay`](../../src/scenes/mod.rs) already boxes `GameplayScene`. Re-run Clippy and check whether `ShowcaseScene`, GLB caches, or other variants need the same treatment after Shop is boxed.

**Explicitly out of scope:** re-enabling a crate-level `#![allow(clippy::too_many_arguments)]` (or siblings); mechanical lints already fixed elsewhere; changing `enum_dispatch` wiring beyond what boxing requires.

## Touchpoints
- [`src/lib.rs`](../../src/lib.rs) — verify no crate-wide Clippy allows return; run `cargo clippy` as the acceptance check.
- [`src/scenes/mod.rs`](../../src/scenes/mod.rs) — `Scene` enum; box `ShopScene` (and audit other large variants).
- [`src/render/tile_glb.rs`](../../src/render/tile_glb.rs) — `LoadedPrimitive` mip-chain type alias.
- [`src/ui/chart_primitives.rs`](../../src/ui/chart_primitives.rs), [`src/ui/chronicle_charts.rs`](../../src/ui/chronicle_charts.rs), [`src/ui/chronicle_dashboard.rs`](../../src/ui/chronicle_dashboard.rs) — chart param structs.
- [`src/ui/controller_hints.rs`](../../src/ui/controller_hints.rs) — inline hint row measurement/emit.
- [`src/render/wgpu_renderer/resources.rs`](../../src/render/wgpu_renderer/resources.rs), [`src/render/lit_mesh.rs`](../../src/render/lit_mesh.rs), [`src/render/wgpu_renderer/impl_room_shadow.rs`](../../src/render/wgpu_renderer/impl_room_shadow.rs) — GPU upload/bind argument bundles.
- [`src/scenes/collection.rs`](../../src/scenes/collection.rs), [`src/scenes/showcase/tile_pack.rs`](../../src/scenes/showcase/tile_pack.rs), [`src/scenes/gameplay/discard_animation.rs`](../../src/scenes/gameplay/discard_animation.rs), [`src/scenes/tutorial_campaign.rs`](../../src/scenes/tutorial_campaign.rs) — scene layout helpers.
- [`src/render/particles.rs`](../../src/render/particles.rs), [`src/render/rain_field.rs`](../../src/render/rain_field.rs), [`src/render/rain_debug_overlay.rs`](../../src/render/rain_debug_overlay.rs), [`src/render/room_env_gltf.rs`](../../src/render/room_env_gltf.rs) — sim/debug GLTF harvest.
- [`src/debug_overlays.rs`](../../src/debug_overlays.rs), [`src/main/headless/`](../../src/main/headless/), [`src/scenes/lamp_moths.rs`](../../src/scenes/lamp_moths.rs) — debug/headless entry points.
- Per-function `#[allow(clippy::too_many_arguments)]` already sprinkled in [`object3d_placement.rs`](../../src/render/wgpu_renderer/runtime/object3d_placement.rs), [`input_handler.rs`](../../src/scenes/gameplay/input_handler.rs), etc. — migrate those sites into the same struct pattern or remove the allows when refactored.

## Open questions
- **Box vs split `ShopScene`.** Boxing is the smallest diff and matches `Gameplay`; splitting shop state out of the scene struct might shrink further but is a larger behavioral refactor.
- **Chart context granularity.** One `ChartEmit` struct per file vs a shared `ui/chart_emit.rs` module — balance duplication against import churn across chronicle dashboard + charts.
- **CI gate.** Whether to add `-D warnings` for Clippy on `mahjuro` lib once this lands, or keep warnings advisory until the count hits zero.
