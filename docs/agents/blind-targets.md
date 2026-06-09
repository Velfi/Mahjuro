# Mahjuro blind target calculation

## The loop

1. **Run starts** with a `base_target` (season-adjusted from the default).
2. For each **wing** (ante), compute that wing's base target with exponential scaling.
3. Multiply by chamber type (**Small / Big / Ordeal**).
4. Apply any boss hook that mutates the target (for example, Famine doubles it again).

---

## Core formula

Runtime target calculation is:

```text
wing_base = round(base_target * TARGET_SCALING^(wing - 1))
target    = round(wing_base * chamber_multiplier)
```

Where:

- `TARGET_SCALING = 1.618034` (golden ratio φ, [OEIS A001622](https://oeis.org/A001622))
- `chamber_multiplier`:
  - Small = `1.0`
  - Big = `1.5`
  - Ordeal = `2.0`

Implementation details:

- `wing` is clamped to at least `1`.
- Rounding is `round()` (not floor/ceil).
- Final value is clamped to at least `1`.

---

## Current constants

From `core::chamber_target`:

- `DEFAULT_BASE_TARGET = 1000`
- `TARGET_SCALING = 1.618034` (golden ratio φ)
- `SMALL_MULT = 1.0`
- `BIG_MULT = 1.5`
- `BOSS_MULT = 2.0`

---

## Season impact on `base_target`

`GameMode::with_material_and_season` applies season scaling once:

```text
base_target = round(DEFAULT_BASE_TARGET * season.base_target_mult())
```

Current season multipliers (`assets/data/seasons.json`):

| Season | base_target_mult | Resulting base_target |
|--------|------------------|-----------------------|
| Spring | 1.00 | 1000 |
| Summer | 1.25 | 1250 |
| Autumn | 1.50 | 1500 |
| Winter | 2.00 | 2000 |

---

## Spring target table (current default run)

With Spring (`base_target = 1000`) and no boss-specific target modifier:

| Wing | Small (1.0x) | Big (1.5x) | Ordeal (2.0x) |
|------|--------------|------------|---------------|
| 1 | 1000 | 1500 | 2000 |
| 2 | 1618 | 2427 | 3236 |
| 3 | 2618 | 3927 | 5236 |
| 4 | 4236 | 6354 | 8472 |
| 5 | 6854 | 10281 | 13708 |
| 6 | 11090 | 16635 | 22180 |
| 7 | 17944 | 26916 | 35888 |

`FINAL_WING` is currently `7`.

---

## Boss effects that can modify target

Base Ordeal target is the 2.0x chamber multiplier above. Some bosses add extra math.

Current explicit target modifier:

- **Famine** (`ordeal::famine_apply`) doubles the already-computed Ordeal target:

```text
target = target * 2
```

So Famine is effectively `wing_base * 4.0` for that round.

---

## Where this runs in code

- Core math: `crates/mahjuro-core/src/core/chamber_target.rs`
- Run call site: `RunState::chamber_score_target` in `src/game/run.rs`
- Applied when entering a chamber: `RunState::apply_chamber` in `src/game/run/round_flow.rs`
- Season-adjusted base target setup: `src/game/game_mode.rs`

---

## One-line mental model

```text
target = round(round((1000 * season_mult) * φ^(wing-1)) * blind_mult), then boss hooks may modify it.
```
