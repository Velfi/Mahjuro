# Separate bot decision-making from rollout

**Priority:** P3 — quality-of-life for balance work; not blocking gameplay.

## Files

- [src/bot.rs](../../src/bot.rs) — 2,306 lines

## Problem

`bot.rs` tangles strategy selection, decision-making (relic valuation, shop buys, consumable use), play rollouts (turn-by-turn simulation), and stats collection. Tuning the bot's shop heuristic means rereading the entire rollout loop. The reporting submodule is already separate but the core decision logic is not.

## Target shape

- `bot/decision_maker.rs` — pure decisions: pick relic, pick consumable, decide whether to visit shop, choose discard tile. Inputs: game state. Outputs: action.
- `bot/play_evaluator.rs` — rollout helpers: score estimation, tenpai checks, expected-value math used by the decision maker.
- `bot.rs` — top-level driver: loop over rounds, call into decision maker, advance simulation, hand off to reporter.

## Acceptance criteria

- `bot.rs` under 800 lines.
- `decision_maker.rs` functions are pure (state in, action out); easy to unit test in isolation.
- Existing balance plots (`docs/bot_balance_*.png`) are reproducible and unchanged.

## Notes

- Lower priority because bot code is self-contained — refactoring it doesn't unblock other work.
- Doing #1 (data externalization) first makes `decision_maker` cleaner since relic value lookups become uniform.
- Good candidate for parameterizing the bot strategy: with pure decision functions, you can A/B different heuristics by swapping the module.
