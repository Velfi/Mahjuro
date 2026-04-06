//! Preset game-mode defaults that define starting conditions for a run.

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::tile::Suit;
use crate::core::yaku::YakuKind;

/// A tile face (suit + rank) and how many copies are in the wall.
#[derive(Clone, Debug)]
pub struct CardEntry {
    pub suit: Suit,
    pub rank: u8,
    pub copies: u8,
}

/// All tuneable starting conditions for a run, bundled into one preset.
#[derive(Clone, Debug)]
pub struct GameMode {
    pub name: &'static str,
    pub starting_gold: u32,
    pub starting_plays: u32,
    pub starting_discards: u32,
    pub hand_size: usize,
    pub base_target: u32,
    pub target_scaling: f32,
    pub starting_relics: Vec<RelicId>,
    pub starting_rules: Vec<RuleModifier>,
    pub starting_yaku: Vec<YakuKind>,
    /// Which tile faces (and how many copies) populate the wall.
    /// `None` means the standard 136-tile mahjong set.
    pub card_inventory: Option<Vec<CardEntry>>,
}

impl GameMode {
    /// The default game mode.
    pub fn standard() -> Self {
        Self {
            name: "Standard",
            starting_gold: 4,
            starting_plays: 4,
            starting_discards: 3,
            hand_size: 14,
            base_target: 500,
            target_scaling: 1.5,
            starting_relics: vec![],
            starting_rules: vec![RuleModifier::PairDoubleScore],
            starting_yaku: vec![
                YakuKind::AllTriplets,
                YakuKind::AllSimples,
                YakuKind::Flush,
                YakuKind::MixedSets,
                YakuKind::FullHand,
            ],
            card_inventory: None,
        }
    }

    /// Build the standard 136-tile card inventory as explicit entries.
    pub fn standard_card_inventory() -> Vec<CardEntry> {
        let mut entries = Vec::new();
        for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
            for rank in 1..=9 {
                entries.push(CardEntry { suit, rank, copies: 4 });
            }
        }
        for rank in 1..=4 {
            entries.push(CardEntry { suit: Suit::Wind, rank, copies: 4 });
        }
        for rank in 1..=3 {
            entries.push(CardEntry { suit: Suit::Dragon, rank, copies: 4 });
        }
        entries
    }
}
