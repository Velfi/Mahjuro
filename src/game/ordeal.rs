//! Boss blinds — Balatro-style themed encounters with distinct effects.
//!
//! Each boss is a `OrdealKind` variant with a `OrdealDef` describing how to
//! apply it. Presentation (name, description, tier, min_ante) lives in
//! `assets/data/ordeals.json` and is loaded once at startup; behaviour
//! (rule_pushes, debuffs, on_apply / on_play / on_reveal hooks) stays in
//! Rust where it can use function pointers freely. Final-tier bosses are
//! kept in a separate pool — they only appear on `FINAL_WING` and are
//! never drawn by the regular roller.
//!
//! Effects dispatch through three hooks:
//! * `rule_pushes` — `RuleModifier`s injected into `round_rules`, picked up by
//!   `score_sets` and `validate_selection_with_rules` like any other rule.
//! * `on_apply` — called from `apply_chamber` when the boss blind starts. Used
//!   for one-shot mutations to `RunState` (zero discards, target bumps,
//!   shrunken hand, etc.).
//! * `on_play` — called from `commit_selection_to_structure` after a successful play,
//!   for per-play taxers (gold cost, wall burn).
//!
//! Adding a new boss is purely a matter of appending to `ordeals.json`
//! (presentation), adding a `OrdealKind` variant, supplying the right
//! hook in `ordeal_behavior`, and adding an atlas cell + slug in
//! `assets/textures/ordeal_icons/atlas.toml` (`OrdealKind::ALL` / `atlas_slug`)
//! — no other file needs to know the boss exists.

use std::sync::OnceLock;

use rand::RngExt;
use serde::Deserialize;

use crate::core::debuff::{TileDebuff, TileDebuffClass};
use mahjuro_core::core::json_asset::load_json_asset;
use crate::core::relic::{RelicId, RelicState};
use crate::core::rules::RuleModifier;
use crate::game::run::RunState;

pub use mahjuro_core::core::ordeal_kind::{OrdealKind, OrdealTier};

/// Static definition of a boss: presentation + effect dispatch.
pub struct OrdealDef {
    pub kind: OrdealKind,
    pub name: &'static str,
    pub description: &'static str,
    pub tier: OrdealTier,
    /// Earliest wing on which this boss may be drawn. Final-tier bosses
    /// ignore this and only appear on `FINAL_WING`.
    pub min_wing: u32,
    pub effect: OrdealEffect,
    /// Reactive bosses use this hook to compute their final effect at the
    /// moment the Ordeal blind is revealed (`ensure_ordeal_revealed` / wing 1
    /// `RunState::new`), not when the previous wing's boss is defeated.
    pub on_reveal: Option<fn(&mut RunState) -> ResolvedOrdealEffect>,
}

/// Three-axis effect descriptor. Each field is independently optional —
/// most bosses use only one axis.
pub struct OrdealEffect {
    pub rule_pushes: &'static [RuleModifier],
    pub tile_debuffs: &'static [TileDebuff],
    pub relic_debuffs: &'static [RelicId],
    pub on_apply: Option<fn(&mut RunState)>,
    pub on_play: Option<fn(&mut RunState)>,
}

/// Owned, ante-scoped sibling of `OrdealEffect`.
#[derive(Clone, Debug)]
pub struct ResolvedOrdealEffect {
    pub rule_pushes: Vec<RuleModifier>,
    pub tile_debuffs: Vec<TileDebuff>,
    pub relic_debuffs: Vec<RelicId>,
    pub on_apply: Option<fn(&mut RunState)>,
    pub on_play: Option<fn(&mut RunState)>,
    pub description_override: Option<String>,
}

impl ResolvedOrdealEffect {
    pub fn from_static(eff: &OrdealEffect) -> Self {
        Self {
            rule_pushes: eff.rule_pushes.to_vec(),
            tile_debuffs: eff.tile_debuffs.to_vec(),
            relic_debuffs: eff.relic_debuffs.to_vec(),
            on_apply: eff.on_apply,
            on_play: eff.on_play,
            description_override: None,
        }
    }
}

pub fn ordeal_def(kind: OrdealKind) -> &'static OrdealDef {
    all_ordeals()
        .iter()
        .chain(final_ordeals().iter())
        .find(|d| d.kind == kind)
        .expect("every OrdealKind must have a definition")
}

pub fn ordeal_name(kind: OrdealKind) -> &'static str {
    ordeal_def(kind).name
}

pub fn ordeal_tier(kind: OrdealKind) -> OrdealTier {
    ordeal_def(kind).tier
}

pub trait OrdealKindExt {
    fn def(self) -> &'static OrdealDef;
    fn name(self) -> &'static str;
    fn tier(self) -> OrdealTier;
}

impl OrdealKindExt for OrdealKind {
    fn def(self) -> &'static OrdealDef {
        ordeal_def(self)
    }

    fn name(self) -> &'static str {
        ordeal_name(self)
    }

    fn tier(self) -> OrdealTier {
        ordeal_tier(self)
    }
}

// ── Effect helpers ───────────────────────────────────────────────────────
//
// These are top-level fns (not closures) so they can be used as `fn` pointers
// in the static `OrdealDef` table — `Fn` trait objects can't sit in a const.

fn drought_apply(run: &mut RunState) {
    // Halve discards (round down). At default 4 starting discards this gives 2,
    // a meaningful tax without removing the lever entirely.
    run.discards_remaining /= 2;
}

fn whisper_apply(run: &mut RunState) {
    // Shrink the hand by 1 for the whole round. The bonus_hand_size delta is
    // honored by `refill_hand` and the `commit_selection_to_structure` draw target.
    run.ordeal.bonus_hand_size -= 1;
    let target = effective_hand_size(run);
    crate::game::engine_state::GameplayCoreState::with_run_mut(run, |core| {
        while core.hand.len() > target {
            core.hand.pop();
        }
        core.selected = vec![false; core.hand.len()];
    });
}

fn tribute_apply(run: &mut RunState) {
    run.ordeal.yen_cost_per_play = 1;
}

fn famine_apply(run: &mut RunState) {
    // Math wall: target doubles on top of whatever the blind multiplier set.
    run.target_score = run.target_score.saturating_mul(2);
}

fn tempest_play(run: &mut RunState) {
    // Burn one tile off the top of the wall. No-op if wall is empty.
    let _ = run.wall.draw();
}

fn tribute_play(run: &mut RunState) {
    // Tax fires after the play has resolved. Gold is allowed to go negative
    // during boss rounds — the player can still finish the round but will
    // need to earn it back in the shop/payout phase.
    let cost = run.ordeal.yen_cost_per_play as i32;
    run.apply_yen_delta(-cost, None);
}

// ── Reactive boss hooks ───────────────────────────────────────────────────
//
// `on_reveal` runs when the Ordeal blind is revealed (`RunState::ensure_ordeal_revealed`)
// — well before `apply_chamber`. It receives an immutable view of the run and
// returns a `ResolvedOrdealEffect` whose hooks/rule_pushes/description are
// frozen for the rest of the ante. The player sees the resolved description
// in pick_chamber from the moment the ante starts.

/// Mirror — silences whichever scoring axis the player has invested most
/// relic support in (pair / sequence / triplet). Reuses the existing rule
/// modifiers from Hermit, Forest, and Censor — no new scoring math needed.
/// Ties favor pairs (most universal yaku component); a player with no
/// axis-leaning relics also gets PairsScoreZero as the default sting.
fn mirror_reveal(run: &mut RunState) -> ResolvedOrdealEffect {
    let mut pair = 0u32;
    let mut seq = 0u32;
    let mut trip = 0u32;
    for &r in &run.relics.active {
        match r {
            RelicId::PairPower => pair += 2,
            RelicId::SequenceSurge => seq += 2,
            RelicId::TripletBoost => trip += 2,
            // Honor / kong / overflow lean weakly toward triplet builds.
            RelicId::HonorFury | RelicId::KanDrum | RelicId::KongsBlessing => trip += 1,
            // Set magnet biases toward sequence completion.
            RelicId::SetMagnet => seq += 1,
            _ => {}
        }
    }
    let (rule, axis_label): (RuleModifier, &str) = if seq > pair && seq > trip {
        (RuleModifier::SequencesHalved, "sequences")
    } else if trip > pair && trip > seq {
        // No "triplets score zero" rule exists — censor repeats is the
        // closest analog since triplet builds tend to repeat the same yaku.
        (RuleModifier::CensorRepeats, "triplets")
    } else {
        (RuleModifier::PairsScoreZero, "pairs")
    };
    ResolvedOrdealEffect {
        rule_pushes: vec![rule],
        tile_debuffs: vec![],
        relic_debuffs: vec![],
        on_apply: None,
        on_play: None,
        description_override: Some(format!(
            "Silenced your build: {} ({})",
            rule.name(),
            axis_label
        )),
    }
}

/// Tax Collector — per-play gold tax that scales with the player's hoard at
/// reveal time. Mirrors Tribute's hook pair, but the cost is dynamic. The
/// chosen cost is written to `RunState::tax_collector_cost` here so the
/// `on_apply` hook can read it during the boss blind. Locking the cost at
/// reveal means a player who spends gold down before the boss blind can't
/// escape — the price was set when the ante began.
fn tax_collector_reveal(run: &mut RunState) -> ResolvedOrdealEffect {
    let cost = (run.yen.max(0) as u32 / 10).clamp(2, 8);
    run.ordeal.tax_collector_cost = cost;
    ResolvedOrdealEffect {
        rule_pushes: vec![],
        tile_debuffs: vec![],
        relic_debuffs: vec![],
        on_apply: Some(tax_collector_apply),
        on_play: Some(tribute_play),
        description_override: Some(format!("Pay {cost} gold each play")),
    }
}

fn tax_collector_apply(run: &mut RunState) {
    // The cost was stashed on RunState by `tax_collector_reveal`. Mirror
    // Tribute's path: set yen_cost_per_play and let `tribute_play` drain it.
    run.ordeal.yen_cost_per_play = run.ordeal.tax_collector_cost;
}

fn blight_reveal(run: &mut RunState) -> ResolvedOrdealEffect {
    let candidates = [
        (
            TileDebuff::Suit(crate::core::tile::Suit::Manzu),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Manzu)
                .count(),
        ),
        (
            TileDebuff::Suit(crate::core::tile::Suit::Souzu),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Souzu)
                .count(),
        ),
        (
            TileDebuff::Suit(crate::core::tile::Suit::Pinzu),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Pinzu)
                .count(),
        ),
        (
            TileDebuff::Class(TileDebuffClass::Honors),
            run.hand()
                .iter()
                .filter(|t| TileDebuffClass::Honors.matches(t))
                .count(),
        ),
        (
            TileDebuff::Class(TileDebuffClass::Terminals),
            run.hand()
                .iter()
                .filter(|t| TileDebuffClass::Terminals.matches(t))
                .count(),
        ),
        (
            TileDebuff::Suit(crate::core::tile::Suit::Flower),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Flower)
                .count(),
        ),
    ];
    let chosen = candidates
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(debuff, _)| debuff)
        .unwrap_or(TileDebuff::Class(TileDebuffClass::Honors));
    ResolvedOrdealEffect {
        rule_pushes: vec![],
        tile_debuffs: vec![chosen],
        relic_debuffs: vec![],
        on_apply: None,
        on_play: None,
        description_override: Some(format!("{} tiles are debuffed", chosen.label())),
    }
}

fn counterweight_reveal(run: &mut RunState) -> ResolvedOrdealEffect {
    let mut manzu = 0u32;
    let mut souzu = 0u32;
    let mut pinzu = 0u32;
    let mut honors = 0u32;
    let mut terminals = 0u32;
    let mut flowers = 0u32;

    for &relic in &run.relics.active {
        match relic {
            RelicId::RubySerpent => manzu += 3,
            RelicId::JadeSerpent => souzu += 3,
            RelicId::LapisSerpent => pinzu += 3,
            RelicId::HonorFury
            | RelicId::DragonRage
            | RelicId::GreenLuck
            | RelicId::WhiteDragonsHush
            | RelicId::WildWinds
            | RelicId::DragonEcho
            | RelicId::WindReader => honors += 2,
            RelicId::EdgeRunner | RelicId::ClosedGate => terminals += 2,
            RelicId::GardenKeeper | RelicId::Ikebana | RelicId::Hanami => flowers += 2,
            _ => {}
        }
    }

    let fallback = run
        .hand()
        .iter()
        .fold(
            (TileDebuff::Class(TileDebuffClass::Honors), 0usize),
            |best, tile| {
                let candidate = match tile.suit {
                    crate::core::tile::Suit::Manzu => {
                        TileDebuff::Suit(crate::core::tile::Suit::Manzu)
                    }
                    crate::core::tile::Suit::Souzu => {
                        TileDebuff::Suit(crate::core::tile::Suit::Souzu)
                    }
                    crate::core::tile::Suit::Pinzu => {
                        TileDebuff::Suit(crate::core::tile::Suit::Pinzu)
                    }
                    crate::core::tile::Suit::Flower => {
                        TileDebuff::Suit(crate::core::tile::Suit::Flower)
                    }
                    crate::core::tile::Suit::Wind | crate::core::tile::Suit::Dragon => {
                        TileDebuff::Class(TileDebuffClass::Honors)
                    }
                    crate::core::tile::Suit::Season => TileDebuff::Class(TileDebuffClass::Honors),
                };
                let count = run.hand().iter().filter(|t| candidate.matches(t)).count();
                if count > best.1 {
                    (candidate, count)
                } else {
                    best
                }
            },
        )
        .0;

    let chosen = [
        (TileDebuff::Suit(crate::core::tile::Suit::Manzu), manzu),
        (TileDebuff::Suit(crate::core::tile::Suit::Souzu), souzu),
        (TileDebuff::Suit(crate::core::tile::Suit::Pinzu), pinzu),
        (TileDebuff::Class(TileDebuffClass::Honors), honors),
        (TileDebuff::Class(TileDebuffClass::Terminals), terminals),
        (TileDebuff::Suit(crate::core::tile::Suit::Flower), flowers),
    ]
    .into_iter()
    .max_by_key(|(_, weight)| *weight)
    .and_then(|(debuff, weight)| (weight > 0).then_some(debuff))
    .unwrap_or(fallback);

    ResolvedOrdealEffect {
        rule_pushes: vec![],
        tile_debuffs: vec![chosen],
        relic_debuffs: vec![],
        on_apply: None,
        on_play: None,
        description_override: Some(format!(
            "Countered your relic loadout: {} tiles are debuffed",
            chosen.label()
        )),
    }
}

fn hex_reveal(run: &mut RunState) -> ResolvedOrdealEffect {
    use crate::core::relic::{Rarity, all_relic_defs};

    let target = run
        .relics
        .active
        .iter()
        .enumerate()
        .max_by_key(|(idx, id)| {
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == **id)
                .map(|d| match d.rarity {
                    Rarity::Common => 0,
                    Rarity::Uncommon => 1,
                    Rarity::Rare => 2,
                    Rarity::Legendary => 3,
                })
                .unwrap_or(0);
            (rarity, std::cmp::Reverse(*idx))
        })
        .map(|(_, &id)| id);
    let description_override = target.map(|id| {
        let name = all_relic_defs()
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name)
            .unwrap_or("Unknown Relic");
        format!("{name} is debuffed and disabled this round")
    });
    ResolvedOrdealEffect {
        rule_pushes: vec![],
        tile_debuffs: vec![],
        relic_debuffs: target.into_iter().collect(),
        on_apply: None,
        on_play: None,
        description_override: description_override
            .or_else(|| Some("No relic to hex this round".to_string())),
    }
}

fn relic_hand_size_delta(relics: &RelicState) -> i32 {
    let mut d = 0i32;
    if relics.has(RelicId::BigHands) {
        d += 2;
    }
    if relics.has(RelicId::TinyHands) {
        d -= 2;
    }
    d
}

/// Hand fill target at round start / after refill: mode hand size (tutorial
/// may differ from [`crate::game::run::HAND_SIZE`]), boss wide/shrink bonus,
/// and Big Hands / Tiny Hands. Clamped to a sane minimum.
pub fn effective_hand_size_components(
    mode_hand_size: usize,
    bonus_hand_size: i32,
    relics: &RelicState,
) -> usize {
    let base = mode_hand_size as i32;
    let adjusted = base + bonus_hand_size + relic_hand_size_delta(relics);
    adjusted.max(8) as usize
}

/// Effective hand size for `run` (see [`effective_hand_size_components`]).
pub fn effective_hand_size(run: &RunState) -> usize {
    effective_hand_size_components(run.mode.hand_size, run.ordeal.bonus_hand_size, &run.relics)
}

// ── Boss catalog ─────────────────────────────────────────────────────────
//
// Presentation (name, description, tier, min_ante) is loaded from
// `assets/data/ordeals.json`. Behaviour (rule_pushes, debuffs, hooks)
// stays here and is keyed off `OrdealKind` in `ordeal_behavior`. The two
// halves are zipped together at first access in `all_ordeals` /
// `final_ordeals`.

#[derive(Deserialize)]
struct OrdealPresentationRaw {
    id: OrdealKind,
    name: String,
    description: String,
    tier: OrdealTier,
    #[serde(alias = "min_ante")]
    min_wing: u32,
}

struct OrdealBehavior {
    rule_pushes: &'static [RuleModifier],
    tile_debuffs: &'static [TileDebuff],
    relic_debuffs: &'static [RelicId],
    on_apply: Option<fn(&mut RunState)>,
    on_play: Option<fn(&mut RunState)>,
    on_reveal: Option<fn(&mut RunState) -> ResolvedOrdealEffect>,
}

const NO_RULES: &[RuleModifier] = &[];
const NO_TILE_DEBUFFS: &[TileDebuff] = &[];
const NO_RELIC_DEBUFFS: &[RelicId] = &[];

fn ordeal_behavior(kind: OrdealKind) -> OrdealBehavior {
    use crate::core::tile::Suit;
    use OrdealKind as B;
    let mut b = OrdealBehavior {
        rule_pushes: NO_RULES,
        tile_debuffs: NO_TILE_DEBUFFS,
        relic_debuffs: NO_RELIC_DEBUFFS,
        on_apply: None,
        on_play: None,
        on_reveal: None,
    };
    match kind {
        B::Drought => b.on_apply = Some(drought_apply),
        B::Whisper => b.on_apply = Some(whisper_apply),
        B::Tribute => {
            b.on_apply = Some(tribute_apply);
            b.on_play = Some(tribute_play);
        }
        B::Gate => b.tile_debuffs = &[TileDebuff::Suit(Suit::Manzu)],
        B::Grove => b.tile_debuffs = &[TileDebuff::Suit(Suit::Souzu)],
        B::Coin => b.tile_debuffs = &[TileDebuff::Suit(Suit::Pinzu)],
        B::Rot => b.rule_pushes = &[RuleModifier::NoFlowerWildcards],
        B::Hermit => b.rule_pushes = &[RuleModifier::PairsScoreZero],
        B::Forest => b.rule_pushes = &[RuleModifier::SequencesHalved],
        B::Bureaucrat => b.rule_pushes = &[RuleModifier::MustPlayFive],
        B::Drunkard => b.tile_debuffs = &[TileDebuff::Class(TileDebuffClass::MiddleTiles)],
        B::Ash => b.tile_debuffs = &[TileDebuff::Class(TileDebuffClass::Simples)],
        B::Furnace => b.tile_debuffs = &[TileDebuff::Class(TileDebuffClass::Terminals)],
        B::Relic => b.tile_debuffs = &[TileDebuff::Class(TileDebuffClass::Honors)],
        B::Blight => b.on_reveal = Some(blight_reveal),
        B::Hex => b.on_reveal = Some(hex_reveal),
        B::Famine => b.on_apply = Some(famine_apply),
        B::Tempest => b.on_play = Some(tempest_play),
        B::Censor => b.rule_pushes = &[RuleModifier::CensorRepeats],
        B::Mirror => b.on_reveal = Some(mirror_reveal),
        B::Counterweight => b.on_reveal = Some(counterweight_reveal),
        B::TaxCollector => b.on_reveal = Some(tax_collector_reveal),
        B::Dragon => {} // honorless structures debuffed in scoring_tile_debuffs
        B::House => b.rule_pushes = &[RuleModifier::CashInRequiresNoDiscards],
    }
    b
}

fn load_ordeal_defs() -> Vec<OrdealDef> {
    const PATH: &str = "data/ordeals.json";
    let raw: Vec<OrdealPresentationRaw> = load_json_asset(PATH, "ordeal data");
    raw.into_iter()
        .map(|r| {
            let beh = ordeal_behavior(r.id);
            OrdealDef {
                kind: r.id,
                name: Box::leak(r.name.into_boxed_str()),
                description: Box::leak(r.description.into_boxed_str()),
                tier: r.tier,
                min_wing: r.min_wing,
                effect: OrdealEffect {
                    rule_pushes: beh.rule_pushes,
                    tile_debuffs: beh.tile_debuffs,
                    relic_debuffs: beh.relic_debuffs,
                    on_apply: beh.on_apply,
                    on_play: beh.on_play,
                },
                on_reveal: beh.on_reveal,
            }
        })
        .collect()
}

struct OrdealDefCaches {
    regular: Vec<OrdealDef>,
    final_: Vec<OrdealDef>,
}

static BOSS_DEF_CACHES: OnceLock<OrdealDefCaches> = OnceLock::new();

fn ordeal_def_caches() -> &'static OrdealDefCaches {
    BOSS_DEF_CACHES.get_or_init(|| {
        let all = load_ordeal_defs();
        let (regular, final_): (Vec<_>, Vec<_>) =
            all.into_iter().partition(|d| d.tier != OrdealTier::Final);
        OrdealDefCaches { regular, final_ }
    })
}

/// Non-final bosses (everything in the regular ante pool).
pub fn all_ordeals() -> &'static [OrdealDef] {
    ordeal_def_caches().regular.as_slice()
}

/// Final-tier bosses. Reserved for `FINAL_WING` and never drawn into the
/// regular pool.
pub fn final_ordeals() -> &'static [OrdealDef] {
    ordeal_def_caches().final_.as_slice()
}

/// All non-final bosses, used to seed the per-run pool.
pub fn regular_pool() -> Vec<OrdealKind> {
    all_ordeals().iter().map(|d| d.kind).collect()
}

/// Pick a random boss for `ante` from `pool`, removing it. Returns the
/// chosen boss, or `None` if the pool is empty after filtering.
///
/// Selection rule: only bosses with `min_ante <= ante` are eligible. If no
/// boss in the remaining pool qualifies (player got unlucky on draws), we
/// widen by ignoring `min_ante` rather than crashing — soft bosses on a late
/// ante are still better than no boss at all.
///
/// `min_wing_floor`: subtracted from each boss's `min_ante` (saturating) so
/// higher stakes can see harder bosses earlier. Use `0` for the Spring default.
pub fn pick_for_wing_with_floor(
    pool: &mut Vec<OrdealKind>,
    ante: u32,
    min_wing_floor: u32,
    rng: &mut impl rand::Rng,
) -> Option<OrdealKind> {
    if pool.is_empty() {
        return None;
    }
    let mut eligible: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, k)| k.def().min_wing.saturating_sub(min_wing_floor) <= ante)
        .map(|(i, _)| i)
        .collect();
    if eligible.is_empty() {
        eligible = (0..pool.len()).collect();
    }
    let pick_idx = eligible[rng.random_range(0..eligible.len())];
    Some(pool.swap_remove(pick_idx))
}

/// Pick a final boss for the final ante. Currently unconditional uniform
/// pick from `final_ordeals()` — separated from the main pool so soft
/// bosses can never appear on the climactic fight.
pub fn pick_final(rng: &mut impl rand::Rng) -> OrdealKind {
    let pool = final_ordeals();
    let idx = rng.random_range(0..pool.len());
    pool[idx].kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};
    use crate::game::game_mode::GameMode;

    fn custom_tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn the_rot_pushes_no_flower_wildcards_rule() {
        let beh = ordeal_behavior(OrdealKind::Rot);
        assert_eq!(beh.rule_pushes, &[RuleModifier::NoFlowerWildcards]);
        assert!(beh.tile_debuffs.is_empty());
    }

    #[test]
    fn the_rot_is_in_regular_pool() {
        assert!(regular_pool().contains(&OrdealKind::Rot));
    }

    #[test]
    fn blight_can_choose_flower_debuffs() {
        let mut run = RunState::new(GameMode::standard());
        *run.hand_mut() = vec![
            custom_tile(Suit::Flower, 1, 1),
            custom_tile(Suit::Flower, 2, 2),
            custom_tile(Suit::Flower, 3, 3),
            custom_tile(Suit::Manzu, 1, 4),
            custom_tile(Suit::Souzu, 2, 5),
        ];
        *run.selected_mut() = vec![false; run.hand().len()];

        let effect = blight_reveal(&mut run);

        assert_eq!(effect.tile_debuffs, vec![TileDebuff::Suit(Suit::Flower)]);
    }

    #[test]
    fn counterweight_targets_relic_supported_family() {
        let mut run = RunState::new(GameMode::standard());
        run.relics.active = vec![RelicId::JadeSerpent, RelicId::GardenKeeper];
        *run.hand_mut() = vec![
            custom_tile(Suit::Manzu, 1, 1),
            custom_tile(Suit::Manzu, 2, 2),
            custom_tile(Suit::Souzu, 3, 3),
            custom_tile(Suit::Souzu, 4, 4),
            custom_tile(Suit::Flower, 1, 5),
        ];
        *run.selected_mut() = vec![false; run.hand().len()];

        let effect = counterweight_reveal(&mut run);

        assert_eq!(effect.tile_debuffs, vec![TileDebuff::Suit(Suit::Souzu)]);
        assert_eq!(
            effect.description_override.as_deref(),
            Some("Countered your relic loadout: Souzu tiles are debuffed")
        );
    }

    /// Every OrdealKind variant must appear in `assets/data/ordeals.json` and
    /// have a `ordeal_behavior` arm. The classify match below is exhaustive,
    /// so a new variant won't compile until it's listed here; a missing
    /// JSON entry trips `def()`.
    #[test]
    fn every_boss_variant_has_one_data_entry() {
        // Force-compile-error if a new variant is added without classifying it.
        for &kind in OrdealKind::ALL {
            #[allow(unused)]
            match kind {
                OrdealKind::Drought
                | OrdealKind::Whisper
                | OrdealKind::Tribute
                | OrdealKind::Gate
                | OrdealKind::Grove
                | OrdealKind::Coin
                | OrdealKind::Rot
                | OrdealKind::Hermit
                | OrdealKind::Forest
                | OrdealKind::Bureaucrat
                | OrdealKind::Drunkard
                | OrdealKind::Ash
                | OrdealKind::Furnace
                | OrdealKind::Relic
                | OrdealKind::Blight
                | OrdealKind::Hex
                | OrdealKind::Famine
                | OrdealKind::Tempest
                | OrdealKind::Censor
                | OrdealKind::Mirror
                | OrdealKind::Counterweight
                | OrdealKind::TaxCollector
                | OrdealKind::Dragon
                | OrdealKind::House => {}
            }
            // Both presentation lookup and behaviour lookup must succeed.
            let _ = kind.def();
            let _ = ordeal_behavior(kind);
        }
        let count = all_ordeals().len() + final_ordeals().len();
        assert_eq!(
            count,
            OrdealKind::ALL.len(),
            "ordeals.json count ({count}) does not match OrdealKind variant count ({})",
            OrdealKind::ALL.len()
        );
    }
}
