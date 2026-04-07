//! Score hands as **chips × mult**, Balatro-style.
//!
//! Each scored hand accumulates two parallel running totals:
//!
//! * **Chips** — additive. Tiles contribute their rank value (honors flat 10),
//!   melds add a flat bonus, and chip-flavored relics pile on more.
//! * **Mult** — multiplicative axis, but built additively (`+N mult`) so it
//!   stacks fast and predictably. Yaku and "explosive" relics live here.
//!
//! Final score = `final_chips × final_mult` (floored).
//!
//! The cascade UI walks the `steps` in order, updating both running totals.
//! The last visible beat is the multiplication itself.

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::{YakuKind, detect_yaku};

/// Which axis a cascade step contributes to. The cascade renders chip and
/// mult deltas slightly differently (color, +N vs +Nx), so the variant lets
/// the UI pick a style without parsing `effect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Chips,
    Mult,
    /// The final `chips × mult` multiplication beat.
    Final,
}

/// One step in the scoring cascade.
#[derive(Clone, Debug)]
pub struct ScoreStep {
    /// Human-readable source, e.g. "Triplet Boost".
    pub source: String,
    pub kind: StepKind,
    /// Running chip total *after* this step. Exposed for richer cascade UIs
    /// that want to render the chip and mult counters separately rather than
    /// just the combined `running_total`.
    #[allow(dead_code)]
    pub running_chips: i32,
    /// Running mult value *after* this step. See note on `running_chips`.
    #[allow(dead_code)]
    pub running_mult: f64,
    /// Running combined score (`running_chips × running_mult`, floored).
    /// Cascades that just want a single ticking number read this.
    pub running_total: i32,
}

/// Rich scoring breakdown for cascade animations.
#[derive(Clone, Debug)]
pub struct ScoreBreakdown {
    /// Chips before any relics, yaku, or rules — just tile values + meld bonuses.
    pub base_chips: i32,
    /// Backwards-compatible alias for `base_chips`. Some UI code still reads
    /// `base_points`; keeping the field avoids a wider rename.
    pub base_points: i32,
    /// Each contribution that fired, in cascade order.
    pub steps: Vec<ScoreStep>,
    /// Yaku patterns detected in this hand.
    pub detected_yaku: Vec<YakuKind>,
    /// Final chip total before multiplication. Exposed for UIs that want to
    /// show the two-axis ending separately (e.g. results screen breakdown).
    #[allow(dead_code)]
    pub final_chips: i32,
    /// Final mult value. See note on `final_chips`.
    #[allow(dead_code)]
    pub final_mult: f64,
    /// Final score = `final_chips × final_mult` (floored).
    pub total: i32,
}

// ── Per-meld base bonuses ──────────────────────────────────────────────
//
// These are *flat chip adds* on top of each scored tile's own value. The
// triplet bonus is intentionally larger than 3× the sequence bonus so that
// triplets feel like the "punchy single-beat" play and sequences feel like
// the "wider but flatter" play. Pairs are tiny because pairs almost always
// also pull weight via Pair Power / Lucky Pair / yaku.
fn meld_chip_bonus(kind: SetKind) -> i32 {
    match kind {
        SetKind::Pair => 15,
        SetKind::Sequence => 30,
        SetKind::Triplet => 50,
    }
}

fn tile_by_id<'a>(tiles: &'a [Tile], id: u32) -> Option<&'a Tile> {
    tiles.iter().find(|t| t.id == id)
}

/// Multiply chips by mult, flooring to i32. Negative chips are clamped to 0
/// before multiplication so an aggressive nerf can't underflow into a positive.
fn combine(chips: i32, mult: f64) -> i32 {
    (chips.max(0) as f64 * mult).floor() as i32
}

/// Score detected sets and return a rich breakdown for cascade display.
pub fn score_sets(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> ScoreBreakdown {
    let mut steps: Vec<ScoreStep> = Vec::new();
    // chips starts at base_chips (computed below); mult starts at the
    // identity ×1 so the cascade reads as +N mult / +N mult / ... .
    let mut chips: i32;
    let mut mult: f64 = 1.0;

    let pair_double = rules.contains(&RuleModifier::PairDoubleScore);
    let honor_triple = rules.contains(&RuleModifier::HonorTripleScore);
    let no_seq_bonus = rules.contains(&RuleModifier::NoSequenceBonus);

    // Tiny helpers that mutate `chips`/`mult` in-place and emit a cascade step.
    macro_rules! push_chips {
        ($source:expr, $delta:expr) => {{
            let delta: i32 = $delta;
            chips += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Chips,
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }
    macro_rules! push_mult {
        ($source:expr, $delta:expr) => {{
            let delta: f64 = $delta;
            mult += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Mult,
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }

    // ── Phase 1: base chips (tile values + meld bonuses) ─────────────────
    //
    // These don't get individual cascade steps — they're rolled into the
    // "Base" line the UI shows before the steps start ticking.
    let mut base_chips: i32 = 0;
    for s in sets {
        base_chips += meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                base_chips += t.point_value() as i32;
            }
        }
    }
    chips = base_chips;

    // ── Phase 2: per-set chip relics ─────────────────────────────────────
    //
    // Walk the sets and apply relics that grant flat chip bonuses to specific
    // melds or tile faces. Order matters only for cascade readability.
    let has_triplet_boost = ctx.relics.has(RelicId::TripletBoost);
    let has_sequence_surge = ctx.relics.has(RelicId::SequenceSurge);
    let has_pair_power = ctx.relics.has(RelicId::PairPower);
    let has_honor_fury = ctx.relics.has(RelicId::HonorFury);
    let has_bamboo_charm = ctx.relics.has(RelicId::BambooCharm);

    for s in sets {
        // Per-meld-kind chip relics.
        match s.kind {
            SetKind::Triplet if has_triplet_boost => push_chips!("Triplet Boost", 40),
            SetKind::Sequence if has_sequence_surge => push_chips!("Sequence Surge", 25),
            SetKind::Pair if has_pair_power => push_chips!("Pair Power", 30),
            _ => {}
        }

        // Per-tile chip relics within this set.
        if has_honor_fury {
            let honor_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32;
            if honor_count > 0 {
                push_chips!("Honor Fury", 15 * honor_count);
            }
        }
        if has_bamboo_charm {
            let bamboo_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| t.suit == Suit::Bamboos)
                .count() as i32;
            if bamboo_count > 0 {
                push_chips!("Bamboo Charm", 4 * bamboo_count);
            }
        }
    }

    // PairDoubleScore rule: every pair gets +30 chips on top of its base.
    // (Replaces the old "double the pair's contribution" — easier to balance
    // when the chip pile is larger.)
    if pair_double {
        let pair_count = sets.iter().filter(|s| s.kind == SetKind::Pair).count() as i32;
        if pair_count > 0 {
            push_chips!("Pair Double (rule)", 30 * pair_count);
        }
    }

    // ── Phase 3: cross-set chip relics ───────────────────────────────────

    // DragonEcho: each dragon triplet adds the *base chips* of its adjacent
    // sets (tile values + meld bonus) to the chip pile. The original semantic
    // was "copy adjacent base points"; we keep that, just measured in chips.
    if ctx.relics.has(RelicId::DragonEcho) {
        // Pre-compute each set's base chip contribution so we can index it
        // without recomputing inside the loop.
        let set_bases: Vec<i32> = sets
            .iter()
            .map(|s| {
                let mut c = meld_chip_bonus(s.kind);
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        c += t.point_value() as i32;
                    }
                }
                c
            })
            .collect();

        for (i, s) in sets.iter().enumerate() {
            if s.kind != SetKind::Triplet {
                continue;
            }
            let is_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon);
            if !is_dragon {
                continue;
            }
            let mut echo = 0i32;
            if i > 0 {
                echo += set_bases[i - 1];
            }
            if i + 1 < sets.len() {
                echo += set_bases[i + 1];
            }
            if echo > 0 {
                push_chips!("Dragon Echo", echo);
            }
        }
    }

    // Dora: each tile matching a dora face is +20 chips.
    if !ctx.dora_faces.is_empty() {
        let dora_count = tiles
            .iter()
            .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
            .count() as i32;
        if dora_count > 0 {
            // Use a custom step so we can label "Dora ×N" instead of the source name.
            let delta = 20 * dora_count;
            chips += delta;
            steps.push(ScoreStep {
                source: format!("Dora ×{dora_count}"),
                kind: StepKind::Chips,
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }
    }

    // ── Phase 4: yaku → mult ─────────────────────────────────────────────

    let all_yaku = detect_yaku(tiles, sets);
    let detected_yaku: Vec<YakuKind> = if ctx.available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| ctx.available_yaku.contains(y))
            .collect()
    };
    for yaku in &detected_yaku {
        push_mult!(yaku.name(), yaku.mult_bonus());
    }

    // ── Phase 5: per-set mult relics ─────────────────────────────────────

    if ctx.relics.has(RelicId::RedDragonRage) {
        for s in sets {
            if s.kind != SetKind::Triplet {
                continue;
            }
            let is_red_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon && t.rank == 1);
            if is_red_dragon {
                push_mult!("Red Dragon Rage", 10.0);
            }
        }
    }

    if ctx.relics.has(RelicId::WhiteSilence) {
        for s in sets {
            if s.kind != SetKind::Pair {
                continue;
            }
            let is_white_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon && t.rank == 3);
            if is_white_dragon {
                push_mult!("White Silence", 4.0);
            }
        }
    }

    // LuckyPair: any pair in the hand grants +3 mult, once.
    if ctx.relics.has(RelicId::LuckyPair) && sets.iter().any(|s| s.kind == SetKind::Pair) {
        push_mult!("Lucky Pair", 3.0);
    }

    // HonorTripleScore rule: honor triplets each grant +3 mult.
    if honor_triple {
        for s in sets {
            if s.kind != SetKind::Triplet {
                continue;
            }
            let is_honor = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            if is_honor {
                push_mult!("Honor Triple (rule)", 3.0);
            }
        }
    }

    // NoSequenceBonus rule: a hand with no sequences gets +3 mult.
    if no_seq_bonus && !sets.iter().any(|s| s.kind == SetKind::Sequence) {
        push_mult!("No-Seq Bonus (rule)", 3.0);
    }

    // ── Phase 6: global mult relics ──────────────────────────────────────

    if ctx.relics.has(RelicId::MultiplierMaster) {
        // +0.5 mult per relic owned (including itself).
        let bonus = 0.5 * ctx.relics.active.len() as f64;
        if bonus > 0.0 {
            push_mult!("Multiplier Master", bonus);
        }
    }

    if ctx.relics.has(RelicId::ChainReaction) && ctx.scored_last_turn {
        push_mult!("Chain Reaction", 4.0);
    }

    // ── Phase 7: final multiplication beat ───────────────────────────────

    let final_chips = chips;
    let final_mult = mult;
    let total = combine(final_chips, final_mult);
    steps.push(ScoreStep {
        source: format!("{} × {}", final_chips, fmt_mult(final_mult)),
        kind: StepKind::Final,
        running_chips: final_chips,
        running_mult: final_mult,
        running_total: total,
    });

    ScoreBreakdown {
        base_chips,
        base_points: base_chips,
        steps,
        detected_yaku,
        final_chips,
        final_mult,
        total,
    }
}

/// Mystery-preserving "Balatro-style" score preview for the current selection.
///
/// Shows only what the player can derive from the tiles in front of them:
///   * **chips** = sum of tile values + meld bonuses (no relic/dora bonuses)
///   * **mult**  = 1 + Σ yaku.mult_bonus() for visible yaku patterns
///
/// Relic contributions, rule modifiers, dora hits, and chain effects are
/// intentionally excluded so the cascade still has surprises to reveal.
#[derive(Clone, Copy, Debug)]
pub struct ScorePreview {
    pub chips: i32,
    pub mult: f64,
}

pub fn preview_score(
    tiles: &[Tile],
    sets: &[DetectedSet],
    available_yaku: &[YakuKind],
) -> ScorePreview {
    let mut chips: i32 = 0;
    for s in sets {
        chips += meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                chips += t.point_value() as i32;
            }
        }
    }
    let all_yaku = detect_yaku(tiles, sets);
    let visible_yaku: Vec<YakuKind> = if available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| available_yaku.contains(y))
            .collect()
    };
    let mut mult: f64 = 1.0;
    for y in &visible_yaku {
        mult += y.mult_bonus();
    }
    ScorePreview { chips, mult }
}

/// Format an absolute mult value for the final beat display.
fn fmt_mult(m: f64) -> String {
    if (m - m.round()).abs() < 1e-6 {
        format!("×{}", m.round() as i64)
    } else {
        format!("×{m:.1}")
    }
}

/// Convenience: get just the total as i32 (for tests and simple callers).
#[allow(dead_code)]
pub fn score_sets_total(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> i32 {
    score_sets(tiles, sets, ctx, rules).total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hand::find_pairs_and_triplets;
    use crate::core::relic::RelicState;
    use crate::core::tile::Tile;

    fn ctx_with(relics: &RelicState, scored_last_turn: bool) -> ScoreContext<'_> {
        ScoreContext {
            relics,
            scored_last_turn,
            dora_faces: vec![],
            available_yaku: vec![],
        }
    }

    fn relics(ids: Vec<RelicId>) -> RelicState {
        RelicState {
            active: ids,
            ..Default::default()
        }
    }

    // ── Base chips ─────────────────────────────────────────────────

    #[test]
    fn bare_triplet_of_threes() {
        // Triplet of 3s: tile chips = 3+3+3 = 9, meld bonus = 50, total chips = 59,
        // mult = ×1, final = 59. No yaku (only 3 tiles).
        let hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Bamboos, 3, 1),
            Tile::new(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        assert_eq!(breakdown.base_chips, 59);
        assert_eq!(breakdown.final_mult, 1.0);
        assert_eq!(breakdown.total, 59);
    }

    #[test]
    fn honor_triplet_uses_flat_value() {
        // East wind triplet: honors are flat 10 chips each, meld bonus 50 → 80 chips.
        let hand = vec![
            Tile::new(Suit::Wind, 1, 0),
            Tile::new(Suit::Wind, 1, 1),
            Tile::new(Suit::Wind, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        assert_eq!(breakdown.base_chips, 80);
    }

    // ── Triplet Boost ───────────────────────────────────────────────

    #[test]
    fn triplet_boost_adds_chips_to_triplet() {
        let hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Bamboos, 3, 1),
            Tile::new(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::TripletBoost]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 59 base + 40 triplet boost = 99 chips, ×1 mult.
        assert_eq!(breakdown.final_chips, 99);
        assert_eq!(breakdown.total, 99);
        assert!(breakdown.steps.iter().any(|s| s.source == "Triplet Boost"));
    }

    // ── Sequence Surge ──────────────────────────────────────────────

    #[test]
    fn sequence_surge_adds_chips_to_sequence() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        }];
        let r = relics(vec![RelicId::SequenceSurge]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 1+2+3 + 30 (meld) + 25 (surge) = 61
        assert_eq!(breakdown.final_chips, 61);
    }

    // ── Pair Power & Lucky Pair stack ───────────────────────────────

    #[test]
    fn pair_power_and_lucky_pair_stack() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::PairPower, RelicId::LuckyPair]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // chips: 7+7 + 15 (meld) + 30 (PairPower) = 59
        // mult: 1 + 3 (LuckyPair) = 4
        // total: 59 × 4 = 236
        assert_eq!(breakdown.final_chips, 59);
        assert_eq!(breakdown.final_mult, 4.0);
        assert_eq!(breakdown.total, 236);
    }

    // ── White Silence ───────────────────────────────────────────────

    #[test]
    fn white_silence_mults_white_dragon_pair() {
        let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::WhiteSilence]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // chips: 10+10 + 15 = 35, mult: 1 + 4 = 5, total 175
        assert_eq!(breakdown.final_chips, 35);
        assert_eq!(breakdown.final_mult, 5.0);
        assert_eq!(breakdown.total, 175);
    }

    // ── Honor Fury ──────────────────────────────────────────────────

    #[test]
    fn honor_fury_adds_chips_per_honor_tile() {
        let hand = vec![
            Tile::new(Suit::Wind, 1, 0),
            Tile::new(Suit::Wind, 1, 1),
            Tile::new(Suit::Wind, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::HonorFury]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 80 base + 15×3 = 125
        assert_eq!(breakdown.final_chips, 125);
    }

    // ── Red Dragon Rage ─────────────────────────────────────────────

    #[test]
    fn red_dragon_rage_mults_red_triplet() {
        let hand = vec![
            Tile::new(Suit::Dragon, 1, 0),
            Tile::new(Suit::Dragon, 1, 1),
            Tile::new(Suit::Dragon, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // chips: 10+10+10 + 50 = 80, mult: 1 + 10 = 11, total 880
        assert_eq!(breakdown.final_chips, 80);
        assert_eq!(breakdown.final_mult, 11.0);
        assert_eq!(breakdown.total, 880);
    }

    #[test]
    fn red_dragon_rage_ignores_green_dragon() {
        let hand = vec![
            Tile::new(Suit::Dragon, 2, 0),
            Tile::new(Suit::Dragon, 2, 1),
            Tile::new(Suit::Dragon, 2, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert_eq!(breakdown.final_mult, 1.0);
    }

    // ── Bamboo Charm ────────────────────────────────────────────────

    #[test]
    fn bamboo_charm_on_bamboo_triplet() {
        let hand = vec![
            Tile::new(Suit::Bamboos, 5, 0),
            Tile::new(Suit::Bamboos, 5, 1),
            Tile::new(Suit::Bamboos, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::BambooCharm]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 5+5+5 + 50 + 4×3 = 77
        assert_eq!(breakdown.final_chips, 77);
    }

    #[test]
    fn bamboo_charm_ignores_other_suits() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::BambooCharm]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(!breakdown.steps.iter().any(|s| s.source == "Bamboo Charm"));
    }

    // ── Multiplier Master ───────────────────────────────────────────

    #[test]
    fn multiplier_master_scales_with_relic_count() {
        let hand = vec![
            Tile::new(Suit::Characters, 9, 0),
            Tile::new(Suit::Characters, 9, 1),
            Tile::new(Suit::Characters, 9, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![
            RelicId::MultiplierMaster,
            RelicId::PairPower,
            RelicId::BambooCharm,
        ]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 3 relics → +1.5 mult on top of base ×1 → ×2.5
        assert_eq!(breakdown.final_mult, 2.5);
    }

    // ── Dragon Echo ─────────────────────────────────────────────────

    #[test]
    fn dragon_echo_copies_adjacent_set_chips() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Dragon, 1, 3),
            Tile::new(Suit::Dragon, 1, 4),
            Tile::new(Suit::Dragon, 1, 5),
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // Adjacent set chips:
        //   left seq: 1+2+3 + 30 = 36
        //   right seq: 7+8+9 + 30 = 54
        //   echo total = 90
        let echo = breakdown
            .steps
            .iter()
            .find(|s| s.source == "Dragon Echo")
            .unwrap();
        assert_eq!(echo.effect, "+90 chips");
    }

    #[test]
    fn dragon_echo_ignores_non_dragon_triplet() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Wind, 1, 3),
            Tile::new(Suit::Wind, 1, 4),
            Tile::new(Suit::Wind, 1, 5),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(!breakdown.steps.iter().any(|s| s.source == "Dragon Echo"));
    }

    // ── Chain Reaction ──────────────────────────────────────────────

    #[test]
    fn chain_reaction_adds_mult_when_scored_last_turn() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::ChainReaction]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, true), &[]);
        // chips: 5+5+5 + 50 = 65, mult: 1 + 4 = 5, total 325
        assert_eq!(breakdown.final_mult, 5.0);
        assert_eq!(breakdown.total, 325);
    }

    #[test]
    fn chain_reaction_inactive_when_not_scored_last_turn() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::ChainReaction]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert_eq!(breakdown.final_mult, 1.0);
    }

    // ── Pair Double rule ────────────────────────────────────────────

    #[test]
    fn pair_double_rule_adds_chips() {
        let hand = vec![
            Tile::new(Suit::Circles, 5, 0),
            Tile::new(Suit::Circles, 5, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(
            &hand,
            &sets,
            &ctx_with(&RelicState::default(), false),
            &[RuleModifier::PairDoubleScore],
        );
        // chips: 5+5 (tiles) + 15 (pair meld) + 30 (rule) = 55, mult ×1
        assert_eq!(breakdown.final_chips, 55);
        assert_eq!(breakdown.total, 55);
    }

    // ── Dora ────────────────────────────────────────────────────────

    #[test]
    fn dora_chips_per_matching_tile() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = RelicState::default();
        let ctx = ScoreContext {
            relics: &r,
            scored_last_turn: false,
            dora_faces: vec![(Suit::Characters, 5)],
            available_yaku: vec![],
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        // 3 dora tiles × 20 = +60 chips
        let dora = breakdown
            .steps
            .iter()
            .find(|s| s.source.starts_with("Dora"))
            .unwrap();
        assert_eq!(dora.effect, "+60 chips");
    }

    // ── Showpiece: explosive flush full hand ────────────────────────

    #[test]
    fn explosive_flush_full_hand_demonstration() {
        // 14-tile bamboo full hand: 4 melds (3 sequences + 1 triplet) + 1 pair.
        // This is the "this should feel huge" scenario from the design pitch.
        let hand = vec![
            // Sequence 1-2-3
            Tile::new(Suit::Bamboos, 1, 0),
            Tile::new(Suit::Bamboos, 2, 1),
            Tile::new(Suit::Bamboos, 3, 2),
            // Sequence 4-5-6
            Tile::new(Suit::Bamboos, 4, 3),
            Tile::new(Suit::Bamboos, 5, 4),
            Tile::new(Suit::Bamboos, 6, 5),
            // Sequence 7-8-9
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
            // Triplet of 5s
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 5, 10),
            Tile::new(Suit::Bamboos, 5, 11),
            // Pair of 7s
            Tile::new(Suit::Bamboos, 7, 12),
            Tile::new(Suit::Bamboos, 7, 13),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        // base chips: tiles (1..9 once + 5+5+5 + 7+7) = 45 + 15 + 14 = 74
        //           + meld bonuses: 30+30+30+50+15 = 155
        //           = 229
        // mult: ×1 + Flush(+4) + FullHand(+5) = ×10
        // total: 229 × 10 = 2290
        assert_eq!(breakdown.base_chips, 229);
        assert_eq!(breakdown.final_mult, 10.0);
        assert_eq!(breakdown.total, 2290);
    }
}
