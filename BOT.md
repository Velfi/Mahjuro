# Headless Bot — Tuning Guide

The bot is a headless CLI mode of the game that plays full runs against itself
and prints aggregate stats. It exists to surface tuning signals (where does the
difficulty wall sit? does this scaling curve permit completion? how does adding
a starting play affect win-rate?) without you having to play the game.

The bot is an **oracle** — it has perfect information about future tile draws
when it considers strategic discards, and it brute-forces every possible
selection from its hand to find the highest-scoring play. Treat its results as
a **ceiling**, not as a typical-player baseline. If even the oracle can't clear
Ante 2, no human will.

## Quick start

```bash
# 100 runs with the default standard mode.
cargo run --release -- --bot

# 200 runs.
cargo run --release -- --bot 200

# Override individual mode fields without recompiling.
cargo run --release -- --bot 200 --base-target 250 --target-scale 1.3

# Try giving the player an extra play.
cargo run --release -- --bot 200 --plays 5

# Sweep a 4×4×2 grid of (base_target × target_scale × plays).
cargo run --release -- --sweep --runs 30
```

Always use `--release`. The bot is brute-force search across hand subsets and
each turn evaluates ~16k bitmasks; debug builds are 10–20× slower.

## CLI flags

| Flag | Type | Default | Notes |
|---|---|---|---|
| `--bot [N]` | u32 | 100 | Run N games with current overrides and print aggregate stats. |
| `--sweep` | flag | — | Run a parameter grid (see below) instead of a single config. |
| `--runs N` | u32 | 40 | Runs per cell when sweeping. |
| `--base-target N` | u32 | 300 | Ante 1 base score (Small Blind target). |
| `--target-scale F` | f32 | 1.5 | Multiplier applied to `base_target` when an ante completes. |
| `--plays N` | u32 | 4 | `starting_plays` per blind. |
| `--discards N` | u32 | 3 | `starting_discards` per blind. |
| `--gold N` | u32 | 4 | `starting_gold`. |

Anything not overridden uses `GameMode::standard()`.

## Output

A normal `--bot` run prints stats like:

```
=== Bot Stats (200 runs) ===
victories:           0 / 200 (0.0%)
avg blinds cleared:  3.44      ← excludes skipped blinds
avg antes cleared:   0.99      ← Boss-cleared count, the main difficulty signal
max ante reached:    3
avg total score:     2911
avg plays used:      13.65
avg discards used:   3.06 (2.77 strategic, 0.30 random)
avg blinds skipped:  1.50      ← Small/Big skipped to bank gold
avg relics bought:   2.04 (avg gold spent: 12.4)

deaths by ante:
  ante  1:    4 (  2.0%) #
  ante  2:  193 ( 96.5%) ################################################
  ante  3:    3 (  1.5%) #

deaths by blind:
  Big Blind      10 (  5.0%)
  Boss Blind    190 ( 95.0%)
```

The two numbers worth tracking for tuning are **`antes_cleared`** (how far
along the curve the bot makes it) and the **deaths-by-ante histogram** (where
the wall sits). `victories` is binary — if you ever see >0%, the bot can win
the game.

## Sweep output

`--sweep` prints a matrix per `starting_plays` value. Each cell shows
`antes_cleared / win_rate% (avg_blinds, avg_score)`:

```
── starting_plays = 4 ──
 base \ sc |    1.20    |    1.30    |    1.40    |    1.50    |
-----------+------------+------------+------------+------------+
       200 | 4.3/ 0.0%  | 3.0/ 0.0%  | 2.1/ 0.0%  | 1.7/ 0.0%  |
       250 | 2.3/ 0.0%  | 1.4/ 0.0%  | 1.3/ 0.0%  | 1.2/ 0.0%  |
       300 | 1.4/ 0.0%  | 1.1/ 0.0%  | 1.1/ 0.0%  | 1.0/ 0.0%  |
       350 | 1.0/ 0.0%  | 0.9/ 0.0%  | 0.9/ 0.0%  | 0.9/ 0.0%  |
```

Read this as: every cell where `antes_cleared` is below ~6 is "the bot fails
to win" — that's most of the grid. The progression of values across each row
tells you how sensitive the difficulty curve is to each axis. Steeper drop
across a row → that axis is the dominant lever.

To customize the sweep grid edit the constants in [src/main.rs](src/main.rs)
under the `--sweep` block:

```rust
let bases: &[u32] = &[200, 250, 300, 350];
let scales: &[f32] = &[1.20, 1.30, 1.40, 1.50];
let plays: &[u32] = &[4, 5];
```

## How the bot plays

Per turn, in [src/bot.rs](src/bot.rs)::`play_blind`:

1. **Find the best play.** Brute-force every subset of the hand (skipping sizes
   that can't decompose into 2s and 3s), validate via `validate_selection_with_rules`,
   score via `score_sets`, keep the highest. ~16k subsets per turn for a 14-tile
   hand — fast enough in release.
2. **Strategic discard via 1-step rollout.** Generate up to 5 discard candidates
   (drop the K least-participating tiles for K=1..=5), peek the actual upcoming
   wall tiles for each, evaluate the post-discard best play, and discard if it
   beats the current best by `margin = max(5, need_to_target / (plays_remaining + 1))`.
3. **Otherwise play the best hand.**
4. **Random discard fallback** when no positive-scoring play and no rollout
   candidate helps.

Per blind, in `play_run`:

1. **Skip strategy.** Skip Small/Big blinds when projected score
   (`best_play × plays_remaining`) ≥ 2× the target — banks `skip_reward()` gold
   without burning plays. Boss never skipped.
2. **Apply blind, play it.** See above.
3. **Smart relic pick.** Score each offered relic by adding it to a hypothetical
   `RelicState` and re-evaluating best-play across the current hand + 4 synthetic
   random hands (so hand-conditional relics like `TripletBoost` get fair credit).
   Lex-sort by `(marginal_value, rarity_weight)` and pick the winner.
4. **Shop visit.** Roll 3 random non-owned relics (mirroring `ShopScene::new`),
   then loop buying the affordable item with the largest positive marginal
   value until none remain.

## Limitations

- **Wall-mutating relics are under-valued.** `Overflow`, `SetMagnet`, `QuickDraw`,
  `WildWinds`, `JokerTile` change the hand or wall in ways the marginal-value
  estimator can't see. Rarity tie-break partially compensates.
- **No relic synergies.** The bot picks each relic in isolation; it doesn't
  notice that `TripletBoost + PairPower + WhiteSilence` compound.
- **No pre-play sorting / hand restructuring.** Some game modes might allow
  the player to swap tiles or restructure — bot ignores this.
- **Bot uses `GameMode::standard()` as a base.** Other game modes need
  `BotConfig` extension or a `--mode` flag.

## Common tuning workflows

**"Where does the wall sit right now?"**
```bash
cargo run --release -- --bot 200
```
Look at `deaths_by_ante`. The mode (most common ante) is your hard wall.

**"Is target_scaling the problem or base_target?"**
```bash
cargo run --release -- --sweep --runs 30
```
The cell where `antes_cleared` improves the most when you change one axis is
the dominant lever. Walk down a column (fixed scaling, varying base) to isolate
base-target sensitivity; walk across a row to isolate scaling sensitivity.

**"Will adding a starting play save the game?"**
```bash
cargo run --release -- --bot 200 --plays 5
```
Compare `antes_cleared` to the baseline.

**"Find the sweet spot."**
Iterate: pick the grid cell with the best `antes_cleared`, then re-sweep a
finer grid around it. The bot is fast enough that 30 runs/cell × 32 cells
finishes in well under a minute.

## Adding new tuning levers

1. Add the field to `BotConfig` in [src/bot.rs](src/bot.rs).
2. Apply it in `BotConfig::into_mode`.
3. Wire a CLI flag in [src/main.rs](src/main.rs)'s `bot_config` block.
4. Optionally surface it in `run_sweep` as a new axis.

## Files

- [src/bot.rs](src/bot.rs) — bot logic, run loop, stats, sweep
- [src/main.rs](src/main.rs) — CLI parsing for `--bot` / `--sweep`
- [src/game/run.rs](src/game/run.rs) — `RunState`, `advance_round`, ante progression
- [src/game/game_mode.rs](src/game/game_mode.rs) — `GameMode` (the thing `BotConfig` overrides)
