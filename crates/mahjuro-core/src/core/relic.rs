//! Relic definitions and runtime application hooks.
//!
//! Display metadata (name, description, rarity) lives in
//! `assets/data/relics.json` so balance edits don't require recompiling
//! the core crate. At runtime the file is loaded from shipped asset packs
//! or from the repo `assets/` tree in development.
//! Behaviour (scoring hooks, prices, visuals) stays in Rust.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::tile::Suit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelicId {
    AncestorEcho,
    AntTrail,
    BeggarsCup,
    BlueTilesWhiteDragon,
    BigHands,
    Bonfire,
    BrocadePouch,
    ChainReaction,
    Charity,
    Chastity,
    ChowLine,
    Chrysalis,
    ClosedGate,
    Cosmopolitan,
    CrackedTile,
    CrownOfPatterns,
    CurioCabinet,
    Diligence,
    Disgust,
    DoraCrown,
    DragonEcho,
    DragonRage,
    EdgeRunner,
    EightTreasures,
    EulersNumber,
    EvenKeel,
    FortunesFavor,
    GardenKeeper,
    Geese,
    GhostHand,
    GlassCannon,
    GoldenEngine,
    GoldIdol,
    GreenLuck,
    GreenTilesGreenDragon,
    Hanami,
    Heirloom,
    HighTide,
    HonorFury,
    Humility,
    HungryGhost,
    IGotAGuy,
    Ikebana,
    JadeAbacus,
    JadeSerpent,
    JokerTile,
    KanDrum,
    Kindling,
    Kindness,
    KingKong,
    Kintsugi,
    KongCollector,
    KongsBlessing,
    LastBreath,
    LapisSerpent,
    LotusBloom,
    LowTide,
    LuckySeven,
    MeltingIce,
    MerchantsEye,
    Minimalist,
    MirrorTile,
    Momentum,
    MonarchButterfly,
    MultiplierMaster,
    NestEgg,
    NoHonorButWealth,
    Obsession,
    OpenGate,
    PairPower,
    PaperLantern,
    Patience,
    PiConstant,
    PlainDealing,
    QuickDraw,
    Rakuware,
    RiverRunner,
    #[serde(rename = "xxxl_egg")]
    XxxlEgg,
    RedTilesRedDragon,
    RubySerpent,
    SecondWind,
    SequenceSurge,
    SetMagnet,
    ShadowHand,
    SilkMoth,
    SilkThread,
    Snowball,
    SolitarySage,
    StarTile,
    StoneLantern,
    StrengthInNumbers,
    Sweepstakes,
    Taotie,
    TeaCeremony,
    Temperance,
    TilePolisher,
    TinyHands,
    Tourist,
    TripletBoost,
    TurtleShell,
    VoiceOfTheElite,
    VoiceOfThePeople,
    WallWeaver,
    WayOfPairs,
    WayOfPurity,
    WayOfSequences,
    WayOfTriplets,
    WhiteDragonsHush,
    WildWinds,
    WindReader,
}

/// Total absorbed excess (post-target score) needed for Chrysalis to transform.
pub const CHRYSALIS_HATCH_EXCESS_THRESHOLD: i32 = 10_000;

/// Chips per log-tier from [`monarch_butterfly_tier`].
pub const MONARCH_CHIPS_PER_TIER: i32 = 27;

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
            RelicId::DragonRage => "dragon_rage.png",
            RelicId::GreenLuck => "green_luck.png",
            RelicId::GreenTilesGreenDragon => "green_tiles_green_dragon.png",
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
            RelicId::WindReader => "wind_reader.png",
            RelicId::EightTreasures => "eight_treasures.png",
            RelicId::KongsBlessing => "kongs_blessing.png",
            RelicId::GardenKeeper => "garden_keeper.png",
            RelicId::Ikebana => "ikebana.png",
            RelicId::Hanami => "hanami.png",
            RelicId::JadeSerpent => "jade_serpent.png",
            RelicId::RubySerpent => "ruby_serpent.png",
            RelicId::LapisSerpent => "lapis_serpent.png",
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
            RelicId::StoneLantern => "stone_lantern.png",
            RelicId::MirrorTile => "mirror_tile.png",
            RelicId::WayOfPurity => "way_of_purity.png",
            RelicId::Geese => "geese.png",
            RelicId::VoiceOfThePeople => "voice_of_the_people.png",
            RelicId::VoiceOfTheElite => "voice_of_the_elite.png",
            RelicId::XxxlEgg => "xxxl_egg.png",
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
            RelicId::Kindling => "kindling.png",
            RelicId::Kindness => "kindness.png",
            RelicId::Temperance => "temperance.png",
            RelicId::Chastity => "chastity.png",
            RelicId::ChowLine => "chow_line.png",
            RelicId::Charity => "charity.png",
            RelicId::Diligence => "diligence.png",
            RelicId::WayOfPairs => "way_of_pairs.png",
            RelicId::WayOfTriplets => "way_of_triplets.png",
            RelicId::WayOfSequences => "way_of_sequences.png",
            RelicId::FortunesFavor => "fortunes_favor.png",
            RelicId::CrackedTile => "cracked_tile.png",
            RelicId::StarTile => "star_tile.png",
            RelicId::HungryGhost => "hungry_ghost.png",
            RelicId::Disgust => "disgust.png",
            RelicId::CurioCabinet => "curio_cabinet.png",
            RelicId::LotusBloom => "lotus_bloom.png",
            RelicId::WallWeaver => "wall_weaver.png",
            RelicId::KongCollector => "kong_collector.png",
            RelicId::KingKong => "king_kong.png",
            RelicId::NoHonorButWealth => "no_honor_but_wealth.png",
            RelicId::Sweepstakes => "sweepstakes.png",
            RelicId::BeggarsCup => "beggars_cup.png",
            RelicId::BlueTilesWhiteDragon => "blue_tiles_white_dragon.png",
            RelicId::RedTilesRedDragon => "red_tiles_red_dragon.png",
            RelicId::Cosmopolitan => "cosmopolitan.png",
            RelicId::Heirloom => "heirloom.png",
            RelicId::Tourist => "tourist.png",
            RelicId::Kintsugi => "kintsugi.png",
            RelicId::AntTrail => "ant_trail.png",
            RelicId::BrocadePouch => "brocade_pouch.png",
            RelicId::EulersNumber => "eulers_number.png",
            RelicId::EvenKeel => "even_keel.png",
            RelicId::OpenGate => "open_gate.png",
            RelicId::PiConstant => "pi_constant.png",
            RelicId::PlainDealing => "plain_dealing.png",
            RelicId::BigHands => "big_hands.png",
            RelicId::TinyHands => "tiny_hands.png",
            RelicId::Chrysalis => "chrysalis.png",
            RelicId::MonarchButterfly => "monarch_butterfly.png",
            RelicId::AncestorEcho => "ancestor_echo.png",
            RelicId::CrownOfPatterns => "crown_of_patterns.png",
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

impl Rarity {
    /// 0..=3 tier index, suitable for `theme::color::rarity(tier)` and any
    /// other rarity-keyed table that wants a numeric ladder. Centralized so
    /// the relic / yaku / blind UIs all walk the same rungs in the same
    /// order (iron → bronze → silver → gold).
    pub fn tier(self) -> u8 {
        match self {
            Rarity::Common => 0,
            Rarity::Uncommon => 1,
            Rarity::Rare => 2,
            Rarity::Legendary => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelicRenderMaterial {
    Iron,
    Copper,
    Silver,
    Gold,
}

/// One styled run for relic inspect flavor text (floating overlay only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RelicFlavorSpan {
    pub text: &'static str,
    pub bold: bool,
    pub italic: bool,
}

/// Plain copy for font sizing / layout (`\n` preserved).
pub fn flavor_spans_plain_text(spans: &[RelicFlavorSpan]) -> String {
    spans.iter().map(|s| s.text).collect()
}

/// Cache key for rasterized flavor labels (must change when content or styles change).
pub fn flavor_spans_cache_key(spans: &[RelicFlavorSpan]) -> String {
    let mut s = String::new();
    for sp in spans {
        s.push_str(sp.text);
        s.push('\x1f');
        s.push(if sp.bold { 'B' } else { 'b' });
        s.push(if sp.italic { 'I' } else { 'i' });
        s.push('\x1f');
    }
    s
}

#[derive(Clone, Debug)]
pub struct RelicDef {
    pub id: RelicId,
    pub name: &'static str,
    pub description: &'static str,
    pub rarity: Rarity,
    /// Optional flavor lines for inspect UI only; empty when absent in data.
    pub flavor: &'static [RelicFlavorSpan],
}

#[derive(Deserialize)]
struct RelicFlavorSpanRaw {
    text: String,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
}

#[derive(Deserialize)]
struct RelicDefRaw {
    id: RelicId,
    name: String,
    description: String,
    rarity: Rarity,
    #[serde(default)]
    flavor_spans: Vec<RelicFlavorSpanRaw>,
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
        format!("textures/relics/{}_object.png", stem)
    }

    /// Optional binary or transparent silhouette image used to derive the
    /// runtime relic mesh more deterministically than the shaded object render.
    pub fn source_mask_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/{}_mask.png", stem)
    }

    /// Optional offline-generated grayscale relief source for future embossed
    /// or carved detailing on the 3D relic mesh.
    pub fn source_heightmap_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/{}_height.png", stem)
    }

    /// Optional offline-generated grayscale specular mask for soft-enamel pins.
    /// Loaded into the relief texture G channel at runtime; derived from height
    /// when this file is missing.
    pub fn source_specular_path(self) -> String {
        let stem = self.asset_filename().trim_end_matches(".png");
        format!("textures/relics/{}_specular.png", stem)
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

    let material = match id {
        // Rare tier for shop math, but the art direction calls for a gold
        // cloisonné frame (jade + gold speckle), not the default Rare silver.
        RelicId::Chrysalis => Gold,
        _ => match rarity {
            Rarity::Common => Iron,
            Rarity::Uncommon => Copper,
            Rarity::Rare => Silver,
            Rarity::Legendary => Gold,
        },
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

/// Catalog shop price with Merchant's Eye's 25% discount (before season scaling).
/// Floors to at least 1 gold.
pub fn apply_merchants_eye_discount(base: u32, relics: &RelicState) -> u32 {
    if relics.has(RelicId::MerchantsEye) {
        (base * 3 / 4).max(1)
    } else {
        base
    }
}

/// Effective gold cost to buy a relic in the shop after active price
/// modifiers are applied.
pub fn relic_shop_price(id: RelicId, relics: &RelicState) -> u32 {
    apply_merchants_eye_discount(relic_buy_price(id), relics)
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
///
/// When `inventory_focus` is `Some((relics, slot_index))`, Mirror Tile and
/// Shadow Hand append inventory help after the expanded template.
///
/// Live counter tokens in `assets/data/relics.json` are only filled from run
/// state when `inventory_focus` is `Some` (gameplay / shop). Archive catalog
/// passes `None` and gets design-time defaults so entries don't leak run state.
pub fn relic_description_live(
    id: RelicId,
    counters: &std::collections::BTreeMap<RelicId, i32>,
    gold: i32,
    inventory_focus: Option<(&RelicState, usize)>,
    ghost_hand_chips_preview: Option<i32>,
    wing: Option<u32>,
) -> String {
    use crate::core::relic_desc_template::{RelicDescContext, expand_relic_description_templates};

    let base = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.description)
        .unwrap_or("");
    let (relics, slot) = match inventory_focus {
        Some((relics, slot)) => (Some(relics), Some(slot)),
        None => (None, None),
    };
    let ctx = RelicDescContext {
        id,
        counters,
        gold,
        relics,
        slot,
        ghost_hand_chips_preview,
        wing,
        live: inventory_focus.is_some(),
    };
    let mut s = expand_relic_description_templates(base, &ctx);
    if inventory_focus.is_some() {
        if id == RelicId::MirrorTile {
            if let Some((relics, slot)) = inventory_focus {
                let extra = format_mirror_tile_inventory_help(relics, slot);
                if !extra.is_empty() {
                    s.push_str("\n\n");
                    s.push_str(&extra);
                }
            }
        } else if id == RelicId::ShadowHand {
            if let Some((relics, slot)) = inventory_focus {
                let extra = format_shadow_hand_inventory_help(relics, slot);
                if !extra.is_empty() {
                    s.push_str("\n\n");
                    s.push_str(&extra);
                }
            }
        }
    }
    s
}

pub fn all_relic_defs() -> &'static [RelicDef] {
    static DEFS: OnceLock<Vec<RelicDef>> = OnceLock::new();
    DEFS.get_or_init(load_relic_defs).as_slice()
}

fn load_relic_defs() -> Vec<RelicDef> {
    const PATH: &str = "data/relics.json";
    let raw: Vec<RelicDefRaw> = load_json_asset(PATH, "relic data");
    raw.into_iter()
        .map(|r| {
            let flavor: &'static [RelicFlavorSpan] = if r.flavor_spans.is_empty() {
                &[]
            } else {
                let v: Vec<RelicFlavorSpan> = r
                    .flavor_spans
                    .into_iter()
                    .filter(|s| !s.text.is_empty())
                    .map(|s| RelicFlavorSpan {
                        text: Box::leak(s.text.into_boxed_str()),
                        bold: s.bold,
                        italic: s.italic,
                    })
                    .collect();
                if v.is_empty() || v.iter().all(|s| s.text.chars().all(char::is_whitespace)) {
                    &[]
                } else {
                    Box::leak(v.into_boxed_slice())
                }
            };
            RelicDef {
                id: r.id,
                name: Box::leak(r.name.into_boxed_str()),
                description: Box::leak(r.description.into_boxed_str()),
                rarity: r.rarity,
                flavor,
            }
        })
        .collect()
}

/// Default relic inventory capacity (shop dish + gameplay anchors).
pub const BASE_RELIC_SLOTS: usize = 5;

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
            max_slots: BASE_RELIC_SLOTS,
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

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
    "Hand scoring counts this like a second copy of that relic.";

#[inline]
fn push_debuffed_self_copy_relic_line(
    parts: &mut Vec<String>,
    relics: &RelicState,
    self_id: RelicId,
) {
    if relics.is_debuffed(self_id) {
        let name = relic_display_name(self_id);
        parts.push(format!("{name} is debuffed (off)."));
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
            " That relic is debuffed; {copy_source_display_name} copy still applies for scoring."
        ));
    }
    parts.push(line);
    if !relic_scoring_copy_dup_is_incompatible(target) {
        parts.push(COPY_RELIC_COMPATIBLE_SCORING_LINE.to_string());
    }
}

/// Tooltip helper: which relic Mirror Tile copies and when another mirror wins.
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
                format!("Copies {name}."),
                &mirror_name,
            );
        }
        None => parts.push("Put a relic in the slot to the right.".to_string()),
    }

    if let Some(fm) = first_mirror_slot
        && fm != mirror_slot
    {
        let tgt = active.get(fm + 1).copied().map(relic_display_name);
        let tgt_s = tgt.unwrap_or_else(|| "nothing".into());
        parts.push(format!(
            "Only the leftmost mirror does anything (it copies {tgt_s})."
        ));
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
        None => parts.push("Nothing to copy.".to_string()),
        Some(RelicId::ShadowHand) => {
            parts.push(
                "Move Shadow Hand right so a different relic is in the first slot.".to_string(),
            );
        }
        Some(tid) => {
            let name = relic_display_name(tid);
            push_copy_target_line_with_debuff_note(
                &mut parts,
                relics,
                tid,
                format!("Copies {name} (left slot)."),
                &shadow_name,
            );
        }
    }

    parts.join("\n")
}

/// True when Mirror Tile / Shadow Hand should **not** show the “compatible” scoring
/// tip (`COPY_RELIC_COMPATIBLE_SCORING_LINE`): meta copies, shop-only relics,
/// draw/validation/wall hooks that do not go through [`EffectiveRelics`](crate::core::scoring::EffectiveRelics),
/// **blind-clear round payout** gold only (Gold Idol, Jade Abacus, Patience, Green Luck,
/// Beggar’s Cup, Cosmopolitan — see `RunState::resolve_round_end`), sell-value
/// growth (`NestEgg`), round-start rolls (`Sweepstakes`), or post-score bookkeeping unrelated
/// to in-round play (`StarTile`, `Chrysalis`).
///
/// In-round economy wired to mirror/shadow includes Kong Collector (kong counter),
/// No Honor But Wealth (discard), Eight Treasures (full-hand zodiac), and Quick Draw
/// (post-commit draw size); those stay **off** this list.
///
/// New [`RelicId`] variants default to compatible; extend this list when the tooltip would
/// mislead or integration is still missing.
fn relic_scoring_copy_dup_is_incompatible(target: RelicId) -> bool {
    matches!(
        target,
        RelicId::MirrorTile
            | RelicId::ShadowHand
            | RelicId::MerchantsEye
            | RelicId::IGotAGuy
            | RelicId::SecondWind
            | RelicId::BigHands
            | RelicId::TinyHands
            | RelicId::BrocadePouch
            | RelicId::GoldIdol
            | RelicId::JadeAbacus
            | RelicId::NestEgg
            | RelicId::Patience
            | RelicId::Kindling
            | RelicId::Kindness
            | RelicId::KingKong
            | RelicId::Diligence
            | RelicId::Temperance
            | RelicId::Charity
            | RelicId::Sweepstakes
            | RelicId::BeggarsCup
            | RelicId::Cosmopolitan
            | RelicId::GreenLuck
            | RelicId::FortunesFavor
            | RelicId::SetMagnet
            | RelicId::JokerTile
            | RelicId::WildWinds
            | RelicId::StrengthInNumbers
            | RelicId::Disgust
            | RelicId::AntTrail
            | RelicId::StarTile
            | RelicId::Chrysalis
    )
}

/// Relic roster and counter map for scoring hooks.
pub struct ScoreRelicBundle<'a> {
    pub roster: &'a RelicState,
    pub counters: std::collections::BTreeMap<RelicId, i32>,
}

/// Tile debuffs (boss/class) and unscored-hand snapshot for Ghost Hand.
pub struct ScoreTileBundle<'a> {
    pub debuffs: &'a [crate::core::debuff::TileDebuff],
    pub hand_for_ghost: &'a [crate::core::tile::Tile],
}

/// Per-round timing, wind, and repeat-yaku state (e.g. Censor, Momentum, Chain Reaction).
pub struct ScoreRoundBundle {
    /// Whether the player scored on their previous play (for ChainReaction).
    pub scored_last_turn: bool,
    /// Plays already consumed this round at scoring (current play is `plays_used + 1`-th).
    pub plays_used: u32,
    /// Round wind (1=East … 4=North) for Yakuhai; `None` outside a run.
    pub round_wind: Option<u8>,
    /// Second round wind from Windreader (1–4); `None` when inactive.
    pub bonus_round_wind: Option<u8>,
    /// Yaku already scored this round (The Censor halves repeats).
    pub played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    /// Last play of the round (`plays_remaining == 0`); powers Last Breath with structure.
    pub is_final_play: bool,
}

/// Dora indicators, yaku availability (Kokushi secret), and zodiac yaku levels.
pub struct ScorePatternBundle {
    pub dora_faces: Vec<(Suit, u8)>,
    pub available_yaku: Vec<crate::core::yaku::YakuKind>,
    pub yaku_levels: Option<crate::core::zodiac::YakuLevels>,
}

/// Run economy totals read during scoring (e.g. Golden Engine).
pub struct ScoreEconomyBundle {
    pub yen: i32,
    pub total_score: u64,
}

/// Cash-ins counted toward Kindling across the run (stack cap).
pub const KINDLING_STACK_CAP: i32 = 30;
/// Mult added per prior cash-in (run-total) on Kindling.
pub const KINDLING_MULT_PER_CASHIN: f64 = 0.4;
/// Maximum mult Kindling can add on a cash-in.
pub const KINDLING_MULT_CAP: f64 = 10.0;

/// Cleared blinds counted toward Snowball (cap).
pub const SNOWBALL_STACK_CAP: i32 = 15;
/// Chips added per scored hand for each blind clear counted on Snowball (before mult).
pub const SNOWBALL_CHIPS_PER_CLEAR: i32 = 25;

/// Flat chips from Turtle Shell while the run still holds yen (`yen > 0` at score time).
/// The relic is removed when run yen hits zero or below (handled in the run yen-change hook).
pub const TURTLE_SHELL_CHIPS: i32 = 300;

/// Starting chip bonus for Melting Ice (also used when the counter is first set).
pub const MELTING_ICE_START_CHIPS: i32 = 120;

/// Chip bonus lost from Melting Ice after each play.
pub const MELTING_ICE_DECAY_PER_PLAY: i32 = 12;

/// Flat chip bonus from Taotie on every cash-in.
pub const TAOTIE_BASE_CHIPS: i32 = 120;

/// Permanent chip growth per honor devoured by Taotie.
pub const TAOTIE_CHIPS_PER_DEVOURED: i32 = 30;

/// Permanent chip growth per sequence scored while River Runner is owned.
pub const RIVER_RUNNER_CHIPS_PER_SEQUENCE: i32 = 30;

/// Permanent chip growth per tile in a scored cash-in while Tile Polisher is owned.
pub const TILE_POLISHER_CHIPS_PER_TILE: i32 = 5;

/// Mult bonus from Golden Engine (+1 per 3 gold held at score time).
#[inline]
pub fn golden_engine_mult_bonus(gold: i32) -> i32 {
    ((gold.max(0) as f64 / 3.0).floor() as i32).min(12)
}

/// Mult from Kindling for one scored hand (`total_cash_ins` = run-total cash-ins while owned).
#[inline]
pub fn kindling_mult_bonus(total_cash_ins: i32) -> f64 {
    if total_cash_ins <= 0 {
        return 0.0;
    }
    let s = total_cash_ins.clamp(0, KINDLING_STACK_CAP) as f64;
    (s * KINDLING_MULT_PER_CASHIN).min(KINDLING_MULT_CAP)
}

/// Chips from Snowball for one scored hand (`stacks` = blind clears while owned, capped).
#[inline]
pub fn snowball_score_chips(stacks: i32) -> i32 {
    if stacks <= 0 {
        return 0;
    }
    let s = stacks.clamp(0, SNOWBALL_STACK_CAP);
    s * SNOWBALL_CHIPS_PER_CLEAR
}

/// Scoring context for relic hooks, grouped for readability at call sites.
pub struct ScoreContext<'a> {
    pub relic: ScoreRelicBundle<'a>,
    pub tiles: ScoreTileBundle<'a>,
    pub round: ScoreRoundBundle,
    pub pattern: ScorePatternBundle,
    pub economy: ScoreEconomyBundle,
    /// Structure cash-in metadata. Set when scoring from `trigger_structure`, not a direct play.
    pub structure: Option<crate::core::structure::StructureTriggerMeta>,
}

// Scoring applies relic effects in `core::scoring::score_sets` via `ScoreContext` — one match
// arm per relic.

#[cfg(test)]
mod tests {
    use super::{
        KINDLING_MULT_CAP, KINDLING_STACK_CAP, RelicFlavorSpan, RelicId, RelicState,
        SNOWBALL_CHIPS_PER_CLEAR, SNOWBALL_STACK_CAP, all_relic_defs, apply_merchants_eye_discount,
        golden_engine_mult_bonus, kindling_mult_bonus, relic_buy_price, relic_shop_price,
        snowball_score_chips,
    };

    #[test]
    fn golden_engine_mult_scales_per_three_gold() {
        assert_eq!(golden_engine_mult_bonus(0), 0);
        assert_eq!(golden_engine_mult_bonus(2), 0);
        assert_eq!(golden_engine_mult_bonus(3), 1);
        assert_eq!(golden_engine_mult_bonus(5), 1);
        assert_eq!(golden_engine_mult_bonus(24), 8);
        assert_eq!(golden_engine_mult_bonus(-3), 0);
    }

    #[test]
    fn snowball_chips_flat_per_clear_and_caps_stacks() {
        assert_eq!(snowball_score_chips(3), 3 * SNOWBALL_CHIPS_PER_CLEAR);
        assert_eq!(
            snowball_score_chips(SNOWBALL_STACK_CAP + 50),
            SNOWBALL_STACK_CAP * SNOWBALL_CHIPS_PER_CLEAR
        );
    }

    #[test]
    fn kindling_mult_scales_per_cashin_and_caps() {
        assert_eq!(kindling_mult_bonus(0), 0.0);
        assert_eq!(kindling_mult_bonus(2), 0.8);
        assert_eq!(kindling_mult_bonus(KINDLING_STACK_CAP), KINDLING_MULT_CAP);
        assert_eq!(
            kindling_mult_bonus(KINDLING_STACK_CAP + 10),
            KINDLING_MULT_CAP
        );
    }

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

    #[test]
    fn flavor_load_preserves_embedded_and_separator_newlines() {
        let cosmopolitan = all_relic_defs()
            .iter()
            .find(|d| d.id == RelicId::Cosmopolitan)
            .expect("cosmopolitan def");
        assert!(
            cosmopolitan.flavor.iter().any(|s| s.text.contains('\n')),
            "embedded newlines should survive relic flavor load"
        );

        let separator: Vec<RelicFlavorSpan> = vec![
            RelicFlavorSpan {
                text: "First",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "\n",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "Second",
                bold: false,
                italic: false,
            },
        ];
        // Mirror load filter: keep `\n`-only separator spans between copy runs.
        let kept: Vec<_> = separator
            .into_iter()
            .filter(|s| !s.text.is_empty())
            .collect();
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[1].text, "\n");
    }

    #[test]
    fn merchants_eye_discount_applies_to_non_relic_catalog_prices() {
        let mut relics = RelicState::default();
        relics.active.push(RelicId::MerchantsEye);
        assert_eq!(apply_merchants_eye_discount(8, &relics), 6);
        let no_eye = RelicState::default();
        assert_eq!(apply_merchants_eye_discount(8, &no_eye), 8);
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
                    | RelicId::DragonRage
                    | RelicId::GreenLuck
                    | RelicId::GreenTilesGreenDragon
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
                    | RelicId::WindReader
                    | RelicId::EightTreasures
                    | RelicId::KongsBlessing
                    | RelicId::GardenKeeper
                    | RelicId::Ikebana
                    | RelicId::Hanami
                    | RelicId::JadeSerpent
                    | RelicId::RubySerpent
                    | RelicId::LapisSerpent
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
                    | RelicId::StoneLantern
                    | RelicId::MirrorTile
                    | RelicId::WayOfPurity
                    | RelicId::Geese
                    | RelicId::VoiceOfThePeople
                    | RelicId::VoiceOfTheElite
                    | RelicId::XxxlEgg
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
                    | RelicId::Kindling
                    | RelicId::Kindness
                    | RelicId::KingKong
                    | RelicId::Temperance
                    | RelicId::Chastity
                    | RelicId::ChowLine
                    | RelicId::Charity
                    | RelicId::Diligence
                    | RelicId::EvenKeel
                    | RelicId::OpenGate
                    | RelicId::PlainDealing
                    | RelicId::WayOfPairs
                    | RelicId::WayOfTriplets
                    | RelicId::WayOfSequences
                    | RelicId::FortunesFavor
                    | RelicId::CrackedTile
                    | RelicId::StarTile
                    | RelicId::HungryGhost
                    | RelicId::Disgust
                    | RelicId::CurioCabinet
                    | RelicId::LotusBloom
                    | RelicId::WallWeaver
                    | RelicId::KongCollector
                    | RelicId::NoHonorButWealth
                    | RelicId::Sweepstakes
                    | RelicId::BeggarsCup
                    | RelicId::BlueTilesWhiteDragon
                    | RelicId::RedTilesRedDragon
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
                    | RelicId::MonarchButterfly
                    | RelicId::AncestorEcho
                    | RelicId::CrownOfPatterns => {}
                }
            }
            &[
                RelicId::TripletBoost,
                RelicId::SequenceSurge,
                RelicId::PairPower,
                RelicId::HonorFury,
                RelicId::DragonRage,
                RelicId::GreenLuck,
                RelicId::GreenTilesGreenDragon,
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
                RelicId::WindReader,
                RelicId::EightTreasures,
                RelicId::KongsBlessing,
                RelicId::GardenKeeper,
                RelicId::Ikebana,
                RelicId::Hanami,
                RelicId::JadeSerpent,
                RelicId::RubySerpent,
                RelicId::LapisSerpent,
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
                RelicId::StoneLantern,
                RelicId::MirrorTile,
                RelicId::WayOfPurity,
                RelicId::Geese,
                RelicId::VoiceOfThePeople,
                RelicId::VoiceOfTheElite,
                RelicId::XxxlEgg,
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
                RelicId::Kindling,
                RelicId::Kindness,
                RelicId::KingKong,
                RelicId::Temperance,
                RelicId::Chastity,
                RelicId::ChowLine,
                RelicId::Charity,
                RelicId::Diligence,
                RelicId::EvenKeel,
                RelicId::OpenGate,
                RelicId::PlainDealing,
                RelicId::WayOfPairs,
                RelicId::WayOfTriplets,
                RelicId::WayOfSequences,
                RelicId::FortunesFavor,
                RelicId::CrackedTile,
                RelicId::StarTile,
                RelicId::HungryGhost,
                RelicId::Disgust,
                RelicId::CurioCabinet,
                RelicId::LotusBloom,
                RelicId::WallWeaver,
                RelicId::KongCollector,
                RelicId::NoHonorButWealth,
                RelicId::Sweepstakes,
                RelicId::BeggarsCup,
                RelicId::BlueTilesWhiteDragon,
                RelicId::RedTilesRedDragon,
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
                RelicId::AncestorEcho,
                RelicId::CrownOfPatterns,
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
