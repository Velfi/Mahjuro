# Room-bake compile surface trim

## Status
Partial work landed: `mahjuro-bake` uses dedicated [`RoomBakeApp`](../../crates/mahjuro-headless/src/room_bake_app.rs) and [`mahjuro::room_bake`](../../src/room_bake/mod.rs). Steam/audio bake stubs removed. SDL gamepad/rumble paths live in [`ui/input/sdl.rs`](../../src/ui/input/sdl.rs) (`game | headless-screenshot` only); bake builds no longer compile [`sdl_shell`](../../src/sdl_shell.rs). Scene cfg-gating and optional minimal render frames remain open.

## Why
`mahjuro-bake` only needs six resting-room views (shop, hallway, archive, main menu, staircase, gameplay) to produce `.mgi` / `.msh` via the normal render path. Today it still links almost all of `mahjuro` — credits, tutorials, labs, showcase transitions, gamepad input, etc. — which slows bake-tool rebuilds and surfaces dead-code warnings from code paths bake never runs. Trimming the bake link surface makes headless tooling cheaper to iterate on without forking draw logic away from the shipped game.

## Scope
1. ~~**Split `ui/input.rs`:**~~ Done — [`ui/input/mod.rs`](../../src/ui/input/mod.rs) + cfg-gated [`ui/input/sdl.rs`](../../src/ui/input/sdl.rs); [`sdl_shell`](../../src/sdl_shell.rs) is `game | headless-screenshot` only.
2. ~~**Drop `sdl_shell` from bake-only `mahjuro`:**~~ Done (depends on step 1).
3. **Cfg-gate scene modules for bake-only:** Under `all(bake-support, not(game), not(headless-screenshot))`, compile only the six room scenes plus their dependency cone (e.g. shop → showcase/object3d_inspect, gameplay → pause_menu/journal_transition/options). Gate `Scene` enum variants and `active_scene_key` arms to match. Fix or cfg-gate transition code in shop/gameplay that references gated scenes (e.g. `YakuJournalScene`, `ShowcaseScene` pushes).
4. **Evaluate a render-side minimal frame (optional, larger):** If scene cfg-gating is too brittle, add a `mahjuro-render` helper that builds resting-camera `UiFrame`s per [`RoomGiRoom`](../../crates/mahjuro-render/src/room_gi_bake.rs) using existing GLB helpers (`hallway_glb`, `room_glb`, `gameplay_glb`, …). Only pursue if visual parity with scene `draw_frame()` is verified (rebake diff + spot-check captures).

Out of scope: changing bake output formats, stamp hashing, or rebake CLI flags. Out of scope: splitting `mahjuro` into separate crates unless the cfg approach fails. Screenshot harness (`headless-screenshot`) must keep the full scene + input stack.

## Touchpoints
- [`Cargo.toml`](../../Cargo.toml) — `bake-support` feature; optional new `room-bake-only` alias if clearer than compound cfgs.
- [`src/lib.rs`](../../src/lib.rs) — module gates (`sdl_shell`, `scenes`, `game`, `ui`).
- [`src/room_bake/`](../../src/room_bake/) — `scene_for_room`, fixtures; may grow if scene setup moves out of headless.
- [`src/scenes/mod.rs`](../../src/scenes/mod.rs) — `Scene` enum, `enum_dispatch`, `active_scene_key`; primary cfg-gating surface.
- [`src/scenes/shop/`](../../src/scenes/shop/) — showcase/object3d_inspect imports; transition arms to cfg or stub.
- [`src/scenes/gameplay/`](../../src/scenes/gameplay/) — `input_handler.rs`, `scene_behavior.rs`; heavy `GameEngine` + cross-scene refs.
- [`src/ui/input.rs`](../../src/ui/input.rs) — SDL split; [`src/ui/mod.rs`](../../src/ui/mod.rs) re-exports.
- [`src/sdl_shell.rs`](../../src/sdl_shell.rs) — interactive-only after split.
- [`crates/mahjuro-headless/src/room_bake_app.rs`](../../crates/mahjuro-headless/src/room_bake_app.rs) — bake tick loop; may simplify further if scenes shrink.
- [`crates/mahjuro-headless/Cargo.toml`](../../crates/mahjuro-headless/Cargo.toml) — bake vs screenshot feature deps.
- [`docs/agents/room-shadows-and-baking.md`](../../docs/agents/room-shadows-and-baking.md) — document bake compile expectations after trim.

## Open questions
- **Scene cfg vs minimal `UiFrame`.** Is maintaining parallel bake draw builders in `mahjuro-render` worth faster compile, or is cfg-gating the existing six scenes enough?
- **Showcase dependency in shop.** Can shop bake avoid compiling showcase presenters (inspect/orbit paths) while keeping stock layout identical, or is showcase too entangled in `ShopScene` update/draw?
- **Separate `mahjuro-room-bake` crate.** If cfg sprawl in `scenes/mod.rs` becomes unmaintainable, is a thin crate (render + core + six scene copies) preferable to feature-matrix hell?
