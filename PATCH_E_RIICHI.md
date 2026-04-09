# Patch E — Riichi Declaration

## Goal

Add a **riichi declaration** mechanic: when the player is one tile from a complete hand, they can declare riichi to lock in a high-risk / high-reward state. Unblocks the **Riichi Stick** relic and gives the existing scoring hook (`ScoreContext::riichi_active`) a real input.

This patch can land independently of Patch D, but if both ship together, riichi-declared hands should also respect the furiten rule (a riichi-declared hand whose winning meld is tainted still scores base chips and still pays the failure penalty if the round ends without a clear).

## Open design decisions (confirm before coding)

The relic description is intentionally vague:
> *"First riichi each round is free; failed riichi floors at 80% target instead of 60%."*

Two systems are referenced that don't exist in the code today: **the cost of declaring riichi** and **a target floor on round failure**. The implementing agent must lock both before writing code.

### Decision 1 — Cost of declaring riichi

Pick one (or propose your own). Recommended default in **bold**:

- (a) Costs 1 discard. Locked-in hand = no more discards. Cheap mechanically; matches the "discards are your action budget" framing.
- (b) Costs 1 play. Burns the play that triggered the declaration. Heavy.
- (c) **★ Locks discards but doesn't consume one.** Declaration freezes the hand: no more discards allowed for the rest of the round, but the declaration itself is free. Riichi Stick's "free" clause becomes something else (see Decision 4).
- (d) Costs gold (e.g. $5). Doesn't fit a roguelite economy where gold is for the shop.

### Decision 2 — Reward for succeeding

Default and recommended: **just the existing 2× mult** on the closing FullHand, which is already wired in the scoring path via `ScoreContext::riichi_active` (currently always false). The implementing agent should:
1. Verify the 2× mult hook actually fires when `riichi_active = true`.
2. Confirm the user wants this as the *entire* reward, or if they want to add a flat chip bonus / gold reward on top.

If the user wants more, simplest add-ons:
- +50 chips one-shot bonus on the closing play (additive, before mult).
- +$3 gold on the next round-clear payout.

### Decision 3 — Failure semantics & target floor

The relic implies a "60% target floor" exists by default and the relic raises it to 80%. **No such floor exists today** — missing target = game over. Two paths:

- (A) **★ Add the floor only for declared-riichi rounds.** If the player declared riichi and *did not* clear the target, but their final score ≥ 60% of the target (or 80% with Riichi Stick), the round is treated as a soft pass: no shop, no gold, but the run continues to the next blind. Self-contained, no general partial-credit system needed.
- (B) Add a general partial-credit floor (always 60%, riichi raises to 80%) — bigger design change, leaks into non-riichi rounds.

(A) is the recommended path. The rest of this doc assumes (A).

A "failed riichi" specifically means: the player declared riichi, played out the round, and either ran out of plays or scored < target on their final play. The check happens at game-over time.

### Decision 4 — Riichi Stick's "free" clause

Under Decision 1(c), declaration is already free. Suggested re-interpretations:
- **★ Stick raises the failure floor from 60% to 80%.** (The relic's second clause is the *only* clause; the "free" line just becomes flavor.)
- Stick allows declaring riichi *twice* per round (first declaration replaces the locked hand wholesale).

Recommended: keep both clauses by making "free" mean "doesn't burn the per-round declaration charge" — i.e. without the relic, the player can declare at most once per round; with the relic, they can re-declare. Combined with the floor change, the relic gives both insurance and flexibility.

## Data model

### `RunState` additions ([src/game/run.rs](src/game/run.rs))

```rust
/// True after the player declares riichi this round. Locks discards
/// (no further `discard_selected` allowed) and sets the scoring path's
/// `riichi_active` flag, which applies a 2× mult to a closing FullHand.
/// Reset on round start.
#[serde(default)]
pub riichi_declared: bool,
/// True if the player has already used their per-round riichi declaration
/// charge. Without Riichi Stick the player gets one declaration per round;
/// with the relic they can re-declare. Reset on round start.
#[serde(default)]
pub riichi_declaration_used: bool,
```

Initialize both to `false` in `RunState::new()` and the test constructor.

Reset to `false` in `advance_round()` and `skip_to_next_blind()` alongside the existing per-round flags.

### Tenpai detection

Already exists: [src/core/shanten.rs:58](src/core/shanten.rs#L58)
```rust
pub fn is_tenpai(tiles: &[Tile]) -> bool { ... }
```

Use this directly. Riichi declaration is only allowed when `shanten::is_tenpai(&run.hand)` returns true.

### Declaration entry point

Add to `RunState`:

```rust
/// Attempt to declare riichi. Returns true on success. Fails if:
/// - hand is not tenpai,
/// - declaration charge already spent and Riichi Stick not owned,
/// - already declared this round.
pub fn declare_riichi(&mut self, bus: &mut EventBus) -> bool {
    if self.riichi_declared {
        return false;
    }
    if self.riichi_declaration_used && !self.relics.has(RelicId::RiichiStick) {
        return false;
    }
    if !crate::core::shanten::is_tenpai(&self.hand) {
        return false;
    }
    self.riichi_declared = true;
    self.riichi_declaration_used = true;
    bus.push(GameEvent::RiichiDeclared);
    true
}
```

Add the new `GameEvent::RiichiDeclared` variant to [src/game/event_bus.rs](src/game/event_bus.rs).

### Discard lockout

In `discard_selected_no_refill()` ([src/game/run.rs:665](src/game/run.rs#L665)), early-return 0 if `self.riichi_declared`. Same for the variants if any.

## Scoring hook

The hook already exists. Find the `ScoreContext` construction in `RunState::score_selected_tiles` and set:

```rust
riichi_active: self.riichi_declared,
```

(Currently it's hardcoded to `false`.)

**Verify** that the 2× mult is actually applied somewhere downstream. Search [src/core/scoring.rs](src/core/scoring.rs) for `riichi_active`. If the field is read and a multiplier is applied, the patch is done on the scoring side. If the field is read but the mult is missing, add the multiplication step in the scoring cascade. If the field isn't read at all, wire it in: at the FullHand detection step, multiply final mult by 2.0 when `ctx.riichi_active`.

## Failure floor (Decision 3 path A)

In the round-end / game-over path of [src/game/run.rs](src/game/run.rs), find where `GameEvent::GameOver` is emitted (currently around line 540). Wrap it:

```rust
} else if self.plays_remaining == 0 {
    if self.riichi_declared && self.round_score >= self.failure_floor_score() {
        // Soft pass: skip the shop and the gold reward, advance to the
        // next blind anyway. The player gets nothing but a second chance.
        bus.push(GameEvent::RoundComplete { reached_target: false });
    } else {
        bus.push(GameEvent::GameOver { final_score: self.round_score });
    }
}
```

with helper:

```rust
/// Score the player must reach to "soft-pass" a failed riichi round.
/// Default 60% of target; Riichi Stick raises it to 80%.
fn failure_floor_score(&self) -> u32 {
    let pct = if self.relics.has(RelicId::RiichiStick) { 0.80 } else { 0.60 };
    (self.target_score as f32 * pct) as u32
}
```

The downstream scene flow needs to handle `RoundComplete { reached_target: false }` differently from the success case:
- **Success path** (`reached_target: true`) goes to the shop.
- **Soft-pass path** (`reached_target: false`) skips the shop and rolls directly into the next `PickBlind`. No gold reward, no relic offer. Find the existing `RoundComplete` handler in [src/scenes/gameplay.rs](src/scenes/gameplay.rs) and branch on the new field.

The `GameEvent::RoundComplete` variant currently has a `reached_target: bool` field — verify that's actually wired through to scene transitions. If scenes only branch on the event being present (not the field), add the branching now.

## UI

### Riichi declaration button ([src/scenes/gameplay.rs](src/scenes/gameplay.rs))

Add a third action button alongside Play and Discard. The existing `GameplayButton` enum is at [src/scenes/gameplay.rs:63](src/scenes/gameplay.rs#L63):

```rust
enum GameplayButton {
    Play,
    Discard,
    Riichi,
}
```

The button should:
- Render only when the relevant relic isn't required *or* when `is_tenpai(&run.hand)` returns true. Always show it; gray it out when not declarable. Reason: discoverability.
- Show as a small wooden plaque (matches the existing button style) with a red ribbon tied around it (signature riichi-stick visual).
- Clicking emits `UiAction::DeclareRiichi` (new variant in [src/ui/input.rs](src/ui/input.rs) or wherever `UiAction` lives).
- Handler in `update()` calls `ctx.run.declare_riichi(ctx.bus)`. On success, the button locks and the discard button grays out.

### Visual state when declared

- Hand row: rotate the locked hand 90° or add a red border tint so the player sees the lock.
- Discard bowl: dim it.
- Add a small "RIICHI" text label or red ribbon on the table when active.
- Tenpai hint: when not yet declared, show a small dot near the riichi button if `is_tenpai` (telegraphs that declaration is available).

### HUD line

Add a "Riichi declared" status line to the round HUD when active. Find the existing HUD format string in [src/scenes/gameplay.rs](src/scenes/gameplay.rs) (search for `"Mahjuro —"` or similar — line 1095 in current state).

## Relic wiring

[src/core/relic.rs](src/core/relic.rs) — **Riichi Stick**:

1. Remove the "no-op until Patch E" comment ([relic.rs:36-39](src/core/relic.rs#L36-L39)).
2. Update the `description` to match the locked semantics. Recommended new copy:
   ```
   "Re-declare riichi each round; failed riichi soft-passes at 80% target"
   ```
   (Old copy mentioned "free first riichi"; the new model has free declarations by default — see Decision 4.)
3. Re-add to the shop pool if it was filtered out during the dead-relic culling.

## Save compatibility

Both new fields use `#[serde(default)]`. Old saves load with `riichi_declared = false`, `riichi_declaration_used = false`. No migration needed.

## Bot updates ([src/bot.rs](src/bot.rs))

The bot needs a riichi policy. Suggested simple heuristic:

1. Each play, check `is_tenpai(&run.hand)`. If true, and the bot has not yet declared this round, and the bot's evaluator estimates the next play has ≥ 70% chance of completing a FullHand, declare riichi.
2. Otherwise don't declare. The bot is allowed to be conservative — we just want it to *exercise* the system so the headless sweeps cover the riichi code path.

Add a metric to the bot stats output: `avg riichi declarations / run` and `riichi success rate (cleared blinds with riichi active)`.

## Testing & validation

Required tests in `game::run::tests`:

- `declare_riichi_fails_when_not_tenpai`
- `declare_riichi_succeeds_when_tenpai`
- `declare_riichi_locks_discard` — `discard_selected` returns 0 after declaration.
- `declare_riichi_blocked_after_first_without_relic`
- `declare_riichi_allowed_twice_with_riichi_stick`
- `riichi_failure_soft_passes_at_60_percent`
- `riichi_failure_game_overs_below_60_percent`
- `riichi_stick_raises_floor_to_80_percent`
- `riichi_active_doubles_full_hand_mult` (in `core::scoring::tests`)

Then `cargo test` and `cargo run --release -- --bot 200`. Compare bot stats:
- Win rate should *increase* slightly (riichi gives a real reward).
- New `avg riichi declarations / run` metric should be > 0.
- Death-by-blind distribution should slightly favor "soft pass" rounds for declared-riichi runs.

## File checklist

- [src/game/run.rs](src/game/run.rs) — fields, init, reset, `declare_riichi`, discard lockout, scoring-context plumbing, failure-floor branch, `failure_floor_score` helper.
- [src/core/scoring.rs](src/core/scoring.rs) — verify/wire the 2× mult on `riichi_active` if missing.
- [src/game/event_bus.rs](src/game/event_bus.rs) — `GameEvent::RiichiDeclared`.
- [src/core/relic.rs](src/core/relic.rs) — Riichi Stick comment/description cleanup.
- [src/scenes/gameplay.rs](src/scenes/gameplay.rs) — Riichi button, declaration UI, locked-hand visuals, HUD line, RoundComplete branching.
- [src/ui/input.rs](src/ui/input.rs) (or wherever) — `UiAction::DeclareRiichi`.
- [src/bot.rs](src/bot.rs) — riichi policy + new metrics.
- [src/scenes/shop.rs](src/scenes/shop.rs) — verify Riichi Stick is in the shop pool.
- New tests in `game::run::tests` and `core::scoring::tests`.

## Out of scope for Patch E

- Ippatsu, ura-dora, double-riichi, and other riichi variants.
- Yakuhai-tied riichi bonuses.
- Furiten enforcement during riichi (covered by Patch D if it's also live).
- Animated ribbon-tying or other riichi-specific juice — the red ribbon tint is enough for v1.
