# Refactor backlog

One file per candidate. Numbers are rough priority order — start at the top.

| # | Item | File(s) | Priority |
|---|------|---------|----------|
| 1 | [Externalize game data](01-externalize-game-data.md) | `core/{relic,yaku,boss}.rs` | P0 |
| 3 | [Continue gameplay scene split](03-split-gameplay-scene-behavior.md) | `scenes/gameplay/scene_behavior.rs` (4,854 LOC) | P1 |
| 5 | [Decompose Run state](05-decompose-run-state.md) | `game/run.rs` (2,555 LOC) | P2 |
| 6 | [Modularize wgpu init](06-modularize-wgpu-init.md) | `render/wgpu_renderer/init.rs` (2,765 LOC) | P2 |
| 7 | [Split hand decomposition](07-split-hand-decomposition.md) | `core/hand.rs` (1,622 LOC) | P3 |
| 8 | [Separate bot decision/rollout](08-separate-bot-decision-rollout.md) | `bot.rs` (2,306 LOC) | P3 |

## Suggested order

1 → 3 → 5 → 7 → 6 → 8.

#1 unblocks faster balance iteration *and* shrinks three of the largest core files in one stroke. #3 finishes an in-flight refactor.
