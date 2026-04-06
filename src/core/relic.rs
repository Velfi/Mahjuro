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

#[derive(Clone, Debug)]
pub struct RelicDef {
    pub id: RelicId,
    pub name: &'static str,
    #[allow(dead_code)]
    pub description: &'static str,
}

pub fn all_relic_defs() -> &'static [RelicDef] {
    &[
        RelicDef {
            id: RelicId::TripletBoost,
            name: "Triplet Boost",
            description: "Triplets score ×2",
        },
        RelicDef {
            id: RelicId::SequenceSurge,
            name: "Sequence Surge",
            description: "Sequences score +50%",
        },
        RelicDef {
            id: RelicId::PairPower,
            name: "Pair Power",
            description: "Pairs score +10",
        },
        RelicDef {
            id: RelicId::HonorFury,
            name: "Honor Fury",
            description: "Honor tiles +3 base points each in sets",
        },
        RelicDef {
            id: RelicId::BambooCharm,
            name: "Bamboo Charm",
            description: "Bamboo tiles +2 points in any set",
        },
        RelicDef {
            id: RelicId::RedDragonRage,
            name: "Red Dragon Rage",
            description: "Red dragon triplets ×5",
        },
        RelicDef {
            id: RelicId::GreenLuck,
            name: "Green Luck",
            description: "Hands with no honors heal meta (stub)",
        },
        RelicDef {
            id: RelicId::WhiteSilence,
            name: "White Silence",
            description: "White dragon pairs worth double",
        },
        RelicDef {
            id: RelicId::JokerTile,
            name: "Joker Tile",
            description: "Once per round: treat one tile as wild (stub)",
        },
        RelicDef {
            id: RelicId::Overflow,
            name: "Overflow",
            description: "Duplicate tiles allowed in scoring (stub)",
        },
        RelicDef {
            id: RelicId::QuickDraw,
            name: "Quick Draw",
            description: "First draw each round is +1 tile (stub)",
        },
        RelicDef {
            id: RelicId::ChainReaction,
            name: "Chain Reaction",
            description: "+25% if you scored last turn (stub)",
        },
        RelicDef {
            id: RelicId::MultiplierMaster,
            name: "Multiplier Master",
            description: "+10% score per relic owned",
        },
        RelicDef {
            id: RelicId::SetMagnet,
            name: "Set Magnet",
            description: "Completing a triplet draws attention (stub)",
        },
        RelicDef {
            id: RelicId::WildWinds,
            name: "Wild Winds",
            description: "Winds count as same rank for triplets (stub)",
        },
        RelicDef {
            id: RelicId::DragonEcho,
            name: "Dragon Echo",
            description: "Dragon sets score twice (stub)",
        },
        RelicDef {
            id: RelicId::ReverseTile,
            name: "Reverse Tile",
            description: "Once per round: swap two tiles in hand (stub)",
        },
        RelicDef {
            id: RelicId::StealthTile,
            name: "Stealth Tile",
            description: "One random tile is hidden from opponents (stub)",
        },
        RelicDef {
            id: RelicId::LockedSet,
            name: "Locked Set",
            description: "Lock a completed set to protect it (stub)",
        },
        RelicDef {
            id: RelicId::LuckyPair,
            name: "Lucky Pair",
            description: "Pairs have a chance to score triple (stub)",
        },
    ]
}

/// Active relics during a run (by id).
#[derive(Clone, Debug, Default)]
pub struct RelicState {
    pub active: Vec<RelicId>,
}

impl RelicState {
    pub fn has(&self, id: RelicId) -> bool {
        self.active.contains(&id)
    }
}

/// Scoring context for relic hooks.
pub struct ScoreContext<'a> {
    pub relics: &'a RelicState,
}

pub fn triplet_multiplier(ctx: &ScoreContext) -> f64 {
    let mut m = 1.0;
    if ctx.relics.has(RelicId::TripletBoost) {
        m *= 2.0;
    }
    if ctx.relics.has(RelicId::MultiplierMaster) {
        m *= 1.0 + 0.1 * ctx.relics.active.len() as f64;
    }
    m
}

pub fn sequence_multiplier(ctx: &ScoreContext) -> f64 {
    let mut m = 1.0;
    if ctx.relics.has(RelicId::SequenceSurge) {
        m *= 1.5;
    }
    if ctx.relics.has(RelicId::MultiplierMaster) {
        m *= 1.0 + 0.1 * ctx.relics.active.len() as f64;
    }
    m
}

pub fn pair_bonus_points(ctx: &ScoreContext) -> i32 {
    let mut b = 0;
    if ctx.relics.has(RelicId::PairPower) {
        b += 10;
    }
    if ctx.relics.has(RelicId::WhiteSilence) {
        b += 5;
    }
    b
}

pub fn suit_tile_bonus(suit: Suit, ctx: &ScoreContext) -> i32 {
    if ctx.relics.has(RelicId::BambooCharm) && suit == Suit::Bamboos {
        return 2;
    }
    0
}
