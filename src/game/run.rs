//! Single-run state: wall, hand, score target, round modifiers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::boss::{self, BossKind};
use crate::core::debuff::TileDebuff;
use crate::core::deck::Wall;
use crate::core::hand::{DetectedSet, SetKind, detect_all_sets, validate_selection_with_rules};
use crate::core::structure::{
    StructureTriggerKind, StructureTriggerMeta, banked_meld_chips, can_trigger_structure,
    is_winning_structure_shape,
};

use crate::core::relic::{RelicId, RelicState, ScoreContext};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::{ScoreBreakdown, ScorePreview, preview_score, score_sets_with_original};
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::GameMode;
use crate::game::tutorial::TutorialState;

/// Boss-blind state for the current run.  Extracted from `RunState` so
/// boss-specific logic has a single owner.
#[derive(Debug, Serialize, Deserialize)]
pub struct BossState {
    /// Bosses still available for this run, drawn without replacement.
    pub pool_remaining: Vec<BossKind>,
    /// The boss for the current ante.
    pub upcoming: Option<BossKind>,
    /// Resolved effect for `upcoming`, rebuilt from the kind on load.
    #[serde(skip)]
    pub effect: Option<crate::core::boss::ResolvedBossEffect>,
    /// Per-round hand-size delta from boss effects.
    pub bonus_hand_size: i32,
    /// Gold cost charged after each successful play (set by The Tribute).
    pub gold_cost_per_play: u32,
    /// Per-play cost baked in by The Tax Collector at reveal time.
    #[serde(default)]
    pub tax_collector_cost: u32,
}

impl Default for BossState {
    fn default() -> Self {
        Self {
            pool_remaining: Vec::new(),
            upcoming: None,
            effect: None,
            bonus_hand_size: 0,
            gold_cost_per_play: 0,
            tax_collector_cost: 0,
        }
    }
}

/// Result of consuming a slot from the shared consumable inventory.
#[derive(Clone, Copy, Debug)]
pub enum ConsumableUseResult {
    Zodiac {
        yaku: crate::core::yaku::YakuKind,
        new_level: u32,
    },
    Talisman {
        kind: crate::core::talisman::TalismanKind,
    },
    /// The player activated a Kiln — the gameplay scene should enter tile
    /// selection mode so the player can pick up to 3 tiles to destroy.
    KilnMode,
}

pub const HAND_SIZE: usize = 14;
/// Defeating the Boss of this ante completes the run (Balatro-style).
pub const FINAL_ANTE: u32 = 8;

fn default_auto_cash_in_on_full_structure() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunState {
    pub wall: Wall,
    pub hand: Vec<Tile>,
    /// Which hand tiles are marked for discard (parallel with `hand`).
    pub selected: Vec<bool>,
    /// Melds committed from hand into the structure (deferred scoring until trigger).
    #[serde(default)]
    pub structure_sets: Vec<DetectedSet>,
    /// Tile copies held in the structure (same ids as in `structure_sets`).
    #[serde(default)]
    pub structure_tiles: Vec<Tile>,
    pub round_score: u64,
    pub target_score: u32,
    pub base_target: u32,
    pub relics: RelicState,
    pub round_rules: Vec<RuleModifier>,
    pub run_number: u32,
    /// Current ante (1-indexed). Increments after defeating each Boss blind.
    pub ante: u32,
    pub plays_remaining: u32,
    /// Effective play peg row length for the current round after round-start
    /// modifiers (bosses, skip tags, relic taxes) have been applied.
    #[serde(default)]
    pub plays_max: u32,
    pub discards_remaining: u32,
    /// Effective discard peg row length for the current round after round-start
    /// modifiers (bosses, skip tags, relic taxes) have been applied.
    #[serde(default)]
    pub discards_max: u32,
    pub gold: i32,
    pub blind: BlindKind,
    /// Next blind the player will face in the Small→Big→Boss cycle.
    pub upcoming_blind: BlindKind,
    /// Last scoring breakdown for UI cascade display. Not persisted across
    /// quit/resume — the cascade is a transient UI artifact, not run state.
    #[serde(skip)]
    pub last_breakdown: Option<ScoreBreakdown>,
    /// Yaku available at the player's progression level.
    pub available_yaku: Vec<crate::core::yaku::YakuKind>,
    /// Rules available at the player's progression level.
    pub available_rules: Vec<RuleModifier>,
    /// Whether the player scored on their last play (for ChainReaction relic).
    pub scored_last_turn: bool,
    /// Whether QuickDraw extra tile was used this round.
    pub quickdraw_used: bool,
    /// Whether JokerTile was used this round.
    pub joker_used: bool,
    /// Whether the player has scored a FullHand yaku this round. The Tenpai
    /// Bonus (`scoring.rs` Phase 4.5) fires only on the *first* such play.
    pub full_hand_played_this_round: bool,
    /// Per-yaku level (default 1). Incremented by Zodiac card use.
    pub yaku_levels: crate::core::zodiac::YakuLevels,
    /// Player's shared consumable inventory — holds both Zodiacs and Talismans
    /// in the same capped slot list. Capacity expands via Zodiac Pouch and
    /// Lunar Almanac relics.
    pub consumables: crate::core::consumable::ConsumableInventory,
    /// Game mode preset used for this run (drives advance_round resets).
    pub mode: GameMode,
    /// When true, automatically cash in the structure as soon as it reaches
    /// a full valid shape.
    #[serde(default = "default_auto_cash_in_on_full_structure")]
    pub auto_cash_in_on_full_structure: bool,
    // ── Boss blind state ─────────────────────────────────────────────────
    #[serde(flatten)]
    pub boss: BossState,
    /// Yaku detected on prior plays in this round. Used by The Censor to
    /// halve repeat-yaku contributions. Reset on round start.
    #[serde(skip)]
    pub played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    /// Round-scoped tile debuffs applied by the active boss.
    #[serde(default)]
    pub tile_debuffs: Vec<TileDebuff>,
    /// Set true the first time this round a scored hand contains a Wind
    /// or Dragon tile. Powers Green Luck's per-round payout — the relic
    /// awards its bonus at round clear iff this stays false. Reset by
    /// `advance_round` and `skip_to_next_blind`.
    #[serde(skip)]
    pub honors_scored_this_round: bool,
    /// Cumulative score earned across the entire run (for Snowball relic).
    #[serde(default)]
    pub total_score_earned: u64,
    /// Per-yaku cumulative play counter for the entire run. Powers the
    /// Yaku Journal overlay's "Played N×" line. Persisted across save/load
    /// (defaults to empty for old saves).
    #[serde(default)]
    pub yaku_times_played: std::collections::HashMap<crate::core::yaku::YakuKind, u32>,
    /// Per-run tile enhancement map, keyed by tile id. Talismans stamp every
    /// hand tile's id into this map; whenever tiles are drawn (initial deal,
    /// post-play refill, mid-round draws, new-round redeals), we re-apply the
    /// stored enhancement so it persists for the rest of the run. Tile ids are
    /// stable across walls because `build_wall` assigns them deterministically
    /// (so id 5 is always the same suit+rank, even after a reshuffle).
    #[serde(default)]
    pub tile_enhancements: BTreeMap<u32, TileEnhancement>,
    /// Tile IDs permanently removed from the wall via the Kiln talisman.
    /// Filtered out during wall construction each round.
    #[serde(default)]
    pub removed_tile_ids: std::collections::HashSet<u32>,
    /// Tile packs purchased from the shop. Each pack permanently injects
    /// extra tiles into the wall every round. Append-only.
    #[serde(default)]
    pub tile_packs: Vec<crate::core::tile_pack::TilePackKind>,

    // ── Skip-reward tags ──────────────────────────────────────────────
    /// Tag assigned to the Small blind this ante.
    #[serde(default)]
    pub small_blind_tag: Option<crate::core::tag::TagKind>,
    /// Tag assigned to the Big blind this ante.
    #[serde(default)]
    pub big_blind_tag: Option<crate::core::tag::TagKind>,
    /// Tag-granted: next shop's first reroll is free.
    #[serde(default)]
    pub tag_free_reroll: bool,
    /// Tag-granted: one random relic in the next shop costs 0.
    #[serde(default)]
    pub tag_patron_gift: bool,
    /// Tag-granted: next shop stocks 2 extra relics.
    #[serde(default)]
    pub tag_rich_stock: bool,
    /// Tag-granted bonus plays for the next round.
    #[serde(default)]
    pub tag_bonus_plays: u32,
    /// Tag-granted bonus discards for the next round.
    #[serde(default)]
    pub tag_bonus_discards: u32,
    /// Tag-granted bonus hand size for the next round.
    #[serde(default)]
    pub tag_bonus_hand_size: i32,
    /// Pending zodiac activation from a ZodiacBlessing skip-reward tag.
    /// Consumed by the pick-blind scene to trigger the celebration overlay.
    #[serde(skip)]
    pub pending_zodiac_celebration: Option<(
        crate::core::zodiac::ZodiacKind,
        crate::core::yaku::YakuKind,
        u32,
    )>,
    /// Permanent per-tile chip bonus accumulated by the Tile Polisher
    /// relic. Each scored tile adds +3 to this counter for the rest of
    /// the run. Applied in Phase 2 of scoring.
    #[serde(default)]
    pub tile_polisher_bonus: i32,
    /// Per-relic mutable counters. Key is RelicId, value meaning depends
    /// on the relic:
    ///   CleanStreak  → consecutive plays without honor tiles
    ///   Obsession    → rounds without most-used yaku
    ///   Bonfire      → relics sold this run
    ///   MeltingIce   → remaining chip bonus (starts 80, -8 per play)
    ///   SilkThread   → remaining mult ×10 (starts 40, -3 per discard)
    ///   NestEgg      → rounds held (sell value grows)
    ///   TeaCeremony  → plays remaining before destruction
    ///   PhantomRelic → rounds held
    ///   RitualBlade  → permanent mult bonus ×10
    #[serde(default)]
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    /// Permanent chip bonus from River Runner relic. Each scored sequence
    /// adds +20 chips permanently.
    #[serde(default)]
    pub river_runner_bonus: i32,
    /// Tutorial state. `None` for normal (non-tutorial) runs. Present
    /// during the player's very first run to gate mechanics by lesson.
    #[serde(default)]
    pub tutorial: Option<TutorialState>,

    /// Relics whose effects just fired this frame. Scenes drain this each
    /// frame to drive glow + wiggle animations. Populated by `run.rs`
    /// methods whenever a relic triggers (scoring, round-end, discard,
    /// draw, consumable interaction, etc.).
    #[serde(skip)]
    pub relic_activations: Vec<RelicId>,
}

impl RunState {
    fn round_play_cap(&self) -> u32 {
        let mut plays = self.mode.starting_plays;
        if self.relics.has(crate::core::relic::RelicId::SecondWind) {
            plays += 1;
        }
        if self.relics.has(crate::core::relic::RelicId::GlassCannon) {
            plays = plays.saturating_sub(1);
        }
        plays
    }

    fn round_discard_cap(&self) -> u32 {
        self.mode.starting_discards
    }

    /// Rebuild round-start plays/discards from current mode + permanent
    /// round modifiers. This keeps all "per round" bonuses sourced from one
    /// place so shop-bought relics and material bonuses apply on the next blind.
    fn reset_round_resources(&mut self) {
        self.plays_remaining = self.round_play_cap();
        self.discards_remaining = self.round_discard_cap();
        self.sync_round_resource_caps();
    }

    fn sync_round_resource_caps(&mut self) {
        self.plays_max = self.plays_remaining;
        self.discards_max = self.discards_remaining;
    }

    /// Consume skip-tag bonuses that apply to the next blind only.
    fn apply_pending_round_resource_bonuses(&mut self) {
        if self.tag_bonus_plays > 0 {
            self.plays_remaining += self.tag_bonus_plays;
            self.tag_bonus_plays = 0;
        }
        if self.tag_bonus_discards > 0 {
            self.discards_remaining += self.tag_bonus_discards;
            self.tag_bonus_discards = 0;
        }
    }

    pub fn new(mode: GameMode) -> Self {
        let hand_size = mode.hand_size;
        let mut wall = Wall::from_standard_shuffled();
        let mut hand = Vec::with_capacity(hand_size);
        for _ in 0..hand_size {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        hand.sort();
        let selected = vec![false; hand.len()];

        let mut relics = RelicState::default();
        for &r in &mode.starting_relics {
            if !relics.is_full() {
                relics.active.push(r);
            }
        }

        let mut boss_pool_remaining = boss::regular_pool();
        let mut rng = rand::rng();
        let upcoming_boss = boss::pick_for_ante(&mut boss_pool_remaining, 1, &mut rng);

        let mut state = Self {
            wall,
            hand,
            selected,
            structure_sets: Vec::new(),
            structure_tiles: Vec::new(),
            round_score: 0,
            target_score: mode.base_target,
            base_target: mode.base_target,
            relics,
            round_rules: mode.starting_rules.clone(),
            run_number: 1,
            ante: 1,
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
            discards_remaining: mode.starting_discards,
            discards_max: mode.starting_discards,
            gold: mode.starting_gold as i32,
            blind: BlindKind::Small,
            upcoming_blind: BlindKind::Small,
            last_breakdown: None,
            available_yaku: mode.starting_yaku.clone(),
            available_rules: mode.starting_rules.clone(),
            scored_last_turn: false,
            quickdraw_used: false,
            joker_used: false,
            full_hand_played_this_round: false,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            consumables: crate::core::consumable::ConsumableInventory {
                items: Vec::new(),
                capacity: mode.consumable_capacity,
            },
            mode,
            auto_cash_in_on_full_structure: true,
            boss: BossState {
                pool_remaining: boss_pool_remaining,
                upcoming: upcoming_boss,
                effect: None,
                bonus_hand_size: 0,
                gold_cost_per_play: 0,
                tax_collector_cost: 0,
            },
            played_yaku_this_round: Vec::new(),
            tile_debuffs: Vec::new(),
            honors_scored_this_round: false,
            total_score_earned: 0,
            yaku_times_played: std::collections::HashMap::new(),
            tile_enhancements: BTreeMap::new(),
            removed_tile_ids: std::collections::HashSet::new(),
            tile_packs: Vec::new(),
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebration: None,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            river_runner_bonus: 0,
            tutorial: None,
            relic_activations: Vec::new(),
        };
        // Roll skip-reward tags for ante 1.
        state.roll_ante_tags();
        // Resolve the first ante's boss now so reactive variants are baked
        // in before pick_blind ever reads `upcoming_boss_effect`.
        state.resolve_upcoming_boss();
        // No-op for a fresh run (empty enhancement map), but kept here so the
        // invariant "hand always reflects tile_enhancements" holds uniformly.
        state.restamp_hand_enhancements();
        state
    }

    pub fn set_auto_cash_in_on_full_structure(&mut self, enabled: bool) {
        self.auto_cash_in_on_full_structure = enabled;
    }

    /// Build the `ResolvedBossEffect` for the current `upcoming_boss`. For
    /// static bosses this is a thin wrap of `BossDef::effect`. For reactive
    /// bosses (those with an `on_reveal` hook), the hook runs against the
    /// current `RunState` and produces a tailored effect that's locked in
    /// for the rest of the ante. Idempotent — safe to call from
    /// `RunState::new`, `advance_round`, and the save-load rehydrate path.
    pub fn resolve_upcoming_boss(&mut self) {
        use crate::core::boss::ResolvedBossEffect;
        // Reset any reactive scratch — the new boss may not need it.
        self.boss.tax_collector_cost = 0;
        let Some(kind) = self.boss.upcoming else {
            self.boss.effect = None;
            return;
        };
        let def = kind.def();
        // Two-step to keep borrows clean: pull `on_reveal` out as a
        // function pointer (Copy), then call it with `&mut *self`.
        let effect = match def.on_reveal {
            Some(hook) => hook(self),
            None => ResolvedBossEffect::from_static(&def.effect),
        };
        self.boss.effect = Some(effect);
    }

    /// Convenience constructor using the standard game mode.
    pub fn new_demo() -> Self {
        Self::new(GameMode::standard())
    }

    /// Start a new run with the given tile material set.
    pub fn new_with_material(material: crate::persistence::TileMaterial) -> Self {
        Self::new(GameMode::with_material(material))
    }

    /// Start a tutorial run for first-time players. Uses a stripped-down
    /// game mode and enables the tutorial state machine.
    pub fn new_tutorial() -> Self {
        let mut state = Self::new(GameMode::tutorial());
        state.tutorial = Some(TutorialState::new(1));
        // Seed the initial hand for lesson 1 (pairs).
        state.seed_tutorial_hand();
        state
    }

    /// Advance the tutorial to the next lesson. Adjusts hand size, target,
    /// discards, and available yaku per the lesson definition. Returns the
    /// new lesson number, or `None` if the tutorial finished.
    pub fn advance_tutorial_lesson(&mut self) -> Option<u32> {
        let tutorial = self.tutorial.as_mut()?;
        let next = tutorial.advance()?;
        let lesson = tutorial.current_lesson_def();

        // Apply lesson overrides to the mode.
        self.mode.apply_lesson(lesson);

        // Update run state from mode.
        self.available_yaku = self.mode.starting_yaku.clone();
        self.target_score = self.mode.base_target;
        self.base_target = self.mode.base_target;
        self.plays_remaining = self.round_play_cap();
        self.discards_remaining = self.round_discard_cap();
        self.sync_round_resource_caps();

        // Adjust hand size: grow by drawing more tiles if needed.
        let target_hand_size = self.mode.hand_size;
        while self.hand.len() < target_hand_size {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            } else {
                break;
            }
        }
        self.hand.sort();
        self.selected.resize(self.hand.len(), false);

        // Seed guaranteed melds for early lessons.
        self.seed_tutorial_hand();

        Some(next)
    }

    /// Retry the current tutorial blind after a failure. Records the failure
    /// for adaptive difficulty, re-deals the hand, and resets plays/discards
    /// without advancing the lesson or ante. The lowered target is applied
    /// via `retry_target_factor` in `apply_blind`.
    pub fn retry_tutorial_blind(&mut self) {
        if let Some(ref mut tut) = self.tutorial {
            tut.record_failure();
        }

        // Reset round state (same blind, same lesson).
        self.round_score = 0;
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_used = false;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;

        // Re-deal the wall and hand.
        let overflow = self.relics.has(crate::core::relic::RelicId::Overflow);
        self.wall = Wall::from_filtered_with_packs(
            &self.removed_tile_ids,
            &self.tile_packs,
            &self.tile_enhancements,
            overflow,
        );
        self.hand.clear();
        let draw_count = self.mode.hand_size;
        for _ in 0..draw_count {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        self.restamp_hand_enhancements();
        self.seed_tutorial_hand();

        // Re-apply blind (with lowered target via retry_target_factor).
        self.apply_blind(self.blind);
    }

    /// Check if a set of detected meld kinds is valid for the current
    /// tutorial lesson. Returns `Ok(())` or an error message.
    pub fn tutorial_validate_sets(&self, set_kinds: &[SetKind]) -> Result<(), &'static str> {
        if let Some(ref tutorial) = self.tutorial {
            if tutorial.is_active() {
                let lesson = tutorial.current_lesson_def();
                return crate::game::tutorial::validate_sets_for_lesson(set_kinds, lesson);
            }
        }
        Ok(())
    }

    /// Check if discarding is allowed in the current tutorial lesson.
    pub fn tutorial_discard_allowed(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().discard_enabled,
            _ => true,
        }
    }

    /// Check if the shop should be shown in the current tutorial lesson.
    pub fn tutorial_shop_enabled(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().shop_enabled,
            _ => true,
        }
    }

    /// Whether tile affinity glow should be active.
    pub fn tutorial_affinity_glow(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().affinity_glow,
            _ => false,
        }
    }

    /// Whether the scoring cascade should run in annotated slow-mo.
    pub fn tutorial_annotated_cascade(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() && !tutorial.cascade_annotated => {
                tutorial.current_lesson_def().annotated_cascade
            }
            _ => false,
        }
    }

    /// Seed guaranteed melds into the current hand for tutorial lessons.
    /// Shuffles the hand first so retries produce different tile layouts,
    /// then overwrites a few positions to ensure the player can form the
    /// melds that the current lesson teaches. Only affects lessons 1-5.
    fn seed_tutorial_hand(&mut self) {
        use rand::seq::SliceRandom;

        let lesson_id = match &self.tutorial {
            Some(t) if t.is_active() => t.current_lesson,
            _ => return,
        };

        // Only certain lessons need seeded guaranteed melds; later lessons
        // play with whatever the wall gives.
        if lesson_id > 7 || lesson_id == 6 {
            return;
        }

        // Shuffle before seeding so the base tiles vary between attempts.
        let mut rng = rand::rng();
        self.hand.shuffle(&mut rng);

        match lesson_id {
            1 => {
                // Ensure at least 2 pairs. Pick two tiles and duplicate them.
                if self.hand.len() >= 4 {
                    let face0 = (self.hand[0].suit, self.hand[0].rank);
                    // Make hand[1] match hand[0].
                    self.hand[1].suit = face0.0;
                    self.hand[1].rank = face0.1;
                    // Make another pair from hand[2].
                    let face1 = (self.hand[2].suit, self.hand[2].rank);
                    self.hand[3].suit = face1.0;
                    self.hand[3].rank = face1.1;
                    self.hand.sort();
                }
            }
            2 => {
                // Ensure at least 1 triplet.
                if self.hand.len() >= 3 {
                    let face = (self.hand[0].suit, self.hand[0].rank);
                    self.hand[1].suit = face.0;
                    self.hand[1].rank = face.1;
                    self.hand[2].suit = face.0;
                    self.hand[2].rank = face.1;
                    self.hand.sort();
                }
            }
            3 => {
                // Ensure at least 1 sequence. Pick a numbered suit tile and
                // make the next two consecutive.
                let len = self.hand.len();
                if len >= 3 {
                    let base_face = self
                        .hand
                        .iter()
                        .find(|t| t.is_number_tile() && t.rank <= 7)
                        .map(|t| (t.suit, t.rank));
                    if let Some((suit, rank)) = base_face {
                        let i = len - 2;
                        let j = len - 1;
                        self.hand[i].suit = suit;
                        self.hand[i].rank = rank + 1;
                        self.hand[j].suit = suit;
                        self.hand[j].rank = rank + 2;
                    }
                    self.hand.sort();
                }
            }
            4 | 5 => {
                // Ensure the 14-tile hand has a sequence + triplet + pair
                // (close to FullHand territory for lesson 5).
                let len = self.hand.len();
                if len >= 9 {
                    // Triplet from first tile.
                    let face = (self.hand[0].suit, self.hand[0].rank);
                    self.hand[1].suit = face.0;
                    self.hand[1].rank = face.1;
                    self.hand[2].suit = face.0;
                    self.hand[2].rank = face.1;
                    // Sequence from a mid-range numbered suit.
                    let seq_face = self.hand[4..]
                        .iter()
                        .find(|t| t.is_number_tile() && t.rank <= 7)
                        .map(|t| (t.suit, t.rank));
                    if let Some((suit, rank)) = seq_face {
                        self.hand[5].suit = suit;
                        self.hand[5].rank = rank + 1;
                        self.hand[6].suit = suit;
                        self.hand[6].rank = rank + 2;
                    }
                    // Pair from hand[7].
                    let pf = (self.hand[7].suit, self.hand[7].rank);
                    self.hand[8].suit = pf.0;
                    self.hand[8].rank = pf.1;
                    self.hand.sort();
                }
            }
            7 => {
                // Guarantee an honor triplet so the player can trigger Yakuhai.
                use crate::core::tile::Suit;
                use rand::RngExt;
                let len = self.hand.len();
                if len >= 3 {
                    let honor_suits = [Suit::Wind, Suit::Dragon];
                    let suit = honor_suits[rng.random_range(0..honor_suits.len())];
                    let rank: u8 = if suit == Suit::Wind {
                        rng.random_range(1..=4) // East, South, West, North
                    } else {
                        rng.random_range(1..=3) // Red, Green, White
                    };
                    self.hand[0].suit = suit;
                    self.hand[0].rank = rank;
                    self.hand[1].suit = suit;
                    self.hand[1].rank = rank;
                    self.hand[2].suit = suit;
                    self.hand[2].rank = rank;
                    self.hand.sort();
                }
            }
            _ => {}
        }
    }

    /// Use a consumable from the shared inventory at `index`. Zodiacs level
    /// their yaku for the run; Talismans stamp their enhancement onto every
    /// tile currently in the player's hand. Returns a [`ConsumableUseResult`]
    /// describing what happened so the UI can log/animate appropriately.
    pub fn use_consumable(&mut self, index: usize) -> Option<ConsumableUseResult> {
        use crate::core::consumable::Consumable;
        let item = self.consumables.take(index)?;
        match item {
            Consumable::Zodiac(z) => {
                let yaku = z.yaku();
                let new_level = self.yaku_levels.level_up(yaku);
                Some(ConsumableUseResult::Zodiac { yaku, new_level })
            }
            Consumable::Talisman(t) => {
                if t == crate::core::talisman::TalismanKind::Kiln {
                    return Some(ConsumableUseResult::KilnMode);
                }
                let enh = t.enhancement().expect("non-Kiln talisman has enhancement");
                // Record the enhancement against each current hand tile's id
                // so it persists when those tiles get redrawn next round.
                for tile in &self.hand {
                    self.tile_enhancements.insert(tile.id, enh);
                }
                crate::core::talisman::apply_to_hand(&mut self.hand, t);
                Some(ConsumableUseResult::Talisman { kind: t })
            }
        }
    }

    /// Maximum number of tiles that can be removed from the wall via the Kiln.
    /// The wall needs enough tiles to deal a full hand each round.
    const MAX_REMOVED_TILES: usize = 56;

    /// Permanently destroy the selected tiles from the hand (Kiln talisman).
    /// Removed tile IDs are recorded so they never appear in future walls.
    /// Returns the number of tiles actually destroyed.
    pub fn destroy_selected_tiles(&mut self, bus: &mut crate::game::event_bus::EventBus) -> usize {
        let budget = Self::MAX_REMOVED_TILES.saturating_sub(self.removed_tile_ids.len());
        let mut destroyed = 0usize;
        let mut kept_hand = Vec::new();
        let mut kept_sel = Vec::new();
        for (i, tile) in self.hand.iter().enumerate() {
            if self.selected[i] && destroyed < budget {
                self.removed_tile_ids.insert(tile.id);
                self.tile_enhancements.remove(&tile.id);
                destroyed += 1;
            } else {
                kept_hand.push(*tile);
                kept_sel.push(false);
            }
        }
        self.hand = kept_hand;
        self.selected = kept_sel;
        // Refill hand from the wall.
        while self.hand.len() < self.mode.hand_size {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
                self.selected.push(false);
            } else {
                break;
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        self.restamp_hand_enhancements();
        if destroyed > 0 {
            bus.push(crate::game::event_bus::GameEvent::TilesDestroyed { count: destroyed });
        }
        destroyed
    }

    /// Re-stamp every tile in the current hand with whatever enhancement is
    /// stored against its id in `tile_enhancements`. Called after any path
    /// that adds tiles to the hand (initial deal, post-play refill, mid-round
    /// draws, new-round redeal) so talisman effects survive for the whole run.
    fn restamp_hand_enhancements(&mut self) {
        if self.tile_enhancements.is_empty() {
            return;
        }
        for tile in &mut self.hand {
            if let Some(&enh) = self.tile_enhancements.get(&tile.id) {
                tile.enhancement = Some(enh);
            }
        }
    }

    /// Try to add a Zodiac card to the inventory; returns `true` on success.
    #[allow(dead_code)]
    pub fn grant_zodiac(&mut self, z: crate::core::zodiac::ZodiacKind) -> bool {
        self.consumables
            .try_push(crate::core::consumable::Consumable::Zodiac(z))
    }

    /// Recompute consumable inventory capacity from
    /// currently-owned relics. Idempotent — call after any relic add/remove.
    /// The inventory is shared between Zodiacs and Talismans; the base
    /// capacity comes from `GameMode::consumable_capacity` (default 2).
    /// (Patch C: ZodiacPouch +1, LunarAlmanac +1.)
    pub fn recompute_capacities(&mut self) {
        let mut consumable_cap = self.mode.consumable_capacity;
        if self.relics.has(RelicId::ZodiacPouch) {
            consumable_cap += 1;
        }
        if self.relics.has(RelicId::LunarAlmanac) {
            consumable_cap += 1;
        }
        self.consumables.capacity = consumable_cap;
    }

    /// Whether a run is in progress (not a fresh/default state).
    pub fn is_in_progress(&self) -> bool {
        self.round_score > 0 || self.run_number > 1 || self.gold != self.mode.starting_gold as i32
    }

    /// True once the player has defeated the Boss of the final ante.
    pub fn is_run_complete(&self) -> bool {
        self.ante > FINAL_ANTE
    }

    /// Apply a blind choice: sets target score, dispatches boss effect on
    /// boss blinds, and applies any per-round resource resets.
    pub fn apply_blind(&mut self, blind: BlindKind) {
        self.blind = blind;
        self.round_score = 0;
        self.reset_round_resources();
        self.tile_debuffs.clear();
        self.relics.clear_debuffs();
        let mut target = (self.base_target as f32 * blind.target_multiplier()) as u32;
        // Tutorial adaptive difficulty: lower the target after repeated failures.
        if let Some(ref tut) = self.tutorial {
            if tut.is_active() {
                target = (target as f32 * tut.retry_target_factor()) as u32;
            }
        }
        self.target_score = target;
        // Boss dispatch — push rule modifiers and run the on_apply hook so
        // category-C taxers (zero discards, hand-size shrink, gold cost) take
        // effect before the player draws their first hand.
        let simplified = self
            .tutorial
            .as_ref()
            .is_some_and(|t| t.is_active() && t.current_lesson_def().simplified_boss);
        if blind == BlindKind::Boss && !simplified {
            // Read from the resolved effect (built at reveal time) so reactive
            // bosses' chosen variants land correctly. Take/restore to dodge
            // the &mut self conflict when calling on_apply.
            if let Some(eff) = self.boss.effect.take() {
                for &m in &eff.rule_pushes {
                    if !self.round_rules.contains(&m) {
                        self.round_rules.push(m);
                    }
                }
                self.tile_debuffs = eff.tile_debuffs.clone();
                self.relics.set_debuffed(eff.relic_debuffs.iter().copied());
                if let Some(hook) = eff.on_apply {
                    hook(self);
                }
                self.boss.effect = Some(eff);
            }
        }
        // Fold next-round Wide Hand into the round's effective hand size so
        // every refill and hand-size check sees the same total for the round.
        if self.tag_bonus_hand_size != 0 {
            self.boss.bonus_hand_size += self.tag_bonus_hand_size;
            self.tag_bonus_hand_size = 0;
        }
        // ReducedPlays modifier reduces plays from 4 to 3.
        if self.round_rules.contains(&RuleModifier::ReducedPlays) {
            self.plays_remaining = self.plays_remaining.min(3);
        }
        self.apply_pending_round_resource_bonuses();
        self.sync_round_resource_caps();
        // Deal the wall and hand for this round.
        let overflow = self.relics.has(crate::core::relic::RelicId::Overflow);
        self.wall = Wall::from_filtered_with_packs(
            &self.removed_tile_ids,
            &self.tile_packs,
            &self.tile_enhancements,
            overflow,
        );
        self.hand.clear();
        let draw_count = boss::effective_hand_size(self);
        for _ in 0..draw_count {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        self.structure_sets.clear();
        self.structure_tiles.clear();
        self.restamp_hand_enhancements();
    }

    /// Commit selected melds into structure (costs one play). Alias for the
    /// structure-system primary action — same as [`Self::commit_selection_to_structure`].
    pub fn score_selected_tiles(&mut self, bus: &mut EventBus) -> u64 {
        self.commit_selection_to_structure(bus)
    }

    /// Move validated melds from hand into **structure**; consumes one play.
    /// Returns points added this step (`0` or `1` success token for UI; real score is on trigger).
    pub fn commit_selection_to_structure(&mut self, bus: &mut EventBus) -> u64 {
        if self.plays_remaining == 0 || self.selected_count() == 0 {
            return 0;
        }

        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();

        let (sets, scoring_tiles) = match self.try_validate_with_wildcards(&selected_tiles) {
            Some(result) => result,
            None => {
                bus.push(GameEvent::InvalidAction);
                return 0;
            }
        };

        {
            let set_kinds: Vec<SetKind> = sets.iter().map(|s| s.kind).collect();
            if self.tutorial_validate_sets(&set_kinds).is_err() {
                bus.push(GameEvent::InvalidAction);
                return 0;
            }
        }
        if scoring_tiles != selected_tiles && self.relics.has(RelicId::JokerTile) {
            self.joker_used = true;
            self.relic_activations.push(RelicId::JokerTile);
        }

        if self.mode.structure_bank {
            let current_tile_count = self.structure_tiles.len();
            if current_tile_count + scoring_tiles.len() > HAND_SIZE {
                bus.push(GameEvent::InvalidAction);
                return 0;
            }
            for s in &sets {
                self.structure_sets.push(s.clone());
            }
            self.structure_tiles.extend(scoring_tiles.iter().copied());
            bus.push(GameEvent::StructureCommitted);
        } else {
            let original = scoring_tiles.clone();
            let _ =
                self.apply_scored_melds(sets.clone(), scoring_tiles.clone(), original, None, bus);
        }

        self.plays_remaining = self.plays_remaining.saturating_sub(1);

        if self.relics.has(RelicId::MeltingIce) {
            let v = self.relic_counters.entry(RelicId::MeltingIce).or_insert(80);
            *v = (*v - 8).max(0);
            if *v == 0 {
                self.relics.active.retain(|&r| r != RelicId::MeltingIce);
                self.relic_counters.remove(&RelicId::MeltingIce);
            }
        }
        if self.relics.has(RelicId::TeaCeremony) {
            let v = self.relic_counters.entry(RelicId::TeaCeremony).or_insert(3);
            *v -= 1;
            if *v <= 0 {
                self.relics.active.retain(|&r| r != RelicId::TeaCeremony);
                self.relic_counters.remove(&RelicId::TeaCeremony);
            }
        }
        if self.relics.has(RelicId::CleanStreak) {
            let has_honors = scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            let v = self.relic_counters.entry(RelicId::CleanStreak).or_insert(0);
            if has_honors {
                *v = 0;
            } else {
                *v += 1;
            }
        }

        if self.mode.structure_bank {
            self.scored_last_turn = false;
        }

        let indices: Vec<usize> = self
            .selected
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s)
            .map(|(i, _)| i)
            .rev()
            .collect();
        for &i in &indices {
            self.hand.remove(i);
        }

        let effective = boss::effective_hand_size(self);
        let draw_target = if self.relics.has(RelicId::QuickDraw) && !self.quickdraw_used {
            self.quickdraw_used = true;
            self.relic_activations.push(RelicId::QuickDraw);
            effective + 1
        } else {
            effective
        };
        while self.hand.len() < draw_target {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }

        self.hand.sort();
        if !self.mode.structure_bank {
            self.set_magnet_after_score(&sets, &scoring_tiles, bus);
            self.hand.sort();
        }
        self.selected = vec![false; self.hand.len()];
        self.seed_tutorial_hand();
        self.restamp_hand_enhancements();

        if self.blind == BlindKind::Boss {
            if let Some(eff) = self.boss.effect.take() {
                if let Some(hook) = eff.on_play {
                    hook(self);
                }
                self.boss.effect = Some(eff);
            }
        }

        self.try_autotrigger_structure_full(bus);
        if self.mode.structure_bank
            && self.plays_remaining == 0
            && self.round_score < self.target_score as u64
            && !self.structure_sets.is_empty()
        {
            let _ = self.trigger_structure(StructureTriggerKind::AutoNoPlays, bus);
        }
        self.emit_round_resolution_events(bus);
        1
    }

    /// When false, plays score immediately and the structure bank / cash-in UI are disabled.
    #[inline]
    pub fn uses_structure_bank(&self) -> bool {
        self.mode.structure_bank
    }

    /// Core scoring path for resolved melds (structure trigger or classic commit).
    fn apply_scored_melds(
        &mut self,
        sets: Vec<DetectedSet>,
        scoring_tiles: Vec<Tile>,
        original_for_wildcard: Vec<Tile>,
        structure_meta: Option<StructureTriggerMeta>,
        bus: &mut EventBus,
    ) -> u64 {
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        let ctx = ScoreContext {
            relics: &self.relics,
            tile_debuffs: &scoring_tile_debuffs,
            scored_last_turn: self.scored_last_turn,
            dora_faces: self.wall.dora_faces(),
            available_yaku: self.available_yaku.clone(),
            round_wind: rw,
            first_full_hand_of_round: !self.full_hand_played_this_round,
            plays_used: self.round_play_cap().saturating_sub(self.plays_remaining),
            riichi_active: false,
            yaku_levels: Some(self.yaku_levels.clone()),
            played_yaku_this_round: self.played_yaku_this_round.clone(),
            gold: self.gold,
            total_score: self.total_score_earned,
            is_final_play: self.plays_remaining == 0,
            tile_polisher_bonus: self.tile_polisher_bonus,
            relic_counters: self.relic_counters.clone(),
            unscored_hand_tiles: self.hand.len(),
            river_runner_bonus: self.river_runner_bonus,
            structure: structure_meta,
        };
        let breakdown = score_sets_with_original(
            &scoring_tiles,
            &sets,
            &ctx,
            &self.round_rules,
            &original_for_wildcard,
        );
        let earned = breakdown.total;
        self.round_score = self.round_score.saturating_add(earned);
        self.total_score_earned = self.total_score_earned.saturating_add(earned);

        if self.relics.has(RelicId::TilePolisher) {
            let tile_count: i32 = sets.iter().map(|s| s.tile_ids.len() as i32).sum();
            self.tile_polisher_bonus += 3 * tile_count;
            self.relic_activations.push(RelicId::TilePolisher);
        }
        if self.relics.has(RelicId::RiverRunner) {
            let seq_count = sets.iter().filter(|s| s.kind == SetKind::Sequence).count() as i32;
            if seq_count > 0 {
                self.river_runner_bonus += 20 * seq_count;
                self.relic_activations.push(RelicId::RiverRunner);
            }
        }
        if self.relics.has(RelicId::StarTile) && !breakdown.detected_yaku.is_empty() {
            use rand::RngExt;
            use rand::seq::IndexedRandom;
            let mut rng = rand::rng();
            let prob = if self.relics.has(RelicId::FortunesFavor) {
                2
            } else {
                1
            };
            if rng.random_ratio(prob, 4) {
                if let Some(&y) = breakdown.detected_yaku.choose(&mut rng) {
                    let _new_level = self.yaku_levels.level_up(y);
                    self.relic_activations.push(RelicId::StarTile);
                }
            }
        }
        if breakdown.flower_gold > 0 {
            self.gold = self.gold.saturating_add(breakdown.flower_gold);
            bus.push(GameEvent::GoldChanged {
                delta: breakdown.flower_gold,
            });
        }
        let scored_full_hand = breakdown
            .detected_yaku
            .contains(&crate::core::yaku::YakuKind::FullHand);
        if scored_full_hand {
            self.full_hand_played_this_round = true;
        }
        if self.relics.has(RelicId::KanDrum) {
            let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as u32;
            if kong_count > 0 {
                self.plays_remaining = self.plays_remaining.saturating_add(kong_count);
                self.relic_activations.push(RelicId::KanDrum);
            }
        }
        for &y in &breakdown.detected_yaku {
            *self.yaku_times_played.entry(y).or_insert(0) += 1;
            if !self.played_yaku_this_round.contains(&y) {
                self.played_yaku_this_round.push(y);
            }
        }
        self.last_breakdown = Some(breakdown);
        self.scored_last_turn = earned > 0;

        if !self.honors_scored_this_round
            && scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        {
            self.honors_scored_this_round = true;
        }

        if scored_full_hand && self.relics.has(RelicId::EightTreasures) {
            use rand::seq::IndexedRandom;
            let mut rng = rand::rng();
            if let Some(&z) = crate::core::zodiac::ZodiacKind::all().choose(&mut rng) {
                self.consumables
                    .items
                    .push(crate::core::consumable::Consumable::Zodiac(z));
                self.relic_activations.push(RelicId::EightTreasures);
            }
        }

        earned
    }

    fn scoring_tile_debuffs(&self, scoring_tiles: &[Tile]) -> Vec<TileDebuff> {
        let mut debuffs = self.tile_debuffs.clone();
        let dragon_without_honors = self.blind == BlindKind::Boss
            && self.boss.upcoming == Some(BossKind::Dragon)
            && !scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
        if dragon_without_honors {
            for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles, Suit::Flower] {
                if scoring_tiles.iter().any(|t| t.suit == suit) {
                    debuffs.push(TileDebuff::Suit(suit));
                }
            }
        }
        debuffs
    }

    fn set_magnet_after_score(
        &mut self,
        sets: &[DetectedSet],
        scoring_tiles: &[Tile],
        bus: &mut EventBus,
    ) {
        if !self.relics.has(RelicId::SetMagnet) {
            return;
        }
        let scored_triplet = sets
            .iter()
            .find(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong));
        let triplet_tile = scored_triplet.and_then(|s| {
            s.tile_ids
                .first()
                .and_then(|id| scoring_tiles.iter().find(|t| t.id == *id))
                .copied()
        });
        if let Some(ref tt) = triplet_tile {
            if let Some(matching) = self.wall.draw_matching(tt.suit, tt.rank) {
                self.hand.push(matching);
                bus.push(GameEvent::TileDrawn(matching));
                self.relic_activations.push(RelicId::SetMagnet);
            }
        }
    }

    /// Score and clear structure (does not consume a play).
    pub fn trigger_structure(&mut self, kind: StructureTriggerKind, bus: &mut EventBus) -> u64 {
        if self.structure_sets.is_empty() {
            return 0;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        if kind == StructureTriggerKind::Manual
            && !can_trigger_structure(
                &self.structure_tiles,
                &self.structure_sets,
                rw,
                &self.available_yaku,
                &self.round_rules,
            )
        {
            return 0;
        }

        let sets = self.structure_sets.clone();
        let scoring_tiles = self.structure_tiles.clone();
        let original_for_wildcard = scoring_tiles.clone();

        let meta = StructureTriggerMeta {
            kind,
            meld_count: sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        };

        let earned = self.apply_scored_melds(
            sets.clone(),
            scoring_tiles.clone(),
            original_for_wildcard,
            Some(meta),
            bus,
        );

        self.structure_sets.clear();
        self.structure_tiles.clear();

        self.set_magnet_after_score(&sets, &scoring_tiles, bus);
        self.hand.sort();

        earned
    }

    fn try_autotrigger_structure_full(&mut self, bus: &mut EventBus) {
        if !self.auto_cash_in_on_full_structure {
            return;
        }
        if self.structure_sets.is_empty() {
            return;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        if !is_winning_structure_shape(&self.structure_tiles, &self.structure_sets) {
            return;
        }
        if !can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            &self.available_yaku,
            &self.round_rules,
        ) {
            return;
        }
        let _ = self.trigger_structure(StructureTriggerKind::AutoFull, bus);
    }

    fn emit_round_resolution_events(&mut self, bus: &mut EventBus) {
        bus.push(GameEvent::ScoreUpdated(self.round_score));
        if self.round_score >= self.target_score as u64 {
            let base_reward = self.blind.clear_reward();
            let unused_play_bonus = self.plays_remaining;
            let interest = (self.gold.max(0) as u32 / 5).min(3);
            let green_luck_bonus =
                if self.relics.has(RelicId::GreenLuck) && !self.honors_scored_this_round {
                    self.relic_activations.push(RelicId::GreenLuck);
                    4
                } else {
                    0
                };
            let gold_idol_bonus = if self.relics.has(RelicId::GoldIdol) {
                self.relic_activations.push(RelicId::GoldIdol);
                3u32
            } else {
                0
            };
            let jade_abacus_bonus = if self.relics.has(RelicId::JadeAbacus) {
                let bonus = (self.gold.max(0) as u32 / 4).min(4);
                if bonus > 0 {
                    self.relic_activations.push(RelicId::JadeAbacus);
                }
                bonus
            } else {
                0
            };
            let patience_bonus = if self.relics.has(RelicId::Patience) {
                let bonus = 2 * self.discards_remaining;
                if bonus > 0 {
                    self.relic_activations.push(RelicId::Patience);
                }
                bonus
            } else {
                0
            };
            let gold_earned = base_reward
                .saturating_add(unused_play_bonus)
                .saturating_add(interest)
                .saturating_add(green_luck_bonus)
                .saturating_add(gold_idol_bonus)
                .saturating_add(jade_abacus_bonus)
                .saturating_add(patience_bonus);
            bus.push(GameEvent::RoundComplete {
                reached_target: true,
                payout: crate::game::event_bus::RoundPayout {
                    base_reward,
                    unused_play_bonus,
                    interest,
                    green_luck_bonus,
                    total: gold_earned,
                },
            });
        } else if self.plays_remaining == 0 {
            bus.push(GameEvent::GameOver {
                final_score: self.round_score,
            });
        }
    }

    /// Banked meld chips in structure (for HUD tiers).
    pub fn structure_banked_meld_chips(&self) -> i32 {
        banked_meld_chips(&self.structure_sets)
    }

    /// Whether [`Self::trigger_structure_manual`] can score (structure non-empty and rules allow).
    pub fn can_trigger_structure_now(&self) -> bool {
        if !self.mode.structure_bank || self.structure_sets.is_empty() {
            return false;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            &self.available_yaku,
            &self.round_rules,
        )
    }

    /// Read-only scoring breakdown for a manual structure cash-in (no state change).
    /// RNG-driven relic hooks in a real [`Self::trigger_structure`] may differ slightly.
    pub fn preview_manual_trigger_breakdown(&self) -> Option<ScoreBreakdown> {
        if !self.mode.structure_bank || self.structure_sets.is_empty() {
            return None;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        if !can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            &self.available_yaku,
            &self.round_rules,
        ) {
            return None;
        }
        let sets = self.structure_sets.clone();
        let scoring_tiles = self.structure_tiles.clone();
        let original_for_wildcard = scoring_tiles.clone();
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        let meta = StructureTriggerMeta {
            kind: StructureTriggerKind::Manual,
            meld_count: sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        };
        let ctx = ScoreContext {
            relics: &self.relics,
            tile_debuffs: &scoring_tile_debuffs,
            scored_last_turn: self.scored_last_turn,
            dora_faces: self.wall.dora_faces(),
            available_yaku: self.available_yaku.clone(),
            round_wind: rw,
            first_full_hand_of_round: !self.full_hand_played_this_round,
            plays_used: self.round_play_cap().saturating_sub(self.plays_remaining),
            riichi_active: false,
            yaku_levels: Some(self.yaku_levels.clone()),
            played_yaku_this_round: self.played_yaku_this_round.clone(),
            gold: self.gold,
            total_score: self.total_score_earned,
            is_final_play: self.plays_remaining == 0,
            tile_polisher_bonus: self.tile_polisher_bonus,
            relic_counters: self.relic_counters.clone(),
            unscored_hand_tiles: self.hand.len(),
            river_runner_bonus: self.river_runner_bonus,
            structure: Some(meta),
        };
        Some(score_sets_with_original(
            &scoring_tiles,
            &sets,
            &ctx,
            &self.round_rules,
            &original_for_wildcard,
        ))
    }

    /// Read-only preview of points from a manual structure cash-in (no state change).
    /// RNG-driven relic hooks in a real [`Self::trigger_structure`] may differ slightly.
    pub fn preview_manual_trigger_total(&self) -> u64 {
        self.preview_manual_trigger_breakdown()
            .map(|breakdown| breakdown.total)
            .unwrap_or(0)
    }

    /// Manual structure cash-in (no play cost) + round resolution events.
    pub fn trigger_structure_manual(&mut self, bus: &mut EventBus) -> u64 {
        let earned = self.trigger_structure(StructureTriggerKind::Manual, bus);
        self.emit_round_resolution_events(bus);
        earned
    }

    /// Mystery-preserving score preview for the current selection.
    /// Returns `None` if the selection is empty or doesn't decompose into melds.
    /// Honors wildcard relics so the preview matches what an actual play would score.
    #[allow(dead_code)]
    pub fn preview_selection(&self) -> Option<ScorePreview> {
        if self.selected_count() == 0 {
            return None;
        }
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        let (sets, scoring_tiles) = self.try_validate_with_wildcards(&selected_tiles)?;
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        Some(preview_score(
            &scoring_tiles,
            &sets,
            &self.available_yaku,
            &scoring_tile_debuffs,
            Some(&selected_tiles),
        ))
    }

    /// Check if the current selection forms a valid playable hand.
    pub fn is_selection_valid(&self) -> bool {
        if self.selected_count() == 0 {
            return false;
        }
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        self.try_validate_with_wildcards(&selected_tiles).is_some()
    }

    /// Try validating tiles, applying JokerTile / WildWinds substitutions if needed.
    /// Returns the decomposition and the (possibly modified) tiles used for scoring.
    pub fn try_validate_with_wildcards(
        &self,
        tiles: &[Tile],
    ) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
        let validation_rules: Vec<RuleModifier> = if self.mode.structure_bank {
            self.round_rules
                .iter()
                .copied()
                .filter(|rule| *rule != RuleModifier::RequireHonor)
                .collect()
        } else {
            self.round_rules.clone()
        };
        // Try standard validation first.
        if let Some(sets) = validate_selection_with_rules(tiles, &validation_rules) {
            return Some((sets, tiles.to_vec()));
        }

        // JokerTile: try substituting one tile with each possible face.
        if self.relics.has(RelicId::JokerTile) && !self.joker_used {
            if let Some(result) = try_joker_substitution(tiles, &validation_rules) {
                return Some(result);
            }
        }

        // WildWinds: try substituting wind tiles.
        if self.relics.has(RelicId::WildWinds) {
            if let Some(result) = try_wind_substitution(tiles, &validation_rules) {
                return Some(result);
            }
        }

        None
    }

    /// Toggle whether a hand tile is marked for discard.
    pub fn toggle_select(&mut self, index: usize) {
        if index < self.selected.len() {
            self.selected[index] = !self.selected[index];
        }
    }

    /// Clear all selections.
    pub fn clear_selection(&mut self) {
        self.selected.iter_mut().for_each(|s| *s = false);
    }

    /// How many tiles are currently selected for discard.
    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    /// Discard all selected tiles (costs 1 discard), then auto-draw back to HAND_SIZE.
    /// Returns the number of tiles discarded, or 0 if nothing was selected or no discards left.
    pub fn discard_selected(&mut self, bus: &mut EventBus) -> usize {
        let count = self.discard_selected_no_refill(bus);
        if count > 0 {
            self.refill_hand(bus);
        }
        count
    }

    /// Remove all selected tiles and decrement the discard counter, but do NOT
    /// auto-draw replacements. The caller is responsible for invoking
    /// `refill_hand` once the discard departure animation has had time to play.
    /// Returns the number of tiles removed, or 0 if nothing was selected or no
    /// discards remain.
    pub fn discard_selected_no_refill(&mut self, bus: &mut EventBus) -> usize {
        // Tutorial: block discards until the lesson enables them.
        if !self.tutorial_discard_allowed() {
            return 0;
        }
        if self.discards_remaining == 0 {
            return 0;
        }
        let count = self.selected_count();
        if count == 0 {
            return 0;
        }

        // Remove selected tiles in reverse order to keep indices valid.
        let indices: Vec<usize> = self
            .selected
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .map(|(i, _)| i)
            .rev()
            .collect();
        for &i in &indices {
            self.hand.remove(i);
            bus.push(GameEvent::TileDiscarded { slot_index: i });
        }
        self.discards_remaining -= 1;
        self.selected = vec![false; self.hand.len()];

        // Tutorial milestone: celebrate the first discard.
        if let Some(ref mut tut) = self.tutorial {
            if tut.celebrate(crate::game::tutorial::TutorialMilestone::FirstDiscard) {
                bus.push(GameEvent::TutorialMilestone(
                    crate::game::tutorial::TutorialMilestone::FirstDiscard,
                ));
            }
        }

        // Silk Thread: -0.3 mult (stored as -3 in ×10 units) per discard.
        if self.relics.has(RelicId::SilkThread) {
            self.relic_activations.push(RelicId::SilkThread);
            let v = self.relic_counters.entry(RelicId::SilkThread).or_insert(40);
            *v = (*v - 3).max(0);
            if *v == 0 {
                self.relics.active.retain(|&r| r != RelicId::SilkThread);
                self.relic_counters.remove(&RelicId::SilkThread);
            }
        }

        count
    }

    /// Draw tiles from the wall until the hand is full, then sort and reset
    /// the selection vector to match the new hand size. Honors boss-induced
    /// hand-size shrinks (e.g. The Whisper).
    pub fn refill_hand(&mut self, bus: &mut EventBus) {
        let target = boss::effective_hand_size(self);
        while self.hand.len() < target {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }
        // Shanten Shove: if the refilled hand is at tenpai, draw 1 bonus tile.
        if self.relics.has(crate::core::relic::RelicId::ShantenShove)
            && crate::core::shanten::shanten_estimate(&self.hand) == 0
        {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
                bus.push(GameEvent::TileDrawn(t));
                self.relic_activations
                    .push(crate::core::relic::RelicId::ShantenShove);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        // Re-seed guaranteed melds so the tutorial lesson stays solvable
        // after replacement tiles are drawn from the wall.
        self.seed_tutorial_hand();
        // Re-apply persistent enhancements to any newly-drawn tiles.
        self.restamp_hand_enhancements();
    }

    /// Swap two tiles in the hand by index. Clears selection afterward.
    pub fn swap_tiles(&mut self, from: usize, to: usize) {
        if from < self.hand.len() && to < self.hand.len() && from != to {
            self.hand.swap(from, to);
            self.selected = vec![false; self.hand.len()];
        }
    }

    /// Sort hand by suit then rank (Characters → Bamboos → Circles → Wind → Dragon).
    pub fn sort_hand_by_suit(&mut self) {
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
    }

    /// Sort hand by rank then suit (all 1s, all 2s, … then honors).
    pub fn sort_hand_by_rank(&mut self) {
        self.hand.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(a.suit.cmp(&b.suit))
                .then(a.id.cmp(&b.id))
        });
        self.selected = vec![false; self.hand.len()];
    }

    /// Evaluate meld patterns for UI hints.
    #[allow(dead_code)]
    pub fn hint_sets(&self) -> usize {
        detect_all_sets(&self.hand).len()
    }

    /// Add the chosen relic, scale up the base target, and reset for the next round.
    /// The actual target_score is set later by `apply_blind`.
    ///
    /// Balatro-style ante progression: `base_target` is the *ante's* base, and the
    /// Small/Big/Boss multipliers in `apply_blind` derive each blind's actual target.
    /// We only grow `base_target` when the player defeats the Boss and rolls into the
    /// next ante; within an ante, the base stays put.
    pub fn advance_round(&mut self, _bus: &mut EventBus) {
        // Fortune's Favor halves destruction chances (doubles survival).
        let fortunes = self.relics.has(RelicId::FortunesFavor);
        // Paper Lantern: 1-in-5 chance to burn up at round end. When it
        // burns, it's replaced in-place by Iron Lantern.
        // Fortune's Favor: 1-in-10 instead.
        if self.relics.has(RelicId::PaperLantern) {
            use rand::RngExt;
            let mut rng = rand::rng();
            let denom = if fortunes { 10 } else { 5 };
            if rng.random_ratio(1, denom) {
                if let Some(pos) = self
                    .relics
                    .active
                    .iter()
                    .position(|&r| r == RelicId::PaperLantern)
                {
                    self.relics.active[pos] = RelicId::IronLantern;
                }
            }
        }
        // Iron Lantern: 1-in-1000 chance to shatter at round end.
        // Fortune's Favor: 1-in-2000.
        if self.relics.has(RelicId::IronLantern) {
            use rand::RngExt;
            let mut rng = rand::rng();
            let denom = if fortunes { 2000 } else { 1000 };
            if rng.random_ratio(1, denom) {
                self.relics.active.retain(|&r| r != RelicId::IronLantern);
            }
        }
        // Nest Egg: increment rounds held (affects sell value).
        if self.relics.has(RelicId::NestEgg) {
            *self.relic_counters.entry(RelicId::NestEgg).or_insert(0) += 1;
            self.relic_activations.push(RelicId::NestEgg);
        }
        // Phantom Relic: increment rounds held.
        if self.relics.has(RelicId::PhantomRelic) {
            *self
                .relic_counters
                .entry(RelicId::PhantomRelic)
                .or_insert(0) += 1;
            self.relic_activations.push(RelicId::PhantomRelic);
        }
        // Obsession: check if the player's most-used yaku was NOT scored
        // this round. If so, increment the counter.
        if self.relics.has(RelicId::Obsession) {
            let top_yaku = self
                .yaku_times_played
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(&y, _)| y);
            if let Some(top) = top_yaku {
                if !self.played_yaku_this_round.contains(&top) {
                    *self.relic_counters.entry(RelicId::Obsession).or_insert(0) += 1;
                    self.relic_activations.push(RelicId::Obsession);
                } else {
                    // Reset on use — rewards variety, not just avoidance.
                    self.relic_counters.insert(RelicId::Obsession, 0);
                }
            }
        }

        // Defeating the Boss completes an ante and scales the base for the next one.
        let was_boss = self.blind == BlindKind::Boss;
        if was_boss {
            self.ante += 1;
            self.base_target = (self.base_target as f32 * self.mode.target_scaling) as u32;
        }
        self.run_number += 1;
        self.target_score = self.base_target; // will be overridden by apply_blind
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_used = false;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.boss.bonus_hand_size = 0;
        self.boss.gold_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.upcoming_blind = self.upcoming_blind.next();
        self.blind = self.upcoming_blind;
        self.hand.clear();
        self.selected.clear();
        self.structure_sets.clear();
        self.structure_tiles.clear();
        self.tag_bonus_hand_size = 0;

        // Tutorial: advance to the next lesson and apply its overrides.
        // This may resize the hand and adjust the target.
        if self.tutorial.as_ref().is_some_and(|t| t.is_active()) {
            self.advance_tutorial_lesson();
        }

        // Roll the next ante's boss when we cross an ante boundary. Final
        // ante draws from the dedicated final pool; everyone else draws
        // without replacement from the regular pool.
        if was_boss {
            let mut rng = rand::rng();
            self.boss.upcoming = if self.ante == FINAL_ANTE {
                Some(boss::pick_final(&mut rng))
            } else if self.ante > FINAL_ANTE {
                None
            } else {
                boss::pick_for_ante(&mut self.boss.pool_remaining, self.ante, &mut rng)
            };
            // Bake the resolved effect now so reactive bosses see the
            // post-shop run state of the *outgoing* ante (their reveal
            // moment) and pick_blind shows the chosen variant immediately.
            self.resolve_upcoming_boss();
            // Roll fresh skip-reward tags for the new ante. Shop-oriented
            // rewards must survive into the post-boss shop, but any
            // one-blind combat bonuses should expire at the ante boundary.
            self.roll_ante_tags();
            self.clear_next_blind_tag_modifiers();
        }
    }

    /// Skip the upcoming blind: advance to the next in the cycle without
    /// playing or visiting the shop. Resets per-round state. Skipping is
    /// not allowed for the Boss blind — callers should check first.
    pub fn skip_to_next_blind(&mut self) {
        self.upcoming_blind = self.upcoming_blind.next();
        self.run_number += 1;
        // Skipping stays inside the same ante (Boss can't be skipped), so the
        // ante's base target is unchanged — only the blind multiplier shifts.
        self.target_score = self.base_target;
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_used = false;
        self.joker_used = false;
        // Reset per-round boss-effect state. The ante's `upcoming_boss` is
        // unchanged — skipping a Small/Big still leaves the same boss waiting.
        self.boss.bonus_hand_size = 0;
        self.boss.gold_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.blind = self.upcoming_blind;
        self.hand.clear();
        self.selected.clear();
        self.structure_sets.clear();
        self.structure_tiles.clear();
    }

    // ── Skip-reward tags ──────────────────────────────────────────────

    /// Roll fresh tags for the Small and Big blinds of the current ante.
    pub fn roll_ante_tags(&mut self) {
        use crate::core::tag::roll_tag;
        let small = roll_tag(self.ante, None);
        let big = roll_tag(self.ante, Some(small));
        self.small_blind_tag = Some(small);
        self.big_blind_tag = Some(big);
    }

    /// Return the tag assigned to the given blind, if any.
    pub fn tag_for_blind(&self, blind: BlindKind) -> Option<crate::core::tag::TagKind> {
        match blind {
            BlindKind::Small => self.small_blind_tag,
            BlindKind::Big => self.big_blind_tag,
            BlindKind::Boss => None,
        }
    }

    /// Apply a skip-reward tag's effect. Returns a short description for UI feedback.
    pub fn apply_tag(&mut self, tag: crate::core::tag::TagKind) -> &'static str {
        use crate::core::tag::TagKind;
        match tag {
            TagKind::GoldIngot => {
                self.gold = self.gold.saturating_add(8);
                "+8 gold"
            }
            TagKind::TreasureChest => {
                self.gold = self.gold.saturating_add(20);
                "+20 gold"
            }
            TagKind::FreeReroll => {
                self.tag_free_reroll = true;
                "Free reroll"
            }
            TagKind::PatronGift => {
                self.tag_patron_gift = true;
                "Free relic"
            }
            TagKind::RichStock => {
                self.tag_rich_stock = true;
                "+2 shop relics"
            }
            TagKind::ZodiacBlessing => {
                use crate::core::zodiac::ZodiacKind;
                use rand::seq::IndexedRandom;
                let all = ZodiacKind::all();
                let mut rng = rand::rng();
                if let Some(&z) = all.choose(&mut rng) {
                    let yaku = z.yaku();
                    let new_level = self.yaku_levels.level_up(yaku);
                    self.pending_zodiac_celebration = Some((z, yaku, new_level));
                    return "Zodiac activated";
                }
                "No zodiac"
            }
            TagKind::RelicOffering => {
                use crate::core::relic::all_relic_defs;
                use rand::seq::SliceRandom;
                let defs = all_relic_defs();
                let mut pool: Vec<_> = defs.iter().filter(|d| !self.relics.owns(d.id)).collect();
                if pool.is_empty() || self.relics.is_full() {
                    self.gold = self.gold.saturating_add(6);
                    return "+6 gold (full)";
                }
                let mut rng = rand::rng();
                pool.shuffle(&mut rng);
                self.relics.active.push(pool[0].id);
                "Relic gained"
            }
            TagKind::BonusPlay => {
                self.tag_bonus_plays += 1;
                "+1 play"
            }
            TagKind::BonusDiscard => {
                self.tag_bonus_discards += 1;
                "+1 discard"
            }
            TagKind::WideHand => {
                self.tag_bonus_hand_size += 2;
                "+2 hand size"
            }
        }
    }

    /// Clear transient skip-tag bonuses that only apply to the very next blind.
    fn clear_next_blind_tag_modifiers(&mut self) {
        self.tag_bonus_plays = 0;
        self.tag_bonus_discards = 0;
        self.tag_bonus_hand_size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::build_wall;
    use crate::core::hand::DetectedSet;

    /// Standard mode starting plays (Bamboo: 4 base + 1 bonus).
    const STARTING_PLAYS: u32 = 5;
    /// Standard mode starting discards (Bamboo: 4 base + 0 bonus).
    const STARTING_DISCARDS: u32 = 4;

    // Create a RunState with a deterministic (unshuffled) wall for predictable tests.
    fn test_run() -> RunState {
        let tiles = build_wall(); // deterministic order: Char 1-9, Bam 1-9, Cir 1-9, Winds, Dragons
        let mut wall = Wall::from_unshuffled(tiles);
        let mut hand = Vec::with_capacity(HAND_SIZE);
        for _ in 0..HAND_SIZE {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        let selected = vec![false; hand.len()];
        let mode = GameMode {
            starting_gold: 0,
            starting_rules: vec![],
            starting_yaku: vec![],
            ..GameMode::standard()
        };
        RunState {
            ante: 1,
            available_rules: vec![],
            available_yaku: vec![],
            base_target: mode.base_target,
            blind: BlindKind::Small,
            boss: BossState::default(),
            consumables: crate::core::consumable::ConsumableInventory::default(),
            discards_remaining: mode.starting_discards,
            discards_max: mode.starting_discards,
            full_hand_played_this_round: false,
            gold: mode.starting_gold as i32,
            hand,
            structure_sets: vec![],
            structure_tiles: vec![],
            joker_used: false,
            last_breakdown: None,
            mode: mode.clone(),
            auto_cash_in_on_full_structure: true,
            played_yaku_this_round: vec![],
            tile_debuffs: vec![],
            honors_scored_this_round: false,
            yaku_times_played: std::collections::HashMap::new(),
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
            quickdraw_used: false,
            relics: RelicState::default(),
            round_rules: vec![],
            round_score: 0,
            run_number: 1,
            scored_last_turn: false,
            selected,
            target_score: mode.base_target,
            tile_enhancements: BTreeMap::new(),
            removed_tile_ids: std::collections::HashSet::new(),
            upcoming_blind: BlindKind::Small,
            wall,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            tile_packs: vec![],
            total_score_earned: 0,
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebration: None,
            tile_polisher_bonus: 0,
            relic_counters: BTreeMap::new(),
            river_runner_bonus: 0,
            tutorial: None,
            relic_activations: Vec::new(),
        }
    }

    fn bus() -> EventBus {
        EventBus::default()
    }

    fn winning_structure() -> (Vec<Tile>, Vec<DetectedSet>) {
        let tiles = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 1, 2),
            Tile::new(Suit::Characters, 2, 3),
            Tile::new(Suit::Characters, 3, 4),
            Tile::new(Suit::Characters, 4, 5),
            Tile::new(Suit::Circles, 2, 6),
            Tile::new(Suit::Circles, 3, 7),
            Tile::new(Suit::Circles, 4, 8),
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 6, 10),
            Tile::new(Suit::Bamboos, 7, 11),
            Tile::new(Suit::Wind, 1, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Wind, 1, 14),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        (tiles, sets)
    }

    // ── toggle_select ───────────────────────────────────────────────

    #[test]
    fn toggle_select_marks_tile() {
        let mut run = test_run();
        assert!(!run.selected[0]);
        run.toggle_select(0);
        assert!(run.selected[0]);
    }

    #[test]
    fn toggle_select_unmarks_tile() {
        let mut run = test_run();
        run.toggle_select(3);
        assert!(run.selected[3]);
        run.toggle_select(3);
        assert!(!run.selected[3]);
    }

    #[test]
    fn toggle_select_out_of_bounds_is_noop() {
        let mut run = test_run();
        run.toggle_select(999); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn toggle_select_multiple_tiles() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(5);
        run.toggle_select(13);
        assert_eq!(run.selected_count(), 3);
    }

    // ── clear_selection ─────────────────────────────────────────────

    #[test]
    fn clear_selection_resets_all() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(7);
        run.toggle_select(12);
        assert_eq!(run.selected_count(), 3);
        run.clear_selection();
        assert_eq!(run.selected_count(), 0);
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn clear_selection_on_empty_is_noop() {
        let mut run = test_run();
        run.clear_selection(); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    // ── selected_count ──────────────────────────────────────────────

    #[test]
    fn selected_count_starts_at_zero() {
        let run = test_run();
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn selected_count_tracks_toggles() {
        let mut run = test_run();
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
        run.toggle_select(1);
        assert_eq!(run.selected_count(), 2);
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
    }

    // ── discard_selected ────────────────────────────────────────────

    #[test]
    fn discard_selected_removes_tiles_and_redraws() {
        let mut run = test_run();
        let mut bus = bus();
        let original_hand = run.hand.clone();

        run.toggle_select(0);
        run.toggle_select(1);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 2);
        assert_eq!(run.hand.len(), HAND_SIZE); // auto-drew back to full
        // The first two tiles should be gone.
        assert!(!run.hand.contains(&original_hand[0]));
        assert!(!run.hand.contains(&original_hand[1]));
    }

    #[test]
    fn discard_selected_costs_one_discard() {
        let mut run = test_run();
        let mut bus = bus();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);

        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
    }

    #[test]
    fn discard_selected_clears_selection_after() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(0);
        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    #[test]
    fn discard_selected_returns_zero_when_none_selected() {
        let mut run = test_run();
        let mut bus = bus();
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, 0);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS); // not decremented
    }

    #[test]
    fn discard_selected_returns_zero_when_no_discards_left() {
        let mut run = test_run();
        let mut bus = bus();
        run.discards_remaining = 0;

        run.toggle_select(0);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 0);
        assert_eq!(run.hand.len(), HAND_SIZE); // hand unchanged
    }

    #[test]
    fn discard_selected_emits_events() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(2);
        run.toggle_select(4);
        run.discard_selected(&mut bus);

        let events: Vec<_> = bus.drain().collect();
        // Should have TileDiscarded events + TileDrawn events for the redraws.
        let discards: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDiscarded { .. }))
            .collect();
        let draws: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDrawn(_)))
            .collect();
        assert_eq!(discards.len(), 2);
        assert_eq!(draws.len(), 2); // drew 2 to replace the 2 discarded
    }

    #[test]
    fn discard_selected_preserves_non_selected_tiles() {
        let mut run = test_run();
        let mut bus = bus();

        // Remember non-selected tile ids.
        let kept_ids: Vec<u32> = run
            .hand
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 3 && *i != 7)
            .map(|(_, t)| t.id)
            .collect();

        run.toggle_select(3);
        run.toggle_select(7);
        run.discard_selected(&mut bus);

        // All originally-kept tiles should still be in hand.
        for id in &kept_ids {
            assert!(
                run.hand.iter().any(|t| t.id == *id),
                "tile id {} was lost",
                id
            );
        }
    }

    #[test]
    fn multiple_discard_rounds() {
        let mut run = test_run();
        let mut bus = bus();

        // First discard: remove 3 tiles.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);

        // Second discard: remove 1 tile.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 2);

        // Third discard: remove 5 tiles.
        for i in 0..5 {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 3);

        // Fourth discard: removes the last allowance.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.discards_remaining, 0);

        // Fifth attempt: should fail (no discards left).
        run.toggle_select(0);
        let result = run.discard_selected(&mut bus);
        assert_eq!(result, 0);
        assert_eq!(run.discards_remaining, 0);
    }

    #[test]
    fn discard_all_14_tiles_redraws_full_hand() {
        let mut run = test_run();
        let mut bus = bus();

        for i in 0..HAND_SIZE {
            run.toggle_select(i);
        }
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, HAND_SIZE);
        assert_eq!(run.hand.len(), HAND_SIZE); // wall has 136 - 14 = 122 tiles, plenty to redraw
    }

    // ── auto-draw with depleted wall ────────────────────────────────

    #[test]
    fn discard_with_depleted_wall_draws_what_it_can() {
        let mut run = test_run();
        let mut bus = bus();

        // Drain the wall almost completely: wall started with 140, 14 already drawn.
        // Draw remaining 126 tiles to exhaust the wall.
        for _ in 0..126 {
            run.wall.draw();
        }
        assert!(run.wall.draw().is_none()); // wall is empty

        run.toggle_select(0);
        run.toggle_select(1);
        run.discard_selected(&mut bus);

        // Can't redraw, so hand is now 12.
        assert_eq!(run.hand.len(), HAND_SIZE - 2);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    // ── selected vec stays in sync ──────────────────────────────────

    #[test]
    fn selected_vec_length_matches_hand_after_discard() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected.len(), run.hand.len());
        // All should be false after discard.
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn selected_vec_length_matches_hand_at_init() {
        let run = test_run();
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.selected.len(), HAND_SIZE);
    }

    // ── advance_round resets selection ───────────────────────────────

    #[test]
    fn advance_round_resets_selection() {
        let mut run = test_run();
        let mut bus = bus();
        run.toggle_select(0);
        run.toggle_select(5);
        assert_eq!(run.selected_count(), 2);

        run.advance_round(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);
    }

    #[test]
    fn apply_blind_rebuilds_round_resources_from_current_bonuses() {
        let mut run = test_run();
        run.plays_remaining = 1;
        run.discards_remaining = 0;
        run.relics.active.push(RelicId::SecondWind);
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.plays_remaining, STARTING_PLAYS + 2);
        assert_eq!(run.plays_max, STARTING_PLAYS + 2);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 1);
        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn second_wind_round_cap_does_not_accumulate_across_round_transitions() {
        let mut run = test_run();
        let mut bus = bus();
        run.relics.active.push(RelicId::SecondWind);

        run.apply_blind(BlindKind::Small);
        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);

        run.advance_round(&mut bus);
        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);

        run.apply_blind(BlindKind::Big);
        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);
    }

    #[test]
    fn second_wind_plays_used_uses_effective_round_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::SecondWind);
        run.apply_blind(BlindKind::Small);
        run.plays_remaining -= 2;

        let rw = Some(BlindKind::round_wind_for_ante(run.ante));
        let ctx = ScoreContext {
            relics: &run.relics,
            tile_debuffs: &[],
            scored_last_turn: run.scored_last_turn,
            dora_faces: run.wall.dora_faces(),
            available_yaku: run.available_yaku.clone(),
            round_wind: rw,
            first_full_hand_of_round: !run.full_hand_played_this_round,
            plays_used: run.round_play_cap().saturating_sub(run.plays_remaining),
            riichi_active: false,
            yaku_levels: Some(run.yaku_levels.clone()),
            played_yaku_this_round: run.played_yaku_this_round.clone(),
            gold: run.gold,
            total_score: run.total_score_earned,
            is_final_play: run.plays_remaining == 0,
            tile_polisher_bonus: run.tile_polisher_bonus,
            relic_counters: run.relic_counters.clone(),
            unscored_hand_tiles: run.hand.len(),
            river_runner_bonus: run.river_runner_bonus,
            structure: None,
        };

        assert_eq!(ctx.plays_used, 2);
    }

    #[test]
    fn apply_blind_uses_material_starting_discards_before_skip_bonus() {
        let mut run = RunState::new(GameMode::with_material(
            crate::persistence::TileMaterial::Plastic,
        ));
        run.discards_remaining = 0;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.discards_remaining, 6);
        assert_eq!(run.discards_max, 6);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn apply_blind_tracks_reduced_round_caps_for_boss_taxes() {
        let mut run = test_run();
        run.boss.effect = Some(crate::core::boss::ResolvedBossEffect::from_static(
            &crate::core::boss::BossKind::Drought.def().effect,
        ));

        run.apply_blind(BlindKind::Boss);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS / 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS / 2);
    }

    #[test]
    fn apply_blind_promotes_wide_hand_bonus_to_round_hand_size() {
        let mut run = test_run();
        run.apply_tag(crate::core::tag::TagKind::WideHand);

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn skipping_with_wide_hand_carries_bonus_into_next_blind() {
        let mut run = test_run();

        run.apply_tag(crate::core::tag::TagKind::WideHand);
        run.skip_to_next_blind();

        assert_eq!(run.tag_bonus_hand_size, 2);

        run.apply_blind(BlindKind::Big);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn advance_round_after_boss_preserves_pending_shop_skip_rewards() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
        run.tag_free_reroll = true;
        run.tag_patron_gift = true;
        run.tag_rich_stock = true;

        run.advance_round(&mut bus);

        assert!(run.tag_free_reroll);
        assert!(run.tag_patron_gift);
        assert!(run.tag_rich_stock);
    }

    #[test]
    fn advance_round_after_boss_clears_unconsumed_next_blind_skip_bonuses() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;
        run.tag_bonus_hand_size = 2;

        run.advance_round(&mut bus);

        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    // ── score_selected_tiles ──────────────────────────────────────

    #[test]
    fn score_selected_valid_triplet() {
        let mut run = test_run();
        let mut bus = bus();
        // Deterministic hand (sorted): 1m×4, 2m×4, 3m×4, 4m×2
        // Select first 3 tiles (1m, 1m, 1m) — a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let pts = run.score_selected_tiles(&mut bus);
        assert!(pts > 0, "valid triplet should score");
        assert_eq!(run.plays_remaining, STARTING_PLAYS - 1);
        // Scored tiles removed and redrawn.
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn classic_mode_scores_on_commit_without_structure_bank() {
        let mut run = test_run();
        run.mode.structure_bank = false;
        let mut bus = bus();
        let score_before = run.round_score;
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let pts = run.score_selected_tiles(&mut bus);
        assert!(pts > 0);
        assert!(
            run.round_score > score_before,
            "classic play adds to round score immediately"
        );
        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
    }

    #[test]
    fn dragon_allows_non_honor_play_but_debuffs_its_score() {
        let mut baseline = test_run();
        baseline.mode.structure_bank = false;
        baseline.blind = BlindKind::Boss;
        baseline.upcoming_blind = BlindKind::Boss;
        let mut baseline_bus = bus();
        baseline.toggle_select(0);
        baseline.toggle_select(1);
        baseline.toggle_select(2);
        let baseline_pts = baseline.score_selected_tiles(&mut baseline_bus);
        assert!(baseline_pts > 0);
        let baseline_score = baseline.round_score;

        let mut dragon = test_run();
        dragon.mode.structure_bank = false;
        dragon.blind = BlindKind::Boss;
        dragon.upcoming_blind = BlindKind::Boss;
        dragon.boss.upcoming = Some(BossKind::Dragon);
        let mut dragon_bus = bus();
        dragon.toggle_select(0);
        dragon.toggle_select(1);
        dragon.toggle_select(2);
        let dragon_pts = dragon.score_selected_tiles(&mut dragon_bus);
        assert!(dragon_pts > 0, "Dragon should still allow cycling plays");
        assert!(
            dragon.round_score > 0,
            "debuffed Dragon plays should still score something"
        );
        assert!(
            dragon.round_score < baseline_score,
            "Dragon should weaken non-honor plays instead of hard-locking them"
        );
    }

    #[test]
    fn full_structure_autocash_can_be_disabled() {
        let mut run = test_run();
        run.set_auto_cash_in_on_full_structure(false);
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert_eq!(run.structure_sets.len(), 5);
        assert_eq!(run.structure_tiles.len(), 14);
        assert_eq!(run.round_score, 0);
    }

    #[test]
    fn full_structure_autocash_defaults_on() {
        let mut run = test_run();
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
        assert!(run.round_score > 0);
    }

    #[test]
    fn score_selected_invalid_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        // Select 4 tiles: triplet + 1 leftover → invalid.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.toggle_select(4); // 2m — leftover
        let pts = run.score_selected_tiles(&mut bus);
        assert_eq!(pts, 0, "invalid selection should score 0");
        assert_eq!(run.plays_remaining, STARTING_PLAYS, "no play consumed");
        assert_eq!(run.hand.len(), HAND_SIZE, "hand unchanged");
    }

    #[test]
    fn score_selected_nothing_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        let pts = run.score_selected_tiles(&mut bus);
        assert_eq!(pts, 0);
        assert_eq!(run.plays_remaining, STARTING_PLAYS);
    }

    #[test]
    fn score_selected_removes_tiles_from_hand() {
        let mut run = test_run();
        let mut bus = bus();
        // Select a pair: indices 0 and 1 (1m, 1m).
        let tile0 = run.hand[0];
        let tile1 = run.hand[1];
        run.toggle_select(0);
        run.toggle_select(1);
        run.score_selected_tiles(&mut bus);
        // Those specific tiles should be gone.
        assert!(!run.hand.iter().any(|t| t.id == tile0.id));
        assert!(!run.hand.iter().any(|t| t.id == tile1.id));
    }

    #[test]
    fn is_selection_valid_reflects_state() {
        let mut run = test_run();
        assert!(!run.is_selection_valid(), "empty selection is invalid");
        // Select a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        assert!(run.is_selection_valid(), "triplet should be valid");
        // Add a leftover.
        run.toggle_select(4);
        assert!(!run.is_selection_valid(), "triplet + leftover is invalid");
    }

    // ── discard indices are correct (reverse removal) ───────────────

    #[test]
    fn discard_removes_correct_tiles_by_index() {
        let mut run = test_run();
        let mut bus = bus();

        let tile_at_2 = run.hand[2];
        let tile_at_10 = run.hand[10];

        run.toggle_select(2);
        run.toggle_select(10);
        run.discard_selected(&mut bus);

        // These specific tiles should no longer be in hand.
        assert!(!run.hand.iter().any(|t| t.id == tile_at_2.id));
        assert!(!run.hand.iter().any(|t| t.id == tile_at_10.id));
    }
}

/// All tile faces for substitution attempts.
const ALL_FACES: [(Suit, u8); 34] = {
    let mut faces = [(Suit::Characters, 0u8); 34];
    let mut i = 0;
    let suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];
    let mut si = 0;
    while si < 3 {
        let mut r = 1u8;
        while r <= 9 {
            faces[i] = (suits[si], r);
            i += 1;
            r += 1;
        }
        si += 1;
    }
    let mut r = 1u8;
    while r <= 4 {
        faces[i] = (Suit::Wind, r);
        i += 1;
        r += 1;
    }
    r = 1;
    while r <= 3 {
        faces[i] = (Suit::Dragon, r);
        i += 1;
        r += 1;
    }
    faces
};

/// Try substituting one tile with every possible face (JokerTile).
fn try_joker_substitution(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
    for (idx, _) in tiles.iter().enumerate() {
        for &(suit, rank) in &ALL_FACES {
            let mut modified = tiles.to_vec();
            modified[idx] = Tile::new(suit, rank, modified[idx].id);
            if let Some(sets) = validate_selection_with_rules(&modified, rules) {
                return Some((sets, modified));
            }
        }
    }
    None
}

/// Build the set of faces a wild wind tile could usefully become:
/// - Any face already in `tiles` (for pairs/triplets)
/// - Any numbered face within ±2 rank of a same-suit numbered tile (for sequences)
fn wind_candidate_faces(tiles: &[Tile]) -> Vec<(Suit, u8)> {
    use std::collections::HashSet;
    let mut candidates = HashSet::new();
    let number_suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];
    for t in tiles {
        // Exact face: could pair/triplet with existing tiles.
        candidates.insert((t.suit, t.rank));
        // Nearby ranks in numbered suits: could form a sequence.
        if number_suits.contains(&t.suit) {
            for delta in [-2i8, -1, 1, 2] {
                let r = t.rank as i8 + delta;
                if (1..=9).contains(&r) {
                    candidates.insert((t.suit, r as u8));
                }
            }
        }
    }
    // Remove wind/dragon faces that don't already appear — honor tiles can only
    // pair/triplet, so only faces already present are useful.
    candidates.retain(|&(s, _)| number_suits.contains(&s) || tiles.iter().any(|t| t.suit == s));
    candidates.into_iter().collect()
}

/// Try substituting wind tiles with other faces (WildWinds).
/// Recursively substitutes all wind tiles, pruning to only faces that could
/// participate in a meld with the other tiles in the hand.
fn try_wind_substitution(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
    let wind_indices: Vec<usize> = tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.suit == Suit::Wind)
        .map(|(i, _)| i)
        .collect();
    if wind_indices.is_empty() {
        return None;
    }
    let candidates = wind_candidate_faces(tiles);
    if candidates.is_empty() {
        return None;
    }
    fn substitute_recursive(
        tiles: &mut Vec<Tile>,
        wind_indices: &[usize],
        pos: usize,
        candidates: &[(Suit, u8)],
        rules: &[RuleModifier],
    ) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
        if pos == wind_indices.len() {
            return validate_selection_with_rules(tiles, rules).map(|sets| (sets, tiles.clone()));
        }
        let idx = wind_indices[pos];
        let original = tiles[idx];
        for &(suit, rank) in candidates {
            tiles[idx] = Tile::new(suit, rank, original.id);
            if let Some(result) =
                substitute_recursive(tiles, wind_indices, pos + 1, candidates, rules)
            {
                return Some(result);
            }
        }
        tiles[idx] = original;
        None
    }
    let mut modified = tiles.to_vec();
    substitute_recursive(&mut modified, &wind_indices, 0, &candidates, rules)
}

#[cfg(test)]
mod joker_tile_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn joker_completes_sequence() {
        // 1m 2m 5s — joker should turn 5s into 3m
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some(), "joker should complete the sequence");
        let (sets, modified) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
        // The modified tile should now be 3m
        assert_eq!(modified[2].suit, Suit::Characters);
        assert_eq!(modified[2].rank, 3);
    }

    #[test]
    fn joker_completes_triplet() {
        // 7p 7p 1s — joker should turn 1s into 7p
        let tiles = vec![
            tile(Suit::Circles, 7, 0),
            tile(Suit::Circles, 7, 1),
            tile(Suit::Bamboos, 1, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn joker_makes_pair_from_two_tiles() {
        // 1m 5s — joker turns 5s into 1m for a pair
        let tiles = vec![tile(Suit::Characters, 1, 0), tile(Suit::Bamboos, 5, 1)];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, SetKind::Pair);
    }

    #[test]
    fn joker_only_substitutes_one_tile() {
        // 1m 5s 9p — all different, need 2 subs to make a meld, joker can only do 1
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Circles, 9, 2),
        ];
        assert!(try_joker_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn joker_respects_no_sequences_rule() {
        // 1m 2m 5s — would be a sequence with joker, but NoSequences blocks it
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[RuleModifier::NoSequences]);
        // Could still work if joker turns 5s into 1m or 2m for a triplet — but
        // we only have 2 of those, so a triplet needs the joker tile to match one.
        // 1m 2m 1m → not a valid decomposition (pair 1m + leftover 2m).
        // 1m 2m 2m → pair 2m + leftover 1m. Also invalid.
        // No triplet possible, so should be None.
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod wild_wind_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn two_winds_substitute_into_sequences() {
        // Hand: 2m W 4m | 7m 8m 9m | 4s 5s 6s | 7p 8p W
        // With Wild Winds, W->3m and W->9p (or 6p) should yield 4 sequences.
        let tiles = vec![
            tile(Suit::Characters, 2, 1),
            tile(Suit::Wind, 3, 2), // West, should become 3m
            tile(Suit::Characters, 4, 3),
            tile(Suit::Characters, 7, 4),
            tile(Suit::Characters, 8, 5),
            tile(Suit::Characters, 9, 6),
            tile(Suit::Bamboos, 4, 7),
            tile(Suit::Bamboos, 5, 8),
            tile(Suit::Bamboos, 6, 9),
            tile(Suit::Circles, 7, 10),
            tile(Suit::Circles, 8, 11),
            tile(Suit::Wind, 3, 12), // West, should become 9p (or 6p)
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(
            result.is_some(),
            "two-wind substitution should find a valid hand"
        );
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 4);
        assert!(sets.iter().all(|s| s.kind == SetKind::Sequence));
    }

    #[test]
    fn single_wind_substitutes_into_sequence() {
        // 1m 2m W -> W becomes 3m
        let tiles = vec![
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
            tile(Suit::Wind, 1, 3), // East
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn wind_substitutes_into_triplet() {
        // 5s 5s W -> W becomes 5s for a triplet
        let tiles = vec![
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Bamboos, 5, 2),
            tile(Suit::Wind, 2, 3),
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn no_winds_returns_none() {
        let tiles = vec![
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
            tile(Suit::Characters, 3, 3),
        ];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn impossible_hand_returns_none() {
        // W alone can't form any meld
        let tiles = vec![tile(Suit::Wind, 1, 1)];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn candidates_include_nearby_ranks() {
        let tiles = vec![tile(Suit::Characters, 5, 1), tile(Suit::Wind, 3, 2)];
        let candidates = wind_candidate_faces(&tiles);
        // Should include 3m-7m (5 ± 2) and 5m itself
        for r in 3..=7 {
            assert!(
                candidates.contains(&(Suit::Characters, r)),
                "candidates should include {}m",
                r
            );
        }
        // Should NOT include 1m (too far)
        assert!(!candidates.contains(&(Suit::Characters, 1)));
    }
}
