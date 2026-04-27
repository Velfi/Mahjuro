# Decompose Run state monolith

**Priority:** P2 — large but more cohesive than the others; lower urgency.

## Files

- [src/game/run.rs](../../src/game/run.rs) — 2,555 lines
- [src/game/run/scoring_flow.rs](../../src/game/run/scoring_flow.rs) — extraction already started

## Problem

The `Run` struct holds wall, hand, scoring state, blind effects, consumables, tags, zodiac levels, relic counters, and round/scoring flow in a single struct with ~100 methods. Adding or modifying a relic effect means scrolling through unrelated state. The `scoring_flow.rs` extraction is a good start but the trunk is still huge.

## Target shape

Extract sub-structs by concern, keep them as fields on `Run`:

- `RunInventory { active_relics: Vec<RelicId>, counters: BTreeMap<RelicId, u32>, consumables: Vec<Consumable>, talismans: Vec<Talisman> }`
- `ZodiacState { levels: BTreeMap<ZodiacKind, u8>, … }`
- `BlindState { current: BlindKind, effects_active: …, beats_required: u32 }` (if not already separate)
- `ScoringState` — chips, mult, base, current hand under evaluation.

Methods that operate on a single sub-struct move with it. `Run` becomes the orchestrator that owns the sub-structs and routes high-level actions.

## Acceptance criteria

- `run.rs` under 1,200 lines.
- Each sub-struct lives in its own file under `src/game/run/`.
- Bot balance run produces equivalent results (compare `docs/bot_balance_runs.json`).

## Notes

- This is the riskiest refactor on the list — `Run` is the heart of the game state. Do it last and behind a branch.
- Save state serialization is likely tied to `Run` field shape; if save compat matters, plan a migration.
- Doing #1 (data externalization) first will make `RunInventory` simpler since relic metadata lookups become uniform.
