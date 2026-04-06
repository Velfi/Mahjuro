//! Single-run state: wall, hand, score target, round modifiers.

use rand::seq::SliceRandom;

use crate::core::deck::Wall;
use crate::core::hand::{detect_all_sets, validate_selection};

use crate::core::relic::{RelicId, RelicState, ScoreContext, all_relic_defs};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::{ScoreBreakdown, score_sets};
use crate::core::tile::Tile;
use crate::game::event_bus::{EventBus, GameEvent};

pub const HAND_SIZE: usize = 14;
pub const STARTING_PLAYS: u32 = 4;
pub const STARTING_DISCARDS: u32 = 3;

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
    pub plays_remaining: u32,
    pub discards_remaining: u32,
    pub gold: u32,
    pub blind: BlindKind,
    /// Last scoring breakdown for UI cascade display.
    pub last_breakdown: Option<ScoreBreakdown>,
}

impl RunState {
    pub fn new_demo() -> Self {
        let mut wall = Wall::from_standard_shuffled();
        let mut hand = Vec::with_capacity(HAND_SIZE);
        for _ in 0..HAND_SIZE {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        hand.sort();
        let selected = vec![false; hand.len()];
        Self {
            wall,
            hand,
            selected,
            round_score: 0,
            target_score: 500,
            base_target: 500,
            relics: RelicState {
                active: vec![RelicId::TripletBoost, RelicId::BambooCharm],
            },
            round_rules: vec![RuleModifier::PairDoubleScore],
            run_number: 1,
            plays_remaining: STARTING_PLAYS,
            discards_remaining: STARTING_DISCARDS,
            gold: 0,
            blind: BlindKind::Small,
            last_breakdown: None,
        }
    }

    /// Apply a blind choice: sets target score and any forced modifiers.
    pub fn apply_blind(&mut self, blind: BlindKind) {
        self.blind = blind;
        self.target_score = (self.base_target as f32 * blind.target_multiplier()) as u32;
        if let Some(modifier) = blind.forced_modifier() {
            if !self.round_rules.contains(&modifier) {
                self.round_rules.push(modifier);
            }
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

        // Validate: must decompose into melds with no leftovers.
        let sets = match validate_selection(&selected_tiles) {
            Some(sets) => sets,
            None => return 0,
        };

        // Score the selected tiles.
        let ctx = ScoreContext {
            relics: &self.relics,
        };
        let breakdown = score_sets(&selected_tiles, &sets, &ctx, &self.round_rules);
        let earned = breakdown.total.max(0) as u32;
        self.round_score = self.round_score.saturating_add(earned);
        self.last_breakdown = Some(breakdown);
        self.plays_remaining -= 1;

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
        while self.hand.len() < HAND_SIZE {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];

        bus.push(GameEvent::ScoreUpdated(self.round_score));
        if self.round_score >= self.target_score {
            let excess_gold = (self.round_score.saturating_sub(self.target_score)) / 50;
            let gold_earned = ((3 + excess_gold) as f32 * self.blind.gold_multiplier()) as u32;
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
        validate_selection(&selected_tiles).is_some()
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

        // Auto-draw back to full hand.
        while self.hand.len() < HAND_SIZE {
            let Some(t) = self.wall.draw() else { break };
            self.hand.push(t);
            bus.push(GameEvent::TileDrawn(t));
        }

        // Sort and reset selection to match new hand size.
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        count
    }

    /// Sort hand by suit then rank (Characters → Bamboos → Circles → Wind → Dragon).
    pub fn sort_hand_by_suit(&mut self) {
        self.hand.sort();
    }

    /// Sort hand by rank then suit (all 1s, all 2s, … then honors).
    pub fn sort_hand_by_rank(&mut self) {
        self.hand.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(a.suit.cmp(&b.suit))
                .then(a.id.cmp(&b.id))
        });
    }

    /// Evaluate meld patterns for UI hints.
    #[allow(dead_code)]
    pub fn hint_sets(&self) -> usize {
        detect_all_sets(&self.hand).len()
    }

    /// Add the chosen relic, scale up the base target, and reset for the next round.
    /// The actual target_score is set later by `apply_blind`.
    pub fn advance_round(&mut self, chosen_relic: RelicId) {
        self.relics.active.push(chosen_relic);
        self.run_number += 1;
        self.round_score = 0;
        self.base_target = (self.base_target as f32 * 1.5) as u32;
        self.target_score = self.base_target; // will be overridden by apply_blind
        self.round_rules.clear();
        self.plays_remaining = STARTING_PLAYS;
        self.discards_remaining = STARTING_DISCARDS;
        self.last_breakdown = None;
        self.blind = BlindKind::Small;
        self.wall = Wall::from_standard_shuffled();
        self.hand.clear();
        for _ in 0..HAND_SIZE {
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
    use crate::core::tile::Suit;

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
        RunState {
            wall,
            hand,
            selected,
            round_score: 0,
            target_score: 500,
            base_target: 500,
            relics: RelicState { active: vec![] },
            round_rules: vec![],
            run_number: 1,
            plays_remaining: STARTING_PLAYS,
            discards_remaining: STARTING_DISCARDS,
            gold: 0,
            blind: BlindKind::Small,
            last_breakdown: None,
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
            assert!(run.hand.iter().any(|t| t.id == *id), "tile id {} was lost", id);
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
        assert_eq!(run.discards_remaining, 0);

        // Fourth attempt: should fail (no discards left).
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

        run.advance_round(RelicId::TripletBoost);

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

/// Pick `count` relics the player doesn't already own, randomly.
/// Returns up to `count` choices (may be fewer if pool is exhausted).
pub fn pick_relic_choices(relics: &RelicState, count: usize) -> Vec<RelicId> {
    let mut rng = rand::rng();
    let mut pool: Vec<RelicId> = all_relic_defs()
        .iter()
        .map(|d| d.id)
        .filter(|id| !relics.has(*id))
        .collect();
    pool.shuffle(&mut rng);
    let fallbacks = [RelicId::TripletBoost, RelicId::SequenceSurge, RelicId::PairPower];
    (0..count)
        .map(|i| pool.get(i).copied().unwrap_or(fallbacks[i % fallbacks.len()]))
        .collect()
}
