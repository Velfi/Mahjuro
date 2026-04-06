//! Score hands from detected sets + relics + rules.

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::relic::{
    RelicId, ScoreContext, pair_bonus_points, sequence_multiplier, suit_tile_bonus,
    triplet_multiplier,
};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

/// One step in the scoring cascade — each relic/rule contribution is a separate step.
#[derive(Clone, Debug)]
pub struct ScoreStep {
    /// Human-readable source, e.g. "Triplet Boost" or "Pair Double (rule)".
    pub source: String,
    /// Short description of effect, e.g. "×2", "+6".
    pub effect: String,
    /// Running total after this step.
    pub running_total: i32,
}

/// Rich scoring breakdown for cascade animations.
#[derive(Clone, Debug)]
pub struct ScoreBreakdown {
    /// Points before any relics or rules.
    pub base_points: i32,
    /// Each relic/rule that fired, in order.
    pub steps: Vec<ScoreStep>,
    /// Final total.
    pub total: i32,
}

fn tile_by_id<'a>(tiles: &'a [Tile], id: u32) -> Option<&'a Tile> {
    tiles.iter().find(|t| t.id == id)
}

fn set_base_points(kind: SetKind) -> i32 {
    match kind {
        SetKind::Pair => 10,
        SetKind::Triplet => 40,
        SetKind::Sequence => 30,
    }
}

/// Score detected sets and return a rich breakdown for cascade display.
pub fn score_sets(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> ScoreBreakdown {
    let mut steps = Vec::new();
    let pair_double = rules.contains(&RuleModifier::PairDoubleScore);

    // Accumulate base points for all sets first.
    let mut base_points = 0i32;
    for s in sets {
        base_points += set_base_points(s.kind);
    }
    let mut total = base_points;

    // Now apply relics/rules per set, tracking each contribution.
    for s in sets {
        let set_base = set_base_points(s.kind);
        let mut set_pts = set_base;

        if matches!(s.kind, SetKind::Pair) {
            let bonus = pair_bonus_points(ctx);
            if bonus > 0 {
                let source = if ctx.relics.has(RelicId::PairPower) && ctx.relics.has(RelicId::WhiteSilence) {
                    "Pair Power + White Silence"
                } else if ctx.relics.has(RelicId::PairPower) {
                    "Pair Power"
                } else {
                    "White Silence"
                };
                total += bonus;
                steps.push(ScoreStep {
                    source: source.to_string(),
                    effect: format!("+{bonus}"),
                    running_total: total,
                });
                set_pts += bonus;
            }
            if pair_double {
                // Double the pair's contribution (set_pts is what we had, we add another set_pts).
                let extra = set_pts; // doubling = add another copy
                total += extra;
                steps.push(ScoreStep {
                    source: "Pair Double (rule)".to_string(),
                    effect: format!("×2 (+{extra})"),
                    running_total: total,
                });
            }
        }

        if matches!(s.kind, SetKind::Triplet) {
            let m = triplet_multiplier(ctx);
            if m > 1.0 {
                let before = set_base;
                let after = (set_base as f64 * m).round() as i32;
                let diff = after - before;
                total += diff;
                let mut sources = Vec::new();
                if ctx.relics.has(RelicId::TripletBoost) {
                    sources.push("Triplet Boost");
                }
                if ctx.relics.has(RelicId::MultiplierMaster) {
                    sources.push("Multiplier Master");
                }
                steps.push(ScoreStep {
                    source: sources.join(" + "),
                    effect: format!("×{m:.1} (+{diff})"),
                    running_total: total,
                });
                set_pts = after;
            }
            if let Some(first) = s.tile_ids.first().and_then(|id| tile_by_id(tiles, *id)) {
                if first.suit == Suit::Dragon
                    && first.rank == 1
                    && ctx.relics.has(RelicId::RedDragonRage)
                {
                    // ×5 on current set_pts — add the difference.
                    let extra = set_pts * 4; // ×5 total means +4× more
                    total += extra;
                    steps.push(ScoreStep {
                        source: "Red Dragon Rage".to_string(),
                        effect: format!("×5 (+{extra})"),
                        running_total: total,
                    });
                }
                if matches!(first.suit, Suit::Wind | Suit::Dragon)
                    && ctx.relics.has(RelicId::HonorFury)
                {
                    let bonus = 3 * s.tile_ids.len() as i32;
                    total += bonus;
                    steps.push(ScoreStep {
                        source: "Honor Fury".to_string(),
                        effect: format!("+{bonus}"),
                        running_total: total,
                    });
                }
                let suit_bonus = suit_tile_bonus(first.suit, ctx) * s.tile_ids.len() as i32;
                if suit_bonus > 0 {
                    total += suit_bonus;
                    steps.push(ScoreStep {
                        source: "Bamboo Charm".to_string(),
                        effect: format!("+{suit_bonus}"),
                        running_total: total,
                    });
                }
            }
        }

        if matches!(s.kind, SetKind::Sequence) {
            let m = sequence_multiplier(ctx);
            if m > 1.0 {
                let before = set_base;
                let after = (set_base as f64 * m).round() as i32;
                let diff = after - before;
                total += diff;
                let mut sources = Vec::new();
                if ctx.relics.has(RelicId::SequenceSurge) {
                    sources.push("Sequence Surge");
                }
                if ctx.relics.has(RelicId::MultiplierMaster) {
                    sources.push("Multiplier Master");
                }
                steps.push(ScoreStep {
                    source: sources.join(" + "),
                    effect: format!("×{m:.1} (+{diff})"),
                    running_total: total,
                });
            }
            if let Some(first) = s.tile_ids.first().and_then(|id| tile_by_id(tiles, *id)) {
                let suit_bonus = suit_tile_bonus(first.suit, ctx) * 3;
                if suit_bonus > 0 {
                    total += suit_bonus;
                    steps.push(ScoreStep {
                        source: "Bamboo Charm".to_string(),
                        effect: format!("+{suit_bonus}"),
                        running_total: total,
                    });
                }
            }
        }
    }

    ScoreBreakdown {
        base_points,
        steps,
        total,
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

    #[test]
    fn score_triplet_relic() {
        let hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Bamboos, 3, 1),
            Tile::new(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let relics = RelicState {
            active: vec![RelicId::TripletBoost],
        };
        let ctx = ScoreContext { relics: &relics };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        assert_eq!(breakdown.total, 80);
        assert_eq!(breakdown.base_points, 40);
        assert!(!breakdown.steps.is_empty(), "should have relic steps");
    }

    #[test]
    fn pair_double_rule() {
        let hand = vec![
            Tile::new(Suit::Circles, 5, 0),
            Tile::new(Suit::Circles, 5, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let ctx = ScoreContext {
            relics: &RelicState::default(),
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[RuleModifier::PairDoubleScore]);
        assert_eq!(breakdown.total, 20);
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Pair Double")));
    }
}
