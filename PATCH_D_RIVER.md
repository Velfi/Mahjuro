# Patch D — River System

## Goal

Add a per-round **river** (the row of tiles a player has discarded) and a furiten-inspired rule that gives the river mechanical weight. Unblocks two relics that currently have no effect: **River Eraser** and **Furiten Ward**.

This is a self-contained gameplay subsystem: data + scoring hook + UI + relic wiring. It does not depend on Patch E.

## Open design decision (confirm before coding)

Real-mahjong furiten ("can't ron on a tile in your own river") has no analog in Mahjuro because there's no opponent and no ron — you score against a chip target. The relic descriptions imply furiten matters, so we need to invent a meaning. The recommended default is option **(c)**, but the implementing agent should confirm with the user before writing scoring code:

- **(a) Cosmetic only.** River is purely visual. RiverEraser/FuritenWard remain decorative. *Reject — defeats the point of the patch.*
- **(b) Tile-level penalty.** Any hand tile whose `(suit, rank)` also appears in the river scores **0 base chips** when played. Harsh and pure-mahjong-faithful.
- **(c) ★ Recommended: yaku invalidator.** When scoring a meld (sequence/triplet/kong/pair), if **any** tile in the meld matches a `(suit, rank)` in the river, that meld is "furiten-tainted":
  - The meld still contributes **base chips** (so it's not a wasted play).
  - The meld does **not** count toward yaku detection (the yaku step skips it).
  - It does not count toward FullHand detection either.
  - Rationale: punishes sloppy discarding without making the player feel they bricked a hand. Creates a real reason to want RiverEraser ("undo my mistake") and FuritenWard ("limit my exposure window").
- **(d) Passive chip drain.** -5 chips per river tile during scoring. Simple, but feels like a punishment without a learnable rule.

The rest of this doc assumes **(c)**. If the user picks a different option, only the scoring-hook section changes — the data model and UI sections are identical.

## Data model

### `RunState` additions ([src/game/run.rs](src/game/run.rs))

Add a `River` field on `RunState`:

```rust
/// Tiles the player has discarded this round, in discard order.
/// Powers the furiten rule (Patch D): scored melds containing a tile
/// whose (suit, rank) also appears in the river still award base chips
/// but do not count toward yaku detection. Reset on round start.
#[serde(default)]
pub river: Vec<Tile>,
```

`#[serde(default)]` keeps old saves loadable. The river is per-round state, not run-wide — so it goes alongside `played_yaku_this_round` conceptually.

Initialize to `Vec::new()` in:
- `RunState::new()` — the main constructor near line 159.
- The test constructor `test_run()` — near line 845.

Reset to empty in:
- `advance_round()` — alongside the existing `played_yaku_this_round.clear()` line.
- `skip_to_next_blind()` — same place.

### Discard hook ([src/game/run.rs:665](src/game/run.rs#L665))

In `discard_selected_no_refill()`, push each removed tile into the river **before** removing it from the hand. The current loop is:

```rust
for &i in &indices {
    self.hand.remove(i);
    bus.push(GameEvent::TileDiscarded { slot_index: i });
}
```

Change to:

```rust
for &i in &indices {
    let tile = self.hand.remove(i);
    self.river.push(tile);
    bus.push(GameEvent::TileDiscarded { slot_index: i });
}
```

Apply the **Furiten Ward cap** here too: after pushing, if `self.relics.has(RelicId::FuritenWard)`, retain only the last 6 tiles via `if self.river.len() > 6 { let drain = self.river.len() - 6; self.river.drain(0..drain); }`. (Without the relic the river is uncapped within a round.)

## Scoring hook (option (c))

This is the only file outside `run.rs` that needs scoring logic changes: [src/core/scoring.rs](src/core/scoring.rs) — specifically `score_sets()` and the yaku detection path it calls into.

1. **Plumb the river through `ScoreContext`.** Add a field:
   ```rust
   /// (suit, rank) pairs currently in the player's river. Melds whose
   /// tiles match any entry are "furiten-tainted" — they award base chips
   /// but skip yaku detection.
   pub river_faces: Vec<(Suit, u8)>,
   ```
   `RunState::score_selected_tiles` constructs the `ScoreContext`; populate `river_faces` from `self.river.iter().map(|t| (t.suit, t.rank)).collect()`. Find the call site by searching for `ScoreContext {` in `run.rs`.

2. **Tag tainted melds.** In `score_sets`, when iterating sets, compute:
   ```rust
   let tainted = set.tile_ids.iter()
       .filter_map(|id| selected_tiles.iter().find(|t| t.id == *id))
       .any(|t| ctx.river_faces.contains(&(t.suit, t.rank)));
   ```
   - Award base chips for the meld regardless of `tainted`.
   - **Skip the yaku-detection step for that meld** if `tainted`.
   - Skip the meld when computing whether the play is a FullHand.

3. **Bot accounting.** [src/bot.rs](src/bot.rs) constructs its own `ScoreContext` for evaluating moves. Add the same `river_faces` plumbing. Default to empty in tests.

4. **Tests.** Add unit tests in `core::scoring::tests`:
   - A meld with no taint scores yaku normally.
   - A meld with one tainted tile scores base chips but no yaku.
   - A FullHand containing one tainted meld is *not* detected as FullHand.
   - A river with `FuritenWard` does not exceed 6 entries.
   - `RiverEraser` removes 3 tiles when invoked (covered in the relic section below).

## UI

### Visual river on the gameplay table ([src/scenes/gameplay.rs](src/scenes/gameplay.rs))

The river should sit between the hand row and the back of the table — a left-to-right row of small tile meshes growing as the player discards. Reuse the existing tile mesh pipeline.

Concrete steps:

1. **Layout.** Add a `river_row_anchor_px: (f32, f32, f32)` and `river_tile_extents: [f32; 3]` to whatever the gameplay layout struct is in [src/scenes/gameplay.rs](src/scenes/gameplay.rs). Position it behind the hand row, in front of the action buttons. Use `layout.mm(...)` for tile dimensions consistent with the rest of the scene (real tiles are ~26mm × 19mm × 16mm).
2. **Draw.** In the gameplay draw pass, iterate `ctx.run.river` and emit one tile mesh per entry, laid out left-to-right with a small horizontal stride. Wrap to a second row if the river exceeds ~12 tiles. Use `MAX_RIVER_TILES = 24` as a hard cap on rendered tiles to bound the draw budget; older entries get culled visually but remain in the data model (FuritenWard caps the data side at 6 anyway).
3. **Subtle visual tag for furiten matches.** In the *hand row*, dim the tint of any tile whose `(suit, rank)` is in `ctx.run.river` so the player can see which tiles are tainted **before** committing them. A simple ~30% darken on the per-instance color uniform is enough.
4. **Tooltip.** When the player hovers a hand tile that would be tainted, the existing tooltip system should show "Furiten — won't count toward yaku". Find the tooltip path via `tooltip` or `Tooltip` in the gameplay scene.

### River Eraser activation UI

River Eraser is "once per round, clear 3 tiles from the river". Needs:
- A clickable affordance — the simplest path is to make the river itself clickable when the relic is owned and the per-round charge is unused. Render a small "↺" icon over the river anchor when available; click sends a `UiAction::UseRiverEraser` (new variant).
- Per-round charge tracking: add `river_eraser_used: bool` to `RunState`, reset in `advance_round`/`skip_to_next_blind`.
- Handler in `gameplay.rs`'s `update` calls a new `RunState::use_river_eraser()` that pops the 3 oldest tiles (`self.river.drain(0..3.min(self.river.len()))`) and sets the flag.

### HUD counter (optional polish)

The existing peg/HUD shows hands and discards remaining. Adding a small "river: N" counter near the discard counter is a nice-to-have, not required.

## Relic wiring

These all live in [src/core/relic.rs](src/core/relic.rs) for the data and [src/game/run.rs](src/game/run.rs) for the runtime:

1. **River Eraser** ([relic.rs:42-44](src/core/relic.rs#L42-L44)) — remove the "no-op until Patch D" comment; the new `RunState::use_river_eraser` path implements it.
2. **Furiten Ward** ([relic.rs:45-47](src/core/relic.rs#L45-L47)) — remove the "no-op until Patch D" comment; the discard hook above implements the cap.
3. **Re-add to shop pool.** Earlier balance work suggested filtering both relics out of shop offerings while their systems were dead. Search the shop relic-offer generation in [src/scenes/shop.rs](src/scenes/shop.rs) for any "exclude" / dead-relic filter and re-include them. If no such filter exists yet, the relics are already eligible — nothing to do here.

## Save compatibility

The new `river` field has `#[serde(default)]`, so loads of pre-Patch-D saves get an empty river. The new `river_eraser_used` flag should also have `#[serde(default)]`. No migration code required.

## Bot updates ([src/bot.rs](src/bot.rs))

The bot's hand-evaluation logic needs to know about furiten so it doesn't repeatedly discard tiles it then needs to score. Suggested heuristic, in priority order:

1. **Avoid discarding tiles you'll want to score.** When picking discards, prefer tiles whose `(suit, rank)` does *not* appear in any candidate winning hand for the next 1-2 plays.
2. **Account for taint when scoring.** When computing the value of a candidate play, taint melds whose tiles match the current river — same logic as the real scoring path.

This is enough to keep the bot rational. Don't over-engineer it; the bot is for tuning, not for being optimal.

## Testing & validation

1. `cargo test` — all 121+ existing tests must still pass. Add the new tests listed in the scoring-hook section.
2. **Headless bot sweep.** Run `cargo run --release -- --bot 200` before and after the patch and compare:
   - Win rate should not crash (acceptable: ±5% from current 9.5%).
   - Average relics bought should stay near current 3.75.
   - The two new relics should appear as purchases in some runs (verify by adding a temporary print, then remove).
3. **Manual smoke test.** Play one full round, discard a tile, try to score a meld containing that tile's face — verify it loses the yaku but keeps base chips.

## File checklist

- [src/game/run.rs](src/game/run.rs) — `river` field, init, reset, discard hook, `use_river_eraser`, `river_eraser_used` field.
- [src/core/scoring.rs](src/core/scoring.rs) — `ScoreContext::river_faces`, taint logic in `score_sets`.
- [src/core/relic.rs](src/core/relic.rs) — comment cleanup on River Eraser / Furiten Ward.
- [src/scenes/gameplay.rs](src/scenes/gameplay.rs) — layout, draw, hand-tile dimming, River Eraser click target, optional HUD counter.
- [src/bot.rs](src/bot.rs) — `river_faces` plumbing, taint-aware discard heuristic.
- [src/scenes/shop.rs](src/scenes/shop.rs) — verify the two relics are in the shop pool.
- New tests in `core::scoring::tests` and `game::run::tests`.

## Out of scope for Patch D

- Riichi declaration (Patch E).
- Any "winning tile" detection beyond the existing FullHand path.
- Multi-river layouts for non-implemented seat winds.
- New art for the river area — reuse the existing tile mesh.
