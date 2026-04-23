# Mahjuro Design Expansion — Implementation Prompts

Self-contained prompts for the five highest-impact design gaps identified in the
2026-04-22 audit. Each is written so it can be pasted into a fresh Claude Code
session without carrying this conversation's context. Ordered by recommended
build sequence (stakes first — biggest lift-to-effort ratio).

---

## 1. Stakes (difficulty tiers)

**Goal:** Add a Balatro-style "stake" axis so a player who has beaten the
game has new goalposts. Single `GameMode::standard()` today means there's
nothing to do at max level.

**Scope:**
- Introduce a `Stake` enum in `src/core/progression.rs` or a new
  `src/core/stake.rs`. Start with 4 tiers named after seasons:
  `Spring` (baseline — what the game is today), `Summer`, `Autumn`, `Winter`.
  Implement `Default` → `Spring` so existing saves and test fixtures keep
  working when the field is added to `GameMode` / `RunRecord`.
- Each stake is a small bundle of modifiers, not a new rules system.
  Deltas to parameterise on `GameMode`:
  - **Base-target multiplier** (Spring 1.0, Summer 1.15, Autumn 1.3, Winter 1.5).
    Apply once at `RunState::new` by scaling `mode.base_target` (and
    `state.base_target`). Do **not** reuse `mode.target_scaling` — today
    that field is only read by the Smoke Bomb relic branch in
    `engine.rs`, not by `apply_blind`'s per-blind target formula
    (`base_target * run_number`). Scaling `base_target` at run start
    composes cleanly with that linear formula.
  - Shop price multiplier (Spring 1.0, Summer 1.0, Autumn 1.25, Winter 1.5).
    This is the largest plumbing cost in the feature: today
    `relic_buy_price`, `ZodiacKind::shop_price()`, `TalismanKind::shop_price()`,
    and `BoosterPackKind::shop_price()` are all free functions / const
    methods with no `&GameMode` access. Either (a) add a
    `price_multiplier: f32` on `GameMode` and multiply at call sites in
    `scenes/shop.rs` and `core/relic.rs::relic_shop_price` (which already
    takes `&RelicState` and can grow a mode parameter), or (b) route all
    shop-side pricing through a single `shop_price(&self, mode: &GameMode)`
    helper. Prefer (a) — smaller diff.
  - Reroll base cost: replace `const REROLL_BASE_COST: u32 = 5` in
    `scenes/shop.rs:842` with a lookup from the stake
    (Spring 5, Summer 5, Autumn 6, Winter 7).
  - Boss `min_ante` floor — higher stakes reduce the effective `min_ante`
    used by the filter in `core/boss.rs::pick_for_ante` (currently
    `def().min_ante <= ante`), letting harder bosses appear earlier.
    Suggested: Spring 0, Summer 0, Autumn −1, Winter −2.
  - Optional per-stake `RuleModifier` push (e.g. Winter: `NoSequenceBonus`
    also applied run-wide). Reuse the existing `RuleModifier` enum —
    don't invent a parallel rule system.
- Unlock gate in `PlayerProgress`: Spring always available, each higher
  stake unlocks by beating the previous one on any deck. Track in a new
  `unlocked_stakes: HashSet<Stake>`.
- Wire into run creation: `RunState::new` already takes a `GameMode`; add
  a `stake: Stake` field to `GameMode` and thread the modifiers through
  base-target scaling, shop pricing, reroll cost, and boss pool filtering.
- Persist the stake used for each run: add `#[serde(default)] pub stake: Stake`
  to `RunRecord` in `core/progression.rs:86` and populate it in
  `RunRecord::from_run`. The Collection screen can then show "beat Summer
  stake on Bamboo" etc.
- Bot harness: add a `--stake` flag to the CLI in `src/main/cli.rs` and
  thread it through `bot/reporting.rs` so balance snapshots can compare
  stakes.

**UI:**
- `src/scenes/start_game_modal.rs` gains a stake picker (locked tiers show
  the unlock hint). One row per stake, matches how the tile material picker
  works today — don't build a separate modal.
- Blind picker / HUD shows the current stake as a small badge; use an
  existing font/atlas.

**Non-goals for this pass:**
- No per-stake relic pools, no per-stake yaku restrictions, no challenge
  modifiers beyond the 4 numeric knobs above.
- No stake-specific achievements — unlocking the next tier is the reward.
- Tutorials stay on Spring regardless of the active stake (the tutorial
  path in `run.rs:923` already force-sets targets).

**Definition of done:**
- `cargo check` and `cargo test` pass.
- Launching a run with `Stake::Winter` visibly has ~1.5× targets and higher
  shop prices. Beating ante 7 on Spring unlocks Summer in `PlayerProgress`.
- Collection or profile screen surfaces "highest stake cleared" per material.

---

## 2. Booster packs in shop

**Goal:** Bring the "open pack → pick 1 of N" ritual that Balatro leans on.
The assets (`assets/textures/packs/`, pack cover art from
`scripts/generate_pack_covers.py`) and the `src/core/tile_pack.rs` module
exist but the pick-1-of-N UI is only wired for the Festival zodiac pack.

**Scope:**
- Generalise the pick-1-of-N pattern already used for Festival packs in
  `src/scenes/shop.rs` (see `is_tile_pack_pick` / `ShopHit::Dish`). Extract
  a shared "booster" flow that any pack kind can feed into — don't build a
  new second system.
- Define `BoosterPack` variants (in `src/core/consumable.rs` or a new
  `src/core/booster.rs`):
  - `CelestialPack` — 3 random zodiac cards, pick 1 (this is the current
    Festival pack; rename or alias).
  - `TalismanPack` — 3 random talismans, pick 1.
  - `RelicPack` — 2 random relics, pick 1 (rare; higher price).
  - `MegaPack` variants — larger options, pick 2. Mirror Balatro's pack
    size/rarity escalation but keep it to ~6 SKUs total for now.
- Each pack has: content generator (RNG-seeded off `RunState::rng`), cost,
  rarity weight for shop stock, and cover-art asset key. Reuse the existing
  `pack_covers` textures — don't regenerate.
- Shop stock rebuild in `src/scenes/shop.rs` gains a packs row alongside
  the existing relic / consumable / tile-pack rows.
- Picking a pack opens a modal (or in-place expansion — whichever matches
  Festival's current behaviour) showing the N options. Unpicked items
  discard. Skipping returns gold? Don't add refunds — match Balatro (pack
  gold is spent on reveal).

**Non-goals:**
- No pack-opening animation polish beyond what Festival already has.
- No pack-cover-art editor. Use existing generated art.
- Don't touch the `TilePack` wall-enlargement system — that's a separate
  utility and stays as-is.

**Definition of done:**
- Shop has a visible packs row on every reroll.
- Buying a `CelestialPack` shows 3 zodiacs, clicking one adds it to
  consumables, others discard. Identical flow for `TalismanPack`,
  `RelicPack`.
- Cover art renders for each pack kind.
- `cargo test` passes; add a unit test for the pack content generator
  (seeded RNG → deterministic offering).

---

## 3. Promote tile materials → decks

**Goal:** Today `TileMaterial` (Bamboo / Plastic / TortoiseShell in
`src/persistence` / threaded through `GameMode::with_material`) is three
near-identical passives (+1 play / +1 discard / +10 gold). That's a
*tile skin with a number bolted on*, not a deck. Real Balatro decks
change the *opening situation* — starting relic, different hand/discard
counts, altered rules.

**Scope:**
- Rename or retain `TileMaterial` as the visual skin, and introduce a
  parallel `Deck` concept in `src/core/deck.rs` (the file exists but is
  currently wall-building). Consider a new `src/game/deck_profile.rs` if
  `deck.rs` is too dense — don't muddle wall logic with run-config logic.
- Each deck specifies starting state applied in `RunState::new`:
  - Base plays / discards
  - Starting gold
  - Starting relic (optional)
  - Starting consumable (optional)
  - Hand size delta
  - Run-wide `RuleModifier` push
  - Shop stock bias (e.g. +1 relic slot, or zodiacs cost less)
- Ship ~6 decks to start. Examples:
  - **Bamboo Deck** (default) — current behaviour, no modifiers.
  - **Jade Deck** — start with `JadeSerpent` relic; +1 discard; Bamboo
    suit starts slightly boosted.
  - **Iron Deck** — start with `MeltingIce` relic and 2 extra plays, but
    -1 discard and shop prices +25%.
  - **Scholar's Deck** — start with a random consumable every round;
    relics in shop cost +1.
  - **Chaos Deck** — start with `CrackedTile`; boss min_ante is -1 (hard
    bosses appear earlier).
  - **Pure Deck** — start with `WayOfPurity`; cannot buy honor-tile
    relics.
- Unlock gates: Bamboo always available. Each other deck unlocks by a
  clear condition tracked in `PlayerProgress` (e.g. Jade — score 10 bamboo
  triplets in a run; Iron — win without selling a relic). Keep conditions
  legible and track them as counters in `PlayerProgress`.
- `src/scenes/start_game_modal.rs` gains a deck picker. If Stakes (item 1)
  has landed, both stake and deck are picked here.
- Collection scene (`src/scenes/collection.rs`) shows locked decks with
  their unlock hints.

**Non-goals:**
- Don't build more than 6 decks in this pass.
- Don't gate *relics* per-deck (except the `Pure Deck` honor-ban case —
  that's a clean, well-contained exception).

**Definition of done:**
- `RunState::new(GameMode { deck: Deck::Iron, .. })` produces a run that
  starts with Melting Ice in the relic inventory, +2 plays, -1 discard.
- Each deck has an unlock predicate that runs at end-of-run in
  `PlayerProgress::record_run`.
- Starting-screen deck picker shows 6 decks with locked ones greyed.
- Tests: one per deck verifying starting state matches spec.

---

## 4. Vouchers

**Goal:** Persistent-within-run shop upgrades. Cheap to build, huge economy
lever. Balatro's vouchers are one of the most replayable systems in the game
because they compound with deck/stake choice.

**Scope:**
- New `src/core/voucher.rs`:
  - `Voucher` enum with ~10 variants to start. Examples:
    - `Overstock` — shop shows +1 relic slot.
    - `ClearanceSale` — shop items cost 25% less (stacks multiplicatively
      with Merchant's Eye? Cap the stack).
    - `RerollSurplus` — rerolls cost -1 gold (min 1).
    - `Wasteful` — +1 discard per round.
    - `Blank` — does nothing (cheap filler, canonical Balatro gag).
    - `Telescope` — boss blind preview shows one turn earlier.
    - `Director's Cut` — can reroll the boss blind once per ante for $10.
    - `Seed Money` — max interest cap +$1.
    - `Tarot Merchant` — Celestial/Talisman packs 2× more likely in shop.
    - `Hieroglyph` — -1 ante at start but -1 hand size. (Tradeoff voucher.)
  - `VoucherState` on `RunState`: `active: Vec<Voucher>`.
- Shop stock rebuild adds a voucher slot (1 per ante, Balatro-style — not
  every shop). Price floor $10, bumped by stake.
- Each voucher has a `fn apply(&mut RunState)` hook called when purchased,
  plus query helpers:
  - `shop_extra_relic_slots(&VoucherState) -> usize`
  - `shop_price_modifier(&VoucherState) -> f32`
  - `reroll_cost_delta(&VoucherState) -> i32`
  - These read cleaner than sprinkling `voucher_state.has(X)` checks across
    the shop code.
- Persistence: vouchers are per-run, cleared on run end. No meta-unlock.

**Non-goals:**
- Don't tie vouchers to stakes (don't introduce "stake X locks voucher Y").
- Don't build voucher-specific art beyond a placeholder icon this pass.

**Definition of done:**
- Starting ante 2 shop shows 1 voucher for sale.
- Buying `Overstock` increases relic slots in subsequent shops from 4 to 5.
- Buying `ClearanceSale` drops visible prices by 25%.
- `cargo test` includes a voucher-effect test per variant.

---

## 5. Seeds & daily run

**Goal:** Expose the RNG seed so runs can be shared, replayed, and a daily
challenge can exist. Almost purely additive — the determinism is already
there (RunState reveals use seeded RNG per the boss_reveal hooks).

**Scope:**
- Find the current RNG seeding path. Likely `RunState::new` takes or
  derives a seed; surface it as `RunState::seed: u64` explicitly, stored
  for display.
- Seed encoding: 8-char base32 string (e.g. `KZ4M7QR2`). Round-trip
  encode/decode in `src/core/seed.rs`.
- `src/scenes/start_game_modal.rs`: add a "Custom Seed" input field next
  to deck/stake picker. Blank → random seed (current behaviour).
- Display active seed somewhere unobtrusive in-run (pause menu is the
  right home — don't clutter the HUD).
- Daily run:
  - Seed = hash of today's UTC date (e.g. `2026-04-22` → deterministic
    seed). One attempt per day, tracked in `PlayerProgress::daily_runs:
    HashMap<NaiveDate, DailyRunResult>`.
  - Fixed deck + stake for the daily (e.g. Bamboo + Gold).
  - Result posted to `PlayerProgress` — score, win/loss, seed string.
  - No online leaderboard in this pass (out of scope). Just local record.
- Start screen gains a "Daily Run" button separate from "New Run" if one
  hasn't been played today.

**Non-goals:**
- No online leaderboard / submission.
- No run replay (seed lets you re-run, not step-through).
- Don't force seed input on non-daily runs — it's optional.

**Definition of done:**
- Starting a run with seed `KZ4M7QR2` twice produces identical boss
  choices, shop rolls, and draws.
- Pause menu shows current seed, copy-to-clipboard.
- Daily Run button greys out after first attempt, shows that day's result.
- `cargo test` includes round-trip encode/decode and a
  "same seed → same first wall order" test.

---

## Build sequence rationale

- **Stakes** first because it's the smallest lift (numeric modifiers on
  existing knobs) and unlocks the most retention (something to do after
  level 7).
- **Boosters** second because the pattern already exists for Festival
  packs — generalising is lower risk than the other items.
- **Decks** third — bigger surface than stakes but reuses stake-era
  infrastructure (`GameMode` is already the right carrier).
- **Vouchers** fourth — depends on shop architecture being stable, which
  packs work might touch.
- **Seeds** last — purely additive, no coupling to the above, can slip
  or ship independently.

---

## 6. Reliquary — structure bank visual overhaul (in progress)

**Goal:** Give the structure bank a tangible presence. Today committed
melds render as floating Z-lifted tiles above the felt; the plan is to
slip a lacquered tray *underneath* them, add brass frames around each
meld group, and drive a rim-glow pulse off the depth multiplier.

**Status:** Foundation landed. `src/render/reliquary_tray_mesh.rs` has a
procedural tray mesh (rounded rectangle, lacquered body, inset inner
face) modeled on `mirror_mesh.rs`. The mesh is built but not yet
instantiated — the next session picks up from instantiation.

**Next session tasks (in order):**

1. **Wire the tray into gameplay rendering.** In
   `src/scenes/gameplay.rs` around the structure showcase block
   (line ~3030), emit a `ShowcaseObject3d` or equivalent whose bounds
   span the meld strip (`container_x + pad` → `container_x + container_w
   - pad - preview_lane_w`, `meld_top`, width and a fixed depth). Use a
   Z that sits **below** the existing meld tiles (they live at
   `3.0 + lift`; the tray should be around `0.0..2.0`). Cast a small
   shadow so the tray reads as raised off the felt.

2. **Meld-frame rings.** For each set in `showcase.sets`, draw a thin
   brass frame around the tile group. A simple extruded rectangle with
   the brass material (Metal kind, specular 0.9, orange-gold base) will
   read correctly. Pair = 2-tile frame, Sequence/Triplet = 3-tile, Kong
   = 4-tile with slightly thicker outline. Use the same meld detection
   you already iterate over in the showcase loop.

3. **Emissive rim pulse (shader work).** The design brief called for
   repurposing `material_params.w` as `emissive_mult` in
   `shaders/lit_mesh.wgsl`. Today it's unused. The change:
   - Add an `emissive_mult` read in the Metal branch of
     `lit_mesh.wgsl` (lines ~1700 for the Metal block), multiplying the
     base colour by `1.0 + emissive_mult * (0.5 + 0.5 * sin(time * 0.9))`
     where `time` is `lights.extras.y` (already piped through).
   - Thread a new field through `MaterialParams` in
     `src/render/lit_mesh.rs`. I tried this in the previous session and
     backed out — 20+ call sites all needed updating. Better approach:
     add an optional per-instance emissive override on `Object3d` or
     equivalent **without** changing `MaterialParams`, and sample that
     override only on the tray's draw call.

4. **Depth multiplier → glow intensity.** `structure_depth_mult_bonus`
   in `core::structure` already reports the bank's current mult. Map
   it to `emissive_mult`:
   - 1 meld banked (×1.0) → emissive_mult 0.0
   - 2 melds (×1.2) → 0.2
   - 3+ melds (×1.3 cap) → 0.35, locked steady (no sin pulse)
   At cap the rim locks bright — "you're full, trigger now."

5. **Cash-in tablet wiggle coupling.** The wood tablet at
   `gameplay.rs:3582` already wiggles via `trigger_tablet_wiggle_deg`.
   At max depth mult, increase the wiggle amplitude so the button feels
   urgent. Half-day of taste-tuning.

**Explicitly deferred** (Phase 2+):

- Stone abstractions (meld-shape summary shapes). Original art direction
  proposed these; revised direction keeps tile fidelity and adds frames
  around them. Stones remain a possible future iteration.
- Commit animation (tiles slide from hand into tray). Foundational
  tween plumbing exists in `src/render/animation.rs`; the blocker is
  selecting clean timing + start/end positions, not engineering.
- Yaku ghost previews above the tray. Requires running yaku detection
  against banked-but-not-scored melds; worth a separate design pass.
- Trigger ceremony (stones shatter into glyph motes). The existing
  cascade bezier arc is the template.

**Tradeoffs flagged:**

- Adding another world-space object in the already-crowded gameplay
  scene *will* cost some attention from whatever the player was looking
  at. The tray needs to be visually **quiet** at low depth mult and loud
  only when the bank is full — or else it becomes visual noise.
- The emissive shader change affects every Metal material in the game;
  default `emissive_mult = 0` avoids regressions but test the mirror,
  coin, relic dishes, and coin-pile pedestal carefully after wiring.
