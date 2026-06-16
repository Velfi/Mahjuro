# Room shadows, GI, and offline bakes

Headless bake tools live in [`crates/mahjuro-headless`](../../crates/mahjuro-headless) (`mahjuro-bake`, `mahjuro-screenshot`) so they do not build the interactive `mahjuro` binary (no `rodio`, Steam, or SDL). GPU work is in `mahjuro-render`; room scenes still link for draw recording.

Stamp hashes and skip env vars are centralized in [`crates/mahjuro-bake-stamp`](../../crates/mahjuro-bake-stamp). `build.rs` compares committed stamps against current inputs; if they are stale, the build panics and prints the command to run to rebake. Set `MAHJURO_SKIP_OFFLINE_BAKES=1` to disable freshness checks. See [launch options](launch-options.md).

## Room vertex warp + live shadows

`shaders/hallway_vertex_warp.wgsl` is prepended into `room_glb`, `tile_3d`, and `shadow.wgsl` ([`embedded_wgsl.rs`](../../crates/mahjuro-render/src/wgpu_renderer/embedded_wgsl.rs)). The shadow depth pass binds the same `HallwayDistortion` bytes as the lit room pass (`ShopEnvironmentGpu.distortion_buffer` → group 1). Tiles and lit-mesh casters use `shadow_warp_disabled_bind_group`. **Any new scene warp must keep lit and shadow vertex shaders in sync.**

## Offline room shadows (`.msh`)

Static rooms ship offline `assets/data/room_shadow/<room>.msh` (MSH1: depth + contact AO). [`room_glb.wgsl`](../../shaders/room_glb.wgsl) samples baked contact AO via `sample_contact_ao` when a `.msh` is loaded; the live depth array is for **moving catalog props** (static room shells skip the live cast pass when offline bake is active — see `skip_room_env_live_shadow_pass` in [`shadow_setup.rs`](../../crates/mahjuro-render/src/wgpu_renderer/runtime/shadow_setup.rs)).

Contact AO is generated from bake-time GPU captures, not from raw depth alone: [`room_shadow_mask.wgsl`](../../shaders/room_shadow_mask.wgsl) writes receiver/occluder weights, primitive id, and world normal alongside the captured depth. [`room_shadow_bake.rs`](../../crates/mahjuro-render/src/room_shadow_bake.rs) uses those buffers for per-room tuning, receiver/occluder separation, and normal/depth coherence before writing the final AO bytes into `.msh`.

**Rebake all rooms** (refreshes `.inputs_stamp` automatically):

```bash
scripts/rebake-offline.sh room
# or: cargo run -p mahjuro-headless --bin mahjuro-bake --features bake
```

- Stamp logic: [`mahjuro_bake_stamp::room_shadow`](../../crates/mahjuro-bake-stamp/src/room_shadow.rs)
- Freshness checks auto-skip when building `mahjuro-headless` with `--features bake` (`mahjuro/offline-bake-support` on the dependency graph) or when `MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1` is set.
- Implementation: [`room_shadow_bake.rs`](../../crates/mahjuro-render/src/room_shadow_bake.rs)

### Archive runtime sampling

Archive **does not** sample offline `archive.msh` today — the room GLB uses punctual lights only. Offline cubby-only bakes mis-darken receivers as the asset grows (30+ prims). See [`docs/todo/archive-offline-baked-shadows.md`](../todo/archive-offline-baked-shadows.md).

The committed `archive.msh` is still baked and validated with real depth + contact AO so the
room bake set has no placeholder/all-white shadow asset.

The live map is still used on archive for **inspect orbit only**: only [`SHOP_INSPECT_SUBJECT_ANIM_ID`](../../crates/mahjuro-render/src/draw_cmd.rs) casts dynamic shadows (grid featured close-up and cubbies do not, so the pedestal and description signs stay clean). Inspect sets `UiFrame::shop_inspect_shadow_target` for a tight frustum + subject-only cast.

## Offline room GI lightmaps (`.lightmap.rlm`)

Static rooms ship `assets/data/room_lightmap/<room>.lightmap.rlm.zst` (RLM1 container: linear RGBA32F indirect lightmap). `mahjuro-bake --kinds lightmap` runs a dedicated offline GPU compute bake against room GLB geometry/material textures and existing embedded punctual lights, then room environment shaders sample that lightmap directly. Missing or invalid lightmaps panic for real rooms; fallback lighting must not mask broken room bakes.

- Stamp logic: [`mahjuro_bake_stamp::room_gi`](../../crates/mahjuro-bake-stamp/src/room_gi.rs)
- Skip env var: `MAHJURO_SKIP_ROOM_GI_BAKE=1`
- Implementation: [`room_gi_bake.rs`](../../crates/mahjuro-render/src/room_gi_bake.rs)

Rebake after changing room GLB layout, GPU bake sampling constants in [`room_gi_bake.rs`](../../crates/mahjuro-render/src/room_gi_bake.rs), or [`room_gi_bake.wgsl`](../../shaders/room_gi_bake.wgsl).

`<room>.lightmap.png` is only a tonemapped preview for inspection.

## Relic RLC2 bakes

`assets/data/relic_baked/<slug>.rlc` — mask-cut albedo + relief + mesh (BC7 mip chains). Runtime loads RLC2 only (source PNGs under `textures/relics/` are bake-time inputs and are excluded from release asset packs).

**Rebake** (refreshes `.inputs_stamp` automatically):

```bash
scripts/rebake-offline.sh relic
# or: cargo run -p mahjuro-render --bin mahjuro-bake-relics --features relic_bc7_bake
```

Unchanged relics are skipped via per-relic sidecars (`<slug>.rlc.stamp`). When the global stamp is already current, the baker exits immediately after bootstrapping any missing sidecars. Use `--force` or `MAHJURO_FORCE_RELIC_BAKE=1` to rebake every relic.

- Stamp logic: [`mahjuro_bake_stamp::relic`](../../crates/mahjuro-bake-stamp/src/relic.rs)
- Skip env var: `MAHJURO_SKIP_RELIC_BAKE=1`

## Marketing screenshots

```bash
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot -- --scene shop …
```

See [`ScreenshotCli`](../../crates/mahjuro-headless/src/screenshot_cli.rs). Pulls almost all scenes + `bot` for `--bot-play` / run-history game-over captures.

## Room bake binary

```bash
cargo build -p mahjuro-headless --bin mahjuro-bake --features bake
```

Lives in `mahjuro-headless` (`RoomBakeApp` + `room_bake`). On a successful full-room run, `mahjuro-bake` refreshes `.inputs_stamp` for each baked kind so the next `cargo build` does not flag committed outputs as stale.
