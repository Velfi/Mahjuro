# Continue splitting gameplay scene_behavior

**Priority:** P1 — already mid-refactor; finish the job.

## Files

- [src/scenes/gameplay/scene_behavior.rs](../../src/scenes/gameplay/scene_behavior.rs) — 4,854 lines
- [src/scenes/gameplay/](../../src/scenes/gameplay/) — sibling extractions already in flight

## Problem

The bulk of gameplay logic still lives in `scene_behavior.rs`: scene update loop, animation/particle/coin state, input focus tracking, pause-menu sync, cascade queue, hand tile animations, wind timers, UI hint computation. Recent extractions ([action_bar_layout.rs](../../src/scenes/gameplay/action_bar_layout.rs), [candle.rs](../../src/scenes/gameplay/candle.rs), [cascade_hud.rs](../../src/scenes/gameplay/cascade_hud.rs), [focus.rs](../../src/scenes/gameplay/focus.rs), [hand_layout.rs](../../src/scenes/gameplay/hand_layout.rs), [tooltip.rs](../../src/scenes/gameplay/tooltip.rs)) prove the pattern works but the central file is still huge.

## Target shape

Continue extracting into focused modules:

- `gameplay/cascade_controller.rs` — cascade queue, wind delay, score reel logic, blind-end transition.
- `gameplay/animation_state.rs` — particle, coin, tween state shared across frames.
- `gameplay/input_handler.rs` — drag, click, keyboard routing into game actions.

`scene_behavior.rs` should retain only the `SceneBehavior` impl glue: `update()` calls into the controllers, `draw()` calls into the layout helpers.

## Acceptance criteria

- `scene_behavior.rs` under 1,500 lines.
- Each new module owns its state struct and exposes a small API (`tick()`, `draw()`, queries).
- No new circular `mod` dependencies; controllers should not reach back into `SceneBehavior`.

## Notes

- Cascade is the most tangled — extract it first; other state will be easier afterward.
- The existing `focus.rs` extraction is a good template for state + tick + query API shape.
- Animation state may share types with [src/render/](../../src/render/) — keep types in `core` or a shared `types.rs` if cross-cutting.
