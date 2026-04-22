//! Relic definitions and runtime application hooks.

use std::collections::BTreeSet;

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
    /// After refill, if the hand contains a partial you've invested in — a
    /// pair wanting a third, a triplet wanting a kong, or two numbered tiles
    /// of the same suit within 2 ranks wanting a sequence — draw 1 extra tile
    /// from the wall.
    ShantenShove,
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
    /// +1 zodiac inventory slot; every 3rd Zodiac you use is duplicated.
    LunarAlmanac,
    /// Scoring a FullHand grants 1 random Zodiac card (ignores slot cap).
    EightTreasures,
    /// Kongs grant +120 chips and +2 mult each. (The original "counts as
    /// both triplet and pair" semantic was never wired into yaku detection;
    /// this flat bonus replaces it as a real, scoring effect.)
    KongsBlessing,
    /// Reserved relic id; the old yaku-loadout swap mechanic no longer exists.
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
    RedSerpent,
    /// Dots-suit tiles in scored sets: +8 chips each.
    BlueSerpent,
    /// Tiles ranked 1–3 in scored sets: +6 chips each.
    LowTide,
    /// Relics cost 25% less in the shop, rounded down (minimum $1).
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
    /// +1 mult per 5 gold held.
    GoldFurnace,
    /// +0.1 mult per 100 total score earned this run.
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
    /// East + n West tiles count as a pair / triplet / kong (n = 1 / 2 / 3).
    /// Validation happens by relabelling the West tiles as East before the
    /// standard meld decomposition runs.
    Disgust,
    // ── Patch H: economy & scaling relics ─────────────────────────────
    /// +mult equal to the summed live sell value of every *other* relic
    /// in your inventory. Grows as relics accumulate sell-value counters
    /// (e.g. Nest Egg) and as you collect more relics.
    CurioCabinet,
    /// +0.5 mult permanently each time a flower is drawn or scored.
    /// Counter lives in `relic_counters[LotusBloom]`.
    LotusBloom,
    /// +0.2 mult per tile in the wall beyond the base 140. Sums Overflow's
    /// 68 extras and any tiles added mid-run (tracked in
    /// `relic_counters[WallWeaver]`).
    WallWeaver,
    /// +$5 per kong scored this round, paid at round end. Counter in
    /// `relic_counters[KongCollector]` resets on round advance.
    KongCollector,
    /// +$1 each time an honor tile is discarded.
    NoHonorButWealth,
    /// Round start: 25% +$2, 25% +$4, 50% nothing.
    Sweepstakes,
    /// +$1 at round end; permanent +$1 per boss blind defeated
    /// (tracked in `relic_counters[BeggarsCup]`).
    BeggarsCup,
    /// +$1 at round end per unique yaku scored this round.
    Cosmopolitan,
    /// +1 mult per blind *played* this run (skips don't count). Counter in
    /// `relic_counters[Heirloom]` increments in `advance_round`, which runs
    /// only after clearing a blind — `skip_to_next_blind` is a separate path.
    Heirloom,
    /// +3 mult per distinct suit among scored tiles (Flower counts).
    Tourist,
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
            RelicId::KanDrum => "kan_drum.png",
            RelicId::DoraCrown => "dora_crown.png",
            RelicId::RiichiStick => "riichi_stick.png",
            RelicId::TenpaiTalisman => "tenpai_talisman.png",
            RelicId::RiverEraser => "river_eraser.png",
            RelicId::FuritenWard => "furiten_ward.png",
            RelicId::RoundCompass => "round_compass.png",
            RelicId::LunarAlmanac => "lunar_almanac.png",
            RelicId::EightTreasures => "eight_treasures.png",
            RelicId::KongsBlessing => "kongs_blessing.png",
            RelicId::CodexCompass => "codex_compass.png",
            RelicId::GardenKeeper => "garden_keeper.png",
            RelicId::Ikebana => "ikebana.png",
            RelicId::Hanami => "hanami.png",
            RelicId::JadeSerpent => "jade_serpent.png",
            RelicId::RedSerpent => "red_serpent.png",
            RelicId::BlueSerpent => "blue_serpent.png",
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
            RelicId::Disgust => "disgust.png",
            RelicId::CurioCabinet => "curio_cabinet.png",
            RelicId::LotusBloom => "lotus_bloom.png",
            RelicId::WallWeaver => "wall_weaver.png",
            RelicId::KongCollector => "kong_collector.png",
            RelicId::NoHonorButWealth => "no_honor_but_wealth.png",
            RelicId::Sweepstakes => "sweepstakes.png",
            RelicId::BeggarsCup => "beggars_cup.png",
            RelicId::Cosmopolitan => "cosmopolitan.png",
            RelicId::Heirloom => "heirloom.png",
            RelicId::Tourist => "tourist.png",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelicRenderMaterial {
    Iron,
    Copper,
    Silver,
    Gold,
}

#[derive(Clone, Debug)]
pub struct RelicDef {
    pub id: RelicId,
    pub name: &'static str,
    pub description: &'static str,
    pub rarity: Rarity,
}

#[derive(Clone, Copy, Debug)]
pub struct RelicVisualDef {
    pub material: RelicRenderMaterial,
    pub ui_tilt_x_deg: f32,
    pub ui_spin_rate_deg: f32,
    pub thickness_scale: f32,
}

impl RelicId {
    /// Runtime albedo texture used by the 3D relic meshes.
    ///
    /// **Load order** for the renderer (albedo → mask → height) is documented on
    /// [`crate::render::relic_pipeline`].
    pub fn render_texture_path(self) -> String {
        format!("textures/relics/{}", self.asset_filename())
    }

    /// Offline-generated transparent object render used as the source of truth
    /// for relic visual development and future mesh/height derivation.
    pub fn source_object_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/source/{}_object.png", stem)
    }

    /// Optional binary or transparent silhouette image used to derive the
    /// runtime relic mesh more deterministically than the shaded object render.
    pub fn source_mask_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/source/{}_mask.png", stem)
    }

    /// Optional offline-generated grayscale relief source for future embossed
    /// or carved detailing on the 3D relic mesh.
    pub fn source_heightmap_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/source/{}_height.png", stem)
    }

    /// Derived runtime silhouette used when the offline workflow emits a
    /// cleaned-up alpha mask alongside the visible relic texture.
    pub fn render_mask_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/{}_mask.png", stem)
    }
}

pub fn relic_visual(id: RelicId) -> RelicVisualDef {
    use RelicRenderMaterial::{Copper, Gold, Iron, Silver};

    let rarity = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.rarity)
        .unwrap_or(Rarity::Common);

    let material = match rarity {
        Rarity::Common => Iron,
        Rarity::Uncommon => Copper,
        Rarity::Rare => Silver,
        Rarity::Legendary => Gold,
    };

    let (ui_tilt_x_deg, ui_spin_rate_deg, thickness_scale) = match material {
        Iron => (-18.0, 28.0, 1.0),
        Copper => (-18.0, 28.0, 1.0),
        Silver => (-18.0, 28.0, 1.02),
        Gold => (-18.0, 28.0, 1.04),
    };

    RelicVisualDef {
        material,
        ui_tilt_x_deg,
        ui_spin_rate_deg,
        thickness_scale,
    }
}

/// Gold cost to buy a relic in the shop. Stable (deterministic) per relic id so
/// the shop, bot, and any future tooling agree on prices.
pub fn relic_buy_price(id: RelicId) -> u32 {
    let rarity = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.rarity)
        .unwrap_or(Rarity::Common);
    match rarity {
        Rarity::Common => 6,
        Rarity::Uncommon => 8,
        Rarity::Rare => 10,
        Rarity::Legendary => 12,
    }
}

/// Effective gold cost to buy a relic in the shop after active price
/// modifiers are applied.
pub fn relic_shop_price(id: RelicId, relics: &RelicState) -> u32 {
    let mut price = relic_buy_price(id);
    if relics.has(RelicId::MerchantsEye) {
        price = (price * 3 / 4).max(1);
    }
    price
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

/// Effective sell price for a relic, accounting for counter-based bonuses
/// (e.g. Nest Egg grows by +2 per round held).
pub fn relic_sell_price_live(
    id: RelicId,
    counters: &std::collections::BTreeMap<RelicId, i32>,
) -> u32 {
    let mut sell = relic_sell_price(id);
    if id == RelicId::NestEgg {
        let rounds = counters.get(&RelicId::NestEgg).copied().unwrap_or(0);
        sell = sell.saturating_add(2 * rounds as u32);
    }
    sell
}

/// Return a live description for relics whose counters change their tooltip.
/// Falls back to the static `RelicDef::description` when no counter applies.
pub fn relic_description_live(
    id: RelicId,
    counters: &std::collections::BTreeMap<RelicId, i32>,
    total_score: u64,
) -> String {
    let base = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.description)
        .unwrap_or("");
    match id {
        RelicId::MeltingIce => {
            let remaining = counters.get(&RelicId::MeltingIce).copied().unwrap_or(80);
            format!("{base} [{remaining} chips left]")
        }
        RelicId::SilkThread => {
            let thread = counters.get(&RelicId::SilkThread).copied().unwrap_or(40);
            format!("{base} [+{:.1} mult left]", thread as f64 / 10.0)
        }
        RelicId::TeaCeremony => {
            let charges = counters.get(&RelicId::TeaCeremony).copied().unwrap_or(3);
            format!(
                "{base} [{charges} charge{} left]",
                if charges == 1 { "" } else { "s" }
            )
        }
        RelicId::CleanStreak => {
            let streak = counters.get(&RelicId::CleanStreak).copied().unwrap_or(0);
            format!(
                "{base} [streak: {streak}, +{:.1} mult]",
                0.5 * streak as f64
            )
        }
        RelicId::Obsession => {
            let rounds = counters.get(&RelicId::Obsession).copied().unwrap_or(0);
            format!(
                "{base} [{rounds} round{}, +{:.1} mult]",
                if rounds == 1 { "" } else { "s" },
                0.3 * rounds as f64
            )
        }
        RelicId::Bonfire => {
            let sold = counters.get(&RelicId::Bonfire).copied().unwrap_or(0);
            format!("{base} [{sold} sold, +{:.1} mult]", 0.4 * sold as f64)
        }
        RelicId::RitualBlade => {
            let perm = counters.get(&RelicId::RitualBlade).copied().unwrap_or(0);
            format!("{base} [+{:.1} mult stored]", perm as f64 / 10.0)
        }
        RelicId::PhantomRelic => {
            let rounds = counters.get(&RelicId::PhantomRelic).copied().unwrap_or(0);
            if rounds >= 3 {
                format!("{base} [ready to duplicate!]")
            } else {
                format!("{base} [{rounds}/3 rounds]")
            }
        }
        RelicId::NestEgg => {
            let rounds = counters.get(&RelicId::NestEgg).copied().unwrap_or(0);
            let sell = relic_sell_price_live(id, counters);
            format!(
                "{base} [held {rounds} round{}, sell {sell}g]",
                if rounds == 1 { "" } else { "s" }
            )
        }
        RelicId::Snowball => {
            let bonus = total_score as f64 / 1000.0;
            format!("{base} [current +{bonus:.1} mult]")
        }
        _ => base.to_string(),
    }
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
            description: "Drawing a 3rd copy of a tile pulls the 4th from the wall",
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
            description: "After refill, if a pair/triplet/sequence partial is in hand, draw 1 extra tile",
            rarity: Rarity::Uncommon,
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
            id: RelicId::LunarAlmanac,
            name: "Lunar Almanac",
            description: "+1 Zodiac slot; every 3rd Zodiac use is duplicated",
            rarity: Rarity::Rare,
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
            id: RelicId::RedSerpent,
            name: "Red Serpent",
            description: "Characters tiles in scored sets: +8 chips each",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::BlueSerpent,
            name: "Blue Serpent",
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
            description: "Relics cost 25% less in the shop",
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
            description: "+1 mult per 5 gold held",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::Snowball,
            name: "Snowball",
            description: "+0.1 mult per 100 total score this run",
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
        // CodexCompass stays disabled because its old loadout-swap mechanic
        // no longer exists and it has no replacement scoring effect yet.
        // RelicDef {
        //     id: RelicId::CodexCompass,
        //     name: "Codex Compass",
        //     description: "Reserved relic slot",
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
        RelicDef {
            id: RelicId::Disgust,
            name: "Disgust",
            description: "East + 1/2/3 West tiles count as pair/triplet/kong",
            rarity: Rarity::Rare,
        },
        // ── Patch H: economy & scaling relics ──────────────────────────
        RelicDef {
            id: RelicId::CurioCabinet,
            name: "Curio Cabinet",
            description: "+mult equal to the summed sell value of your other relics",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::LotusBloom,
            name: "Lotus Bloom",
            description: "+0.5 mult permanently per flower drawn or scored",
            rarity: Rarity::Rare,
        },
        RelicDef {
            id: RelicId::WallWeaver,
            name: "Wall Weaver",
            description: "+0.2 mult per tile in the wall beyond 140",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::KongCollector,
            name: "Kong Collector",
            description: "+$5 per kong scored this round, paid at round end",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::NoHonorButWealth,
            name: "No Honor But Wealth",
            description: "+$1 each time an honor tile is discarded",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::Sweepstakes,
            name: "Sweepstakes",
            description: "Round start: 25% +$2, 25% +$4, 50% nothing",
            rarity: Rarity::Common,
        },
        RelicDef {
            id: RelicId::BeggarsCup,
            name: "Beggar's Cup",
            description: "+$1 at round end, +$1 more per boss defeated",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Cosmopolitan,
            name: "Cosmopolitan",
            description: "+$1 at round end per unique yaku scored this round",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Heirloom,
            name: "Heirloom",
            description: "+1 mult per blind played (skips don't count)",
            rarity: Rarity::Uncommon,
        },
        RelicDef {
            id: RelicId::Tourist,
            name: "Tourist",
            description: "+3 mult per distinct suit among scored tiles",
            rarity: Rarity::Uncommon,
        },
    ]
}

/// Active relics during a run (by id).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelicState {
    pub active: Vec<RelicId>,
    pub max_slots: usize,
    #[serde(default)]
    pub debuffed: BTreeSet<RelicId>,
}

impl Default for RelicState {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            max_slots: 5,
            debuffed: BTreeSet::new(),
        }
    }
}

impl RelicState {
    pub fn has(&self, id: RelicId) -> bool {
        self.active.contains(&id) && !self.debuffed.contains(&id)
    }

    pub fn owns(&self, id: RelicId) -> bool {
        self.active.contains(&id)
    }

    pub fn is_full(&self) -> bool {
        self.active.len() >= self.max_slots
    }

    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn enabled_len(&self) -> usize {
        self.active
            .iter()
            .filter(|&&id| !self.debuffed.contains(&id))
            .count()
    }

    pub fn is_debuffed(&self, id: RelicId) -> bool {
        self.active.contains(&id) && self.debuffed.contains(&id)
    }

    pub fn clear_debuffs(&mut self) {
        self.debuffed.clear();
    }

    pub fn set_debuffed<I>(&mut self, relics: I)
    where
        I: IntoIterator<Item = RelicId>,
    {
        self.debuffed.clear();
        for id in relics {
            if self.active.contains(&id) {
                self.debuffed.insert(id);
            }
        }
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
    pub tile_debuffs: &'a [crate::core::debuff::TileDebuff],
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
    /// When set, this score is from **structure trigger** (not a direct hand play).
    ///
    /// **Structure migration:** relics that used to fire on every "scoring play" may need a
    /// split: effects that should happen when melds **land** (`RunState::commit_selection_to_structure`)
    /// vs when the player **cashes in** (`trigger_structure`). Examples wired today: MeltingIce /
    /// Tea / CleanStreak on commit; TilePolisher / River Runner / Star Tile / KanDrum / scoring
    /// cascades on trigger. Audit new "per play" relics against this split.
    pub structure: Option<crate::core::structure::StructureTriggerMeta>,
}

// All scoring effects now live in `core::scoring::score_sets` directly,
// reading relic ids off the `ScoreContext`. The chip/mult model made the
// per-relic helper layer redundant — each relic is one match arm in
// `score_sets`, which is easier to read end-to-end than a chain of
// `*_multiplier` accessors.

#[cfg(test)]
mod tests {
    use super::{RelicId, RelicState, relic_buy_price, relic_shop_price};

    #[test]
    fn relic_shop_price_matches_base_without_merchants_eye() {
        let relics = RelicState::default();

        assert_eq!(
            relic_shop_price(RelicId::TripletBoost, &relics),
            relic_buy_price(RelicId::TripletBoost)
        );
    }

    #[test]
    fn merchants_eye_reduces_shop_price_by_25_percent() {
        let mut relics = RelicState::default();
        relics.active.push(RelicId::MerchantsEye);

        let base = relic_buy_price(RelicId::TripletBoost);
        assert_eq!(
            relic_shop_price(RelicId::TripletBoost, &relics),
            (base * 3 / 4).max(1)
        );
    }
}
