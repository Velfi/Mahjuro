# Event-driven run mutations (subscribers beyond gold)

## Why
Yen changes now funnel through [`RunState::apply_yen_delta`](../../src/game/run.rs) / [`apply_yen_reward`](../../src/game/run.rs) / [`notify_run_yen_changed`](../../src/game/run.rs), which emit [`GameEvent::YenChanged`](../../src/game/event_bus.rs) and call [`relic_hooks_on_run_yen_changed`](../../src/game/run.rs) (Turtle Shell today). Other run mutations still scatter across call sites: each path must remember `note_relic_destroyed`, the right `GameEvent`, shop-pool extinction flags, and Steam/App reactions. That duplication is easy to get wrong (especially Kintsugi) and makes new relics expensive to wire.

## Scope
One umbrella refactor — pick pieces in order of leverage; none of this blocks shipping individual features.

1. **Relic destruction / permanent removal.** Replace ad-hoc `relics.active.retain(...)` + `note_relic_destroyed()` + occasional `bus.push` with a single `RunState::destroy_relic` (or equivalent) that owns removal, debuff/counter cleanup, `note_relic_destroyed`, and event emission (`RelicActivated`, `TransformationSuccessorDiscovered`, achievements where applicable). Migrate [`scoring_flow`](../../src/game/run/scoring_flow.rs), [`round_flow`](../../src/game/run/round_flow.rs), [`hand_ops`](../../src/game/run/hand_ops.rs), shop paths in [`engine`](../../src/game/engine.rs), and any other `retain` burn sites until grep for `note_relic_destroyed` outside the helper is gone or documented as intentional exceptions.

2. **Round resources (plays / discards).** If future relics or telemetry need “last play spent” or “discard granted gold” in one place, add `spend_play` / `spend_discard` / `grant_play` style helpers that optionally push events and call a small `relic_hooks_on_round_resources_changed`. Only worth doing once a second consumer exists; plays/discards are less fragmented than relic removal today.

3. **Tile supply / run-scoped wall.** If more systems need to react to permanent tile removal (Taotie, pack injection, future curses), consider a narrow internal event or single `apply_tile_supply_change` after the Taotie path in [`scoring_flow`](../../src/game/run/scoring_flow.rs) is joined by more callers.

4. **Round / ante boundaries.** [`advance_round`](../../src/game/run/round_flow.rs) and [`forfeit_current_blind_second_wind`](../../src/game/run/round_flow.rs) duplicate lantern-style hooks; a private “round boundary” dispatcher could reduce copy-paste. Lower priority than relic destruction — higher risk, less frequent new code.

Out of scope: changing `GameEvent` wire format for saves/replays, moving all Steam unlocks into gameplay (App may stay the drain site), rewriting the entire engine command layer.

## Touchpoints
- [`src/game/run.rs`](../../src/game/run.rs) — `note_relic_destroyed`, `relic_hooks_on_run_gold_changed`, gold helpers; natural home for `destroy_relic` and future `relic_hooks_on_*`.
- [`src/game/event_bus.rs`](../../src/game/event_bus.rs) — new variants only if something truly needs a distinct UI/audio channel; prefer reusing existing events where semantics already match.
- [`src/game/run/scoring_flow.rs`](../../src/game/run/scoring_flow.rs) — Melting Ice, XXXL Egg, Glass Cannon, Tea Ceremony / Chrysalis, Second Wind, Taotie: all mix relic removal with other logic.
- [`src/game/run/round_flow.rs`](../../src/game/run/round_flow.rs) — Paper Lantern, Silver Filigree Lantern, duplicated patterns in `advance_round` vs `forfeit_current_blind_second_wind`.
- [`src/game/run/hand_ops.rs`](../../src/game/run/hand_ops.rs) — Silk Thread burn path calls `note_relic_destroyed` directly.
- [`src/game/engine.rs`](../../src/game/engine.rs) — shop sells/removals that touch relics without going through run helpers.
- [`src/bot.rs`](../../src/bot.rs) — must keep calling the same entry points as the real engine so headless stats stay aligned.
- [`src/scenes/gameplay/animation_state.rs`](../../src/scenes/gameplay/animation_state.rs) — reference for “derive visuals from state delta” vs “push duplicate bus events” (gold flying coins no longer double-`GoldChanged`).

## Open questions
- **`Option<&mut EventBus>` vs throwaway bus.** Headless and bot already mix `None` and real buses; `destroy_relic` must accept optional bus and document when `None` is allowed without skipping Kintsugi (today Kintsugi is gameplay-only counters — safe without bus).
- **Transform in-slot vs remove.** Tea Ceremony / Chrysalis replace a slot rather than pure removal; either exclude them from `destroy_relic` with a separate `transform_relic` API, or generalize the helper with an enum `Removal | TransformTo(RelicId)`.
- **Order of hook vs event.** Gold pushes `GoldChanged` then runs relic hooks so listeners see final `gold`. Destruction might want `RelicActivated` after removal so UI doesn’t flash a stale slot — confirm ordering with whoever touches relic strip rendering.
