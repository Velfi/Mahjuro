//! Score hands from detected sets + relics + rules.

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::relic::{
    RelicId, ScoreContext, pair_bonus_points, sequence_multiplier, suit_tile_bonus,
    triplet_multiplier,
};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::{YakuKind, detect_yaku};

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
    /// Yaku patterns detected in this hand.
    pub detected_yaku: Vec<YakuKind>,
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
    let honor_triple = rules.contains(&RuleModifier::HonorTripleScore);
    let no_seq_bonus = rules.contains(&RuleModifier::NoSequenceBonus);

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
            // LuckyPair: pairs score ×1.5.
            if ctx.relics.has(RelicId::LuckyPair) {
                let extra = (set_pts as f64 * 0.5).round() as i32;
                total += extra;
                steps.push(ScoreStep {
                    source: "Lucky Pair".to_string(),
                    effect: format!("×1.5 (+{extra})"),
                    running_total: total,
                });
                set_pts += extra;
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
                // HonorTripleScore rule: honor triplets score ×3.
                if matches!(first.suit, Suit::Wind | Suit::Dragon) && honor_triple {
                    let extra = set_pts * 2; // ×3 total = +2× more
                    total += extra;
                    steps.push(ScoreStep {
                        source: "Honor Triple (rule)".to_string(),
                        effect: format!("×3 (+{extra})"),
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

    // DragonEcho: dragon triplets add 50% of adjacent sets' base points.
    if ctx.relics.has(RelicId::DragonEcho) {
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
            let mut echo_bonus = 0i32;
            if i > 0 {
                echo_bonus += set_base_points(sets[i - 1].kind) / 2;
            }
            if i + 1 < sets.len() {
                echo_bonus += set_base_points(sets[i + 1].kind) / 2;
            }
            if echo_bonus > 0 {
                total += echo_bonus;
                steps.push(ScoreStep {
                    source: "Dragon Echo".to_string(),
                    effect: format!("+{echo_bonus}"),
                    running_total: total,
                });
            }
        }
    }

    // ChainReaction: +25% if scored last turn.
    if ctx.relics.has(RelicId::ChainReaction) && ctx.scored_last_turn {
        let bonus = (total as f64 * 0.25).round() as i32;
        total += bonus;
        steps.push(ScoreStep {
            source: "Chain Reaction".to_string(),
            effect: format!("+25% (+{bonus})"),
            running_total: total,
        });
    }

    // Dora bonus: +30 per tile matching a dora face.
    if !ctx.dora_faces.is_empty() {
        let dora_count = tiles
            .iter()
            .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
            .count() as i32;
        if dora_count > 0 {
            let bonus = dora_count * 30;
            total += bonus;
            steps.push(ScoreStep {
                source: format!("Dora ×{dora_count}"),
                effect: format!("+{bonus}"),
                running_total: total,
            });
        }
    }

    // NoSequenceBonus rule: if no sequences in the hand, +80 bonus.
    if no_seq_bonus && !sets.iter().any(|s| s.kind == SetKind::Sequence) {
        total += 80;
        steps.push(ScoreStep {
            source: "No-Seq Bonus (rule)".to_string(),
            effect: "+80".to_string(),
            running_total: total,
        });
    }

    // Yaku bonus phase — detect hand patterns and award bonuses (filtered by progression).
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
        let bonus = yaku.bonus_points();
        total += bonus;
        steps.push(ScoreStep {
            source: yaku.name().to_string(),
            effect: format!("+{bonus}"),
            running_total: total,
        });
    }

    ScoreBreakdown {
        base_points,
        steps,
        detected_yaku,
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
            ..Default::default()
        };
        let ctx = ScoreContext { relics: &relics, scored_last_turn: false, dora_faces: vec![], available_yaku: vec![] };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        // base 40 + TripletBoost(+40) + AllTriplets(+100) + AllSimples(+60) + Flush(+120) = 360
        assert_eq!(breakdown.base_points, 40);
        assert_eq!(breakdown.total, 360);
        assert!(!breakdown.steps.is_empty(), "should have relic steps");
        assert!(breakdown.detected_yaku.contains(&YakuKind::AllTriplets));
        assert!(breakdown.detected_yaku.contains(&YakuKind::Flush));
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
            scored_last_turn: false,
            dora_faces: vec![],
            available_yaku: vec![],
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[RuleModifier::PairDoubleScore]);
        // base 10 + PairDouble(+10) + AllSimples(+60) + Flush(+120) = 200
        assert_eq!(breakdown.total, 200);
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Pair Double")));
        assert!(breakdown.detected_yaku.contains(&YakuKind::Flush));
    }

    fn ctx_with(relics: &RelicState, scored_last_turn: bool) -> ScoreContext<'_> {
        ScoreContext {
            relics,
            scored_last_turn,
            dora_faces: vec![],
            available_yaku: vec![],
        }
    }

    fn relics(ids: Vec<RelicId>) -> RelicState {
        RelicState { active: ids, ..Default::default() }
    }

    // ── SequenceSurge ──────────────────────────────────────────────

    #[test]
    fn sequence_surge_multiplies_sequence() {
        // 1-2-3m sequence
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
        // base 30, ×1.5 = 45, + yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Sequence Surge")));
        // The base is 30, the multiplier adds 15
        assert!(breakdown.total >= 45);
    }

    // ── PairPower ──────────────────────────────────────────────────

    #[test]
    fn pair_power_adds_bonus() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::PairPower]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 10 + PairPower(+10) = 20, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Pair Power")));
        assert!(breakdown.total >= 20);
    }

    // ── LuckyPair ──────────────────────────────────────────────────

    #[test]
    fn lucky_pair_multiplies_pair() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::LuckyPair]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 10, ×1.5 = 15, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Lucky Pair")));
        assert!(breakdown.total >= 15);
    }

    #[test]
    fn pair_power_and_lucky_pair_stack() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::PairPower, RelicId::LuckyPair]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 10 + PairPower(+10) = 20, then LuckyPair ×1.5 → 20 + 10 = 30
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Pair Power")));
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Lucky Pair")));
        assert!(breakdown.total >= 30);
    }

    // ── WhiteSilence ───────────────────────────────────────────────

    #[test]
    fn white_silence_bonus_on_pair() {
        let hand = vec![
            Tile::new(Suit::Dragon, 3, 0), // White dragon
            Tile::new(Suit::Dragon, 3, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::WhiteSilence]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 10 + WhiteSilence(+5) = 15, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("White Silence")));
        assert!(breakdown.total >= 15);
    }

    // ── HonorFury ──────────────────────────────────────────────────

    #[test]
    fn honor_fury_on_wind_triplet() {
        let hand = vec![
            Tile::new(Suit::Wind, 1, 0), // East ×3
            Tile::new(Suit::Wind, 1, 1),
            Tile::new(Suit::Wind, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::HonorFury]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 40 + HonorFury(+9) = 49, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Honor Fury")));
        assert!(breakdown.total >= 49);
    }

    // ── RedDragonRage ──────────────────────────────────────────────

    #[test]
    fn red_dragon_rage_on_red_triplet() {
        let hand = vec![
            Tile::new(Suit::Dragon, 1, 0), // Red dragon ×3
            Tile::new(Suit::Dragon, 1, 1),
            Tile::new(Suit::Dragon, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 40, ×5 = 200, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Red Dragon Rage")));
        assert!(breakdown.total >= 200);
    }

    #[test]
    fn red_dragon_rage_ignores_green_dragon() {
        let hand = vec![
            Tile::new(Suit::Dragon, 2, 0), // Green dragon ×3
            Tile::new(Suit::Dragon, 2, 1),
            Tile::new(Suit::Dragon, 2, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(!breakdown.steps.iter().any(|s| s.source.contains("Red Dragon Rage")));
    }

    // ── BambooCharm ────────────────────────────────────────────────

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
        // base 40 + BambooCharm(+6) = 46, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Bamboo Charm")));
        assert!(breakdown.total >= 46);
    }

    #[test]
    fn bamboo_charm_on_bamboo_sequence() {
        let hand = vec![
            Tile::new(Suit::Bamboos, 4, 0),
            Tile::new(Suit::Bamboos, 5, 1),
            Tile::new(Suit::Bamboos, 6, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        }];
        let r = relics(vec![RelicId::BambooCharm]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // base 30 + BambooCharm(+6) = 36, plus yaku
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Bamboo Charm")));
        assert!(breakdown.total >= 36);
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
        assert!(!breakdown.steps.iter().any(|s| s.source.contains("Bamboo Charm")));
    }

    // ── MultiplierMaster ───────────────────────────────────────────

    #[test]
    fn multiplier_master_scales_with_relic_count() {
        let hand = vec![
            Tile::new(Suit::Characters, 9, 0),
            Tile::new(Suit::Characters, 9, 1),
            Tile::new(Suit::Characters, 9, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        // MultiplierMaster + 2 others = 3 relics → ×1.3
        let r = relics(vec![RelicId::MultiplierMaster, RelicId::PairPower, RelicId::BambooCharm]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Multiplier Master")));
        // base 40, ×1.3 = 52 → diff of 12
        assert!(breakdown.total >= 52);
    }

    // ── DragonEcho ─────────────────────────────────────────────────

    #[test]
    fn dragon_echo_adds_adjacent_bonus() {
        // Sequence | Dragon triplet | Sequence
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Dragon, 1, 3), // Red dragon triplet
            Tile::new(Suit::Dragon, 1, 4),
            Tile::new(Suit::Dragon, 1, 5),
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
        ];
        let sets = vec![
            DetectedSet { kind: SetKind::Sequence, tile_ids: vec![0, 1, 2] },
            DetectedSet { kind: SetKind::Triplet, tile_ids: vec![3, 4, 5] },
            DetectedSet { kind: SetKind::Sequence, tile_ids: vec![6, 7, 8] },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 50% of adjacent: seq(30)/2 + seq(30)/2 = 30
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Dragon Echo")));
        let echo_step = breakdown.steps.iter().find(|s| s.source == "Dragon Echo").unwrap();
        assert_eq!(echo_step.effect, "+30");
    }

    #[test]
    fn dragon_echo_ignores_non_dragon_triplet() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Wind, 1, 3), // Wind triplet, not dragon
            Tile::new(Suit::Wind, 1, 4),
            Tile::new(Suit::Wind, 1, 5),
        ];
        let sets = vec![
            DetectedSet { kind: SetKind::Sequence, tile_ids: vec![0, 1, 2] },
            DetectedSet { kind: SetKind::Triplet, tile_ids: vec![3, 4, 5] },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(!breakdown.steps.iter().any(|s| s.source.contains("Dragon Echo")));
    }

    // ── ChainReaction ──────────────────────────────────────────────

    #[test]
    fn chain_reaction_when_scored_last_turn() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::ChainReaction]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, true), &[]);
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Chain Reaction")));
        // Should add 25% of total-before-chain
        let chain_step = breakdown.steps.iter().find(|s| s.source == "Chain Reaction").unwrap();
        assert!(chain_step.effect.contains("+25%"));
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
        assert!(!breakdown.steps.iter().any(|s| s.source.contains("Chain Reaction")));
    }

    // ── Dora bonus ─────────────────────────────────────────────────

    #[test]
    fn dora_bonus_per_matching_tile() {
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
        // 3 tiles match dora → +90
        assert!(breakdown.steps.iter().any(|s| s.source.contains("Dora")));
        let dora_step = breakdown.steps.iter().find(|s| s.source.contains("Dora")).unwrap();
        assert_eq!(dora_step.effect, "+90");
    }
}
