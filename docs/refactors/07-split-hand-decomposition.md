# Split hand pattern detection

**Priority:** P3 — payoff is moderate; mostly readability, possibly perf.

## Files

- [src/core/hand.rs](../../src/core/hand.rs) — 1,622 lines

## Problem

`hand.rs` does hand validation, set decomposition (with what looks like exponential backtracking), shanten calculation, riichi rule checks, and flower-wildcard handling all in one file. The decomposition algorithm is complex with multiple branching paths, and there is no visible memoization despite the same hands being evaluated repeatedly during scoring.

## Target shape

- `core/hand/decomposition.rs` — enumerate and score all valid tile groupings; this is the heavy algorithm.
- `core/hand/validation.rs` — rule checking (yaku eligibility, riichi conditions).
- `core/hand/shanten.rs` — shanten/tenpai computation.
- `core/hand.rs` — public API (`Hand`, `analyze()`, etc.) calling into the submodules.

While in there, evaluate whether decomposition can memoize on a canonical tile-multiset key — bot rollouts call this thousands of times per run.

## Acceptance criteria

- `hand.rs` under 600 lines (public API surface only).
- Decomposition algorithm has a unit test covering the tricky cases (chiitoitsu, kokushi, shared-tile ambiguity).
- Bot balance run completes at least as fast as before; ideally faster if memoization lands.

## Notes

- The algorithm is subtle — preserve behavior with property tests before refactoring (generate random hands, snapshot decomposition output, refactor, confirm output identical).
- Memoization is a nice-to-have, not required; only land it if benchmarks show a win.
- Flower wildcards complicate the canonical key — may not be worth memoizing through them.
