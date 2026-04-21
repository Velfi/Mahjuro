# Current TODO

- Revisit relic rearranging on the shop screen.
- Investigate mirror- and shadow-hand interactions while touching shop relic order.
- Investigate `SecondWind`; it appears to accumulate unexpectedly.
- Rebalance `WildWinds`; it may still be too strong.

## Refactors

- **P0 — Split `src/render/wgpu_renderer.rs` (12.3k LOC).** One `impl WgpuRenderer` runs lines 2005–12235; `new()` ~2,700 lines, `render()` ~5,900, `resize()` 230. Move to sibling modules behind a facade: `render/pipelines/` (one file per pipeline), `render/passes/` (one fn per pass), `render/picking.rs` (the five `pick_*` methods at 5046–5516), `render/offscreens.rs` (depth/scene_prev/scene_color/cascade/post/depth_copy + shared `resize`). Prereq for any meaningful render perf work.
- **P1 — Split `src/scenes/gameplay.rs` (6.7k LOC).** `update()` ~1,340 lines and `draw_frame()` ~3,650 lines are match/if ladders over drag/cascade/pause/tutorial sub-states. Move into the existing [scenes/gameplay/](src/scenes/gameplay/) dir: `update/{input,cascade,pause,tutorial}.rs`, `draw/{board,hand,hud,tooltips,cascade}.rs`. Isolates the tutorial branches that currently riddle the file.
- **P2 — Split `src/scenes/shop.rs` (4.6k LOC) and `src/game/run.rs` (4.4k LOC).** Shop: break up the ~3k-line `ShopScene` impl the same way as gameplay. Run: 139 methods on one struct — group into submodule impl blocks (`run/{tutorial,onboarding,consumables,talisman}.rs`).
- **P3 — Split `src/main.rs` (4.1k LOC) into an `app/` module.** `window_event` is 1,560 lines, `App::draw` 500, `handle_debug_action` 260, plus ~300 lines of unrelated `BotCli`/snapshot code. Target: `app/{events,draw,debug,bot_cli}.rs`.

### Explicitly not refactoring

- The 19 `*_mesh.rs` builders look duplicated but each has distinct geometry and already shares the right primitives (`lit_mesh::push_box/push_quad` + `MaterialKind`). Further abstraction would obscure the shapes.
- No shared scene-base abstraction needed — `SceneBehavior` + `UpdateCtx`/`DrawCtx` + `card_rect`/`relic_row`/`push_tooltip` already cover it.
