use crate::core::consumable::ConsumableInventory;
use crate::core::hand::DetectedSet;
use crate::core::tile::{Tile, cmp_sort_order};
use crate::core::yaku::YakuKind;
use crate::game::run::RunState;
use crate::ui::input::MarqueeSelect;

/// Engine-owned gameplay core state for the first ownership slice.
///
/// This deliberately covers the high-churn gameplay interaction surface:
/// hand contents, selection, structure bank, round resources, and
/// consumables. Legacy `RunState` still owns the rest of the run while the
/// engine migrates incrementally.
#[derive(Clone, Debug)]
pub struct GameplayCoreState {
    pub hand: Vec<Tile>,
    pub selected: Vec<bool>,
    pub structure_sets: Vec<DetectedSet>,
    pub structure_tiles: Vec<Tile>,
    pub round_score: u64,
    pub target_score: u32,
    pub plays_remaining: u32,
    pub plays_max: u32,
    pub discards_remaining: u32,
    pub discards_max: u32,
    pub gold: i32,
    pub available_yaku: Vec<YakuKind>,
    pub consumables: ConsumableInventory,
}

impl GameplayCoreState {
    pub fn from_run(run: &RunState) -> Self {
        Self {
            hand: run.hand.clone(),
            selected: run.selected.clone(),
            structure_sets: run.structure_sets.clone(),
            structure_tiles: run.structure_tiles.clone(),
            round_score: run.round_score,
            target_score: run.target_score,
            plays_remaining: run.plays_remaining,
            plays_max: run.plays_max,
            discards_remaining: run.discards_remaining,
            discards_max: run.discards_max,
            gold: run.gold,
            available_yaku: run.available_yaku.clone(),
            consumables: run.consumables.clone(),
        }
    }

    pub fn write_back(&self, run: &mut RunState) {
        run.hand = self.hand.clone();
        run.selected = self.selected.clone();
        run.structure_sets = self.structure_sets.clone();
        run.structure_tiles = self.structure_tiles.clone();
        run.round_score = self.round_score;
        run.target_score = self.target_score;
        run.plays_remaining = self.plays_remaining;
        run.plays_max = self.plays_max;
        run.discards_remaining = self.discards_remaining;
        run.discards_max = self.discards_max;
        run.gold = self.gold;
        run.available_yaku = self.available_yaku.clone();
        run.consumables = self.consumables.clone();
    }

    pub fn hand_len(&self) -> usize {
        self.hand.len()
    }

    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&selected| selected).count()
    }

    pub fn selected_indices(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(i, selected)| (*selected).then_some(i))
            .collect()
    }

    pub fn sort_hand_by_suit(&mut self) {
        self.hand.sort_by(cmp_sort_order);
        self.selected = vec![false; self.hand.len()];
    }

    pub fn sort_hand_by_rank(&mut self) {
        self.hand.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(a.suit.cmp(&b.suit))
                .then(a.id.cmp(&b.id))
        });
        self.selected = vec![false; self.hand.len()];
    }

    pub fn begin_marquee_selection(&mut self, index: usize) -> Option<(MarqueeSelect, (u32, u32))> {
        if self.hand.is_empty() {
            return None;
        }
        let idx = index.min(self.hand.len() - 1);
        if self.selected.len() < self.hand.len() {
            self.selected.resize(self.hand.len(), false);
        }
        let marquee = MarqueeSelect {
            start_slot: idx,
            current_slot: idx,
            snapshot: self.selected.clone(),
        };
        let delta = marquee.apply(&mut self.selected);
        Some((marquee, delta))
    }

    pub fn apply_marquee_selection(
        &mut self,
        marquee: &mut MarqueeSelect,
        index: usize,
    ) -> Option<(u32, u32)> {
        if self.hand.is_empty() {
            return None;
        }
        let idx = index.min(self.hand.len() - 1);
        if idx == marquee.current_slot {
            return Some((0, 0));
        }
        marquee.current_slot = idx;
        if self.selected.len() < self.hand.len() {
            self.selected.resize(self.hand.len(), false);
        }
        Some(marquee.apply(&mut self.selected))
    }

    /// Remove the currently-selected tiles from the hand, decrement the
    /// discards counter, and reset selection. Returns the removed tiles in
    /// hand-order (not reversed). Caller is responsible for wall refill and
    /// any relic/tutorial side-effects.
    pub fn discard_selected(&mut self) -> Vec<Tile> {
        if self.discards_remaining == 0 {
            return Vec::new();
        }
        let indices: Vec<usize> = self.selected_indices();
        if indices.is_empty() {
            return Vec::new();
        }
        let removed: Vec<Tile> = indices.iter().map(|&i| self.hand[i]).collect();
        for &i in indices.iter().rev() {
            self.hand.remove(i);
        }
        self.discards_remaining -= 1;
        self.selected = vec![false; self.hand.len()];
        removed
    }

    /// Remove the currently-selected tiles from the hand without touching
    /// resources or selection vector length bookkeeping. Used by the
    /// play-selection scoring path which refills differently.
    /// Returns (removed_tiles, removed_indices_in_hand_order).
    pub fn take_selected_tiles(&mut self) -> (Vec<Tile>, Vec<usize>) {
        let indices: Vec<usize> = self.selected_indices();
        let removed: Vec<Tile> = indices.iter().map(|&i| self.hand[i]).collect();
        for &i in indices.iter().rev() {
            self.hand.remove(i);
        }
        (removed, indices)
    }

    /// Push validated sets onto the structure bank, extend structure tiles,
    /// and decrement plays. Used when the run has `structure_bank` mode.
    pub fn commit_sets_to_structure(&mut self, sets: &[DetectedSet], scoring_tiles: &[Tile]) {
        for set in sets {
            self.structure_sets.push(set.clone());
        }
        self.structure_tiles.extend(scoring_tiles.iter().copied());
        self.plays_remaining = self.plays_remaining.saturating_sub(1);
    }

    /// Decrement plays without touching the structure bank. Used when the
    /// play-selection path scores immediately (non-bank mode).
    pub fn consume_play(&mut self) {
        self.plays_remaining = self.plays_remaining.saturating_sub(1);
    }

    /// Append tiles drawn from the wall onto the hand. The caller has
    /// already drawn from the wall and emitted any bus events.
    pub fn push_drawn_tiles(&mut self, drawn: &[Tile]) {
        self.hand.extend_from_slice(drawn);
    }

    /// Sort the hand and reset the selection vector to match the new size.
    /// Called after refill / play-commit draws to normalize state.
    pub fn finalize_hand_after_draw(&mut self) {
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::GameplayCoreState;
    use crate::core::tile::{Suit, Tile};
    use crate::game::run::RunState;

    #[test]
    fn core_state_round_trips_through_run() {
        let mut run = RunState::new_demo();
        run.hand = vec![
            Tile::new(Suit::Circles, 9, 1),
            Tile::new(Suit::Characters, 2, 2),
            Tile::new(Suit::Bamboos, 4, 3),
        ];
        run.selected = vec![true, false, true];
        run.gold = 42;

        let mut core = GameplayCoreState::from_run(&run);
        core.sort_hand_by_rank();
        core.gold += 5;
        core.write_back(&mut run);

        assert_eq!(run.gold, 47);
        assert_eq!(run.selected, vec![false, false, false]);
        assert_eq!(run.hand[0].rank, 2);
        assert_eq!(run.hand[1].rank, 4);
        assert_eq!(run.hand[2].rank, 9);
    }

    #[test]
    fn marquee_selection_updates_owned_selected_mask() {
        let mut run = RunState::new_demo();
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 10),
            Tile::new(Suit::Characters, 2, 11),
            Tile::new(Suit::Characters, 3, 12),
        ];
        run.selected = vec![false; 3];
        let mut core = GameplayCoreState::from_run(&run);

        let (mut marquee, _) = core.begin_marquee_selection(0).unwrap();
        let _ = core.apply_marquee_selection(&mut marquee, 2).unwrap();

        assert_eq!(core.selected, vec![true, true, true]);
        assert_eq!(core.selected_indices(), vec![0, 1, 2]);
    }
}
