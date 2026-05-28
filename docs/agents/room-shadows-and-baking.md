# Room shadows, GI, and offline bakes

Headless bake tools live in [`crates/mahjuro-headless`](../../crates/mahjuro-headless) (`mahjuro-bake`, `mahjuro-screenshot`) so they do not build the interactive `mahjuro` binary (no `rodio`, Steam, or SDL). GPU work is in `mahjuro-render`; room scenes still link for draw recording.

Stamp hashes and skip env vars are centralized in [`crates/mahjuro-bake-stamp`](../../crates/mahjuro-bake-stamp). `build.rs` compares committed `.inputs_stamp` files against current inputs; set the matching `MAHJURO_SKIP_*_BAKE=1` while rebuilding a baker so stale stamps do not block compilation.

## Room vertex warp + live shadows

`shaders/hallway_vertex_warp.wgsl` is prepended into `room_glb`, `tile_3d`, and `shadow.wgsl` ([`embedded_wgsl.rs`](../../crates/mahjuro-render/src/wgpu_renderer/embedded_wgsl.rs)). The shadow depth pass binds the same `HallwayDistortion` bytes as the lit room pass (`ShopEnvironmentGpu.distortion_buffer` → group 1). Tiles and lit-mesh casters use `shadow_warp_disabled_bind_group`. **Any new scene warp must keep lit and shadow vertex shaders in sync.**

## Offline room shadows (`.msh`)

Static rooms ship offline `assets/data/room_shadow/<room>.msh` (MSH1: depth + contact AO). [`room_glb.wgsl`](../../shaders/room_glb.wgsl) samples the baked map when present; the live 1024² shadow map is for **moving catalog props only**.

**Rebake all rooms** (refreshes `.inputs_stamp` automatically):

```bash
MAHJURO_SKIP_ROOM_GI_BAKE=1 MAHJURO_SKIP_ROOM_SHADOW_BAKE=1 \
  cargo build -p mahjuro-headless --bin mahjuro-bake --features bake && \
  ./target/debug/mahjuro-bake
```

- Stamp logic: [`mahjuro_bake_stamp::room_shadow`](../../crates/mahjuro-bake-stamp/src/room_shadow.rs)
- Skip freshness check while rebuilding the baker: `MAHJURO_SKIP_ROOM_SHADOW_BAKE=1` (mirrors `MAHJURO_SKIP_ROOM_GI_BAKE`)
- Implementation: [`room_shadow_bake.rs`](../../crates/mahjuro-render/src/room_shadow_bake.rs)

### Archive (exception)

Archive **does not** sample offline `archive.msh` today — the room GLB uses punctual lights only (`COLOR_0.a = 3` on shell meshes). Offline cubby-only bakes mis-darken receivers as the asset grows (30+ prims). See [`docs/todo/archive-offline-baked-shadows.md`](../todo/archive-offline-baked-shadows.md).

The live map is still used on archive for **inspect orbit only**: only [`SHOP_INSPECT_SUBJECT_ANIM_ID`](../../crates/mahjuro-render/src/draw_cmd.rs) casts dynamic shadows (grid featured close-up and cubbies do not, so the pedestal and description signs stay clean). Inspect sets `UiFrame::shop_inspect_shadow_target` for a tight frustum + subject-only cast.

## Offline room GI (`.mgi`)

Static rooms also ship `assets/data/room_gi/<room>.mgi` (MGI1: emissive probe SH). Rebake via the same `mahjuro-bake` run as shadows (or `--kinds gi` / `--kinds shadow` separately).

- Stamp logic: [`mahjuro_bake_stamp::room_gi`](../../crates/mahjuro-bake-stamp/src/room_gi.rs)
- Skip env var: `MAHJURO_SKIP_ROOM_GI_BAKE=1`
- Implementation: [`room_gi_bake.rs`](../../crates/mahjuro-render/src/room_gi_bake.rs)

Rebake after changing room GLB layout, probe grid, or [`ROOM_EMISSIVE_PROBE_DIR_SAMPLES`](../../crates/mahjuro-render/src/room_glb.rs) / [`ROOM_EMISSIVE_PROBE_MARCH_STEPS`](../../crates/mahjuro-render/src/room_glb.rs).

## Relic RLC1 bakes

`assets/data/relic_baked/<slug>.rlc` — mask-cut albedo + relief + mesh. Runtime loads RLC1 instead of decoding PNGs when all bakes are present.

**Rebake** (refreshes `.inputs_stamp` automatically):

```bash
cargo run -p mahjuro-render --bin mahjuro-bake-relics
```

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

Depends on `mahjuro` with `bake-support` only. On a successful full-room run, `mahjuro-bake` refreshes `.inputs_stamp` for each baked kind so the next `cargo build` does not flag committed outputs as stale.
