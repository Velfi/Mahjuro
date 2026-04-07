//! Relic definitions and runtime application hooks.

use serde::{Deserialize, Serialize};

use crate::core::tile::Suit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelicId {
    TripletBoost,
    SequenceSurge,
    PairPower,
    HonorFury,
    BambooCharm,
    RedDragonRage,
    GreenLuck,
    WhiteSilence,
    JokerTile,
    Overflow,
    QuickDraw,
    ChainReaction,
    MultiplierMaster,
    SetMagnet,
    WildWinds,
    DragonEcho,
    ReverseTile,
    StealthTile,
    LockedSet,
    LuckyPair,
}

impl RelicId {
    /// Asset filename (without directory) for this relic's icon.
    pub fn asset_filename(self) -> &'static str {
        match self {
            RelicId::TripletBoost => "triplet_boost.png",
            RelicId::SequenceSurge => "sequence_surge.png",
            RelicId::PairPower => "pair_power.png",
            RelicId::HonorFury => "honor_fury.png",
            RelicId::BambooCharm => "bamboo_charm.png",
            RelicId::RedDragonRage => "red_dragon_rage.png",
            RelicId::GreenLuck => "green_luck.png",
            RelicId::WhiteSilence => "white_silence.png",
            RelicId::JokerTile => "joker_tile.png",
            RelicId::Overflow => "overflow.png",
            RelicId::QuickDraw => "quick_draw.png",
            RelicId::ChainReaction => "chain_reaction.png",
            RelicId::MultiplierMaster => "multiplier_master.png",
            RelicId::SetMagnet => "set_magnet.png",
            RelicId::WildWinds => "wild_winds.png",
            RelicId::DragonEcho => "dragon_echo.png",
            RelicId::ReverseTile => "reverse_tile.png",
            RelicId::StealthTile => "stealth_tile.png",
            RelicId::LockedSet => "locked_set.png",
            RelicId::LuckyPair => "lucky_pair.png",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Legendary,
}

impl Rarity {
    /// Weight for random selection (higher = more likely).
    pub fn weight(self) -> u32 {
        match self {
            Rarity::Common => 4,
            Rarity::Uncommon => 3,
            Rarity::Rare => 2,
            Rarity::Legendary => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RelicDef {
    pub id: RelicId,
    pub name: &'static str,
    pub description: &'static str,
    pub rarity: Rarity,
}

/// Gold cost to buy a relic in the shop. Stable (deterministic) per relic id so
/// the shop, bot, and any future tooling agree on prices.
pub fn relic_buy_price(id: RelicId) -> u32 {
    let defs = all_relic_defs();
    let idx = defs.iter().position(|d| d.id == id).unwrap_or(0);
    3 + (idx as u32 % 4)
}

/// Refund when selling a relic — half buy price, minimum 1 gold.
pub fn relic_sell_price(id: RelicId) -> u32 {
    (relic_buy_price(id) / 2).max(1)
}

pub fn all_relic_defs() -> &'static [RelicDef] {
    &[
        RelicDef {
            id: RelicId::TripletBoost,
            name: "Triplet Boost",
            description: "Triplets score ×3",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::SequenceSurge,
            name: "Sequence Surge",
            description: "Sequences score ×2",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::PairPower,
            name: "Pair Power",
            description: "Pairs score +40",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::HonorFury,
            name: "Honor Fury",
            description: "Honor tiles +12 each in sets",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::BambooCharm,
            name: "Bamboo Charm",
            description: "Bamboo tiles +5 in any set",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::RedDragonRage,
            name: "Red Dragon Rage",
            description: "Red dragon triplets ×5",
            rarity: Rarity::Legendary,
        },
        RelicDef {
            id: RelicId::GreenLuck,
            name: "Green Luck",
            description: "Hands without honors earn +4 gold",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::WhiteSilence,
            name: "White Silence",
            description: "White dragon pairs +50",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::JokerTile,
            name: "Joker Tile",
            description: "Once per round: one tile acts as wild",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::Overflow,
            name: "Overflow",
            description: "Wall contains 6 copies per tile instead of 4",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::QuickDraw,
            name: "Quick Draw",
            description: "Draw +1 tile after your first play each round",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::ChainReaction,
            name: "Chain Reaction",
            description: "+50% score if you scored last turn",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::MultiplierMaster,
            name: "Multiplier Master",
            description: "+15% score per relic owned",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::SetMagnet,
            name: "Set Magnet",
            description: "Scoring a triplet draws a matching tile",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::WildWinds,
            name: "Wild Winds",
            description: "Wind tiles can substitute in sequences",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::DragonEcho,
            name: "Dragon Echo",
            description: "Dragon triplets add 100% of adjacent sets",
            rarity: Rarity::Legendary,
        },
        RelicDef {
            id: RelicId::ReverseTile,
            name: "Reverse Tile",
            description: "Once per round: swap suit/rank of 2 tiles",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::StealthTile,
            name: "Stealth Tile",
            description: "First discard ignores negative rule effects",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::LockedSet,
            name: "Locked Set",
            description: "Scored triplets lock for 3 turns (can't discard)",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::LuckyPair,
            name: "Lucky Pair",
            description: "Pairs score ×3",
            rarity: Rarity::Common,
        },
    ]
}

/// Active relics during a run (by id).
#[derive(Clone, Debug)]
pub struct RelicState {
    pub active: Vec<RelicId>,
    pub max_slots: usize,
}

impl Default for RelicState {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            max_slots: 5,
        }
    }
}

impl RelicState {
    pub fn has(&self, id: RelicId) -> bool {
        self.active.contains(&id)
    }

    pub fn is_full(&self) -> bool {
        self.active.len() >= self.max_slots
    }
}

/// Scoring context for relic hooks.
pub struct ScoreContext<'a> {
    pub relics: &'a RelicState,
    /// Whether the player scored on their previous play (for ChainReaction).
    pub scored_last_turn: bool,
    /// Dora tile faces (suit, rank) that grant bonus points.
    pub dora_faces: Vec<(Suit, u8)>,
    /// Yaku patterns available at the player's progression level.
    pub available_yaku: Vec<crate::core::yaku::YakuKind>,
}

pub fn triplet_multiplier(ctx: &ScoreContext) -> f64 {
    let mut m = 1.0;
    if ctx.relics.has(RelicId::TripletBoost) {
        m *= 3.0;
    }
    if ctx.relics.has(RelicId::MultiplierMaster) {
        m *= 1.0 + 0.15 * ctx.relics.active.len() as f64;
    }
    m
}

pub fn sequence_multiplier(ctx: &ScoreContext) -> f64 {
    let mut m = 1.0;
    if ctx.relics.has(RelicId::SequenceSurge) {
        m *= 2.0;
    }
    if ctx.relics.has(RelicId::MultiplierMaster) {
        m *= 1.0 + 0.15 * ctx.relics.active.len() as f64;
    }
    m
}

pub fn pair_bonus_points(ctx: &ScoreContext) -> i32 {
    let mut b = 0;
    if ctx.relics.has(RelicId::PairPower) {
        b += 40;
    }
    b
}

pub fn suit_tile_bonus(suit: Suit, ctx: &ScoreContext) -> i32 {
    if ctx.relics.has(RelicId::BambooCharm) && suit == Suit::Bamboos {
        return 5;
    }
    0
}
