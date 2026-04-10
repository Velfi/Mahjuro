//! Relic definitions and runtime application hooks.

use serde::{Deserialize, Serialize};

use crate::core::tile::Suit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelicId {
    // ── 15 retuned keepers from Patch A ────────────────────────────────
    TripletBoost,
    SequenceSurge,
    PairPower,
    HonorFury,
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
    // ── 15 new relics introduced in Patch C ────────────────────────────
    /// When a discard-refill brings the hand to tenpai (shanten 0), draw 1
    /// extra tile from the wall. Tempo relic — one more chance to complete.
    ShantenShove,
    /// Reveals the next 2 wall tiles. Pure-info — no scoring effect.
    WallPeek,
    /// Kongs grant +1 play this round and +4 mult when scored.
    KanDrum,
    /// Reveal an extra dora indicator at round start; dora chips become +35.
    DoraCrown,
    /// First riichi declaration each round costs no discard; failed riichi
    /// floors at 80% target instead of 60%. (No-op until Patch E lands the
    /// declaration UI.)
    RiichiStick,
    /// Tenpai Bonus is doubled.
    TenpaiTalisman,
    /// Once per round, clear 3 tiles from your river. (No-op until Patch D
    /// lands the river system.)
    RiverEraser,
    /// Your river only retains the last 6 tiles instead of 12. (No-op until
    /// Patch D lands the river system.)
    FuritenWard,
    /// Round Wind triplets/kongs grant +6 mult instead of the base +3.
    RoundCompass,
    /// +1 zodiac inventory slot.
    ZodiacPouch,
    /// +1 zodiac inventory slot; every 3rd Zodiac you use is duplicated.
    LunarAlmanac,
    /// Active yaku loadout has 4 slots instead of 3.
    YakuScholar,
    /// Scoring a FullHand grants 1 random Zodiac card (ignores slot cap).
    EightTreasures,
    /// Kongs grant +120 chips and +2 mult each. (The original "counts as
    /// both triplet and pair" semantic was never wired into yaku detection;
    /// this flat bonus replaces it as a real, scoring effect.)
    KongsBlessing,
    /// Once per round, swap one yaku in your loadout for another unlocked.
    /// (No active scoring effect — UI hook only.)
    CodexCompass,
    // ── Flower-synergy relics ──────────────────────────────────────────
    /// Each flower's triggered effect fires a second time.
    GardenKeeper,
    /// Scoring 2+ flowers in one hand grants +6 mult.
    Ikebana,
    /// Each flower scored grants +3 gold immediately.
    Hanami,
    // ── 15 new relics ─────────────────────────────────────────────────
    /// Bamboo-suit tiles in scored sets: +8 chips each.
    JadeSerpent,
    /// Characters-suit tiles in scored sets: +8 chips each.
    InkBrush,
    /// Dots-suit tiles in scored sets: +8 chips each.
    PearlDiver,
    /// Tiles ranked 1–3 in scored sets: +6 chips each.
    LowTide,
    /// Relics cost $1 less in the shop (minimum $1).
    MerchantsEye,
    /// Terminal tiles (rank 1 or 9) in scored sets: +12 chips each.
    EdgeRunner,
    /// Rank-7 tiles in scored sets: +1.5 mult each.
    LuckySeven,
    /// +0.5 mult per play already used this round.
    Momentum,
    /// Playing exactly one set that is a pair: +4 mult.
    Minimalist,
    /// +50 chips if mult is below 3.0 after all other bonuses.
    TurtleShell,
    /// All scored tiles are terminals or honors: +4 mult.
    ClosedGate,
    /// +1 mult per 5 gold held (max +4 mult).
    GoldFurnace,
    /// +0.1 mult per 100 total score earned this run (max +5 mult).
    Snowball,
    /// +1 play per round.
    SecondWind,
    /// ×2 final mult, but 1 fewer play per round.
    GlassCannon,
    // ── Balatro-inspired relics (Patch F) ─────────────────────────────
    /// On your final play of the round, retrigger all scored tiles (they
    /// each contribute their chip value a second time).
    LastBreath,
    /// Every scored tile permanently gains +3 chips for the rest of the
    /// run. Tracked in `RunState::tile_polisher_bonus`.
    TilePolisher,
    /// +6 mult, but 1-in-5 chance to be destroyed at end of each round.
    /// When destroyed, replaced by Iron Lantern.
    PaperLantern,
    /// Replaces Paper Lantern when it burns. ×2 final mult, 1-in-1000
    /// chance to break at end of round.
    IronLantern,
    /// Copies the scoring effect of the relic immediately after it in
    /// the player's relic inventory. No effect if it's the last slot.
    MirrorTile,
    /// ×2.5 mult if every scored tile belongs to a single numbered suit.
    WayOfPurity,
    // ── Patch G: 25 Balatro-inspired relics ───────────────────────────
    // Retrigger
    /// Retrigger the first tile in each scored set.
    LeadingTile,
    /// Retrigger tiles ranked 1–4 in scored sets.
    LowEcho,
    /// Retrigger all scored tiles for 3 plays, then self-destructs.
    TeaCeremony,
    /// Tiles NOT in scored sets each grant +2 chips.
    GhostHand,
    // Scaling
    /// +0.5 mult per consecutive play without honor tiles. Resets when
    /// honors are scored.
    CleanStreak,
    /// +0.3 mult per round you don't score your most-used yaku.
    Obsession,
    /// +0.4 mult per relic sold this run.
    Bonfire,
    /// +20 chips permanently each time you score a sequence.
    RiverRunner,
    // Fragile
    /// +80 chips, loses 8 chips per play. Destroyed at 0.
    MeltingIce,
    /// +4 mult, loses 0.3 mult per discard. Destroyed at 0.
    SilkThread,
    // Copy / Meta
    /// Copies the effect of the first relic in your inventory.
    ShadowHand,
    /// +1.5 mult per empty relic slot.
    EmptyFrame,
    // Economy
    /// +3 gold at round end.
    GoldIdol,
    /// +1 gold interest per 4 gold held (max +4).
    JadeAbacus,
    /// Gains +2 sell value each round. Sell when ripe.
    NestEgg,
    /// +2 gold per unused discard at round end.
    Patience,
    // Conditional ×mult
    /// ×2 mult if scored hand is all pairs.
    WayOfPairs,
    /// ×2.5 mult if scored hand is all triplets/kongs.
    WayOfTriplets,
    /// ×2 mult if scored hand is all sequences.
    WayOfSequences,
    // Probability / Chaos
    /// Doubles all relic trigger probabilities.
    FortunesFavor,
    /// +0 to +8 mult (random per play).
    CrackedTile,
    /// 1-in-4 chance to level up a scored yaku after each play.
    StarTile,
    // Sell-to-activate
    /// Sell to skip the current boss blind.
    SmokeBomb,
    /// After 3 rounds, sell to duplicate a random owned relic.
    PhantomRelic,
    /// Destroy the relic to the right; gain permanent mult equal to
    /// double its sell value.
    RitualBlade,
}

impl RelicId {
    /// Asset filename (without directory) for this relic's icon.
    pub fn asset_filename(self) -> &'static str {
        match self {
            RelicId::TripletBoost => "triplet_boost.png",
            RelicId::SequenceSurge => "sequence_surge.png",
            RelicId::PairPower => "pair_power.png",
            RelicId::HonorFury => "honor_fury.png",
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
            // Patch C new relics — placeholder asset names that fall back to
            // the relic's slug. Art for these can come later.
            RelicId::ShantenShove => "shanten_shove.png",
            RelicId::WallPeek => "wall_peek.png",
            RelicId::KanDrum => "kan_drum.png",
            RelicId::DoraCrown => "dora_crown.png",
            RelicId::RiichiStick => "riichi_stick.png",
            RelicId::TenpaiTalisman => "tenpai_talisman.png",
            RelicId::RiverEraser => "river_eraser.png",
            RelicId::FuritenWard => "furiten_ward.png",
            RelicId::RoundCompass => "round_compass.png",
            RelicId::ZodiacPouch => "zodiac_pouch.png",
            RelicId::LunarAlmanac => "lunar_almanac.png",
            RelicId::YakuScholar => "yaku_scholar.png",
            RelicId::EightTreasures => "eight_treasures.png",
            RelicId::KongsBlessing => "kongs_blessing.png",
            RelicId::CodexCompass => "codex_compass.png",
            RelicId::GardenKeeper => "garden_keeper.png",
            RelicId::Ikebana => "ikebana.png",
            RelicId::Hanami => "hanami.png",
            RelicId::JadeSerpent => "jade_serpent.png",
            RelicId::InkBrush => "ink_brush.png",
            RelicId::PearlDiver => "pearl_diver.png",
            RelicId::LowTide => "low_tide.png",
            RelicId::MerchantsEye => "merchants_eye.png",
            RelicId::EdgeRunner => "edge_runner.png",
            RelicId::LuckySeven => "lucky_seven.png",
            RelicId::Momentum => "momentum.png",
            RelicId::Minimalist => "minimalist.png",
            RelicId::TurtleShell => "turtle_shell.png",
            RelicId::ClosedGate => "closed_gate.png",
            RelicId::GoldFurnace => "gold_furnace.png",
            RelicId::Snowball => "snowball.png",
            RelicId::SecondWind => "second_wind.png",
            RelicId::GlassCannon => "glass_cannon.png",
            RelicId::LastBreath => "last_breath.png",
            RelicId::TilePolisher => "tile_polisher.png",
            RelicId::PaperLantern => "paper_lantern.png",
            RelicId::IronLantern => "iron_lantern.png",
            RelicId::MirrorTile => "mirror_tile.png",
            RelicId::WayOfPurity => "way_of_purity.png",
            RelicId::LeadingTile => "leading_tile.png",
            RelicId::LowEcho => "low_echo.png",
            RelicId::TeaCeremony => "tea_ceremony.png",
            RelicId::GhostHand => "ghost_hand.png",
            RelicId::CleanStreak => "clean_streak.png",
            RelicId::Obsession => "obsession.png",
            RelicId::Bonfire => "bonfire.png",
            RelicId::RiverRunner => "river_runner.png",
            RelicId::MeltingIce => "melting_ice.png",
            RelicId::SilkThread => "silk_thread.png",
            RelicId::ShadowHand => "shadow_hand.png",
            RelicId::EmptyFrame => "empty_frame.png",
            RelicId::GoldIdol => "gold_idol.png",
            RelicId::JadeAbacus => "jade_abacus.png",
            RelicId::NestEgg => "nest_egg.png",
            RelicId::Patience => "patience.png",
            RelicId::WayOfPairs => "way_of_pairs.png",
            RelicId::WayOfTriplets => "way_of_triplets.png",
            RelicId::WayOfSequences => "way_of_sequences.png",
            RelicId::FortunesFavor => "fortunes_favor.png",
            RelicId::CrackedTile => "cracked_tile.png",
            RelicId::StarTile => "star_tile.png",
            RelicId::SmokeBomb => "smoke_bomb.png",
            RelicId::PhantomRelic => "phantom_relic.png",
            RelicId::RitualBlade => "ritual_blade.png",
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

/// Find the relic whose display name exactly matches `name`. The scoring
/// cascade reveals steps labeled with the relic's display name (e.g.
/// "Triplet Boost"); the gameplay scene calls this to map a cascade step
/// back to a relic id so it can glow the matching badge.
pub fn relic_by_name(name: &str) -> Option<RelicId> {
    all_relic_defs()
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.id)
}

/// Refund when selling a relic — half buy price, minimum 1 gold.
pub fn relic_sell_price(id: RelicId) -> u32 {
    (relic_buy_price(id) / 2).max(1)
}

pub fn all_relic_defs() -> &'static [RelicDef] {
    &[
        // ── Retuned keepers ─────────────────────────────────────────────
        RelicDef {
            id: RelicId::TripletBoost,
            name: "Triplet Boost",
            description: "Triplets/Kongs: +40 chips and +0.2 mult",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::SequenceSurge,
            name: "Sequence Surge",
            description: "Sequences +25 chips and +0.5 mult",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::PairPower,
            name: "Pair Power",
            description: "Pairs +30 chips and +1 mult",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::HonorFury,
            name: "Honor Fury",
            description: "+28 chips per honor tile in sets",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::RedDragonRage,
            name: "Red Dragon Rage",
            description: "Any dragon triplet/kong: +5 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::GreenLuck,
            name: "Green Luck",
            description: "+4 gold at round end if no honors scored",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::WhiteSilence,
            name: "White Silence",
            description: "White dragon pair: +4 mult, draws a Zodiac",
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
            description: "+4 mult if you scored a yaku last turn",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::MultiplierMaster,
            name: "Multiplier Master",
            description: "+0.5 mult per relic owned",
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
            description: "Dragon triplets/kongs copy every other set's base chips",
            rarity: Rarity::Legendary,
        },
        // ── New Patch C relics ──────────────────────────────────────────
        RelicDef {
            id: RelicId::ShantenShove,
            name: "Shanten Shove",
            description: "When a discard brings you to tenpai, draw 1 extra tile from the wall",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::WallPeek,
            name: "Wall Peek",
            description: "See the next 2 tiles in the wall",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::KanDrum,
            name: "Kan Drum",
            description: "Kongs grant +1 play this round and +4 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::DoraCrown,
            name: "Dora Crown",
            description: "+1 dora indicator; dora chips become +35",
            rarity: Rarity::Rare,
        },
        // RiichiStick — disabled until Patch E (riichi declaration system).
        // See PATCH_E_RIICHI.md. Re-enable by uncommenting once `RunState`
        // gains `riichi_declared`, the failure-floor branch lands, and the
        // gameplay scene grows a declaration button.
        // RelicDef {
        //     id: RelicId::RiichiStick,
        //     name: "Riichi Stick",
        //     description: "First riichi each round is free; failed riichi floors at 80%",
        //     rarity: Rarity::Rare,
        // },
        RelicDef {
            id: RelicId::TenpaiTalisman,
            name: "Tenpai Talisman",
            description: "Tenpai Bonus is doubled",
            rarity: Rarity::Rare,
        },
        // RiverEraser & FuritenWard — disabled until Patch D (river system).
        // See PATCH_D_RIVER.md. Re-enable by uncommenting once `RunState`
        // gains a `river: Vec<Tile>`, the discard hook populates it, and the
        // furiten taint rule lands in `score_sets`.
        // RelicDef {
        //     id: RelicId::RiverEraser,
        //     name: "River Eraser",
        //     description: "Once per round: clear 3 tiles from your river",
        //     rarity: Rarity::Uncommon,
        // },
        // RelicDef {
        //     id: RelicId::FuritenWard,
        //     name: "Furiten Ward",
        //     description: "Your river only retains the last 6 tiles",
        //     rarity: Rarity::Uncommon,
        // },
        RelicDef {
            id: RelicId::RoundCompass,
            name: "Round Compass",
            description: "Round Wind triplets/kongs: +6 mult",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::ZodiacPouch,
            name: "Zodiac Pouch",
            description: "+1 Zodiac inventory slot",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::LunarAlmanac,
            name: "Lunar Almanac",
            description: "+1 Zodiac slot; every 3rd Zodiac use is duplicated",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::YakuScholar,
            name: "Yaku Scholar",
            description: "Active loadout has 4 yaku slots instead of 3",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::EightTreasures,
            name: "Eight Treasures",
            description: "Scoring a Full Hand grants a random Zodiac",
            rarity: Rarity::Legendary,
        },
        RelicDef {
            id: RelicId::KongsBlessing,
            name: "Kong's Blessing",
            description: "Kongs: +120 chips and +2 mult",
            rarity: Rarity::Legendary,
        },
        // ── Flower-synergy relics ──────────────────────────────────────
        RelicDef {
            id: RelicId::GardenKeeper,
            name: "Garden Keeper",
            description: "Flower effects fire twice",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Ikebana,
            name: "Ikebana",
            description: "Scoring 2+ flowers in one hand: +6 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::Hanami,
            name: "Hanami",
            description: "+$3 gold each time a flower is scored",
            rarity: Rarity::Common,
        },
        // ── 15 new relics ──────────────────────────────────────────────
        RelicDef {
            id: RelicId::JadeSerpent,
            name: "Jade Serpent",
            description: "Bamboo tiles in scored sets: +8 chips each",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::InkBrush,
            name: "Ink Brush",
            description: "Characters tiles in scored sets: +8 chips each",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::PearlDiver,
            name: "Pearl Diver",
            description: "Dots tiles in scored sets: +8 chips each",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::LowTide,
            name: "Low Tide",
            description: "Tiles ranked 1-3 in scored sets: +6 chips each",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::MerchantsEye,
            name: "Merchant's Eye",
            description: "Relics cost $1 less in the shop",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::EdgeRunner,
            name: "Edge Runner",
            description: "Terminal tiles (1s and 9s) in scored sets: +12 chips each",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::LuckySeven,
            name: "Lucky Seven",
            description: "Rank-7 tiles in scored sets: +1.5 mult each",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Momentum,
            name: "Momentum",
            description: "+0.5 mult per play already used this round",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Minimalist,
            name: "Minimalist",
            description: "Playing a single pair: +4 mult",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::TurtleShell,
            name: "Turtle Shell",
            description: "+50 chips if your mult is below 3",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::ClosedGate,
            name: "Closed Gate",
            description: "All scored tiles are terminals or honors: +4 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::GoldFurnace,
            name: "Gold Furnace",
            description: "+1 mult per 5 gold held (max +4)",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::Snowball,
            name: "Snowball",
            description: "+0.1 mult per 100 total score this run (max +5)",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::SecondWind,
            name: "Second Wind",
            description: "+1 play per round",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::GlassCannon,
            name: "Glass Cannon",
            description: "×2 final mult, but 1 fewer play per round",
            rarity: Rarity::Legendary,
        },
        // CodexCompass — disabled because the relic has no scoring effect
        // and the in-round yaku-loadout swap UI doesn't exist. Re-enable
        // when the loadout-swap action is wired into the gameplay scene.
        // RelicDef {
        //     id: RelicId::CodexCompass,
        //     name: "Codex Compass",
        //     description: "Once per round: swap one yaku in your loadout",
        //     rarity: Rarity::Uncommon,
        // },
        // ── Balatro-inspired relics (Patch F) ──────────────────────────
        RelicDef {
            id: RelicId::LastBreath,
            name: "Last Breath",
            description: "On your final play, retrigger all scored tiles",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::TilePolisher,
            name: "Tile Polisher",
            description: "Every scored tile permanently gains +3 chips this run",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::PaperLantern,
            name: "Paper Lantern",
            description: "+6 mult; 1-in-5 chance to burn at round end",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::IronLantern,
            name: "Iron Lantern",
            description: "×2 mult; 1-in-1000 chance to break at round end",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::MirrorTile,
            name: "Mirror Tile",
            description: "Copies the effect of the next relic in your inventory",
            rarity: Rarity::Legendary,
        },
        RelicDef {
            id: RelicId::WayOfPurity,
            name: "Way of Purity",
            description: "All scored tiles are one suit: ×2.5 mult",
            rarity: Rarity::Rare,
        },
        // ── Patch G: 25 Balatro-inspired relics ────────────────────────
        // Retrigger
        RelicDef {
            id: RelicId::LeadingTile,
            name: "Leading Tile",
            description: "Retrigger the first tile in each scored set",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::LowEcho,
            name: "Low Echo",
            description: "Retrigger tiles ranked 1-4 in scored sets",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::TeaCeremony,
            name: "Tea Ceremony",
            description: "Retrigger all tiles for 3 plays, then destroyed",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::GhostHand,
            name: "Ghost Hand",
            description: "Unscored hand tiles each grant +2 chips",
            rarity: Rarity::Uncommon,
        },
        // Scaling
        RelicDef {
            id: RelicId::CleanStreak,
            name: "Clean Streak",
            description: "+0.5 mult per consecutive play without honors",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Obsession,
            name: "Obsession",
            description: "+0.3 mult per round without your top yaku",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::Bonfire,
            name: "Bonfire",
            description: "+0.4 mult per relic sold this run",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::RiverRunner,
            name: "River Runner",
            description: "+20 chips permanently each time you score a sequence",
            rarity: Rarity::Rare,
        },
        // Fragile
        RelicDef {
            id: RelicId::MeltingIce,
            name: "Melting Ice",
            description: "+80 chips, loses 8 per play (destroyed at 0)",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::SilkThread,
            name: "Silk Thread",
            description: "+4 mult, loses 0.3 per discard (destroyed at 0)",
            rarity: Rarity::Uncommon,
        },
        // Copy / Meta
        RelicDef {
            id: RelicId::ShadowHand,
            name: "Shadow Hand",
            description: "Copies the effect of your first relic",
            rarity: Rarity::Legendary,
        },
        RelicDef {
            id: RelicId::EmptyFrame,
            name: "Empty Frame",
            description: "+1.5 mult per empty relic slot",
            rarity: Rarity::Uncommon,
        },
        // Economy
        RelicDef {
            id: RelicId::GoldIdol,
            name: "Gold Idol",
            description: "+3 gold at round end",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::JadeAbacus,
            name: "Jade Abacus",
            description: "+1 interest per 4 gold held (max +4)",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::NestEgg,
            name: "Nest Egg",
            description: "Gains +2 sell value each round; sell when ripe",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::Patience,
            name: "Patience",
            description: "+2 gold per unused discard at round end",
            rarity: Rarity::Common,
        },
        // Conditional ×mult
        RelicDef {
            id: RelicId::WayOfPairs,
            name: "Way of Pairs",
            description: "All pairs scored: ×2 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::WayOfTriplets,
            name: "Way of Triplets",
            description: "All triplets/kongs scored: ×2.5 mult",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::WayOfSequences,
            name: "Way of Sequences",
            description: "All sequences scored: ×2 mult",
            rarity: Rarity::Rare,
        },
        // Probability / Chaos
        RelicDef {
            id: RelicId::FortunesFavor,
            name: "Fortune's Favor",
            description: "Doubles all relic trigger probabilities",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::CrackedTile,
            name: "Cracked Tile",
            description: "+0 to +8 mult (random per play)",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::StarTile,
            name: "Star Tile",
            description: "1-in-4 chance to level up a scored yaku",
            rarity: Rarity::Uncommon,
        },
        // Sell-to-activate
        RelicDef {
            id: RelicId::SmokeBomb,
            name: "Smoke Bomb",
            description: "Sell to skip the current boss blind",
            rarity: Rarity::Rare,
        },
        // PhantomRelic not in shop — appears via special means only.
        RelicDef {
            id: RelicId::PhantomRelic,
            name: "Phantom Relic",
            description: "After 3 rounds, sell to duplicate a random relic",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::RitualBlade,
            name: "Ritual Blade",
            description: "Destroy the next relic for permanent mult",
            rarity: Rarity::Rare,
        },
    ]
}

/// Active relics during a run (by id).
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Swap two relics by index. Used by the rearrange UI (MirrorTile
    /// copies the relic to its right, so ordering matters).
    pub fn swap_relics(&mut self, a: usize, b: usize) {
        if a < self.active.len() && b < self.active.len() {
            self.active.swap(a, b);
        }
    }

    /// Returns the RelicId immediately *after* `id` in the active list,
    /// or `None` if `id` is the last relic (or absent). Used by Mirror
    /// Tile to find the relic it copies.
    pub fn relic_after(&self, id: RelicId) -> Option<RelicId> {
        let pos = self.active.iter().position(|&r| r == id)?;
        self.active.get(pos + 1).copied()
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
    /// Current ante's round wind (1=East, 2=South, 3=West, 4=North) — drives
    /// the round-wind branch of the Yakuhai yaku. `None` outside a run.
    pub round_wind: Option<u8>,
    /// True if this play would be the *first* FullHand of the round. The
    /// scoring cascade fires the Tenpai Bonus only when this is true and the
    /// hand actually completes as a FullHand. The bonus's chip pile scales
    /// down as `plays_used` grows.
    pub first_full_hand_of_round: bool,
    /// Plays already consumed this round at the moment of scoring (so the
    /// current play is `plays_used + 1`-th). Used to scale the Tenpai Bonus.
    pub plays_used: u32,
    /// True if the player has declared riichi this round and this hand
    /// completes the wait. When set and the play scores a FullHand, the
    /// Phase 6.5 hook applies a 2× mult. Riichi UI is Patch E; the field
    /// exists now so the scoring spine is ready.
    pub riichi_active: bool,
    /// Per-yaku level (Zodiac-card-driven). `None` falls back to all level 1
    /// — used by tests and the bot.
    pub yaku_levels: Option<crate::core::zodiac::YakuLevels>,
    /// Active yaku loadout: yaku in this list contribute at full strength,
    /// others (except the always-active FullHand and Yakuhai) contribute at
    /// half. Empty list = treat all detected yaku as full-strength (test/bot
    /// default).
    pub yaku_loadout: Vec<crate::core::yaku::YakuKind>,
    /// Yaku detected on prior plays in the current round, used by The Censor
    /// boss to halve repeat-yaku contributions. Empty in normal rounds.
    pub played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    /// Player's current gold at the moment of scoring (for Gold Furnace).
    pub gold: i32,
    /// Cumulative score earned across the entire run (for Snowball).
    pub total_score: u64,
    /// True when this is the player's last remaining play this round
    /// (plays_remaining == 1 at scoring time). Powers Last Breath.
    pub is_final_play: bool,
    /// Permanent per-tile chip bonus accumulated by the Tile Polisher
    /// relic over the course of the run.
    pub tile_polisher_bonus: i32,
    /// Per-relic mutable counters (clean_streak, melting_ice chips, etc.).
    /// Keyed by RelicId, value meaning varies by relic.
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    /// Number of hand tiles NOT in the scored sets (for Ghost Hand).
    pub unscored_hand_tiles: usize,
    /// River Runner accumulated permanent chip bonus.
    pub river_runner_bonus: i32,
}

// All scoring effects now live in `core::scoring::score_sets` directly,
// reading relic ids off the `ScoreContext`. The chip/mult model made the
// per-relic helper layer redundant — each relic is one match arm in
// `score_sets`, which is easier to read end-to-end than a chain of
// `*_multiplier` accessors.
