//! Single-run state: wall, hand, score target, round modifiers.

use serde::{Deserialize, Serialize};

use crate::core::deck::Wall;
use crate::core::hand::{DetectedSet, SetKind, detect_all_sets, validate_selection_with_rules};

use crate::core::relic::{RelicId, RelicState, ScoreContext};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::{ScoreBreakdown, ScorePreview, preview_score, score_sets};
use crate::core::tile::{Suit, Tile};
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::GameMode;

pub const HAND_SIZE: usize = 14;
pub const STARTING_PLAYS: u32 = 4;
pub const STARTING_DISCARDS: u32 = 4;
/// Defeating the Boss of this ante completes the run (Balatro-style).
pub const FINAL_ANTE: u32 = 8;

#[derive(Debug, Serialize, Deserialize)]
pub struct RunState {
    pub wall: Wall,
    pub hand: Vec<Tile>,
    /// Which hand tiles are marked for discard (parallel with `hand`).
    pub selected: Vec<bool>,
    pub round_score: u32,
    pub target_score: u32,
    pub base_target: u32,
    pub relics: RelicState,
    pub round_rules: Vec<RuleModifier>,
    pub run_number: u32,
    /// Current ante (1-indexed). Increments after defeating each Boss blind.
    pub ante: u32,
    pub plays_remaining: u32,
    pub discards_remaining: u32,
    pub gold: u32,
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
    /// Player's current Zodiac card inventory.
    pub zodiac_inventory: crate::core::zodiac::ZodiacInventory,
    /// Active yaku loadout — these score at full strength. Yaku not in the
    /// loadout still detect but at 50% chip/mult per Patch B finishing's
    /// "amplification, not gating" rule. `FullHand` and `Yakuhai` are always
    /// implicitly full-strength regardless of loadout membership.
    pub yaku_loadout: Vec<crate::core::yaku::YakuKind>,
    /// Maximum loadout size (default 3, +1 from Yaku Scholar relic).
    pub yaku_loadout_capacity: usize,
    /// Game mode preset used for this run (drives advance_round resets).
    pub mode: GameMode,
}

impl RunState {
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

        Self {
            wall,
            hand,
            selected,
            round_score: 0,
            target_score: mode.base_target,
            base_target: mode.base_target,
            relics,
            round_rules: mode.starting_rules.clone(),
            run_number: 1,
            ante: 1,
            plays_remaining: mode.starting_plays,
            discards_remaining: mode.starting_discards,
            gold: mode.starting_gold,
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
            zodiac_inventory: crate::core::zodiac::ZodiacInventory::default(),
            yaku_loadout: vec![
                crate::core::yaku::YakuKind::Tanyao,
                crate::core::yaku::YakuKind::Toitoi,
                crate::core::yaku::YakuKind::Chinitsu,
            ],
            yaku_loadout_capacity: 3,
            mode,
        }
    }

    /// Convenience constructor using the standard game mode.
    pub fn new_demo() -> Self {
        Self::new(GameMode::standard())
    }

    /// Use a Zodiac card from the inventory: removes it and levels its yaku.
    /// Returns the yaku and its new level on success.
    #[allow(dead_code)]
    pub fn use_zodiac(&mut self, index: usize) -> Option<(crate::core::yaku::YakuKind, u32)> {
        let z = self.zodiac_inventory.take(index)?;
        let yaku = z.yaku();
        let new_level = self.yaku_levels.level_up(yaku);
        Some((yaku, new_level))
    }

    /// Try to add a Zodiac card to the inventory; returns `true` on success.
    #[allow(dead_code)]
    pub fn grant_zodiac(&mut self, z: crate::core::zodiac::ZodiacKind) -> bool {
        self.zodiac_inventory.try_push(z)
    }

    /// Replace one yaku in the loadout with another. The new yaku must not
    /// already be in the loadout. Returns `true` on success.
    #[allow(dead_code)]
    pub fn swap_loadout(&mut self, index: usize, replacement: crate::core::yaku::YakuKind) -> bool {
        if index >= self.yaku_loadout.len() {
            return false;
        }
        if self.yaku_loadout.contains(&replacement) {
            return false;
        }
        self.yaku_loadout[index] = replacement;
        true
    }

    /// Recompute Zodiac inventory capacity and yaku-loadout capacity from
    /// currently-owned relics. Idempotent — call after any relic add/remove.
    /// (Patch C: ZodiacPouch +1, LunarAlmanac +1, YakuScholar loadout +1.)
    pub fn recompute_capacities(&mut self) {
        let mut zodiac_cap = 2usize;
        if self.relics.has(RelicId::ZodiacPouch) {
            zodiac_cap += 1;
        }
        if self.relics.has(RelicId::LunarAlmanac) {
            zodiac_cap += 1;
        }
        self.zodiac_inventory.capacity = zodiac_cap;

        let loadout_cap = if self.relics.has(RelicId::YakuScholar) {
            4
        } else {
            3
        };
        self.yaku_loadout_capacity = loadout_cap;
    }

    /// Whether a run is in progress (not a fresh/default state).
    pub fn is_in_progress(&self) -> bool {
        self.round_score > 0 || self.run_number > 1 || self.gold != self.mode.starting_gold
    }

    /// True once the player has defeated the Boss of the final ante.
    pub fn is_run_complete(&self) -> bool {
        self.ante > FINAL_ANTE
    }

    /// Apply a blind choice: sets target score and any forced modifiers.
    pub fn apply_blind(&mut self, blind: BlindKind) {
        self.blind = blind;
        self.target_score = (self.base_target as f32 * blind.target_multiplier()) as u32;
        if let Some(modifier) = blind.forced_modifier(self.run_number) {
            if !self.round_rules.contains(&modifier) {
                self.round_rules.push(modifier);
            }
        }
        // ReducedPlays modifier reduces plays from 4 to 3.
        if self.round_rules.contains(&RuleModifier::ReducedPlays) {
            self.plays_remaining = self.plays_remaining.min(3);
        }
    }

    /// Score the currently-selected tiles as a played hand.
    /// Returns the points earned (0 if selection is invalid, empty, or no plays left).
    /// Valid selections decompose perfectly into melds (pairs, triplets, sequences)
    /// with no leftover tiles. Scored tiles are removed and replacements drawn.
    pub fn score_selected_tiles(&mut self, bus: &mut EventBus) -> u32 {
        if self.plays_remaining == 0 || self.selected_count() == 0 {
            return 0;
        }

        // Extract selected tiles.
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();

        // Validate: must decompose into melds with no leftovers (respecting rules).
        // Try wildcard substitution if JokerTile or WildWinds relics are active.
        let (sets, scoring_tiles) = match self.try_validate_with_wildcards(&selected_tiles) {
            Some(result) => result,
            None => return 0,
        };
        // If tiles were modified by JokerTile substitution, mark it used.
        if scoring_tiles != selected_tiles && self.relics.has(RelicId::JokerTile) {
            self.joker_used = true;
        }

        // Score the tiles (using substituted tiles if wildcards were applied).
        let ctx = ScoreContext {
            relics: &self.relics,
            scored_last_turn: self.scored_last_turn,
            dora_faces: self.wall.dora_faces(),
            available_yaku: self.available_yaku.clone(),
            round_wind: Some(BlindKind::round_wind_for_ante(self.ante)),
            first_full_hand_of_round: !self.full_hand_played_this_round,
            plays_used: self
                .mode
                .starting_plays
                .saturating_sub(self.plays_remaining),
            riichi_active: false,
            yaku_levels: Some(self.yaku_levels.clone()),
            yaku_loadout: self.yaku_loadout.clone(),
        };
        let breakdown = score_sets(&scoring_tiles, &sets, &ctx, &self.round_rules);
        let earned = breakdown.total.max(0) as u32;
        self.round_score = self.round_score.saturating_add(earned);
        // Latch the first-FullHand-of-round flag so the Tenpai Bonus only
        // fires once per round.
        let scored_full_hand = breakdown
            .detected_yaku
            .contains(&crate::core::yaku::YakuKind::FullHand);
        if scored_full_hand {
            self.full_hand_played_this_round = true;
        }
        // KanDrum (Patch C): every Kong scored grants +1 play this round.
        if self.relics.has(RelicId::KanDrum) {
            let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as u32;
            if kong_count > 0 {
                self.plays_remaining = self.plays_remaining.saturating_add(kong_count);
            }
        }
        self.last_breakdown = Some(breakdown);
        self.plays_remaining = self.plays_remaining.saturating_sub(1);
        self.scored_last_turn = earned > 0;

        // GreenLuck (Patch C retune): hands without honors earn +6 gold.
        if self.relics.has(RelicId::GreenLuck)
            && !selected_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        {
            self.gold = self.gold.saturating_add(6);
        }

        // EightTreasures (Patch C): scoring a FullHand grants a random Zodiac
        // (ignores inventory cap so the relic always feels good).
        if scored_full_hand && self.relics.has(RelicId::EightTreasures) {
            use rand::seq::IndexedRandom;
            let mut rng = rand::rng();
            if let Some(&z) = crate::core::zodiac::ZodiacKind::all().choose(&mut rng) {
                self.zodiac_inventory.items.push(z);
            }
        }

        // Check if any triplet (or kong) was scored (for SetMagnet).
        let scored_triplet = sets
            .iter()
            .find(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong));
        let triplet_tile = scored_triplet.and_then(|s| {
            s.tile_ids
                .first()
                .and_then(|id| selected_tiles.iter().find(|t| t.id == *id))
                .copied()
        });

        // Remove scored tiles from hand (reverse order to keep indices valid).
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

        // Auto-draw back to full hand.
        let draw_target = if self.relics.has(RelicId::QuickDraw) && !self.quickdraw_used {
            self.quickdraw_used = true;
            HAND_SIZE + 1
        } else {
            HAND_SIZE
        };
        while self.hand.len() < draw_target {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }

        // SetMagnet: scoring a triplet draws a matching tile from the wall.
        if self.relics.has(RelicId::SetMagnet) {
            if let Some(ref tt) = triplet_tile {
                if let Some(matching) = self.wall.draw_matching(tt.suit, tt.rank) {
                    self.hand.push(matching);
                    bus.push(GameEvent::TileDrawn(matching));
                }
            }
        }

        self.hand.sort();
        self.selected = vec![false; self.hand.len()];

        bus.push(GameEvent::ScoreUpdated(self.round_score));
        if self.round_score >= self.target_score {
            let excess_gold = (self.round_score.saturating_sub(self.target_score)) / 50;
            let gold_earned = ((5 + excess_gold) as f32 * self.blind.gold_multiplier()) as u32;
            self.gold = self.gold.saturating_add(gold_earned);
            bus.push(GameEvent::RoundComplete {
                reached_target: true,
            });
        } else if self.plays_remaining == 0 {
            bus.push(GameEvent::GameOver {
                final_score: self.round_score,
            });
        }
        earned
    }

    /// Mystery-preserving score preview for the current selection.
    /// Returns `None` if the selection is empty or doesn't decompose into melds.
    /// Honors wildcard relics so the preview matches what an actual play would score.
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
        Some(preview_score(&scoring_tiles, &sets, &self.available_yaku))
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
    fn try_validate_with_wildcards(&self, tiles: &[Tile]) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
        // Try standard validation first.
        if let Some(sets) = validate_selection_with_rules(tiles, &self.round_rules) {
            return Some((sets, tiles.to_vec()));
        }

        // JokerTile: try substituting one tile with each possible face.
        if self.relics.has(RelicId::JokerTile) && !self.joker_used {
            if let Some(result) = try_joker_substitution(tiles, &self.round_rules) {
                return Some(result);
            }
        }

        // WildWinds: try substituting wind tiles.
        if self.relics.has(RelicId::WildWinds) {
            if let Some(result) = try_wind_substitution(tiles, &self.round_rules) {
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
        count
    }

    /// Draw tiles from the wall until the hand is full, then sort and reset
    /// the selection vector to match the new hand size.
    pub fn refill_hand(&mut self, bus: &mut EventBus) {
        while self.hand.len() < HAND_SIZE {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
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
    pub fn advance_round(&mut self) {
        // Defeating the Boss completes an ante and scales the base for the next one.
        if self.blind == BlindKind::Boss {
            self.ante += 1;
            self.base_target = (self.base_target as f32 * self.mode.target_scaling) as u32;
        }
        self.run_number += 1;
        self.round_score = 0;
        self.target_score = self.base_target; // will be overridden by apply_blind
        self.round_rules.clear();
        self.plays_remaining = self.mode.starting_plays;
        self.discards_remaining = self.mode.starting_discards;
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_used = false;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.upcoming_blind = self.upcoming_blind.next();
        self.blind = self.upcoming_blind;
        self.wall = Wall::from_standard_shuffled();
        self.hand.clear();
        for _ in 0..self.mode.hand_size {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
    }

    /// Skip the upcoming blind: advance to the next in the cycle without
    /// playing or visiting the shop. Resets per-round state. Skipping is
    /// not allowed for the Boss blind — callers should check first.
    pub fn skip_to_next_blind(&mut self) {
        self.upcoming_blind = self.upcoming_blind.next();
        self.run_number += 1;
        self.round_score = 0;
        // Skipping stays inside the same ante (Boss can't be skipped), so the
        // ante's base target is unchanged — only the blind multiplier shifts.
        self.target_score = self.base_target;
        self.round_rules.clear();
        self.plays_remaining = self.mode.starting_plays;
        self.discards_remaining = self.mode.starting_discards;
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_used = false;
        self.joker_used = false;
        self.blind = self.upcoming_blind;
        self.wall = Wall::from_standard_shuffled();
        self.hand.clear();
        for _ in 0..self.mode.hand_size {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::build_wall;

    /// Create a RunState with a deterministic (unshuffled) wall for predictable tests.
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
            wall,
            hand,
            selected,
            round_score: 0,
            target_score: mode.base_target,
            base_target: mode.base_target,
            relics: RelicState::default(),
            round_rules: vec![],
            run_number: 1,
            ante: 1,
            plays_remaining: mode.starting_plays,
            discards_remaining: mode.starting_discards,
            gold: mode.starting_gold,
            blind: BlindKind::Small,
            upcoming_blind: BlindKind::Small,
            last_breakdown: None,
            available_yaku: vec![],
            available_rules: vec![],
            scored_last_turn: false,
            quickdraw_used: false,
            joker_used: false,
            full_hand_played_this_round: false,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            zodiac_inventory: crate::core::zodiac::ZodiacInventory::default(),
            yaku_loadout: vec![
                crate::core::yaku::YakuKind::Tanyao,
                crate::core::yaku::YakuKind::Toitoi,
                crate::core::yaku::YakuKind::Chinitsu,
            ],
            yaku_loadout_capacity: 3,
            mode,
        }
    }

    fn bus() -> EventBus {
        EventBus::default()
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

        // Drain the wall almost completely: wall started with 136, 14 already drawn.
        // Draw remaining 122 tiles to exhaust the wall.
        for _ in 0..122 {
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
        run.toggle_select(0);
        run.toggle_select(5);
        assert_eq!(run.selected_count(), 2);

        run.advance_round();

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);
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
