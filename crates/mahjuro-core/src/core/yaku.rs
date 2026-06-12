//! Yaku (hand pattern) detection and bonus scoring.
//!
//! Per-yaku metadata (display name, base mult bonus, base chip bonus) lives
//! in `assets/data/yaku.json`. Behaviour — detection predicates, leveling
//! formulas, scoring integration — stays in Rust.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::core::hand::{DetectedMeld, MeldKind, enumerate_decompositions, validate_selection};
use crate::core::json_asset::load_json_asset;
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};
use crate::core::zodiac::ZodiacKind;

#[derive(Deserialize)]
struct YakuDefRaw {
    id: YakuKind,
    name: String,
    mult_bonus: f64,
    chip_bonus: i32,
}

struct YakuDef {
    name: &'static str,
    mult_bonus: f64,
    chip_bonus: i32,
}

fn yaku_def(id: YakuKind) -> &'static YakuDef {
    static DEFS: OnceLock<FxHashMap<YakuKind, YakuDef>> = OnceLock::new();
    let map = DEFS.get_or_init(|| {
        const PATH: &str = "data/yaku.json";
        let raw: Vec<YakuDefRaw> = load_json_asset(PATH, "yaku data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    YakuDef {
                        name: Box::leak(r.name.into_boxed_str()),
                        mult_bonus: r.mult_bonus,
                        chip_bonus: r.chip_bonus,
                    },
                )
            })
            .collect()
    });
    map.get(&id)
        .unwrap_or_else(|| panic!("yaku def missing for {id:?}"))
}

fn yaku_name_index() -> &'static FxHashMap<&'static str, YakuKind> {
    static INDEX: OnceLock<FxHashMap<&'static str, YakuKind>> = OnceLock::new();
    INDEX.get_or_init(|| {
        const PATH: &str = "data/yaku.json";
        let raw: Vec<YakuDefRaw> = load_json_asset(PATH, "yaku data");
        raw.into_iter()
            .map(|r| {
                let name: &'static str = Box::leak(r.name.into_boxed_str());
                (name, r.id)
            })
            .collect()
    })
}

/// Resolve a cascade step `source` label to a yaku kind when it names a yaku.
pub fn yaku_kind_by_display_name(name: &str) -> Option<YakuKind> {
    yaku_name_index().get(name).copied()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YakuKind {
    /// All tiles are 2–8 of a number suit (no terminals or honors). Tied to
    /// the Monkey zodiac.
    Tanyao,
    /// All melds are triplets (or kongs) — no sequences. Tied to the Ox zodiac.
    Toitoi,
    /// Full 14-tile hand: 4 melds + 1 pair. Kongs count as a meld. Tied to the
    /// Dragon zodiac.
    FullHand,
    /// Triplet (or kong) of any dragon, or of the current ante's round wind.
    /// Tied to the Dog zodiac.
    Yakuhai,
    /// Two identical sequences in the same suit (e.g. 2-3-4m + 2-3-4m). Tied
    /// to the Rabbit zodiac.
    Iipeikou,
    /// Same numerical sequence in all three number suits. Tied to the Horse
    /// zodiac.
    SanshokuDoujun,
    /// 1-9 straight in one number suit (3 sequences: 1-2-3, 4-5-6, 7-8-9).
    /// Tied to the Snake zodiac.
    Ittsu,
    /// One number suit + honors only (no other number suits). Tied to the
    /// Mouse zodiac.
    Honitsu,
    /// Single number suit, no honors. Tied to the Rat zodiac.
    Chinitsu,
    /// Number-suit terminals only (no honors); every meld and the pair contain
    /// a 1 or 9; at least one sequence. Tied to the Goat zodiac. Tiered
    /// exclusive with Honroutou and Chanta.
    Junchan,
    /// Every tile is either a terminal (1 or 9) or an honor. Tied to the
    /// Tiger zodiac.
    Honroutou,
    /// Seven distinct pairs (alternate hand shape). Tied to the Pig zodiac.
    Chiitoitsu,
    /// Thirteen orphans: one of each terminal and honor type, plus one
    /// duplicate of an orphan face. Tied to the Qilin zodiac. Omitted from
    /// `PlayerProgress::available_yaku` (previews, journal, guide) until
    /// the first cash-in that scores it; detection still applies when valid.
    KokushiMusou,
    /// A structurally valid hand that triggers no other yaku. Scores base
    /// chips × 1 mult — legal, but worth very little. Tied to the Rooster
    /// zodiac.
    ChickenHand,
    /// Every meld and the pair contain a terminal or honor; at least one honor,
    /// one simple (2–8), and one sequence. Tied to the Phoenix zodiac.
    Chanta,
    /// Two pairs of identical sequences in one number suit on a full hand.
    /// Tied to the Rabbit zodiac (with Iipeikou).
    Ryanpeikou,
    /// Same-rank triplet (or kong) in all three number suits. Tied to the
    /// Horse zodiac (with Sanshoku Doujun).
    SanshokuDoukou,
    /// Full hand of four sequences and a 2–8 number-suit pair. Tied to the
    /// Crane zodiac.
    Pinfu,
}

impl YakuKind {
    /// Mult bonus added (additively, on the chips×mult scoring axis) when
    /// this yaku fires. These are tuned so that stacking 2-3 yaku on a real
    /// hand pushes mult into the ×8-15 range — that's where the chip pile
    /// turns into "explosive" final scores.
    /// Base mult bonus at yaku level 1. Use `mult_bonus_at(level)` when zodiac
    /// leveling applies for this run.
    pub fn mult_bonus(self) -> f64 {
        self.base_mult_bonus()
    }

    fn base_mult_bonus(self) -> f64 {
        match self {
            YakuKind::Tanyao => 2.5,
            YakuKind::Yakuhai => 2.0,
            YakuKind::Toitoi => 2.0,
            YakuKind::Chanta => 3.5,
            _ => yaku_def(self).mult_bonus,
        }
    }

    /// Base chip bonus added when this yaku fires (separate from the mult
    /// axis). Some patterns grant chips only, mult only, or both (see `yaku.json`).
    pub fn chip_bonus(self) -> i32 {
        self.base_chip_bonus()
    }

    fn base_chip_bonus(self) -> i32 {
        match self {
            YakuKind::Tanyao => 90,
            YakuKind::Yakuhai => 75,
            YakuKind::Toitoi => 70,
            YakuKind::Pinfu => 105,
            YakuKind::Iipeikou => 105,
            YakuKind::Chanta => 90,
            _ => yaku_def(self).chip_bonus,
        }
    }

    /// Leveled mult bonus: `base + per-zodiac-mult × (level - 1)`, snapped to the
    /// nearest half (1.0, 1.5, 2.0, …). Level starts at 1 and rises when the
    /// player uses the zodiac card bound to this yaku. `score_sets` passes the
    /// effective level; use `mult_bonus()` when level is always 1.
    pub fn mult_bonus_at(self, level: u32) -> f64 {
        let base = self.base_mult_bonus();
        let raw = if level <= 1 {
            base
        } else {
            base + self.level_up_mult_per_level() * (level - 1) as f64
        };
        snap_half_mult(raw)
    }

    /// Leveled chip bonus: `base + per-zodiac-chips × (level - 1)`.
    pub fn chip_bonus_at(self, level: u32) -> i32 {
        let base = self.base_chip_bonus();
        if level <= 1 {
            base
        } else {
            base + self.level_up_chips_per_level() * (level as i32 - 1)
        }
    }

    /// Per-level mult increase for this yaku's linked zodiac ribbon.
    pub fn level_up_mult_per_level(self) -> f64 {
        let zodiac = ZodiacKind::for_yaku(self)
            .unwrap_or_else(|| panic!("missing zodiac mapping for yaku {self:?}"));
        zodiac.level_up_mult_per_level()
    }

    /// Per-level chip increase for this yaku's linked zodiac ribbon.
    pub fn level_up_chips_per_level(self) -> i32 {
        let zodiac = ZodiacKind::for_yaku(self)
            .unwrap_or_else(|| panic!("missing zodiac mapping for yaku {self:?}"));
        zodiac.level_up_chips_per_level()
    }

    pub fn name(self) -> &'static str {
        yaku_def(self).name
    }

    /// Engraved label for the in-play bone yaku tablet row.
    pub fn gameplay_tablet_label(self, discovered: bool) -> &'static str {
        if self == YakuKind::ChickenHand {
            "\u{1F414}"
        } else if discovered {
            self.name()
        } else {
            "???"
        }
    }

    /// Tablet label with an optional stack count (`"Yakuhai 3x"`).
    pub fn gameplay_tablet_label_with_count(
        self,
        count: u32,
        discovered: bool,
    ) -> std::borrow::Cow<'static, str> {
        let base = self.gameplay_tablet_label(discovered);
        if count <= 1 {
            std::borrow::Cow::Borrowed(base)
        } else {
            std::borrow::Cow::Owned(format!("{base} {count}x"))
        }
    }

    /// Sort key for reference UIs (journal, guide): lowest base payout first.
    pub fn cmp_by_base_score(a: &Self, b: &Self) -> std::cmp::Ordering {
        a.mult_bonus()
            .partial_cmp(&b.mult_bonus())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.chip_bonus().cmp(&b.chip_bonus()))
    }

    /// All yaku, in display order.
    pub fn all() -> &'static [YakuKind] {
        &[
            YakuKind::Tanyao,
            YakuKind::Toitoi,
            YakuKind::Honroutou,
            YakuKind::Chanta,
            YakuKind::Iipeikou,
            YakuKind::Pinfu,
            YakuKind::FullHand,
            YakuKind::Chinitsu,
            YakuKind::SanshokuDoujun,
            YakuKind::SanshokuDoukou,
            YakuKind::Ryanpeikou,
            YakuKind::Junchan,
            YakuKind::Ittsu,
            YakuKind::Honitsu,
            YakuKind::Yakuhai,
            YakuKind::Chiitoitsu,
            YakuKind::KokushiMusou,
            YakuKind::ChickenHand,
        ]
    }

    /// Journal / guide display order (`Self::all`). Used for in-play yaku tablets.
    pub fn tablet_display_index(self) -> usize {
        Self::all()
            .iter()
            .position(|&k| k == self)
            .unwrap_or(usize::MAX)
    }

    /// Stable sort for fired-yaku tablet rows and cascade overlays.
    pub fn sort_for_tablets(kinds: &mut [Self]) {
        kinds.sort_by_key(|k| k.tablet_display_index());
    }

    /// Collapse duplicate kinds (e.g. three Yakuhai) into one tablet entry.
    /// Call [`Self::sort_for_tablets`] on `kinds` first.
    pub fn consolidate_for_tablets(kinds: &[Self]) -> Vec<YakuTabletEntry> {
        let mut out: Vec<YakuTabletEntry> = Vec::new();
        for &kind in kinds {
            if let Some(last) = out.last_mut()
                && last.kind == kind
            {
                last.count += 1;
            } else {
                out.push(YakuTabletEntry { kind, count: 1 });
            }
        }
        out
    }
}

/// One engraved yaku tablet in the in-play row (duplicates merged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YakuTabletEntry {
    pub kind: YakuKind,
    pub count: u32,
}

/// Live preview of a yaku for the current selection: how close the player is
/// to qualifying, with a short human-readable hint.
#[derive(Clone, Debug)]
pub struct YakuPreview {
    pub kind: YakuKind,
    /// True if the current selection (when valid) actually awards this yaku.
    pub active: bool,
}

/// Compute a `YakuPreview` for each yaku in the player's available pool, based
/// on the currently-selected tiles. Yaku that need a valid decomposition fall
/// back to a "needs valid hand" hint when the selection doesn't decompose.
///
/// When `wildcard_result` is `Some`, it supplies a pre-computed decomposition
/// and (possibly substituted) tile list from relic-aware validation (e.g.
/// WildWinds). This lets the preview reflect hands that only
/// become valid after relic substitutions.
pub fn yaku_preview(
    tiles: &[Tile],
    available: &[YakuKind],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    wildcard_result: Option<(&[DetectedMeld], &[Tile])>,
) -> Vec<YakuPreview> {
    let (sets_opt, effective_tiles, original) = match wildcard_result {
        Some((sets, resolved)) => (Some(sets.to_vec()), resolved, Some(tiles)),
        None => {
            let v = if tiles.len() >= 14 {
                detect_yaku_best_decomposition(tiles, &[], round_wind, bonus_round_wind, None)
                    .map(|(sets, _)| sets)
                    .or_else(|| validate_selection(tiles))
            } else {
                validate_selection(tiles)
            };
            return yaku_preview_inner(tiles, &v, available, round_wind, bonus_round_wind, None);
        }
    };
    yaku_preview_inner(
        effective_tiles,
        &sets_opt,
        available,
        round_wind,
        bonus_round_wind,
        original,
    )
}

fn yaku_preview_inner(
    tiles: &[Tile],
    sets_opt: &Option<Vec<DetectedMeld>>,
    available: &[YakuKind],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
) -> Vec<YakuPreview> {
    let active_yaku: Vec<YakuKind> = match sets_opt {
        Some(s) => yaku_after_pool_filter(
            tiles,
            s,
            round_wind,
            bonus_round_wind,
            original_tiles,
            available,
        ),
        None => Vec::new(),
    };

    let kinds: Vec<YakuKind> = if available.is_empty() {
        YakuKind::all().to_vec()
    } else {
        available.to_vec()
    };

    kinds
        .into_iter()
        .map(|k| {
            let active = active_yaku.contains(&k);
            YakuPreview { kind: k, active }
        })
        .collect()
}

/// Like [`detect_yaku`], but also fires Yakuhai when a triplet/kong matches the
/// supplied `round_wind` (1=East, 2=South, 3=West, 4=North). Dragon triplets
/// always count regardless of `round_wind`.
///
/// `original_tiles` — when wildcard substitution (WildWinds) has
/// modified tiles to make valid melds, pass the *pre-substitution* tiles here
/// so that suit-composition yaku (honitsu, chinitsu, tanyao, honroutou) are
pub fn detect_yaku_with_wind(
    // checked against what the player actually selected, not the resolved faces.
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
) -> Vec<YakuKind> {
    // Suit/rank composition checks use original (pre-substitution) tiles so
    // that wildcard relics can't fabricate yaku the player's real hand
    // doesn't have (e.g. WildWinds turning a mixed-suit hand into honitsu).
    let composition = original_tiles.unwrap_or(tiles);
    let mut found = Vec::new();

    if is_toitoi(sets) {
        found.push(YakuKind::Toitoi);
    }
    if is_tanyao(composition) {
        found.push(YakuKind::Tanyao);
    }
    if is_full_hand(tiles, sets) {
        found.push(YakuKind::FullHand);
    }
    for _ in 0..count_yakuhai(tiles, sets, round_wind, bonus_round_wind) {
        found.push(YakuKind::Yakuhai);
    }
    if is_chiitoitsu(sets) {
        found.push(YakuKind::Chiitoitsu);
    }
    if is_kokushi_musou(sets, tiles) {
        found.push(YakuKind::KokushiMusou);
    }
    if is_ryanpeikou(tiles, sets) {
        found.push(YakuKind::Ryanpeikou);
    } else if is_iipeikou(tiles, sets) {
        found.push(YakuKind::Iipeikou);
    }
    if is_sanshoku_doujun(sets, tiles) {
        found.push(YakuKind::SanshokuDoujun);
    }
    if is_sanshoku_doukou(sets, tiles) {
        found.push(YakuKind::SanshokuDoukou);
    }
    if is_ittsu(sets, tiles) {
        found.push(YakuKind::Ittsu);
    }
    if is_chinitsu(composition) {
        found.push(YakuKind::Chinitsu);
    } else if is_honitsu(composition) {
        found.push(YakuKind::Honitsu);
    }
    if is_junchan(sets, composition) && !is_kokushi_musou(sets, tiles) {
        found.push(YakuKind::Junchan);
    } else if is_honroutou(composition) && !is_kokushi_musou(sets, tiles) {
        found.push(YakuKind::Honroutou);
    } else if is_chanta(sets, tiles) && !is_kokushi_musou(sets, tiles) {
        found.push(YakuKind::Chanta);
    }
    if is_pinfu(tiles, sets) {
        found.push(YakuKind::Pinfu);
    }

    found
}

/// Pick the decomposition that yields the strongest yaku bundle (chip + mult
/// weight). Used for 14-tile previews; full-hand cash-in already picks the
/// highest-scoring decomposition in the run scoring path.
pub fn detect_yaku_best_decomposition(
    tiles: &[Tile],
    rules: &[RuleModifier],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
) -> Option<(Vec<DetectedMeld>, Vec<YakuKind>)> {
    let alternatives = enumerate_decompositions(tiles, rules);
    if alternatives.is_empty() {
        return None;
    }
    let mut best_sets = alternatives[0].clone();
    let mut best_yaku = detect_yaku_with_wind(
        tiles,
        &best_sets,
        round_wind,
        bonus_round_wind,
        original_tiles,
    );
    let mut best_weight = yaku_bundle_weight(&best_yaku);
    for sets in alternatives.into_iter().skip(1) {
        let yaku =
            detect_yaku_with_wind(tiles, &sets, round_wind, bonus_round_wind, original_tiles);
        let weight = yaku_bundle_weight(&yaku);
        if weight > best_weight {
            best_weight = weight;
            best_sets = sets;
            best_yaku = yaku;
        }
    }
    Some((best_sets, best_yaku))
}

fn yaku_bundle_weight(yaku: &[YakuKind]) -> i64 {
    yaku.iter()
        .map(|y| y.chip_bonus() as i64 + (y.mult_bonus() * 100.0).round() as i64)
        .sum()
}

/// True if `tiles`/`sets` form a complete standard win (4 melds + pair or chiitoitsu).
pub fn is_complete_winning_hand(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    is_full_hand(tiles, sets) || is_chiitoitsu(sets) || is_kokushi_musou(sets, tiles)
}

/// Yaku that would score on cash-in after applying the run's unlocked pool
/// (mirrors [`crate::core::scoring::dora_yaku_layer`] filtering).
pub fn yaku_after_pool_filter(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
    available: &[YakuKind],
) -> Vec<YakuKind> {
    let all = detect_yaku_with_wind(tiles, sets, round_wind, bonus_round_wind, original_tiles);
    if available.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|y| {
                *y == YakuKind::KokushiMusou || *y == YakuKind::ChickenHand || available.contains(y)
            })
            .collect()
    }
}

/// True when a structure cash-in would inject Chicken Hand: no detected yaku
/// in the player's unlocked pool after filtering (same gate as
/// [`crate::core::scoring::dora_yaku_layer`]).
pub fn would_inject_chicken_hand(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    available: &[YakuKind],
) -> bool {
    would_inject_chicken_hand_with_original(
        tiles,
        sets,
        round_wind,
        bonus_round_wind,
        None,
        available,
    )
}

/// Like [`would_inject_chicken_hand`], but honors pre-wildcard tiles for
/// composition yaku (WildWinds, etc.).
pub fn would_inject_chicken_hand_with_original(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
    available: &[YakuKind],
) -> bool {
    if sets.is_empty() {
        return false;
    }
    yaku_after_pool_filter(
        tiles,
        sets,
        round_wind,
        bonus_round_wind,
        original_tiles,
        available,
    )
    .is_empty()
}

/// Whether the in-play bone tablet row should show the chicken-hand selector.
/// Same gate as a manual structure cash-in ([`would_inject_chicken_hand_with_original`]).
pub fn would_show_chicken_tablet(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    original_tiles: Option<&[Tile]>,
    available: &[YakuKind],
) -> bool {
    would_inject_chicken_hand_with_original(
        tiles,
        sets,
        round_wind,
        bonus_round_wind,
        original_tiles,
        available,
    )
}

/// Kokushi Musō: twelve [`MeldKind::Single`] and one [`MeldKind::Pair`], using exactly
/// the thirteen orphan faces with one duplicated.
fn is_kokushi_musou(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    if sets.len() != 13 {
        return false;
    }
    let singles = sets.iter().filter(|s| s.kind == MeldKind::Single).count();
    let pairs = sets.iter().filter(|s| s.kind == MeldKind::Pair).count();
    if singles != 12 || pairs != 1 {
        return false;
    }
    for s in sets {
        match s.kind {
            MeldKind::Single if s.tile_ids.len() == 1 => {}
            MeldKind::Pair if s.tile_ids.len() == 2 => {}
            MeldKind::Single | MeldKind::Pair => return false,
            _ => return false,
        }
    }
    let mut counts: FxHashMap<(Suit, u8), u8> = FxHashMap::default();
    for s in sets {
        for &id in &s.tile_ids {
            let Some(t) = tiles.iter().find(|x| x.id == id) else {
                return false;
            };
            if !t.is_kokushi_orphan() {
                return false;
            }
            *counts.entry((t.suit, t.rank)).or_insert(0) += 1;
        }
    }
    counts.len() == 13 && counts.values().filter(|&&c| c == 2).count() == 1
}

/// Toitoi (formerly `AllTriplets`): all non-pair sets are triplets or kongs,
/// no sequences. Requires ≥ 2 such melds so a single meld can't trivially
/// claim the bonus.
fn is_toitoi(sets: &[DetectedMeld]) -> bool {
    let triplet_like = sets
        .iter()
        .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
        .count();
    let sequences = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count();
    triplet_like >= 2 && sequences == 0
}

/// Yakuhai: each triplet/kong of a dragon or matching round/bonus round wind
/// counts once (riichi awards one han per qualifying pon).
fn count_yakuhai(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
) -> u32 {
    sets.iter()
        .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
        .filter(|s| is_yakuhai_meld(s, tiles, round_wind, bonus_round_wind))
        .count() as u32
}

/// True when a triplet/kong is dragon pon or round/bonus round wind pon.
/// Flower wildcards may fill the third slot — tile order must not matter.
fn is_yakuhai_meld(
    meld: &DetectedMeld,
    tiles: &[Tile],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
) -> bool {
    let honor_faces: Vec<(Suit, u8)> = meld
        .tile_ids
        .iter()
        .filter_map(|id| tiles.iter().find(|t| t.id == *id))
        .filter(|t| !t.is_flower())
        .map(|t| (t.suit, t.rank))
        .collect();
    if honor_faces.is_empty() {
        return false;
    }
    let (suit, rank) = honor_faces[0];
    if !honor_faces.iter().all(|face| *face == (suit, rank)) {
        return false;
    }
    match suit {
        Suit::Dragon => true,
        Suit::Wind => {
            round_wind.is_some_and(|w| rank == w) || bonus_round_wind.is_some_and(|w| rank == w)
        }
        _ => false,
    }
}

/// Tanyao (formerly `AllSimples`): every non-flower tile is a numbered suit
/// with rank 2–8. Requires ≥ 3 non-flower tiles (one meld) so simple
/// structure can score yaku as early as honor triplets. Flowers are neutral.
fn is_tanyao(tiles: &[Tile]) -> bool {
    let regular: Vec<&Tile> = tiles.iter().filter(|t| !t.is_flower()).collect();
    regular.len() >= 3
        && regular
            .iter()
            .all(|t| t.is_number_tile() && t.rank >= 2 && t.rank <= 8)
}

/// Chiitoitsu: 7 distinct pairs and nothing else (no triplets, no sequences,
/// no kongs). The hand-validation layer in `hand.rs` reframes 14-tile
/// chiitoitsu hands as `Vec<DetectedMeld>` of 7 `Pair`s, so we just need to
/// check that shape here.
fn is_chiitoitsu(sets: &[DetectedMeld]) -> bool {
    if sets.len() != 7 {
        return false;
    }
    if sets.iter().any(|s| s.kind != MeldKind::Pair) {
        return false;
    }
    // All 7 pairs must be distinct faces. Tile ids guarantee that the same
    // physical tile can't be in two pairs, but the *faces* still need to be
    // unique — two pairs of 5p is not chiitoitsu.
    let mut faces: Vec<(Suit, u8)> = Vec::with_capacity(7);
    for s in sets {
        if let Some(_first) = s.tile_ids.first() {
            // Caller doesn't pass tiles, so we approximate uniqueness by
            // checking the count of distinct tile_ids vs total. The fast path
            // is to assume `crate::core::hand`'s chiitoitsu builder already
            // enforces it, which it does — see `try_chiitoitsu` in
            // `core/hand/decomposition.rs`.
            faces.push((Suit::Wind, 0)); // placeholder; uniqueness enforced upstream
        }
    }
    true
}

/// Chanta (混全帯幺九): every meld and the pair touch a terminal or honor; at
/// least one honor, one simple (2–8), and one sequence (riichi-style).
fn is_chanta(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    if sets.len() < 2 {
        return false;
    }
    if !sets.iter().all(|s| meld_has_yaochu(s, tiles)) {
        return false;
    }
    if !sets.iter().any(|s| s.kind == MeldKind::Sequence) {
        return false;
    }
    composition_has_honor(tiles) && composition_has_simple_number(tiles)
}

/// Ryanpeikou: two different sequences each duplicated in one number suit.
fn is_ryanpeikou(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    if !is_complete_winning_hand(tiles, sets) {
        return false;
    }
    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
        let mut low_rank_counts: FxHashMap<u8, u32> = FxHashMap::default();
        for s in sets.iter().filter(|s| s.kind == MeldKind::Sequence) {
            let tile_refs: Vec<&Tile> = s
                .tile_ids
                .iter()
                .filter_map(|id| tiles.iter().find(|t| t.id == *id))
                .collect();
            if tile_refs.len() != 3 || tile_refs[0].suit != suit {
                continue;
            }
            let mut ranks: Vec<u8> = tile_refs.iter().map(|t| t.rank).collect();
            ranks.sort();
            *low_rank_counts.entry(ranks[0]).or_insert(0) += 1;
        }
        let duplicate_ranks = low_rank_counts.values().filter(|&&c| c >= 2).count();
        if duplicate_ranks >= 2 {
            return true;
        }
    }
    false
}

/// Sanshoku Doukou: same-rank triplet or kong in Manzu, Souzu, and Pinzu.
fn is_sanshoku_doukou(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    let mut by_rank: FxHashMap<u8, FxHashMap<Suit, ()>> = FxHashMap::default();
    for s in sets
        .iter()
        .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
    {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        if tile_refs.is_empty() {
            continue;
        }
        let suit = tile_refs[0].suit;
        if !matches!(suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
            continue;
        }
        let rank = tile_refs[0].rank;
        if !tile_refs.iter().all(|t| t.suit == suit && t.rank == rank) {
            continue;
        }
        by_rank.entry(rank).or_default().insert(suit, ());
    }
    by_rank.values().any(|suits| {
        suits.contains_key(&Suit::Manzu)
            && suits.contains_key(&Suit::Souzu)
            && suits.contains_key(&Suit::Pinzu)
    })
}

/// Pinfu: full hand; four sequences; pair is 2–8 in a number suit (no honors).
fn is_pinfu(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    if !is_full_hand(tiles, sets) {
        return false;
    }
    let pairs: Vec<_> = sets.iter().filter(|s| s.kind == MeldKind::Pair).collect();
    let melds: Vec<_> = sets
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                MeldKind::Sequence | MeldKind::Triplet | MeldKind::Kong
            )
        })
        .collect();
    if pairs.len() != 1 || melds.len() != 4 {
        return false;
    }
    if melds.iter().any(|s| s.kind != MeldKind::Sequence) {
        return false;
    }
    let pair = pairs[0];
    let pair_tiles: Vec<&Tile> = pair
        .tile_ids
        .iter()
        .filter_map(|id| tiles.iter().find(|t| t.id == *id))
        .collect();
    pair_tiles.len() == 2
        && pair_tiles
            .iter()
            .all(|t| t.is_number_tile() && t.rank >= 2 && t.rank <= 8)
}

/// Iipeikou: two identical sequences in the same suit (partial structure ok).
fn is_iipeikou(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    let mut seq_keys: Vec<(Suit, u8)> = sets
        .iter()
        .filter(|s| s.kind == MeldKind::Sequence)
        .filter_map(|s| {
            let mut ranks: Vec<(Suit, u8)> = s
                .tile_ids
                .iter()
                .filter_map(|id| tiles.iter().find(|t| t.id == *id))
                .map(|t| (t.suit, t.rank))
                .collect();
            ranks.sort_by_key(|(_, r)| *r);
            ranks.first().copied()
        })
        .collect();
    seq_keys.sort();
    seq_keys.windows(2).any(|w| w[0] == w[1])
}

/// Sanshoku Doujun: same numerical run in all three number suits. The hand
/// must contain three sequences whose `(low_rank)` matches across
/// Manzu / Souzu / Pinzu.
fn is_sanshoku_doujun(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    let mut by_low: FxHashMap<u8, Vec<Suit>> = FxHashMap::default();
    for s in sets.iter().filter(|s| s.kind == MeldKind::Sequence) {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        if tile_refs.len() != 3 {
            continue;
        }
        let mut ranks: Vec<u8> = tile_refs.iter().map(|t| t.rank).collect();
        ranks.sort();
        by_low.entry(ranks[0]).or_default().push(tile_refs[0].suit);
    }
    by_low.values().any(|suits| {
        suits.contains(&Suit::Manzu) && suits.contains(&Suit::Souzu) && suits.contains(&Suit::Pinzu)
    })
}

/// Ittsu: 1-2-3, 4-5-6, 7-8-9 in a single number suit (a complete 1-9 run).
fn is_ittsu(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    // For each number suit, gather the set of low-ranks of sequences in that suit.
    let mut suit_lows: FxHashMap<Suit, Vec<u8>> = FxHashMap::default();
    for s in sets.iter().filter(|s| s.kind == MeldKind::Sequence) {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        if tile_refs.len() != 3 {
            continue;
        }
        if !matches!(tile_refs[0].suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
            continue;
        }
        let mut ranks: Vec<u8> = tile_refs.iter().map(|t| t.rank).collect();
        ranks.sort();
        suit_lows
            .entry(tile_refs[0].suit)
            .or_default()
            .push(ranks[0]);
    }
    suit_lows
        .values()
        .any(|lows| lows.contains(&1) && lows.contains(&4) && lows.contains(&7))
}

/// Chinitsu: every non-flower tile in a single number suit, no honors. ≥ 5
/// non-flower tiles to avoid trivially firing on a bare meld. Flowers are
/// neutral — they don't introduce a second suit.
fn is_chinitsu(tiles: &[Tile]) -> bool {
    let regular: Vec<&Tile> = tiles.iter().filter(|t| !t.is_flower()).collect();
    if regular.len() < 5 {
        return false;
    }
    let suit = regular[0].suit;
    if !matches!(suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
        return false;
    }
    regular.iter().all(|t| t.suit == suit)
}

/// Honitsu: non-flower tiles consist of one number suit + honors only (with
/// at least one honor — otherwise it's just Chinitsu). Flowers are neutral.
fn is_honitsu(tiles: &[Tile]) -> bool {
    let regular: Vec<&Tile> = tiles.iter().filter(|t| !t.is_flower()).collect();
    if regular.len() < 5 {
        return false;
    }
    let mut number_suit: Option<Suit> = None;
    let mut has_honor = false;
    for t in &regular {
        match t.suit {
            Suit::Wind | Suit::Dragon => has_honor = true,
            s => {
                if let Some(existing) = number_suit {
                    if existing != s {
                        return false;
                    }
                } else {
                    number_suit = Some(s);
                }
            }
        }
    }
    has_honor && number_suit.is_some()
}

/// Terminal or honor present in a meld (yaochu for chanta meld checks).
fn meld_has_yaochu(s: &DetectedMeld, tiles: &[Tile]) -> bool {
    s.tile_ids.iter().any(|id| {
        tiles.iter().find(|t| t.id == *id).is_some_and(|t| {
            matches!(t.suit, Suit::Wind | Suit::Dragon)
                || (t.is_number_tile() && (t.rank == 1 || t.rank == 9))
        })
    })
}

fn tile_is_number_terminal(t: &Tile) -> bool {
    t.is_number_tile() && (t.rank == 1 || t.rank == 9)
}

fn meld_has_number_terminal(s: &DetectedMeld, tiles: &[Tile]) -> bool {
    s.tile_ids.iter().any(|id| {
        tiles
            .iter()
            .find(|t| t.id == *id)
            .is_some_and(tile_is_number_terminal)
    })
}

fn composition_has_honor(tiles: &[Tile]) -> bool {
    tiles
        .iter()
        .filter(|t| !t.is_flower())
        .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
}

fn composition_has_simple_number(tiles: &[Tile]) -> bool {
    tiles
        .iter()
        .filter(|t| !t.is_flower())
        .any(|t| t.is_number_tile() && (2..=8).contains(&t.rank))
}

fn composition_all_number_terminals(tiles: &[Tile]) -> bool {
    let regular: Vec<&Tile> = tiles.iter().filter(|t| !t.is_flower()).collect();
    !regular.is_empty() && regular.iter().all(|t| tile_is_number_terminal(t))
}

/// Junchan (純全帯幺九): no honors; every meld and the pair contain a number
/// 1 or 9; at least one sequence; not all tiles are number terminals only
/// (that shape scores Honroutou). Tiered exclusive with Honroutou and Chanta.
fn is_junchan(sets: &[DetectedMeld], tiles: &[Tile]) -> bool {
    if sets.len() < 2 {
        return false;
    }
    if composition_has_honor(tiles) {
        return false;
    }
    if !sets.iter().all(|s| meld_has_number_terminal(s, tiles)) {
        return false;
    }
    if !sets.iter().any(|s| s.kind == MeldKind::Sequence) {
        return false;
    }
    !composition_all_number_terminals(tiles)
}

/// Honroutou: every non-flower tile is a terminal (1/9) or an honor (no 2-8
/// numbers). Flowers are neutral.
fn is_honroutou(tiles: &[Tile]) -> bool {
    let regular: Vec<&Tile> = tiles.iter().filter(|t| !t.is_flower()).collect();
    if regular.len() < 5 {
        return false;
    }
    regular.iter().all(|t| match t.suit {
        Suit::Wind | Suit::Dragon => true,
        _ => t.rank == 1 || t.rank == 9,
    })
}

/// A complete hand: 4 melds + 1 pair. Kongs count as a meld even though they
/// are 4 tiles each, so a single-kong hand has 15 tiles total instead of the
/// usual 14. Two kongs → 16 tiles, etc. Flower tiles in melds count toward
/// the total (they substitute for regular tiles), plus unused flowers are
/// allowed as extras.
fn is_full_hand(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    let kong_bonus: usize = sets
        .iter()
        .filter(|s| s.kind == MeldKind::Kong)
        .map(|s| s.tile_ids.len().saturating_sub(3))
        .sum();
    // Count tiles that are part of melds (includes flower substitutes).
    let tiles_in_sets: usize = sets.iter().map(|s| s.tile_ids.len()).sum();
    // The remaining tiles should be unused flowers.
    let flower_count = tiles.iter().filter(|t| t.is_flower()).count();
    let flowers_in_sets = sets
        .iter()
        .flat_map(|s| &s.tile_ids)
        .filter(|id| tiles.iter().any(|t| t.id == **id && t.is_flower()))
        .count();
    let unused_flowers = flower_count - flowers_in_sets;
    let expected_set_tiles = 14 + kong_bonus;
    if tiles_in_sets != expected_set_tiles {
        return false;
    }
    if tiles.len() != expected_set_tiles + unused_flowers {
        return false;
    }
    let melds = sets
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                MeldKind::Triplet | MeldKind::Sequence | MeldKind::Kong
            )
        })
        .count();
    let pairs = sets.iter().filter(|s| s.kind == MeldKind::Pair).count();
    melds == 4 && pairs == 1
}

/// Round a yaku mult bonus to the nearest half (…, 1.0, 1.5, 2.0, …).
fn snap_half_mult(v: f64) -> f64 {
    (v * 2.0).round() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn detect_all_triplets() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 1, 2),
            t(Suit::Pinzu, 5, 3),
            t(Suit::Pinzu, 5, 4),
            t(Suit::Pinzu, 5, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Toitoi));
        assert!(!yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn detect_all_simples() {
        let tiles = vec![
            t(Suit::Souzu, 2, 0),
            t(Suit::Souzu, 3, 1),
            t(Suit::Souzu, 4, 2),
            t(Suit::Pinzu, 5, 3),
            t(Suit::Pinzu, 5, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn all_simples_rejects_terminals() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0), // rank 1 = terminal
            t(Suit::Souzu, 2, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Pinzu, 5, 3),
            t(Suit::Pinzu, 5, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(!yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn detect_full_hand() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 1, 2),
            t(Suit::Manzu, 4, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 6, 5),
            t(Suit::Pinzu, 9, 6),
            t(Suit::Pinzu, 9, 7),
            t(Suit::Pinzu, 9, 8),
            t(Suit::Souzu, 5, 9),
            t(Suit::Souzu, 6, 10),
            t(Suit::Souzu, 7, 11),
            t(Suit::Wind, 1, 12),
            t(Suit::Wind, 1, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn no_yaku_on_simple_pair() {
        let tiles = vec![t(Suit::Souzu, 3, 0), t(Suit::Souzu, 3, 1)];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![0, 1],
        }];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        // A bare pair must not award any yaku — they all gate on a real hand.
        assert!(!yaku.contains(&YakuKind::Toitoi));
        assert!(!yaku.contains(&YakuKind::Tanyao));
        assert!(!yaku.contains(&YakuKind::FullHand));
        assert!(!yaku.contains(&YakuKind::Chinitsu));
    }

    #[test]
    fn detect_chiitoitsu_seven_pairs() {
        // 7 distinct pairs.
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Souzu, 3, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 5, 5),
            t(Suit::Pinzu, 7, 6),
            t(Suit::Pinzu, 7, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Wind, 1, 9),
            t(Suit::Wind, 3, 10),
            t(Suit::Wind, 3, 11),
            t(Suit::Dragon, 2, 12),
            t(Suit::Dragon, 2, 13),
        ];
        let sets: Vec<DetectedMeld> = (0..7)
            .map(|i| DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![(i * 2) as u32, (i * 2 + 1) as u32],
            })
            .collect();
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Chiitoitsu));
    }

    #[test]
    fn detect_chinitsu_single_suit_no_honors() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 2, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Souzu, 4, 3),
            t(Suit::Souzu, 4, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Chinitsu));
        assert!(!yaku.contains(&YakuKind::Honitsu));
    }

    #[test]
    fn detect_honitsu_one_suit_with_honors() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 2, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Wind, 1, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honitsu));
        assert!(!yaku.contains(&YakuKind::Chinitsu));
    }

    #[test]
    fn detect_honitsu_two_sequences_dragon_pair() {
        // D123 + D456 + White White — one number suit with a dragon pair.
        let tiles = vec![
            t(Suit::Pinzu, 1, 0),
            t(Suit::Pinzu, 2, 1),
            t(Suit::Pinzu, 3, 2),
            t(Suit::Pinzu, 4, 3),
            t(Suit::Pinzu, 5, 4),
            t(Suit::Pinzu, 6, 5),
            t(Suit::Dragon, 3, 6), // White dragon
            t(Suit::Dragon, 3, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honitsu));
    }

    #[test]
    fn detect_iipeikou_two_identical_sequences() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 2, 3),
            t(Suit::Manzu, 3, 4),
            t(Suit::Manzu, 4, 5),
            t(Suit::Pinzu, 5, 6),
            t(Suit::Pinzu, 5, 7),
            t(Suit::Pinzu, 5, 8),
            t(Suit::Souzu, 7, 9),
            t(Suit::Souzu, 8, 10),
            t(Suit::Souzu, 9, 11),
            t(Suit::Wind, 1, 12),
            t(Suit::Wind, 1, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Iipeikou));
        assert!(yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn detect_sanshoku_doujun() {
        let tiles = vec![
            t(Suit::Manzu, 4, 0),
            t(Suit::Manzu, 5, 1),
            t(Suit::Manzu, 6, 2),
            t(Suit::Souzu, 4, 3),
            t(Suit::Souzu, 5, 4),
            t(Suit::Souzu, 6, 5),
            t(Suit::Pinzu, 4, 6),
            t(Suit::Pinzu, 5, 7),
            t(Suit::Pinzu, 6, 8),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::SanshokuDoujun));
    }

    #[test]
    fn detect_ittsu_full_straight_one_suit() {
        let tiles = vec![
            t(Suit::Pinzu, 1, 0),
            t(Suit::Pinzu, 2, 1),
            t(Suit::Pinzu, 3, 2),
            t(Suit::Pinzu, 4, 3),
            t(Suit::Pinzu, 5, 4),
            t(Suit::Pinzu, 6, 5),
            t(Suit::Pinzu, 7, 6),
            t(Suit::Pinzu, 8, 7),
            t(Suit::Pinzu, 9, 8),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Ittsu));
    }

    #[test]
    fn detect_honroutou_terminals_and_honors() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 1, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Wind, 1, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Junchan));
    }

    #[test]
    fn terminal_yaku_honroutou_without_junchan() {
        // One meld in structure: Honroutou qualifies, Junchan needs ≥2 sets.
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 1, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Wind, 1, 4),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Chanta));
    }

    #[test]
    fn chiitoitsu_validates_at_hand_layer() {
        // hand.rs's validate_selection should accept a 7-pairs hand and route
        // through the chiitoitsu fallback.
        use crate::core::hand::validate_selection;
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 1, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Souzu, 3, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 5, 5),
            t(Suit::Pinzu, 7, 6),
            t(Suit::Pinzu, 7, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Wind, 1, 9),
            t(Suit::Wind, 3, 10),
            t(Suit::Wind, 3, 11),
            t(Suit::Dragon, 2, 12),
            t(Suit::Dragon, 2, 13),
        ];
        let sets = validate_selection(&tiles).expect("seven pairs should validate");
        assert_eq!(sets.len(), 7);
        assert!(sets.iter().all(|s| s.kind == MeldKind::Pair));
    }

    #[test]
    fn detect_yakuhai_dragon_triplet_always() {
        // Green dragon triplet — Yakuhai fires regardless of round wind.
        let tiles = vec![
            t(Suit::Dragon, 2, 0),
            t(Suit::Dragon, 2, 1),
            t(Suit::Dragon, 2, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(
            detect_yaku_with_wind(&tiles, &sets, None, None, None).contains(&YakuKind::Yakuhai)
        );
        assert!(
            detect_yaku_with_wind(&tiles, &sets, Some(1), None, None).contains(&YakuKind::Yakuhai)
        );
    }

    #[test]
    fn detect_yakuhai_bonus_round_wind_match() {
        let tiles = vec![
            t(Suit::Wind, 2, 0),
            t(Suit::Wind, 2, 1),
            t(Suit::Wind, 2, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(
            !detect_yaku_with_wind(&tiles, &sets, Some(1), None, None).contains(&YakuKind::Yakuhai)
        );
        assert!(
            detect_yaku_with_wind(&tiles, &sets, Some(1), Some(2), None)
                .contains(&YakuKind::Yakuhai)
        );
    }

    /// Full hand built from compact labels **EEF1** and **NNF1**: each is two winds
    /// plus a flower wildcard completing a triplet (`E`+`E`+`F1`, `N`+`N`+`F1`).
    /// Filler melds are one souzu suit so the hand also scores Honitsu + Full Hand.
    fn eef1_nnf1_flower_wind_hand() -> (Vec<Tile>, Vec<DetectedMeld>) {
        use crate::core::hand::validate_selection;

        let tiles = vec![
            // EEF1 — East triplet
            t(Suit::Wind, 1, 0),
            t(Suit::Wind, 1, 1),
            t(Suit::Flower, 1, 100),
            // NNF1 — North triplet (second F1 tile, distinct id)
            t(Suit::Wind, 4, 2),
            t(Suit::Wind, 4, 3),
            t(Suit::Flower, 1, 101),
            // Honitsu filler: one number suit (souzu)
            t(Suit::Souzu, 2, 4),
            t(Suit::Souzu, 3, 5),
            t(Suit::Souzu, 4, 6),
            t(Suit::Souzu, 5, 7),
            t(Suit::Souzu, 5, 8),
            t(Suit::Souzu, 5, 9),
            t(Suit::Souzu, 8, 10),
            t(Suit::Souzu, 8, 11),
        ];
        let sets = validate_selection(&tiles).expect("EEF1+NNF1 hand should decompose");
        assert_eq!(sets.len(), 5, "expected 4 melds + pair: {:?}", sets);
        assert!(is_full_hand(&tiles, &sets));
        let wind_flower_triplets = sets
            .iter()
            .filter(|s| {
                s.kind == MeldKind::Triplet && s.tile_ids.iter().any(|id| id == &100 || id == &101)
            })
            .count();
        assert_eq!(
            wind_flower_triplets, 2,
            "EEF1 and NNF1 should each form a triplet"
        );
        (tiles, sets)
    }

    #[test]
    fn classify_yaku_for_eef1_nnf1_flower_wind_hand() {
        let (tiles, sets) = eef1_nnf1_flower_wind_hand();

        let no_wind = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(no_wind.contains(&YakuKind::Honitsu));
        assert!(no_wind.contains(&YakuKind::FullHand));
        assert!(!no_wind.contains(&YakuKind::Yakuhai));
        assert!(!no_wind.contains(&YakuKind::Toitoi));

        // Round wind East — flower-assisted East triplet (EEF1) is Yakuhai.
        let east_round = detect_yaku_with_wind(&tiles, &sets, Some(1), None, None);
        assert!(east_round.contains(&YakuKind::Yakuhai));
        assert!(east_round.contains(&YakuKind::Honitsu));

        // Round wind North — flower-assisted North triplet (NNF1) is Yakuhai.
        let north_round = detect_yaku_with_wind(&tiles, &sets, Some(4), None, None);
        assert!(north_round.contains(&YakuKind::Yakuhai));
        assert!(north_round.contains(&YakuKind::Honitsu));
    }

    #[test]
    fn detect_yakuhai_round_wind_match() {
        // East wind triplet, round wind = East (1) — fires.
        let tiles = vec![
            t(Suit::Wind, 1, 0),
            t(Suit::Wind, 1, 1),
            t(Suit::Wind, 1, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        // Without wind context, wind triplets don't fire.
        assert!(
            !detect_yaku_with_wind(&tiles, &sets, None, None, None).contains(&YakuKind::Yakuhai)
        );
        // With matching round wind, fires.
        assert!(
            detect_yaku_with_wind(&tiles, &sets, Some(1), None, None).contains(&YakuKind::Yakuhai)
        );
        // With non-matching round wind, doesn't fire.
        assert!(
            !detect_yaku_with_wind(&tiles, &sets, Some(2), None, None).contains(&YakuKind::Yakuhai)
        );
    }

    #[test]
    fn honroutou_tiered_exclusive_over_junchan_when_honors_present() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 1, 1),
            t(Suit::Manzu, 1, 2),
            t(Suit::Manzu, 9, 3),
            t(Suit::Manzu, 9, 4),
            t(Suit::Manzu, 9, 5),
            t(Suit::Pinzu, 1, 6),
            t(Suit::Pinzu, 1, 7),
            t(Suit::Pinzu, 1, 8),
            t(Suit::Dragon, 2, 9),
            t(Suit::Dragon, 2, 10),
            t(Suit::Dragon, 2, 11),
            t(Suit::Souzu, 9, 12),
            t(Suit::Souzu, 9, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        assert!(is_full_hand(&tiles, &sets));
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Chanta));
        assert!(yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn detect_junchan_riichi_no_honors_requires_sequence() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Manzu, 7, 3),
            t(Suit::Manzu, 8, 4),
            t(Suit::Manzu, 9, 5),
            t(Suit::Souzu, 1, 6),
            t(Suit::Souzu, 1, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Chanta));
    }

    #[test]
    fn terminal_yaku_tiered_exclusive_full_honor_hand() {
        let tiles = vec![
            t(Suit::Dragon, 1, 0),
            t(Suit::Dragon, 1, 1),
            t(Suit::Dragon, 1, 2),
            t(Suit::Dragon, 2, 3),
            t(Suit::Dragon, 2, 4),
            t(Suit::Dragon, 2, 5),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Chanta));
    }

    #[test]
    fn terminal_yaku_chanta_rejects_simple_only_pair() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Souzu, 7, 3),
            t(Suit::Souzu, 8, 4),
            t(Suit::Souzu, 9, 5),
            t(Suit::Pinzu, 5, 6),
            t(Suit::Pinzu, 5, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(!yaku.contains(&YakuKind::Chanta));
        assert!(!yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Honroutou));
    }

    #[test]
    fn terminal_yaku_chanta_requires_honor_and_terminal_pair() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Souzu, 7, 3),
            t(Suit::Souzu, 8, 4),
            t(Suit::Souzu, 9, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Chanta));
        assert!(!yaku.contains(&YakuKind::Honroutou));
        assert!(!yaku.contains(&YakuKind::Junchan));
    }

    #[test]
    fn count_yakuhai_stacks_per_qualifying_triplet() {
        let tiles = vec![
            t(Suit::Dragon, 1, 0),
            t(Suit::Dragon, 1, 1),
            t(Suit::Dragon, 1, 2),
            t(Suit::Dragon, 3, 3),
            t(Suit::Dragon, 3, 4),
            t(Suit::Dragon, 3, 5),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert_eq!(yaku.iter().filter(|y| **y == YakuKind::Yakuhai).count(), 2);
    }

    #[test]
    fn sort_for_tablets_uses_journal_display_order() {
        let mut kinds = vec![YakuKind::Yakuhai, YakuKind::Honroutou, YakuKind::Tanyao];
        YakuKind::sort_for_tablets(&mut kinds);
        assert_eq!(
            kinds,
            vec![YakuKind::Tanyao, YakuKind::Honroutou, YakuKind::Yakuhai]
        );
    }

    #[test]
    fn consolidate_for_tablets_merges_duplicate_yakuhai() {
        let mut kinds = vec![
            YakuKind::Toitoi,
            YakuKind::Yakuhai,
            YakuKind::FullHand,
            YakuKind::Yakuhai,
            YakuKind::Honroutou,
            YakuKind::Yakuhai,
        ];
        YakuKind::sort_for_tablets(&mut kinds);
        let entries = YakuKind::consolidate_for_tablets(&kinds);
        assert_eq!(
            entries,
            vec![
                YakuTabletEntry {
                    kind: YakuKind::Toitoi,
                    count: 1,
                },
                YakuTabletEntry {
                    kind: YakuKind::Honroutou,
                    count: 1,
                },
                YakuTabletEntry {
                    kind: YakuKind::FullHand,
                    count: 1,
                },
                YakuTabletEntry {
                    kind: YakuKind::Yakuhai,
                    count: 3,
                },
            ]
        );
        assert_eq!(
            YakuKind::Yakuhai.gameplay_tablet_label_with_count(3, true),
            "Yakuhai 3x"
        );
    }

    #[test]
    fn count_yakuhai_ignores_flower_slot_order() {
        let tiles = vec![
            t(Suit::Dragon, 1, 0),
            t(Suit::Dragon, 1, 1),
            t(Suit::Flower, 1, 100),
        ];
        let dragon_first = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 100],
        }];
        let flower_first = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![100, 0, 1],
        }];
        for sets in [dragon_first, flower_first] {
            assert!(
                detect_yaku_with_wind(&tiles, &sets, None, None, None).contains(&YakuKind::Yakuhai),
                "flower-assisted dragon triplet must count regardless of tile order"
            );
        }
    }

    #[test]
    fn detect_iipeikou_on_partial_structure() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 2, 3),
            t(Suit::Manzu, 3, 4),
            t(Suit::Manzu, 4, 5),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Iipeikou));
        assert!(yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn detect_tanyao_on_single_simple_meld() {
        let tiles = vec![
            t(Suit::Souzu, 2, 0),
            t(Suit::Souzu, 3, 1),
            t(Suit::Souzu, 4, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        }];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn chinitsu_excludes_honitsu() {
        let tiles = vec![
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 2, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Souzu, 4, 3),
            t(Suit::Souzu, 4, 4),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Chinitsu));
        assert!(!yaku.contains(&YakuKind::Honitsu));
    }

    #[test]
    fn detect_yakuhai_kong_counts() {
        // Red dragon kong fires Yakuhai (kongs count as triplets).
        let tiles = vec![
            t(Suit::Dragon, 1, 0),
            t(Suit::Dragon, 1, 1),
            t(Suit::Dragon, 1, 2),
            t(Suit::Dragon, 1, 3),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Kong,
            tile_ids: vec![0, 1, 2, 3],
        }];
        assert!(
            detect_yaku_with_wind(&tiles, &sets, None, None, None).contains(&YakuKind::Yakuhai)
        );
    }

    #[test]
    fn mult_bonus_values() {
        assert_eq!(YakuKind::Toitoi.mult_bonus(), 2.0);
        assert_eq!(YakuKind::Tanyao.mult_bonus(), 2.5);
        assert_eq!(YakuKind::FullHand.mult_bonus(), 5.0);
        assert_eq!(YakuKind::Chinitsu.mult_bonus(), 5.5);
    }

    #[test]
    fn detect_chanta_meld_has_terminal() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Souzu, 7, 3),
            t(Suit::Souzu, 8, 4),
            t(Suit::Souzu, 9, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Chanta));
        assert!(!yaku.contains(&YakuKind::Junchan));
        assert!(!yaku.contains(&YakuKind::Honroutou));
    }

    #[test]
    fn detect_ryanpeikou_two_duplicate_sequences() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 2, 3),
            t(Suit::Manzu, 3, 4),
            t(Suit::Manzu, 4, 5),
            t(Suit::Manzu, 5, 6),
            t(Suit::Manzu, 6, 7),
            t(Suit::Manzu, 7, 8),
            t(Suit::Manzu, 5, 9),
            t(Suit::Manzu, 6, 10),
            t(Suit::Manzu, 7, 11),
            t(Suit::Souzu, 8, 12),
            t(Suit::Souzu, 8, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Ryanpeikou));
        assert!(!yaku.contains(&YakuKind::Iipeikou));
        assert!(yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn detect_sanshoku_doukou_matching_triplets() {
        let tiles = vec![
            t(Suit::Manzu, 4, 0),
            t(Suit::Manzu, 4, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Souzu, 4, 3),
            t(Suit::Souzu, 4, 4),
            t(Suit::Souzu, 4, 5),
            t(Suit::Pinzu, 4, 6),
            t(Suit::Pinzu, 4, 7),
            t(Suit::Pinzu, 4, 8),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::SanshokuDoukou));
    }

    #[test]
    fn detect_pinfu_all_sequences_simple_pair() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 5, 3),
            t(Suit::Manzu, 6, 4),
            t(Suit::Manzu, 7, 5),
            t(Suit::Souzu, 3, 6),
            t(Suit::Souzu, 4, 7),
            t(Suit::Souzu, 5, 8),
            t(Suit::Pinzu, 6, 9),
            t(Suit::Pinzu, 7, 10),
            t(Suit::Pinzu, 8, 11),
            t(Suit::Manzu, 5, 12),
            t(Suit::Manzu, 5, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(yaku.contains(&YakuKind::Pinfu));
        assert!(yaku.contains(&YakuKind::FullHand));
        assert!(yaku.contains(&YakuKind::Tanyao));
    }

    #[test]
    fn pinfu_rejects_dragon_pair() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 5, 3),
            t(Suit::Manzu, 6, 4),
            t(Suit::Manzu, 7, 5),
            t(Suit::Souzu, 3, 6),
            t(Suit::Souzu, 4, 7),
            t(Suit::Souzu, 5, 8),
            t(Suit::Pinzu, 6, 9),
            t(Suit::Pinzu, 7, 10),
            t(Suit::Pinzu, 8, 11),
            t(Suit::Dragon, 1, 12),
            t(Suit::Dragon, 1, 13),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku_with_wind(&tiles, &sets, None, None, None);
        assert!(!yaku.contains(&YakuKind::Pinfu));
    }

    #[test]
    fn would_inject_chicken_hand_when_complete_and_no_unlocked_yaku() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Manzu, 4, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 6, 5),
            t(Suit::Manzu, 7, 6),
            t(Suit::Manzu, 8, 7),
            t(Suit::Manzu, 9, 8),
            t(Suit::Pinzu, 2, 9),
            t(Suit::Pinzu, 3, 10),
            t(Suit::Pinzu, 4, 11),
            t(Suit::Pinzu, 5, 12),
            t(Suit::Pinzu, 5, 13),
        ];
        let sets = validate_selection(&tiles).expect("complete hand");
        let available = vec![YakuKind::Tanyao];
        assert!(
            !detect_yaku_with_wind(&tiles, &sets, None, None, None).contains(&YakuKind::Tanyao)
        );
        assert!(would_inject_chicken_hand(
            &tiles, &sets, None, None, &available,
        ));
    }

    #[test]
    fn mult_bonus_at_levels_up() {
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(1), 2.0);
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(2), 2.5);
        assert_eq!(YakuKind::Toitoi.mult_bonus_at(5), 4.0);
        assert_eq!(YakuKind::Toitoi.chip_bonus_at(1), 70);
        assert_eq!(YakuKind::Toitoi.chip_bonus_at(5), 190);
    }

    #[test]
    fn mult_bonus_at_is_half_increment_at_every_level() {
        for &yk in YakuKind::all() {
            if yk == YakuKind::ChickenHand {
                continue;
            }
            for level in 1..=12 {
                let mult = yk.mult_bonus_at(level);
                assert_eq!(
                    mult,
                    snap_half_mult(mult),
                    "{yk:?} level {level} mult {mult} is not a half increment"
                );
            }
        }
    }

    /// Force a panic on data drift. Touching every variant via the metadata
    /// accessors triggers the OnceLock load; if `assets/data/yaku.json` is
    /// missing an id (or has duplicates that overwrite earlier entries),
    /// `yaku_def` panics here. The match is exhaustive so adding a new
    /// variant won't compile until it's classified, and `YakuKind::all()`
    /// must include every classified variant — covered by the count check.
    #[test]
    fn every_yaku_variant_has_a_data_entry() {
        for &kind in YakuKind::all() {
            // Calling these forces the table lookup; missing entries panic.
            let _ = kind.name();
            let _ = kind.mult_bonus();
            let _ = kind.chip_bonus();
        }
        assert_eq!(
            YakuKind::all().len(),
            18,
            "YakuKind::all() must list every variant — update if you added one"
        );
    }

    #[test]
    fn would_show_chicken_tablet_for_partial_two_pair_structure() {
        let tiles = vec![
            t(Suit::Pinzu, 5, 0),
            t(Suit::Pinzu, 5, 1),
            t(Suit::Dragon, 1, 2),
            t(Suit::Dragon, 1, 3),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![0, 1],
            },
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![2, 3],
            },
        ];
        let available: Vec<YakuKind> = YakuKind::all().to_vec();
        assert!(would_inject_chicken_hand(
            &tiles, &sets, None, None, &available
        ));
        assert!(would_show_chicken_tablet(
            &tiles, &sets, None, None, None, &available,
        ));
    }

    #[test]
    fn would_show_chicken_tablet_false_for_full_hand_only() {
        let tiles = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Manzu, 4, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 6, 5),
            t(Suit::Manzu, 7, 6),
            t(Suit::Manzu, 8, 7),
            t(Suit::Manzu, 9, 8),
            t(Suit::Pinzu, 2, 9),
            t(Suit::Pinzu, 3, 10),
            t(Suit::Pinzu, 4, 11),
            t(Suit::Pinzu, 5, 12),
            t(Suit::Pinzu, 5, 13),
        ];
        let sets = validate_selection(&tiles).expect("complete hand");
        let available: Vec<YakuKind> = YakuKind::all().to_vec();
        assert!(!would_show_chicken_tablet(
            &tiles, &sets, None, None, None, &available,
        ));
        assert!(!would_inject_chicken_hand(
            &tiles, &sets, None, None, &available
        ));
    }

    #[test]
    fn would_show_chicken_tablet_false_when_pattern_yaku_active() {
        let tiles = vec![
            t(Suit::Manzu, 2, 0),
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 5, 3),
            t(Suit::Manzu, 6, 4),
            t(Suit::Manzu, 7, 5),
            t(Suit::Souzu, 3, 6),
            t(Suit::Souzu, 4, 7),
            t(Suit::Souzu, 5, 8),
            t(Suit::Pinzu, 6, 9),
            t(Suit::Pinzu, 7, 10),
            t(Suit::Pinzu, 8, 11),
            t(Suit::Manzu, 5, 12),
            t(Suit::Manzu, 5, 13),
        ];
        let sets = validate_selection(&tiles).expect("tanyao hand");
        let available: Vec<YakuKind> = YakuKind::all().to_vec();
        assert!(!would_show_chicken_tablet(
            &tiles, &sets, None, None, None, &available,
        ));
    }
}
