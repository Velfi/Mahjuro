//! Hand pattern detection: pairs, triplets, sequences.
//!
//! Implementation is split between [`decomposition`], which enumerates and scores
//! tile groupings, and [`validation`], which applies turn rules to selections.

use serde::{Deserialize, Serialize};

pub mod decomposition;
mod validation;

#[cfg(test)]
pub use decomposition::find_pairs_and_triplets;
pub use decomposition::{detect_all_sets, enumerate_decompositions};
pub use validation::{
    non_contributing_tile_ids, selection_rejection_hint, staging_preview_melds,
    suggest_completions, validate_selection, validate_selection_with_rules,
};

/// Player-facing meld variant. `Single` is a decomposition artefact (only
/// produced by the Kokushi Musō layout: twelve singles + one pair) and is
/// not a meld in any player-visible rule sense; it is kept here to avoid
/// splitting the enum. A future cleanup might lift it into a separate
/// `KokushiPart` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MeldKind {
    Pair,
    Triplet,
    Sequence,
    /// Four of a kind (mahjong "kan"). Counts as a triplet for yaku and meld
    /// detection but uses a higher base chip table entry than a triplet.
    Kong,
    /// One tile. Decomposition artefact only (see [`MeldKind`] doc comment).
    Single,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedMeld {
    pub kind: MeldKind,
    /// Tile ids participating in this meld (references into the hand).
    pub tile_ids: Vec<u32>,
}

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;
