//! Boss blinds — Balatro-style themed encounters with distinct effects.
//!
//! Each boss is a `BossKind` variant with a static `BossDef` containing its
//! name, description, `min_ante` (the earliest ante on which it may appear),
//! and a `BossEffect` describing how to apply it. Final-tier bosses are kept
//! in a separate pool — they only appear on `FINAL_ANTE` and are never drawn
//! by the regular roller.
//!
//! Effects dispatch through three hooks:
//! * `rule_pushes` — `RuleModifier`s injected into `round_rules`, picked up by
//!   `score_sets` and `validate_selection_with_rules` like any other rule.
//! * `on_apply` — called from `apply_blind` when the boss blind starts. Used
//!   for one-shot mutations to `RunState` (zero discards, target bumps,
//!   shrunken hand, etc.).
//! * `on_play` — called from `score_selected_tiles` after a successful play,
//!   for per-play taxers (gold cost, wall burn).
//!
//! Adding a new boss is purely a matter of appending to `ALL_BOSSES` (or
//! `FINAL_BOSSES`) and supplying the right hook closures — no other file
//! needs to know the boss exists.

use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::game::run::RunState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BossKind {
    // ── Soft (min_ante 1) ────────────────────────────────────────────────
    Drought,
    Whisper,
    Veil,
    Tribute,
    // ── Medium (min_ante 3) ──────────────────────────────────────────────
    Hermit,
    Forest,
    Bureaucrat,
    Drunkard,
    // ── Hard (min_ante 5) ────────────────────────────────────────────────
    Famine,
    Tempest,
    Censor,
    // ── Reactive (min_ante 3) ────────────────────────────────────────────
    // These bosses pick their rule at reveal time based on RunState. The
    // chosen variant is locked in immediately and displayed on the boss card
    // — no mid-blind goalpost-moving. See `on_reveal` on `BossDef`.
    Mirror,
    TaxCollector,
    // ── Final (ante 8 only) ──────────────────────────────────────────────
    Dragon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// One-shot mutation when the boss applies (start of round). `None` if
    /// the boss is purely rule-based.
    pub on_apply: Option<fn(&mut RunState)>,
    /// Per-play hook fired from `score_selected_tiles` after a successful
    /// play (post-scoring, post-refill). Used for taxers and wall burn.
    pub on_play: Option<fn(&mut RunState)>,
}

/// Owned, ante-scoped sibling of `BossEffect`. Static bosses get one of these
/// built from their static `BossEffect` at reveal time; reactive bosses build
/// their own from scratch via `BossDef::on_reveal`. Stored on `RunState` and
/// read by `apply_blind` / `score_selected_tiles` instead of the static def
/// so reactive variants land at the right moment.
#[derive(Clone, Debug)]
pub struct ResolvedBossEffect {
    pub rule_pushes: Vec<RuleModifier>,
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
            on_apply: eff.on_apply,
            on_play: eff.on_play,
            description_override: None,
        }
    }
}

impl BossKind {
    pub fn def(self) -> &'static BossDef {
        ALL_BOSSES
            .iter()
            .chain(FINAL_BOSSES.iter())
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
        match self {
            BossTier::Soft => [0.55, 0.65, 0.85, 1.0], // muted indigo
            BossTier::Medium => [0.91, 0.69, 0.29, 1.0], // GOLD
            BossTier::Hard => [0.94, 0.66, 0.28, 1.0], // AMBER
            BossTier::Final => [0.91, 0.35, 0.42, 1.0], // RUBY
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BossTier::Soft => "Soft",
            BossTier::Medium => "Medium",
            BossTier::Hard => "Hard",
            BossTier::Final => "Final",
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
    // honored by `refill_hand` and the `score_selected_tiles` draw target.
    run.boss.bonus_hand_size -= 1;
    while run.hand.len() > effective_hand_size(run) {
        run.hand.pop();
    }
    run.selected = vec![false; run.hand.len()];
}

fn veil_apply(run: &mut RunState) {
    // Burn half of current gold. Pure economy tax — doesn't touch scoring or
    // resources, so it's the "softest" of the soft bosses. A poor player feels
    // it less than a rich one, which we like (Balatro's Tooth/Wheel pattern).
    run.gold /= 2;
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
    run.gold -= run.boss.gold_cost_per_play as i32;
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
        on_apply: Some(tax_collector_apply),
        on_play: Some(tribute_play),
        description_override: Some(format!("Pay {cost} gold each play (locked at reveal)")),
    }
}

fn tax_collector_apply(run: &mut RunState) {
    // The cost was stashed on RunState by `tax_collector_reveal`. Mirror
    // Tribute's path: set gold_cost_per_play and let `tribute_play` drain it.
    run.boss.gold_cost_per_play = run.boss.tax_collector_cost;
}

/// Effective hand size after applying any per-round bonus_hand_size delta.
/// Clamped to a sane minimum so a stacked debuff can't reduce the hand to 0.
pub fn effective_hand_size(run: &RunState) -> usize {
    let base = crate::game::run::HAND_SIZE as i32;
    let adjusted = base + run.boss.bonus_hand_size;
    adjusted.max(8) as usize
}

// ── Static catalog ───────────────────────────────────────────────────────

pub static ALL_BOSSES: &[BossDef] = &[
    BossDef {
        kind: BossKind::Drought,
        name: "The Drought",
        description: "Start with half discards",
        tier: BossTier::Soft,
        min_ante: 1,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: Some(drought_apply),
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Whisper,
        name: "The Whisper",
        description: "Hand size −1",
        tier: BossTier::Soft,
        min_ante: 1,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: Some(whisper_apply),
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Veil,
        name: "The Veil",
        description: "Lose half your gold",
        tier: BossTier::Soft,
        min_ante: 1,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: Some(veil_apply),
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Tribute,
        name: "The Tribute",
        description: "Pay 1 gold each play",
        tier: BossTier::Soft,
        min_ante: 1,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: Some(tribute_apply),
            on_play: Some(tribute_play),
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Hermit,
        name: "The Hermit",
        description: "Pairs score 0",
        tier: BossTier::Medium,
        min_ante: 3,
        effect: BossEffect {
            rule_pushes: &[RuleModifier::PairsScoreZero],
            on_apply: None,
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Forest,
        name: "The Forest",
        description: "Sequences score half base chips",
        tier: BossTier::Medium,
        min_ante: 3,
        effect: BossEffect {
            rule_pushes: &[RuleModifier::SequencesHalved],
            on_apply: None,
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Bureaucrat,
        name: "The Bureaucrat",
        description: "Must play exactly 5 tiles",
        tier: BossTier::Medium,
        min_ante: 3,
        effect: BossEffect {
            rule_pushes: &[RuleModifier::MustPlayFive],
            on_apply: None,
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Drunkard,
        name: "The Drunkard",
        description: "Rank-5 tiles score 0",
        tier: BossTier::Medium,
        min_ante: 3,
        effect: BossEffect {
            rule_pushes: &[RuleModifier::MiddleTilesZero],
            on_apply: None,
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Famine,
        name: "The Famine",
        description: "Target doubled",
        tier: BossTier::Hard,
        min_ante: 5,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: Some(famine_apply),
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Tempest,
        name: "The Tempest",
        description: "Wall burns 1 tile after each play",
        tier: BossTier::Hard,
        min_ante: 5,
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: None,
            on_play: Some(tempest_play),
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Censor,
        name: "The Censor",
        description: "Repeated yaku score at half",
        tier: BossTier::Hard,
        min_ante: 5,
        effect: BossEffect {
            rule_pushes: &[RuleModifier::CensorRepeats],
            on_apply: None,
            on_play: None,
        },
        on_reveal: None,
    },
    BossDef {
        kind: BossKind::Mirror,
        name: "The Mirror",
        description: "Silences your strongest scoring axis",
        tier: BossTier::Medium,
        min_ante: 3,
        // The static effect is a no-op fallback. `on_reveal` always replaces
        // it with a `ResolvedBossEffect` that pushes the chosen RuleModifier
        // and sets a description telling the player exactly which axis was
        // silenced — locked in at ante reveal so it can't move mid-fight.
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: None,
            on_play: None,
        },
        on_reveal: Some(mirror_reveal),
    },
    BossDef {
        kind: BossKind::TaxCollector,
        name: "The Tax Collector",
        description: "Per-play gold cost scales to your hoard",
        tier: BossTier::Medium,
        min_ante: 3,
        // Static fallback again — `on_reveal` computes the actual per-play
        // cost from `run.gold` at reveal time and stashes it on
        // `RunState::tax_collector_cost` for `tax_collector_apply` to read.
        effect: BossEffect {
            rule_pushes: &[],
            on_apply: None,
            on_play: None,
        },
        on_reveal: Some(tax_collector_reveal),
    },
];

pub static FINAL_BOSSES: &[BossDef] = &[BossDef {
    kind: BossKind::Dragon,
    name: "The Dragon",
    description: "Every play must contain a Wind or Dragon tile",
    tier: BossTier::Final,
    min_ante: 8,
    effect: BossEffect {
        rule_pushes: &[RuleModifier::RequireHonor],
        on_apply: None,
        on_play: None,
    },
    on_reveal: None,
}];

/// All non-final bosses, used to seed the per-run pool.
pub fn regular_pool() -> Vec<BossKind> {
    ALL_BOSSES.iter().map(|d| d.kind).collect()
}

/// Pick a random boss for `ante` from `pool`, removing it. Returns the
/// chosen boss, or `None` if the pool is empty after filtering.
///
/// Selection rule: only bosses with `min_ante <= ante` are eligible. If no
/// boss in the remaining pool qualifies (player got unlucky on draws), we
/// widen by ignoring `min_ante` rather than crashing — soft bosses on a late
/// ante are still better than no boss at all.
pub fn pick_for_ante(
    pool: &mut Vec<BossKind>,
    ante: u32,
    rng: &mut impl rand::Rng,
) -> Option<BossKind> {
    if pool.is_empty() {
        return None;
    }
    let mut eligible: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, k)| k.def().min_ante <= ante)
        .map(|(i, _)| i)
        .collect();
    if eligible.is_empty() {
        eligible = (0..pool.len()).collect();
    }
    let pick_idx = eligible[rng.random_range(0..eligible.len())];
    Some(pool.swap_remove(pick_idx))
}

/// Pick a final boss for the final ante. Currently unconditional uniform
/// pick from `FINAL_BOSSES` — separated from the main pool so soft bosses
/// can never appear on the climactic fight.
pub fn pick_final(rng: &mut impl rand::Rng) -> BossKind {
    let idx = rng.random_range(0..FINAL_BOSSES.len());
    FINAL_BOSSES[idx].kind
}
