//! Hand pattern detection: pairs, triplets, sequences.
//!
//! Implementation is split between [`decomposition`], which enumerates and scores
//! tile groupings, and [`validation`], which applies turn rules to selections.

use serde::{Deserialize, Serialize};

mod decomposition;
mod validation;

pub use decomposition::{detect_all_sets, enumerate_decompositions};
#[cfg(test)]
pub use decomposition::find_pairs_and_triplets;
pub use validation::{suggest_completions, validate_selection, validate_selection_with_rules};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SetKind {
    Pair,
    Triplet,
    Sequence,
    /// Four of a kind (mahjong "kan"). Counts as a triplet for yaku and meld
    /// detection but uses a higher base chip table entry than a triplet.
    Kong,
    /// One tile (used only for Kokushi Musō decomposition: twelve singles + one pair).
    Single,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedSet {
    pub kind: SetKind,
    /// Tile ids participating in this set (references into the hand).
    pub tile_ids: Vec<u32>,
}

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;
