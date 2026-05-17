//! Boss blinds — Balatro-style themed encounters with distinct effects.
//!
//! Each boss is a `BossKind` variant with a `BossDef` describing how to
//! apply it. Presentation (name, description, tier, min_ante) lives in
//! `assets/data/bosses.json` and is loaded once at startup; behaviour
//! (rule_pushes, debuffs, on_apply / on_play / on_reveal hooks) stays in
//! Rust where it can use function pointers freely. Final-tier bosses are
//! kept in a separate pool — they only appear on `FINAL_ANTE` and are
//! never drawn by the regular roller.
//!
//! Effects dispatch through three hooks:
//! * `rule_pushes` — `RuleModifier`s injected into `round_rules`, picked up by
//!   `score_sets` and `validate_selection_with_rules` like any other rule.
//! * `on_apply` — called from `apply_blind` when the boss blind starts. Used
//!   for one-shot mutations to `RunState` (zero discards, target bumps,
//!   shrunken hand, etc.).
//! * `on_play` — called from `commit_selection_to_structure` after a successful play,
//!   for per-play taxers (gold cost, wall burn).
//!
//! Adding a new boss is purely a matter of appending to `bosses.json`
//! (presentation), adding a `BossKind` variant, and supplying the right
//! hook in `boss_behavior` — no other file needs to know the boss exists.

use std::sync::OnceLock;

use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::core::debuff::{TileDebuff, TileDebuffClass};
use crate::core::json_asset::load_json_asset;
use crate::core::relic::{RelicId, RelicState};
use crate::core::rules::RuleModifier;
use crate::game::run::RunState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BossKind {
    // ── Soft / early bosses ──────────────────────────────────────────────
    Drought,
    Whisper,
    Tribute,
    Gate,
    Grove,
    Coin,
    Bloom,
    // ── Medium (min_ante 3) ──────────────────────────────────────────────
    Hermit,
    Forest,
    Bureaucrat,
    Drunkard,
    Ash,
    Furnace,
    Relic,
    Blight,
    Hex,
    // ── Hard (min_ante 5) ────────────────────────────────────────────────
    Famine,
    Tempest,
    Censor,
    // ── Reactive (min_ante 3) ────────────────────────────────────────────
    // These bosses pick their rule at reveal time based on RunState. The
    // chosen variant is locked in immediately and displayed on the boss card
    // — no mid-blind goalpost-moving. See `on_reveal` on `BossDef`.
    Mirror,
    Counterweight,
    TaxCollector,
    // ── Final (final ante only) ──────────────────────────────────────────
    Dragon,
    House,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BossTier {
    Soft,
    Medium,
    Hard,
    Final,
}

/// Static definition of a boss: presentation + effect dispatch.
pub struct BossDef {
    pub kind: BossKind,
    pub name: &'static str,
    pub description: &'static str,
    pub tier: BossTier,
    /// Earliest ante on which this boss may be drawn. Final-tier bosses
    /// ignore this and only appear on `FINAL_ANTE`.
    pub min_ante: u32,
    pub effect: BossEffect,
    /// Reactive bosses use this hook to compute their final effect at the
    /// moment the ante reveals (i.e. inside `advance_round`/`RunState::new`).
    /// The returned `ResolvedBossEffect` is locked in for the whole ante and
    /// shown verbatim in pick_blind — the player sees the *resolved* rule
    /// before they ever set foot in the boss blind. Static bosses leave this
    /// `None` and get a wrapped copy of `effect`.
    ///
    /// Takes `&mut RunState` so reactive bosses can also stash auxiliary
    /// scratch data (e.g. The Tax Collector writes the chosen per-play cost
    /// to `RunState::tax_collector_cost`) — the actual scoring/resource
    /// effects still happen in `on_apply` / `on_play` during the boss blind,
    /// but the *parameters* of those effects are baked in here.
    pub on_reveal: Option<fn(&mut RunState) -> ResolvedBossEffect>,
}

/// Three-axis effect descriptor. Each field is independently optional —
/// most bosses use only one axis.
pub struct BossEffect {
    /// Rule modifiers pushed into `round_rules` when the boss applies. The
    /// scoring/validation paths read these the same way they read starting
    /// rules, so adding category A/B effects is purely a data change.
    pub rule_pushes: &'static [RuleModifier],
    pub tile_debuffs: &'static [TileDebuff],
    pub relic_debuffs: &'static [RelicId],
    /// One-shot mutation when the boss applies (start of round). `None` if
    /// the boss is purely rule-based.
    pub on_apply: Option<fn(&mut RunState)>,
    /// Per-play hook fired from `commit_selection_to_structure` after a successful
    /// commit (post-refill). Used for taxers and wall burn.
    pub on_play: Option<fn(&mut RunState)>,
}

/// Owned, ante-scoped sibling of `BossEffect`. Static bosses get one of these
/// built from their static `BossEffect` at reveal time; reactive bosses build
/// their own from scratch via `BossDef::on_reveal`. Stored on `RunState` and
/// read by `apply_blind` / `commit_selection_to_structure` instead of the static def
/// so reactive variants land at the right moment.
#[derive(Clone, Debug)]
pub struct ResolvedBossEffect {
    pub rule_pushes: Vec<RuleModifier>,
    pub tile_debuffs: Vec<TileDebuff>,
    pub relic_debuffs: Vec<RelicId>,
    pub on_apply: Option<fn(&mut RunState)>,
    pub on_play: Option<fn(&mut RunState)>,
    /// Replaces `BossDef::description` in the UI when present. Reactive
    /// bosses use this to report the *chosen* variant ("Pay 4 gold each
    /// play") rather than the generic static text.
    pub description_override: Option<String>,
}

impl ResolvedBossEffect {
    /// Wrap a static def's effect verbatim. Used by every non-reactive boss.
    pub fn from_static(eff: &BossEffect) -> Self {
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

impl BossKind {
    pub fn def(self) -> &'static BossDef {
        all_bosses()
            .iter()
            .chain(final_bosses().iter())
            .find(|d| d.kind == self)
            .expect("every BossKind must have a definition")
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn tier(self) -> BossTier {
        self.def().tier
    }
}

impl BossTier {
    /// RGBA tint used to colour-code boss cards by severity. Soft is the
    /// neutral indigo of regular blinds; Medium/Hard/Final escalate through
    /// gold → amber → ruby so the player can read tier at a glance.
    pub fn halo_color(self) -> [f32; 4] {
        use crate::render::theme::color;
        match self {
            BossTier::Soft => [0.55, 0.65, 0.85, 1.0], // muted indigo
            BossTier::Medium => color::GOLD,
            BossTier::Hard => color::AMBER,
            BossTier::Final => color::RUBY,
        }
    }
}

// ── Effect helpers ───────────────────────────────────────────────────────
//
// These are top-level fns (not closures) so they can be used as `fn` pointers
// in the static `BossDef` table — `Fn` trait objects can't sit in a const.

fn drought_apply(run: &mut RunState) {
    // Halve discards (round down). At default 4 starting discards this gives 2,
    // a meaningful tax without removing the lever entirely.
    run.discards_remaining /= 2;
}

fn whisper_apply(run: &mut RunState) {
    // Shrink the hand by 1 for the whole round. The bonus_hand_size delta is
    // honored by `refill_hand` and the `commit_selection_to_structure` draw target.
    run.boss.bonus_hand_size -= 1;
    let target = effective_hand_size(run);
    crate::game::engine_state::GameplayCoreState::with_run_mut(run, |core| {
        while core.hand.len() > target {
            core.hand.pop();
        }
        core.selected = vec![false; core.hand.len()];
    });
}

fn tribute_apply(run: &mut RunState) {
    run.boss.gold_cost_per_play = 1;
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
    let cost = run.boss.gold_cost_per_play as i32;
    run.apply_gold_delta(-cost, None);
}

// ── Reactive boss hooks ───────────────────────────────────────────────────
//
// `on_reveal` runs at the ante boundary (inside `RunState::resolve_upcoming_boss`)
// — well before `apply_blind`. It receives an immutable view of the run and
// returns a `ResolvedBossEffect` whose hooks/rule_pushes/description are
// frozen for the rest of the ante. The player sees the resolved description
// in pick_blind from the moment the ante starts.

/// Mirror — silences whichever scoring axis the player has invested most
/// relic support in (pair / sequence / triplet). Reuses the existing rule
/// modifiers from Hermit, Forest, and Censor — no new scoring math needed.
/// Ties favor pairs (most universal yaku component); a player with no
/// axis-leaning relics also gets PairsScoreZero as the default sting.
fn mirror_reveal(run: &mut RunState) -> ResolvedBossEffect {
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
    ResolvedBossEffect {
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
fn tax_collector_reveal(run: &mut RunState) -> ResolvedBossEffect {
    let cost = (run.gold.max(0) as u32 / 10).clamp(2, 8);
    run.boss.tax_collector_cost = cost;
    ResolvedBossEffect {
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
    // Tribute's path: set gold_cost_per_play and let `tribute_play` drain it.
    run.boss.gold_cost_per_play = run.boss.tax_collector_cost;
}

fn blight_reveal(run: &mut RunState) -> ResolvedBossEffect {
    let candidates = [
        (
            TileDebuff::Suit(crate::core::tile::Suit::Characters),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Characters)
                .count(),
        ),
        (
            TileDebuff::Suit(crate::core::tile::Suit::Bamboos),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Bamboos)
                .count(),
        ),
        (
            TileDebuff::Suit(crate::core::tile::Suit::Dots),
            run.hand()
                .iter()
                .filter(|t| t.suit == crate::core::tile::Suit::Dots)
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
    ResolvedBossEffect {
        rule_pushes: vec![],
        tile_debuffs: vec![chosen],
        relic_debuffs: vec![],
        on_apply: None,
        on_play: None,
        description_override: Some(format!("{} tiles are debuffed", chosen.label())),
    }
}

fn counterweight_reveal(run: &mut RunState) -> ResolvedBossEffect {
    let mut characters = 0u32;
    let mut bamboos = 0u32;
    let mut dots = 0u32;
    let mut honors = 0u32;
    let mut terminals = 0u32;
    let mut flowers = 0u32;

    for &relic in &run.relics.active {
        match relic {
            RelicId::RubySerpent => characters += 3,
            RelicId::JadeSerpent => bamboos += 3,
            RelicId::LapisSerpent => dots += 3,
            RelicId::HonorFury
            | RelicId::DragonRage
            | RelicId::GreenLuck
            | RelicId::WhiteDragonsHush
            | RelicId::WildWinds
            | RelicId::DragonEcho
            | RelicId::RoundCompass => honors += 2,
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
                    crate::core::tile::Suit::Characters => {
                        TileDebuff::Suit(crate::core::tile::Suit::Characters)
                    }
                    crate::core::tile::Suit::Bamboos => {
                        TileDebuff::Suit(crate::core::tile::Suit::Bamboos)
                    }
                    crate::core::tile::Suit::Dots => {
                        TileDebuff::Suit(crate::core::tile::Suit::Dots)
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
        (
            TileDebuff::Suit(crate::core::tile::Suit::Characters),
            characters,
        ),
        (TileDebuff::Suit(crate::core::tile::Suit::Bamboos), bamboos),
        (TileDebuff::Suit(crate::core::tile::Suit::Dots), dots),
        (TileDebuff::Class(TileDebuffClass::Honors), honors),
        (TileDebuff::Class(TileDebuffClass::Terminals), terminals),
        (TileDebuff::Suit(crate::core::tile::Suit::Flower), flowers),
    ]
    .into_iter()
    .max_by_key(|(_, weight)| *weight)
    .and_then(|(debuff, weight)| (weight > 0).then_some(debuff))
    .unwrap_or(fallback);

    ResolvedBossEffect {
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

fn hex_reveal(run: &mut RunState) -> ResolvedBossEffect {
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
    ResolvedBossEffect {
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
    effective_hand_size_components(run.mode.hand_size, run.boss.bonus_hand_size, &run.relics)
}

// ── Boss catalog ─────────────────────────────────────────────────────────
//
// Presentation (name, description, tier, min_ante) is loaded from
// `assets/data/bosses.json`. Behaviour (rule_pushes, debuffs, hooks)
// stays here and is keyed off `BossKind` in `boss_behavior`. The two
// halves are zipped together at first access in `all_bosses` /
// `final_bosses`.

#[derive(Deserialize)]
struct BossPresentationRaw {
    id: BossKind,
    name: String,
    description: String,
    tier: BossTier,
    min_ante: u32,
}

struct BossBehavior {
    rule_pushes: &'static [RuleModifier],
    tile_debuffs: &'static [TileDebuff],
    relic_debuffs: &'static [RelicId],
    on_apply: Option<fn(&mut RunState)>,
    on_play: Option<fn(&mut RunState)>,
    on_reveal: Option<fn(&mut RunState) -> ResolvedBossEffect>,
}

const NO_RULES: &[RuleModifier] = &[];
const NO_TILE_DEBUFFS: &[TileDebuff] = &[];
const NO_RELIC_DEBUFFS: &[RelicId] = &[];

fn boss_behavior(kind: BossKind) -> BossBehavior {
    use crate::core::tile::Suit;
    use BossKind as B;
    let mut b = BossBehavior {
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
        B::Gate => b.tile_debuffs = &[TileDebuff::Suit(Suit::Characters)],
        B::Grove => b.tile_debuffs = &[TileDebuff::Suit(Suit::Bamboos)],
        B::Coin => b.tile_debuffs = &[TileDebuff::Suit(Suit::Dots)],
        B::Bloom => b.tile_debuffs = &[TileDebuff::Suit(Suit::Flower)],
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

fn load_boss_defs() -> Vec<BossDef> {
    const PATH: &str = "data/bosses.json";
    let raw: Vec<BossPresentationRaw> = load_json_asset(PATH, "boss data");
    raw.into_iter()
        .map(|r| {
            let beh = boss_behavior(r.id);
            BossDef {
                kind: r.id,
                name: Box::leak(r.name.into_boxed_str()),
                description: Box::leak(r.description.into_boxed_str()),
                tier: r.tier,
                min_ante: r.min_ante,
                effect: BossEffect {
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

struct BossDefCaches {
    regular: Vec<BossDef>,
    final_: Vec<BossDef>,
}

static BOSS_DEF_CACHES: OnceLock<BossDefCaches> = OnceLock::new();

fn boss_def_caches() -> &'static BossDefCaches {
    BOSS_DEF_CACHES.get_or_init(|| {
        let all = load_boss_defs();
        let (regular, final_): (Vec<_>, Vec<_>) =
            all.into_iter().partition(|d| d.tier != BossTier::Final);
        BossDefCaches { regular, final_ }
    })
}

/// Non-final bosses (everything in the regular ante pool).
pub fn all_bosses() -> &'static [BossDef] {
    boss_def_caches().regular.as_slice()
}

/// Final-tier bosses. Reserved for `FINAL_ANTE` and never drawn into the
/// regular pool.
pub fn final_bosses() -> &'static [BossDef] {
    boss_def_caches().final_.as_slice()
}

/// All non-final bosses, used to seed the per-run pool.
pub fn regular_pool() -> Vec<BossKind> {
    all_bosses().iter().map(|d| d.kind).collect()
}

/// Pick a random boss for `ante` from `pool`, removing it. Returns the
/// chosen boss, or `None` if the pool is empty after filtering.
///
/// Selection rule: only bosses with `min_ante <= ante` are eligible. If no
/// boss in the remaining pool qualifies (player got unlucky on draws), we
/// widen by ignoring `min_ante` rather than crashing — soft bosses on a late
/// ante are still better than no boss at all.
///
/// `min_ante_floor`: subtracted from each boss's `min_ante` (saturating) so
/// higher stakes can see harder bosses earlier. Use `0` for the Spring default.
pub fn pick_for_ante_with_floor(
    pool: &mut Vec<BossKind>,
    ante: u32,
    min_ante_floor: u32,
    rng: &mut impl rand::Rng,
) -> Option<BossKind> {
    if pool.is_empty() {
        return None;
    }
    let mut eligible: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, k)| k.def().min_ante.saturating_sub(min_ante_floor) <= ante)
        .map(|(i, _)| i)
        .collect();
    if eligible.is_empty() {
        eligible = (0..pool.len()).collect();
    }
    let pick_idx = eligible[rng.random_range(0..eligible.len())];
    Some(pool.swap_remove(pick_idx))
}

/// Pick a final boss for the final ante. Currently unconditional uniform
/// pick from `final_bosses()` — separated from the main pool so soft
/// bosses can never appear on the climactic fight.
pub fn pick_final(rng: &mut impl rand::Rng) -> BossKind {
    let pool = final_bosses();
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
    fn static_debuff_bosses_are_in_regular_pool() {
        let pool = regular_pool();
        for boss in [
            BossKind::Gate,
            BossKind::Grove,
            BossKind::Coin,
            BossKind::Bloom,
            BossKind::Ash,
            BossKind::Furnace,
            BossKind::Relic,
            BossKind::Counterweight,
        ] {
            assert!(pool.contains(&boss), "{boss:?} should be rollable");
        }
    }

    #[test]
    fn blight_can_choose_flower_debuffs() {
        let mut run = RunState::new(GameMode::standard());
        *run.hand_mut() = vec![
            custom_tile(Suit::Flower, 1, 1),
            custom_tile(Suit::Flower, 2, 2),
            custom_tile(Suit::Flower, 3, 3),
            custom_tile(Suit::Characters, 1, 4),
            custom_tile(Suit::Bamboos, 2, 5),
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
            custom_tile(Suit::Characters, 1, 1),
            custom_tile(Suit::Characters, 2, 2),
            custom_tile(Suit::Bamboos, 3, 3),
            custom_tile(Suit::Bamboos, 4, 4),
            custom_tile(Suit::Flower, 1, 5),
        ];
        *run.selected_mut() = vec![false; run.hand().len()];

        let effect = counterweight_reveal(&mut run);

        assert_eq!(effect.tile_debuffs, vec![TileDebuff::Suit(Suit::Bamboos)]);
        assert_eq!(
            effect.description_override.as_deref(),
            Some("Countered your relic loadout: Bamboos tiles are debuffed")
        );
    }

    /// Every BossKind variant must appear in `assets/data/bosses.json` and
    /// have a `boss_behavior` arm. The classify match below is exhaustive,
    /// so a new variant won't compile until it's listed here; a missing
    /// JSON entry trips `def()`.
    #[test]
    fn every_boss_variant_has_one_data_entry() {
        const ALL: &[BossKind] = &[
            BossKind::Drought,
            BossKind::Whisper,
            BossKind::Tribute,
            BossKind::Gate,
            BossKind::Grove,
            BossKind::Coin,
            BossKind::Bloom,
            BossKind::Hermit,
            BossKind::Forest,
            BossKind::Bureaucrat,
            BossKind::Drunkard,
            BossKind::Ash,
            BossKind::Furnace,
            BossKind::Relic,
            BossKind::Blight,
            BossKind::Hex,
            BossKind::Famine,
            BossKind::Tempest,
            BossKind::Censor,
            BossKind::Mirror,
            BossKind::Counterweight,
            BossKind::TaxCollector,
            BossKind::Dragon,
            BossKind::House,
        ];
        // Force-compile-error if a new variant is added without classifying it.
        for &kind in ALL {
            #[allow(unused)]
            match kind {
                BossKind::Drought
                | BossKind::Whisper
                | BossKind::Tribute
                | BossKind::Gate
                | BossKind::Grove
                | BossKind::Coin
                | BossKind::Bloom
                | BossKind::Hermit
                | BossKind::Forest
                | BossKind::Bureaucrat
                | BossKind::Drunkard
                | BossKind::Ash
                | BossKind::Furnace
                | BossKind::Relic
                | BossKind::Blight
                | BossKind::Hex
                | BossKind::Famine
                | BossKind::Tempest
                | BossKind::Censor
                | BossKind::Mirror
                | BossKind::Counterweight
                | BossKind::TaxCollector
                | BossKind::Dragon
                | BossKind::House => {}
            }
            // Both presentation lookup and behaviour lookup must succeed.
            let _ = kind.def();
            let _ = boss_behavior(kind);
        }
        let count = all_bosses().len() + final_bosses().len();
        assert_eq!(
            count,
            ALL.len(),
            "bosses.json count ({count}) does not match BossKind variant count ({})",
            ALL.len()
        );
    }
}
