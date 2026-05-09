//! Relic definitions and runtime application hooks.
//!
//! Display metadata (name, description, rarity) lives in
//! `assets/data/relics.json` so balance edits don't require recompiling
//! the core crate. In debug builds rust-embed reads the file from disk on
//! every game start; in release builds it is baked into the binary.
//! Behaviour (scoring hooks, prices, visuals) stays in Rust.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::tile::Suit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelicId {
    // ── Core scoring & dragons ─────────────────────────────────────────
    TripletBoost,
    SequenceSurge,
    PairPower,
    HonorFury,
    RedDragonRage,
    GreenLuck,
    WhiteDragonsHush,
    JokerTile,
    StrengthInNumbers,
    QuickDraw,
    ChainReaction,
    MultiplierMaster,
    SetMagnet,
    WildWinds,
    DragonEcho,
    // ── Draw tempo, dora, wind, zodiac ──────────────────────────────────
    /// Kongs grant +1 play this round and +4 mult when scored.
    KanDrum,
    /// Reveal an extra dora indicator at round start; dora chips become +35.
    DoraCrown,
    /// Round Wind triplets/kongs grant +6 mult instead of the base +3.
    RoundCompass,
    /// Scoring a FullHand grants 1 random Zodiac card (ignores slot cap).
    EightTreasures,
    /// Kongs grant +120 chips and +2 mult each when scored.
    KongsBlessing,
    // ── Flower-synergy relics ──────────────────────────────────────────
    /// Each flower's triggered effect fires a second time.
    GardenKeeper,
    /// Scoring 2+ flowers in one hand grants +6 mult.
    Ikebana,
    /// Each flower scored grants +3 gold immediately.
    Hanami,
    // ── Suit, rank, economy ───────────────────────────────────────────
    /// Bamboo-suit tiles in scored sets: +8 chips each.
    JadeSerpent,
    /// Characters-suit tiles in scored sets: +8 chips each.
    RedSerpent,
    /// Dots-suit tiles in scored sets: +8 chips each.
    BlueSerpent,
    /// Tiles ranked 1–3 in scored sets: +6 chips each.
    LowTide,
    /// Tiles ranked 7–9 in scored sets: +6 chips each.
    HighTide,
    /// Relics cost 25% less in the shop, rounded down (minimum $1).
    MerchantsEye,
    /// Three shop restocks this run cost no gold (escalating restock prices still apply).
    IGotAGuy,
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
    GoldenEngine,
    /// +0.1 mult per 100 total score earned this run.
    Snowball,
    /// +1 play per round.
    SecondWind,
    /// ×3 final mult on the next scored hand, then destroyed (destruction).
    GlassCannon,
    // ── Retrigger starters, mirrors, suit purity ───────────────────────
    /// On your final play of the round, retrigger all scored tiles (they
    /// each contribute their chip value a second time).
    LastBreath,
    /// Every scored tile permanently gains +3 chips for the rest of the
    /// run. Accumulated in `relic_counters[TilePolisher]`.
    TilePolisher,
    /// +6 mult, but 1-in-5 chance to be destroyed at end of each round.
    /// When destroyed, replaced by Silver Filigree Lantern.
    PaperLantern,
    /// Replaces Paper Lantern when it burns. ×2 final mult, 1-in-1000
    /// chance to break at end of round.
    SilverFiligreeLantern,
    /// Copies the scoring effect of the relic immediately after it in
    /// the player's relic inventory. No effect if it's the last slot.
    MirrorTile,
    /// ×2.5 mult if every scored tile belongs to a single numbered suit.
    WayOfPurity,
    // ── Broad relic pool ───────────────────────────────────────────────
    // Retrigger
    /// Retrigger the first 5 scored tiles in the hand. Hidden from Collection
    /// and shops until XXXL Egg burns once (profile unlock).
    Geese,
    /// Retrigger tiles ranked 1–4 in scored sets.
    VoiceOfThePeople,
    /// Retrigger tiles ranked 6–9 in scored sets.
    VoiceOfTheElite,
    /// Retrigger all scored tiles for 3 plays, then burns (slot empties);
    /// XXXL Egg cannot shop-roll again and Geese returns to the pool this run.
    #[serde(rename = "xxxl_egg")]
    RustlingGooseEgg,
    /// Four scored hands — Harmony → Respect → Purity → Tranquility — then this
    /// slot becomes Rakuware (all four beats together on future scores).
    TeaCeremony,
    /// Raku chawan. Each score, applies every Tea Ceremony beat (Harmony, Respect,
    /// Purity, Tranquility) when its condition is met. Shop-only until Tea completes this run.
    Rakuware,
    /// Chips equal to the sum of **point values** of hand tiles that are not in
    /// the scored melds (structure cash-in: all tiles still in hand). HUD shows a live preview.
    GhostHand,
    // Scaling
    /// +0.5 mult per consecutive play without honor tiles. Resets when
    /// honors are scored.
    Humility,
    /// +0.3 mult per round you don't score your most-used yaku.
    Obsession,
    /// +0.4 mult per relic sold this run.
    Bonfire,
    /// +20 chips permanently each time you score a sequence.
    RiverRunner,
    // Fragile
    /// +80 chips, loses 8 chips per play. At 0 the relic burns (slot empties);
    /// Taotie enters the shop pool this run.
    MeltingIce,
    /// Successor to Melting Ice after it burns — buy from shop. Permanent +80 chips base.
    /// At cash-in, every scored honor (wind/dragon) tile is destroyed: the
    /// tile is permanently removed from the run's wall (added to
    /// `removed_tile_ids`, the same primitive Kiln uses) and Taotie's chip
    /// bonus grows by +20 per tile. `relic_counters[Taotie]` holds
    /// the accumulated chip bonus; divide by 20 for the honor count
    /// shown in the live tooltip.
    Taotie,
    /// +4 mult, loses 0.3 mult per discard. At 0 the relic burns (slot empties);
    /// Silk Moth enters the shop pool this run.
    SilkThread,
    /// Successor to Silk Thread after it burns — buy from shop. +2 mult on every
    /// scored hand and +$1 every discard. Counter `relic_counters[SilkMoth]`
    /// tracks cumulative gold paid out across the run for the live tooltip.
    SilkMoth,
    // Copy / Meta
    /// Copies the effect of the first relic in your inventory.
    ShadowHand,
    /// +1.5 mult per empty relic slot.
    SolitarySage,
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
    HungryGhost,
    /// East + n West tiles count as a pair / triplet / kong (n = 1 / 2 / 3).
    /// Validation happens by relabelling the West tiles as East before the
    /// standard meld decomposition runs.
    Disgust,
    // ── Run economy, wall scaling, modifiers ──────────────────────────
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
    /// Each time another relic is destroyed, gain a permanent +1 mult.
    /// Kintsugi itself is never destroyed by any effect. Counter lives in
    /// `relic_counters[Kintsugi]`.
    Kintsugi,
    /// Sequences may wrap 9→1 freely (9-1-2, 8-9-1 are valid sequences).
    AntTrail,
    /// Buff talismans (Pearl/Gilded/Polychrome) apply their enhancement
    /// to every tile drawn for the rest of the run, not just the 14 currently
    /// in hand. Also grants +1 consumable inventory slot.
    BrocadePouch,
    /// Flat +e mult each scored hand (`e` is [`std::f64::consts::E`]).
    EulersNumber,
    /// Flat +π mult each scored hand (`π` is [`std::f64::consts::PI`]).
    PiConstant,
    /// +2 hand tiles per round, −1 discard per round.
    BigHands,
    /// −2 hand tiles per round, +2 discards per round.
    TinyHands,
    /// After the blind target is met, further scoring this round is absorbed
    /// into `relic_counters[MonarchButterfly]`; at threshold excess, hatches
    /// into Monarch Butterfly in-slot.
    Chrysalis,
    /// Shop-only until Chrysalis hatches this run. +chips per score from a
    /// log tier derived from absorbed excess (`relic_counters[MonarchButterfly]`).
    MonarchButterfly,
}

/// Total absorbed excess (post-target score) needed for Chrysalis to transform.
pub const CHRYSALIS_HATCH_EXCESS_THRESHOLD: i32 = 2000;

/// Chips per log-tier from [`monarch_butterfly_tier`].
pub const MONARCH_CHIPS_PER_TIER: i32 = 12;

/// Max tier for Monarch Butterfly chip bonus.
pub const MONARCH_TIER_CAP: i32 = 24;

/// Tier from cumulative absorbed excess: `ilog2(excess + 1)`, capped (log-spaced tiers).
#[inline]
pub fn monarch_butterfly_tier(excess: i32) -> i32 {
    let e = excess.max(0) as u32;
    if e == 0 {
        return 0;
    }
    let t = e.saturating_add(1).ilog2() as i32;
    t.min(MONARCH_TIER_CAP)
}

#[inline]
pub fn monarch_butterfly_bonus_chips(excess: i32) -> i32 {
    monarch_butterfly_tier(excess).saturating_mul(MONARCH_CHIPS_PER_TIER)
}

/// Smallest excess value at which the tier would exceed the current tier (hint for tooltips).
#[inline]
pub fn monarch_next_tier_excess_floor(current_excess: i32) -> Option<i32> {
    let t = monarch_butterfly_tier(current_excess);
    if t >= MONARCH_TIER_CAP {
        return None;
    }
    let k = (t + 1) as u32;
    if k >= 31 {
        return None;
    }
    Some(((1i64 << k) - 1).clamp(0, i32::MAX as i64) as i32)
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
            RelicId::WhiteDragonsHush => "white_dragons_hush.png",
            RelicId::JokerTile => "joker_tile.png",
            RelicId::StrengthInNumbers => "strength_in_numbers.png",
            RelicId::QuickDraw => "quick_draw.png",
            RelicId::ChainReaction => "chain_reaction.png",
            RelicId::MultiplierMaster => "multiplier_master.png",
            RelicId::SetMagnet => "set_magnet.png",
            RelicId::WildWinds => "wild_winds.png",
            RelicId::DragonEcho => "dragon_echo.png",
            // Filenames for icons; loader falls back to the relic slug if missing.
            RelicId::KanDrum => "kan_drum.png",
            RelicId::DoraCrown => "dora_crown.png",
            RelicId::RoundCompass => "round_compass.png",
            RelicId::EightTreasures => "eight_treasures.png",
            RelicId::KongsBlessing => "kongs_blessing.png",
            RelicId::GardenKeeper => "garden_keeper.png",
            RelicId::Ikebana => "ikebana.png",
            RelicId::Hanami => "hanami.png",
            RelicId::JadeSerpent => "jade_serpent.png",
            RelicId::RedSerpent => "red_serpent.png",
            RelicId::BlueSerpent => "blue_serpent.png",
            RelicId::LowTide => "low_tide.png",
            RelicId::HighTide => "high_tide.png",
            RelicId::MerchantsEye => "merchants_eye.png",
            RelicId::IGotAGuy => "i_got_a_guy.png",
            RelicId::EdgeRunner => "edge_runner.png",
            RelicId::LuckySeven => "lucky_seven.png",
            RelicId::Momentum => "momentum.png",
            RelicId::Minimalist => "minimalist.png",
            RelicId::TurtleShell => "turtle_shell.png",
            RelicId::ClosedGate => "closed_gate.png",
            RelicId::GoldenEngine => "golden_engine.png",
            RelicId::Snowball => "snowball.png",
            RelicId::SecondWind => "second_wind.png",
            RelicId::GlassCannon => "glass_cannon.png",
            RelicId::LastBreath => "last_breath.png",
            RelicId::TilePolisher => "tile_polisher.png",
            RelicId::PaperLantern => "paper_lantern.png",
            RelicId::SilverFiligreeLantern => "silver_filigree_lantern.png",
            RelicId::MirrorTile => "mirror_tile.png",
            RelicId::WayOfPurity => "way_of_purity.png",
            RelicId::Geese => "geese.png",
            RelicId::VoiceOfThePeople => "voice_of_the_people.png",
            RelicId::VoiceOfTheElite => "voice_of_the_elite.png",
            RelicId::RustlingGooseEgg => "xxxl_egg.png",
            RelicId::TeaCeremony => "tea_ceremony.png",
            RelicId::Rakuware => "rakuware.png",
            RelicId::GhostHand => "ghost_hand.png",
            RelicId::Humility => "humility.png",
            RelicId::Obsession => "obsession.png",
            RelicId::Bonfire => "bonfire.png",
            RelicId::RiverRunner => "river_runner.png",
            RelicId::MeltingIce => "melting_ice.png",
            RelicId::Taotie => "taotie.png",
            RelicId::SilkThread => "silk_thread.png",
            RelicId::SilkMoth => "silk_moth.png",
            RelicId::ShadowHand => "shadow_hand.png",
            RelicId::SolitarySage => "solitary_sage.png",
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
            RelicId::HungryGhost => "hungry_ghost.png",
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
            RelicId::Kintsugi => "kintsugi.png",
            RelicId::AntTrail => "ant_trail.png",
            RelicId::BrocadePouch => "brocade_pouch.png",
            RelicId::EulersNumber => "eulers_number.png",
            RelicId::PiConstant => "pi_constant.png",
            RelicId::BigHands => "big_hands.png",
            RelicId::TinyHands => "tiny_hands.png",
            RelicId::Chrysalis => "chrysalis.png",
            RelicId::MonarchButterfly => "monarch_butterfly.png",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Deserialize)]
struct RelicDefRaw {
    id: RelicId,
    name: String,
    description: String,
    rarity: Rarity,
}

#[derive(Clone, Copy, Debug)]
pub struct RelicVisualDef {
    pub material: RelicRenderMaterial,
    pub ui_tilt_x_deg: f32,
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

    let (ui_tilt_x_deg, thickness_scale) = match material {
        Iron => (-18.0, 1.0),
        Copper => (-18.0, 1.0),
        Silver => (-18.0, 1.02),
        Gold => (-18.0, 1.04),
    };

    RelicVisualDef {
        material,
        ui_tilt_x_deg,
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
///
/// When `inventory_focus` is `Some((relics, slot_index))`, Mirror Tile and
/// Shadow Hand append which relic they copy and a compatibility line (ordering
/// matches gameplay).
pub fn relic_description_live(
    id: RelicId,
    counters: &std::collections::BTreeMap<RelicId, i32>,
    total_score: u64,
    inventory_focus: Option<(&RelicState, usize)>,
    ghost_hand_chips_preview: Option<i32>,
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
        RelicId::Taotie => {
            // Counter stores accumulated chips (20 per destroyed honor); the
            // honor count is the counter divided by that rate. Both numbers
            // are useful — count is the flavor read, chips is the math.
            let chips = counters.get(&RelicId::Taotie).copied().unwrap_or(0);
            let devoured = chips / 20;
            format!("{base} [{devoured} honors destroyed, +{chips} chips]")
        }
        RelicId::SilkThread => {
            let thread = counters.get(&RelicId::SilkThread).copied().unwrap_or(40);
            format!("{base} [+{:.1} mult left]", thread as f64 / 10.0)
        }
        RelicId::SilkMoth => {
            let paid = counters.get(&RelicId::SilkMoth).copied().unwrap_or(0);
            format!("{base} [${paid} produced]")
        }
        RelicId::RustlingGooseEgg => {
            let charges = counters.get(&RelicId::RustlingGooseEgg).copied().unwrap_or(3);
            format!(
                "{base} [{charges} charge{} left]",
                if charges == 1 { "" } else { "s" }
            )
        }
        RelicId::IGotAGuy => {
            let n = counters.get(&RelicId::IGotAGuy).copied().unwrap_or(0);
            format!(
                "{base} [{n} free restock{} left]",
                if n == 1 { "" } else { "s" }
            )
        }
        RelicId::TeaCeremony => {
            let phase = counters.get(&RelicId::TeaCeremony).copied().unwrap_or(0).clamp(0, 3);
            let names = ["Harmony", "Respect", "Purity", "Tranquility"];
            let label = names[phase as usize];
            let remain = 4 - phase as i32;
            format!(
                "{base} [next: {label}, {remain} hand{}]",
                if remain == 1 { "" } else { "s" }
            )
        }
        RelicId::Rakuware => {
            format!("{base} [Harmony · Respect · Purity · Tranquility]")
        }
        RelicId::Chrysalis => {
            let excess = counters
                .get(&RelicId::MonarchButterfly)
                .copied()
                .unwrap_or(0)
                .max(0);
            let need = CHRYSALIS_HATCH_EXCESS_THRESHOLD.max(1);
            format!("{base} [{excess}/{need} absorbed toward hatch]")
        }
        RelicId::MonarchButterfly => {
            let excess = counters
                .get(&RelicId::MonarchButterfly)
                .copied()
                .unwrap_or(0)
                .max(0);
            let tier = monarch_butterfly_tier(excess);
            let chips = monarch_butterfly_bonus_chips(excess);
            let next = monarch_next_tier_excess_floor(excess).map(|n| format!("next tier ≥{n}")).unwrap_or_else(|| "max tier".to_string());
            format!("{base} [tier {tier}, +{chips} chips, {excess} excess, {next}]")
        }
        RelicId::Humility => {
            let streak = counters.get(&RelicId::Humility).copied().unwrap_or(0);
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
        RelicId::HungryGhost => {
            let perm = counters.get(&RelicId::HungryGhost).copied().unwrap_or(0);
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
            let bonus = total_score as f64 / 5000.0;
            format!("{base} [current +{bonus:.1} mult]")
        }
        RelicId::Kintsugi => {
            let broken = counters.get(&RelicId::Kintsugi).copied().unwrap_or(0);
            format!("{base} [+{broken} mult]")
        }
        RelicId::RiverRunner => {
            let chips = counters.get(&RelicId::RiverRunner).copied().unwrap_or(0);
            format!("{base} [+{chips} chips]")
        }
        RelicId::GhostHand => {
            if let Some(n) = ghost_hand_chips_preview {
                format!("{base} [+{n} chips]")
            } else {
                base.to_string()
            }
        }
        RelicId::LotusBloom => {
            let blooms = counters.get(&RelicId::LotusBloom).copied().unwrap_or(0);
            format!(
                "{base} [{blooms} flower{}, +{:.1} mult]",
                if blooms == 1 { "" } else { "s" },
                0.5 * blooms as f64
            )
        }
        RelicId::MirrorTile => {
            let mut s = base.to_string();
            if let Some((relics, slot)) = inventory_focus {
                let extra = format_mirror_tile_inventory_help(relics, slot);
                if !extra.is_empty() {
                    s.push_str("\n\n");
                    s.push_str(&extra);
                }
            }
            s
        }
        RelicId::ShadowHand => {
            let mut s = base.to_string();
            if let Some((relics, slot)) = inventory_focus {
                let extra = format_shadow_hand_inventory_help(relics, slot);
                if !extra.is_empty() {
                    s.push_str("\n\n");
                    s.push_str(&extra);
                }
            }
            s
        }
        RelicId::Sweepstakes => {
            let ff = inventory_focus
                .is_some_and(|(relics, _)| relics.has(RelicId::FortunesFavor));
            if ff {
                format!(
                    "{base}\n\nFortune's Favor: round start becomes 1/3 +$2, 1/3 +$4, 1/3 nothing."
                )
            } else {
                base.to_string()
            }
        }
        _ => base.to_string(),
    }
}

pub fn all_relic_defs() -> &'static [RelicDef] {
    static DEFS: OnceLock<Vec<RelicDef>> = OnceLock::new();
    DEFS.get_or_init(load_relic_defs).as_slice()
}

fn load_relic_defs() -> Vec<RelicDef> {
    const PATH: &str = "data/relics.json";
    let raw: Vec<RelicDefRaw> = load_json_asset(PATH, "relic data");
    raw.into_iter()
        .map(|r| RelicDef {
            id: r.id,
            name: Box::leak(r.name.into_boxed_str()),
            description: Box::leak(r.description.into_boxed_str()),
            rarity: r.rarity,
        })
        .collect()
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

#[inline]
fn relic_display_name(id: RelicId) -> String {
    all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| format!("{id:?}"))
}

const COPY_RELIC_COMPATIBLE_SCORING_LINE: &str =
    "Compatible: hand scoring treats this relic as duplicated for chips and mult.";

#[inline]
fn push_debuffed_self_copy_relic_line(parts: &mut Vec<String>, relics: &RelicState, self_id: RelicId) {
    if relics.is_debuffed(self_id) {
        let name = relic_display_name(self_id);
        parts.push(format!("Debuffed: {name} does nothing while suppressed."));
    }
}

#[inline]
fn push_copy_target_line_with_debuff_note(
    parts: &mut Vec<String>,
    relics: &RelicState,
    target: RelicId,
    line_body: String,
    copy_source_display_name: &str,
) {
    let mut line = line_body;
    if relics.is_debuffed(target) {
        line.push_str(&format!(
            " That relic is debuffed, but {copy_source_display_name} still duplicates its scoring bonuses."
        ));
    }
    parts.push(line);
    if relic_scoring_copy_dup_is_compatible(target) {
        parts.push(COPY_RELIC_COMPATIBLE_SCORING_LINE.to_string());
    }
}

/// Tooltip helper: explain Mirror Tile's neighbor, scoring driver slot,
/// and rough compatibility with the copied relic.
pub fn format_mirror_tile_inventory_help(relics: &RelicState, mirror_slot: usize) -> String {
    let active = &relics.active;
    if active.get(mirror_slot) != Some(&RelicId::MirrorTile) {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::new();
    push_debuffed_self_copy_relic_line(&mut parts, relics, RelicId::MirrorTile);

    let first_mirror_slot = active.iter().position(|&r| r == RelicId::MirrorTile);
    let neighbor = active.get(mirror_slot + 1).copied();
    let mirror_name = relic_display_name(RelicId::MirrorTile);

    match neighbor {
        Some(tid) => {
            let name = relic_display_name(tid);
            push_copy_target_line_with_debuff_note(
                &mut parts,
                relics,
                tid,
                format!("Copying: {name}."),
                &mirror_name,
            );
        }
        None => {
            parts.push(
                "No relic to the right — reorder so another relic sits after this Mirror Tile."
                    .to_string(),
            );
        }
    }

    if let Some(fm) = first_mirror_slot {
        if fm != mirror_slot {
            let tgt = active.get(fm + 1).copied().map(relic_display_name);
            let tgt_s = tgt.unwrap_or_else(|| "nothing".into());
            parts.push(format!(
                "Scoring only uses the leftmost Mirror Tile (slot {} from the left). That one copies: {tgt_s}.",
                fm + 1
            ));
        } else {
            parts.push("This Mirror Tile is the one scoring checks use.".to_string());
        }
    }

    parts.join("\n")
}

/// Tooltip helper: explain Shadow Hand's copy target (always the leftmost relic
/// slot when that relic isn't Shadow Hand) and compatibility.
pub fn format_shadow_hand_inventory_help(relics: &RelicState, shadow_slot: usize) -> String {
    let active = &relics.active;
    if active.get(shadow_slot) != Some(&RelicId::ShadowHand) {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::new();
    push_debuffed_self_copy_relic_line(&mut parts, relics, RelicId::ShadowHand);

    let shadow_name = relic_display_name(RelicId::ShadowHand);

    match active.first().copied() {
        None => parts.push("No relics to copy.".to_string()),
        Some(RelicId::ShadowHand) => {
            parts.push(
                "Leftmost slot is Shadow Hand — move it right so another relic occupies the first slot; that relic is what gets copied."
                    .to_string(),
            );
        }
        Some(tid) => {
            let name = relic_display_name(tid);
            push_copy_target_line_with_debuff_note(
                &mut parts,
                relics,
                tid,
                format!("Copying: {name} (leftmost relic)."),
                &shadow_name,
            );
        }
    }

    parts.join("\n")
}

/// True when Mirror Tile / Shadow Hand duplication routes through the scoring
/// pipeline's `has` / `count` closure for this relic. Anything checked only with
/// raw `ctx.relics.has` there (e.g. Strength in Numbers overflow) is excluded.
fn relic_scoring_copy_dup_is_compatible(target: RelicId) -> bool {
    matches!(
        target,
        RelicId::TripletBoost
            | RelicId::SequenceSurge
            | RelicId::PairPower
            | RelicId::HonorFury
            | RelicId::KongsBlessing
            | RelicId::JadeSerpent
            | RelicId::RedSerpent
            | RelicId::BlueSerpent
            | RelicId::EdgeRunner
            | RelicId::LowTide
            | RelicId::HighTide
            | RelicId::TilePolisher
            | RelicId::LastBreath
            | RelicId::Geese
            | RelicId::VoiceOfThePeople
            | RelicId::VoiceOfTheElite
            | RelicId::RustlingGooseEgg
            | RelicId::TeaCeremony
            | RelicId::Rakuware
            | RelicId::GhostHand
            | RelicId::RiverRunner
            | RelicId::MeltingIce
            | RelicId::Taotie
            | RelicId::GardenKeeper
            | RelicId::Hanami
            | RelicId::DragonEcho
            | RelicId::DoraCrown
            | RelicId::RedDragonRage
            | RelicId::WhiteDragonsHush
            | RelicId::KanDrum
            | RelicId::RoundCompass
            | RelicId::Ikebana
            | RelicId::LuckySeven
            | RelicId::PaperLantern
            | RelicId::MultiplierMaster
            | RelicId::ChainReaction
            | RelicId::ClosedGate
            | RelicId::GoldenEngine
            | RelicId::Snowball
            | RelicId::Momentum
            | RelicId::Minimalist
            | RelicId::TurtleShell
            | RelicId::SilkThread
            | RelicId::SilkMoth
            | RelicId::Humility
            | RelicId::Obsession
            | RelicId::Bonfire
            | RelicId::Kintsugi
            | RelicId::SolitarySage
            | RelicId::CurioCabinet
            | RelicId::LotusBloom
            | RelicId::WallWeaver
            | RelicId::Heirloom
            | RelicId::Tourist
            | RelicId::CrackedTile
            | RelicId::HungryGhost
            | RelicId::WayOfPurity
            | RelicId::WayOfPairs
            | RelicId::WayOfTriplets
            | RelicId::WayOfSequences
            | RelicId::SilverFiligreeLantern
            | RelicId::GlassCannon
            | RelicId::EulersNumber
            | RelicId::PiConstant
            | RelicId::MonarchButterfly
    )
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
    /// Plays already consumed this round at the moment of scoring (so the
    /// current play is `plays_used + 1`-th). Used by relics such as Momentum.
    pub plays_used: u32,
    /// Per-yaku level (Zodiac-card-driven). `None` falls back to all level 1
    /// — used by tests and the bot.
    pub yaku_levels: Option<crate::core::zodiac::YakuLevels>,
    /// Yaku detected on prior plays in the current round, used by The Censor
    /// boss to halve repeat-yaku contributions. Empty in normal rounds.
    pub played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    /// Player's current gold at the moment of scoring (for Golden Engine).
    pub gold: i32,
    /// Cumulative score earned across the entire run (for Snowball).
    pub total_score: u64,
    /// True when this is the player's last remaining play this round
    /// (plays_remaining == 0 at scoring time). Powers Last Breath when
    /// combined with a structure cash-in.
    pub is_final_play: bool,
    /// Per-relic mutable counters (humility, melting_ice chips,
    /// tile_polisher accumulated bonus, river_runner accumulated bonus, etc.).
    /// Keyed by RelicId, value meaning varies by relic.
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    /// Player hand snapshot at scoring time (before `take_selected_tiles` in
    /// classic mode; at structure cash-in this is tiles still in hand, disjoint
    /// from structure). Used by Ghost Hand to sum point values of unscored tiles.
    pub hand_for_ghost: &'a [crate::core::tile::Tile],
    /// When set, this score is from **structure cash-in** (`trigger_structure`), not from a
    /// direct hand play. Relic hooks attach either when melds **land**
    /// (`commit_selection_to_structure`) or when the player **cashes in** — e.g. MeltingIce /
    /// Tea / Humility on commit; TilePolisher / River Runner / Star Tile / KanDrum / scoring
    /// cascades on trigger. New "per play" relics should hook the correct side.
    pub structure: Option<crate::core::structure::StructureTriggerMeta>,
}

// Scoring applies relic effects in `core::scoring::score_sets` via `ScoreContext` — one match
// arm per relic.

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

    /// `assets/data/relics.json` must have exactly one entry per `RelicId`
    /// variant. The match in `ALL_VARIANTS` below is exhaustive, so the
    /// compiler forces an update when a new variant is added; the data
    /// file is then checked against that list.
    #[test]
    fn every_relic_variant_has_one_data_entry() {
        use super::all_relic_defs;
        use std::collections::BTreeSet;

        // Exhaustive match: adding a new RelicId variant without adding
        // it here is a compile error. The safety chain is:
        // enum -> exhaustive match -> ALL_VARIANTS -> data file.
        const ALL_VARIANTS: &[RelicId] = {
            // Touch every variant so the match below stays exhaustive.
            const fn _exhaustive(id: RelicId) {
                match id {
                    RelicId::TripletBoost
                    | RelicId::SequenceSurge
                    | RelicId::PairPower
                    | RelicId::HonorFury
                    | RelicId::RedDragonRage
                    | RelicId::GreenLuck
                    | RelicId::WhiteDragonsHush
                    | RelicId::JokerTile
                    | RelicId::StrengthInNumbers
                    | RelicId::QuickDraw
                    | RelicId::ChainReaction
                    | RelicId::MultiplierMaster
                    | RelicId::SetMagnet
                    | RelicId::WildWinds
                    | RelicId::DragonEcho
                    | RelicId::KanDrum
                    | RelicId::DoraCrown
                    | RelicId::RoundCompass
                    | RelicId::EightTreasures
                    | RelicId::KongsBlessing
                    | RelicId::GardenKeeper
                    | RelicId::Ikebana
                    | RelicId::Hanami
                    | RelicId::JadeSerpent
                    | RelicId::RedSerpent
                    | RelicId::BlueSerpent
                    | RelicId::LowTide
                    | RelicId::HighTide
                    | RelicId::MerchantsEye
                    | RelicId::IGotAGuy
                    | RelicId::EdgeRunner
                    | RelicId::LuckySeven
                    | RelicId::Momentum
                    | RelicId::Minimalist
                    | RelicId::TurtleShell
                    | RelicId::ClosedGate
                    | RelicId::GoldenEngine
                    | RelicId::Snowball
                    | RelicId::SecondWind
                    | RelicId::GlassCannon
                    | RelicId::LastBreath
                    | RelicId::TilePolisher
                    | RelicId::PaperLantern
                    | RelicId::SilverFiligreeLantern
                    | RelicId::MirrorTile
                    | RelicId::WayOfPurity
                    | RelicId::Geese
                    | RelicId::VoiceOfThePeople
                    | RelicId::VoiceOfTheElite
                    | RelicId::RustlingGooseEgg
                    | RelicId::TeaCeremony
                    | RelicId::Rakuware
                    | RelicId::GhostHand
                    | RelicId::Humility
                    | RelicId::Obsession
                    | RelicId::Bonfire
                    | RelicId::RiverRunner
                    | RelicId::MeltingIce
                    | RelicId::Taotie
                    | RelicId::SilkThread
                    | RelicId::SilkMoth
                    | RelicId::ShadowHand
                    | RelicId::SolitarySage
                    | RelicId::GoldIdol
                    | RelicId::JadeAbacus
                    | RelicId::NestEgg
                    | RelicId::Patience
                    | RelicId::WayOfPairs
                    | RelicId::WayOfTriplets
                    | RelicId::WayOfSequences
                    | RelicId::FortunesFavor
                    | RelicId::CrackedTile
                    | RelicId::StarTile
                    | RelicId::SmokeBomb
                    | RelicId::PhantomRelic
                    | RelicId::HungryGhost
                    | RelicId::Disgust
                    | RelicId::CurioCabinet
                    | RelicId::LotusBloom
                    | RelicId::WallWeaver
                    | RelicId::KongCollector
                    | RelicId::NoHonorButWealth
                    | RelicId::Sweepstakes
                    | RelicId::BeggarsCup
                    | RelicId::Cosmopolitan
                    | RelicId::Heirloom
                    | RelicId::Tourist
                    | RelicId::Kintsugi
                    | RelicId::AntTrail
                    | RelicId::BrocadePouch
                    | RelicId::EulersNumber
                    | RelicId::PiConstant
                    | RelicId::BigHands
                    | RelicId::TinyHands
                    | RelicId::Chrysalis
                    | RelicId::MonarchButterfly => {}
                }
            }
            &[
                RelicId::TripletBoost,
                RelicId::SequenceSurge,
                RelicId::PairPower,
                RelicId::HonorFury,
                RelicId::RedDragonRage,
                RelicId::GreenLuck,
                RelicId::WhiteDragonsHush,
                RelicId::JokerTile,
                RelicId::StrengthInNumbers,
                RelicId::QuickDraw,
                RelicId::ChainReaction,
                RelicId::MultiplierMaster,
                RelicId::SetMagnet,
                RelicId::WildWinds,
                RelicId::DragonEcho,
                RelicId::KanDrum,
                RelicId::DoraCrown,
                RelicId::RoundCompass,
                RelicId::EightTreasures,
                RelicId::KongsBlessing,
                RelicId::GardenKeeper,
                RelicId::Ikebana,
                RelicId::Hanami,
                RelicId::JadeSerpent,
                RelicId::RedSerpent,
                RelicId::BlueSerpent,
                RelicId::LowTide,
                RelicId::HighTide,
                RelicId::MerchantsEye,
                RelicId::IGotAGuy,
                RelicId::EdgeRunner,
                RelicId::LuckySeven,
                RelicId::Momentum,
                RelicId::Minimalist,
                RelicId::TurtleShell,
                RelicId::ClosedGate,
                RelicId::GoldenEngine,
                RelicId::Snowball,
                RelicId::SecondWind,
                RelicId::GlassCannon,
                RelicId::LastBreath,
                RelicId::TilePolisher,
                RelicId::PaperLantern,
                RelicId::SilverFiligreeLantern,
                RelicId::MirrorTile,
                RelicId::WayOfPurity,
                RelicId::Geese,
                RelicId::VoiceOfThePeople,
                RelicId::VoiceOfTheElite,
                RelicId::RustlingGooseEgg,
                RelicId::TeaCeremony,
                RelicId::Rakuware,
                RelicId::GhostHand,
                RelicId::Humility,
                RelicId::Obsession,
                RelicId::Bonfire,
                RelicId::RiverRunner,
                RelicId::MeltingIce,
                RelicId::Taotie,
                RelicId::SilkThread,
                RelicId::SilkMoth,
                RelicId::ShadowHand,
                RelicId::SolitarySage,
                RelicId::GoldIdol,
                RelicId::JadeAbacus,
                RelicId::NestEgg,
                RelicId::Patience,
                RelicId::WayOfPairs,
                RelicId::WayOfTriplets,
                RelicId::WayOfSequences,
                RelicId::FortunesFavor,
                RelicId::CrackedTile,
                RelicId::StarTile,
                RelicId::SmokeBomb,
                RelicId::PhantomRelic,
                RelicId::HungryGhost,
                RelicId::Disgust,
                RelicId::CurioCabinet,
                RelicId::LotusBloom,
                RelicId::WallWeaver,
                RelicId::KongCollector,
                RelicId::NoHonorButWealth,
                RelicId::Sweepstakes,
                RelicId::BeggarsCup,
                RelicId::Cosmopolitan,
                RelicId::Heirloom,
                RelicId::Tourist,
                RelicId::Kintsugi,
                RelicId::AntTrail,
                RelicId::BrocadePouch,
                RelicId::EulersNumber,
                RelicId::PiConstant,
                RelicId::BigHands,
                RelicId::TinyHands,
                RelicId::Chrysalis,
                RelicId::MonarchButterfly,
            ]
        };

        let in_data: BTreeSet<RelicId> = all_relic_defs().iter().map(|d| d.id).collect();
        assert_eq!(
            in_data.len(),
            all_relic_defs().len(),
            "duplicate ids in assets/data/relics.json"
        );

        for &id in ALL_VARIANTS {
            assert!(
                in_data.contains(&id),
                "{id:?} is missing from assets/data/relics.json"
            );
        }

        // Catch entries in the data file that aren't in ALL_VARIANTS
        // (e.g. typo'd ids that snuck past serde because they happen
        // to deserialize).
        let all_set: BTreeSet<RelicId> = ALL_VARIANTS.iter().copied().collect();
        for id in &in_data {
            assert!(
                all_set.contains(id),
                "{id:?} is in relics.json but missing from the test's ALL_VARIANTS list"
            );
        }
    }
}
