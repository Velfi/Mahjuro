# Archive offline baked directional shadows

## Status
Archive currently **does not** sample `assets/data/room_shadow/archive.msh`. A workaround landed: archive uses no room-GLB directional shadow receive/cast path (punctual lights only). Shop / hallway / main menu still use the shared baked + live pipeline.

## Why
The archive room grew from ~9 to 30+ GLB primitives (`btn_*`, `text_*`, `plaque_*`, lowercase shell names). Offline cubby-only bakes still project shelf-wide dark fields onto `main_fixture`, page chrome, and the back wall when receivers sample the `.msh`. Per-node Rust allowlists cannot keep pace with asset edits and do not separate casters from receivers cleanly enough.

## Scope
1. **Asset split (preferred):** In Blender / `archive.glb`, separate **caster** geometry (cubby lattice, thin occluders) from **receiver** shells (`main_fixture`, `wall`, floor/ceiling, UI chrome). Document node naming in [`archive_glb.rs`](../../src/render/archive_glb.rs) module docs.
2. **Re-enable archive in** [`room_env_uses_offline_baked_shadow`](../../src/render/wgpu_renderer/runtime/shadow_setup.rs) once bake + sampling are stable; remove the `ActiveRoomEnv::Archive` early return.
3. **Bake pass:** Only caster nodes draw in `mahjuro bake-room archive --kinds shadow` (restore [`archive_prim_casts_room_shadow`](../../src/render/archive_glb.rs) to cubby-only or explicit caster list). Rebake after every `archive.glb` change; patch `mahjuro-pack-gameplay.zip` or ship via loose assets in dev.
4. **Sampling:** Receivers use baked contact without self-occlusion from their own depth in the map. Options: receiver-only sampling mask, `params.w = 2` baked-only path in [`room_glb.wgsl`](../../shaders/room_glb.wgsl), and/or no contact AO on archive bakes (AO was washing the pedestal when cubbies were the only casters).
5. **Regression check:** Archive Relics tab — no hard black blocks on pedestal, page buttons, description sign, or shelf void behind grid items. Inspect orbit still uses live map via [`SHOP_INSPECT_SUBJECT_ANIM_ID`](../../src/render/draw_cmd.rs) on `lit_mesh`.

Out of scope: rebaking shop/hallway/main_menu policy. Out of scope: dynamic grid relic shadows on the shelf (catalog props stay inspect-only on archive unless product asks otherwise).

## Touchpoints
- [assets/3d/archive.glb](../../assets/3d/archive.glb) — caster/receiver mesh organization; export without Draco.
- [assets/data/room_shadow/archive.msh](../../assets/data/room_shadow/archive.msh) — offline depth + optional AO; rebake via CLI.
- [src/render/archive_glb.rs](../../src/render/archive_glb.rs) — `archive_prim_casts_room_shadow`, `archive_env_skips_directional_room_shadow`; module node-name docs.
- [src/render/room_env_gltf.rs](../../src/render/room_env_gltf.rs) — `ROOM_ENV_COLOR_A_ARCHIVE_*` vertex tags at decode; may narrow tag `3` once baked path works.
- [src/render/wgpu_renderer/runtime/shadow_setup.rs](../../src/render/wgpu_renderer/runtime/shadow_setup.rs) — `room_env_uses_offline_baked_shadow`, `skip_room_env_live_shadow_pass`, `object3d_casts_dynamic_shadow`.
- [src/render/wgpu_renderer/impl_room_shadow.rs](../../src/render/wgpu_renderer/impl_room_shadow.rs) — `write_active_room_baked_shadow_globals`, `room_baked_light_view_proj`, capture readback.
- [src/render/room_shadow_bake.rs](../../src/render/room_shadow_bake.rs) — `bake_contact_ao_from_depth`, MSH1 encode/decode.
- [shaders/room_glb.wgsl](../../shaders/room_glb.wgsl) — `sample_shadow_visibility`, baked vs live `light_view_proj`, archive `params.w` modes.
- [src/main/headless.rs](../../src/main/headless.rs) — `scene_for_room_gi_bake` / `run_bake_room_shadows_command` (Collection Relics tab camera).
- [build/room_shadow_bake.rs](../../build/room_shadow_bake.rs) — release rebake stamp inputs.
- [room-shadows-and-baking.md](../agents/room-shadows-and-baking.md) — baked/live shadow policy (archive exception).

## Open questions
- **Bake-first vs live-only archive.** Is offline contact worth the Blender split, or is punctual-only lighting acceptable long-term for archive?
- **Cubby self-receive.** Should cubby interiors sample the baked map at all, or only `main_fixture` / wall receivers?
- **Second bake pass.** One pass = casters → depth; second pass = receivers only for AO — needed if a single depth map cannot represent the split.
- **GLB hot reload.** `ARCHIVE_GLB_CPU` loads once per process; asset iteration requires a full restart unless cache invalidation is added.
