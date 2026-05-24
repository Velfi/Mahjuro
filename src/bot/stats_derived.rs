//! Build export `derived` views from [`AggregateStats`] (schema v3+).

use super::export_schema::{
    AggregateMaps, AggregateSums, AvgTurnsClearRow, BossBlindChartRow, BotAggregateV2,
    BotIssuesDerived, BotReportDerived, DeathAnteHazardRow, DeathAnteRow, DistributionCandleRow,
    KpiTile, LossBreakdownDerived, MapTable, NamedCount, NamedCountPct, NamedPerRun,
    PerRunAverages, RelicBuyRow, RelicShopTimingRow, RelicWinRateRow, SurplusSlotRow, WilsonCiPct,
    YakuDerived, YakuRow,
};
use super::reporting::human_readable_score;
use super::stats::{
    AggregateStats, MIN_SHOP_TIMING_SPLIT_PER_BUCKET, RELIC_SHOP_TIMING_EARLY_ANTE_MAX, RunStats,
    aggregate_stats_slot_sort_key,
};
use super::stats_wilson::wilson_95_pct;
use crate::core::blind_target::{TARGET_SCALING, score_for};
use crate::core::relic::{Rarity, all_relic_defs};
use crate::core::rules::BlindKind;
use crate::core::yaku::YakuKind;

const MIN_SAMPLES_FOR_WIN_CORR: u32 = 20;

fn relic_rarity_slug(display_name: &str) -> Option<String> {
    all_relic_defs()
        .iter()
        .find(|d| d.name == display_name)
        .map(|d| {
            match d.rarity {
                Rarity::Common => "common",
                Rarity::Uncommon => "uncommon",
                Rarity::Rare => "rare",
                Rarity::Legendary => "legendary",
            }
            .to_string()
        })
}

fn top_string_u32(m: &std::collections::BTreeMap<String, u32>, n: usize) -> Vec<(String, u32)> {
    let mut v: Vec<_> = m.iter().map(|(a, b)| (a.clone(), *b)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

pub fn aggregate_to_v2(a: &AggregateStats) -> BotAggregateV2 {
    BotAggregateV2 {
        sums: AggregateSums {
            runs: a.runs,
            blinds_cleared_total: a.blinds_cleared_total,
            antes_cleared_total: a.antes_cleared_total,
            victories: a.victories,
            max_ante_reached: a.max_ante_reached,
            total_score: a.total_score,
            total_plays: a.total_plays,
            total_discards: a.total_discards,
            total_strategic_discards: a.total_strategic_discards,
            total_blinds_skipped: a.total_blinds_skipped,
            total_relics_bought: a.total_relics_bought,
            total_gold_spent: a.total_gold_spent,
            total_final_gold: a.total_final_gold,
            total_gold_from_clears: a.total_gold_from_clears,
            total_gold_from_clear_base: a.total_gold_from_clear_base,
            total_gold_from_unused_plays: a.total_gold_from_unused_plays,
            total_gold_from_interest: a.total_gold_from_interest,
            total_gold_from_clear_relics: a.total_gold_from_clear_relics,
            total_gold_from_skip_tags: a.total_gold_from_skip_tags,
            total_skip_tag_gold_value: a.total_skip_tag_gold_value,
            total_target_score: a.total_target_score,
            total_overscore: a.total_overscore,
            peak_blind_score: a.peak_blind_score,
            peak_blind_detail: a.peak_blind_detail.clone(),
            total_bot_issue_no_valid_hand: a.total_bot_issue_no_valid_hand,
            total_bot_issue_only_valid_unplayable: a.total_bot_issue_only_valid_unplayable,
            total_bot_issue_only_valid_no_score: a.total_bot_issue_only_valid_no_score,
            total_bot_issue_other_stuck: a.total_bot_issue_other_stuck,
            total_bot_issue_lost_with_available_lines: a.total_bot_issue_lost_with_available_lines,
            total_structure_triggers: a.total_structure_triggers,
            total_structure_trigger_points: a.total_structure_trigger_points,
            total_second_wind_forfeits: a.total_second_wind_forfeits,
            deaths_out_of_plays: a.deaths_out_of_plays,
            deaths_no_actions_remaining: a.deaths_no_actions_remaining,
            total_gold_clear_green_luck: a.total_gold_clear_green_luck,
            total_gold_clear_gold_idol: a.total_gold_clear_gold_idol,
            total_gold_clear_jade_abacus: a.total_gold_clear_jade_abacus,
            total_gold_clear_patience: a.total_gold_clear_patience,
            total_turns: a.total_turns,
            sum_peak_hand_size: a.sum_peak_hand_size,
            total_tiles_destroyed: a.total_tiles_destroyed,
            timed_out_runs: a.timed_out_runs,
        },
        maps: AggregateMaps {
            bot_issues_by_reason: a.bot_issues_by_reason.clone(),
            deaths_by_ante: a
                .deaths_by_ante
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            deaths_by_blind: a
                .deaths_by_blind
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            deaths_by_ante_cause: a.deaths_by_ante_cause.clone(),
            skipped_tags: a
                .skipped_tags
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked: a
                .relics_picked
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked_victories: a
                .relics_picked_victories
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked_shop_early: a
                .relics_picked_shop_early
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked_shop_early_victories: a
                .relics_picked_shop_early_victories
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked_shop_late: a
                .relics_picked_shop_late
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relics_picked_shop_late_victories: a
                .relics_picked_shop_late_victories
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            talismans_picked: a
                .talismans_picked
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            zodiacs_picked: a
                .zodiacs_picked
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            packs_picked: a
                .packs_picked
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            bot_issues_by_blind: a.bot_issues_by_blind.clone(),
            bot_issues_by_boss: a.bot_issues_by_boss.clone(),
            overscore_by_slot: a.overscore_by_slot.clone(),
            cleared_by_slot: a.cleared_by_slot.clone(),
            turns_by_blind_slot: a.turns_by_blind_slot.clone(),
            turns_cleared_by_slot: a.turns_cleared_by_slot.clone(),
            discards_by_blind_slot: a.discards_by_blind_slot.clone(),
            boss_faced: a.boss_faced.clone(),
            boss_beaten: a.boss_beaten.clone(),
            yaku_scored: a
                .yaku_scored
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            zodiacs_used: a
                .total_zodiacs_used
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            talismans_used: a
                .total_talismans_used
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            relic_activations: a
                .relic_activations
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            transformations_successor: a
                .transformations_successor
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
        },
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted[0];
    }
    let idx = p * (n - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let t = idx - lo as f64;
        sorted[lo].mul_add(1.0 - t, sorted[hi] * t)
    }
}

fn distribution_candle(samples: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let low = v[0];
    let high = v[v.len() - 1];
    let open = percentile(&v, 0.25);
    let close = percentile(&v, 0.75);
    Some((open, high, low, close))
}

fn slot_label_compact(slot: &str) -> String {
    let (ante_str, rest) = slot.split_once('-').unwrap_or((slot, ""));
    let ante = ante_str.parse::<u32>().unwrap_or(0);
    let blind = match rest.trim() {
        "Small Blind" => "S",
        "Big Blind" => "B",
        "Boss Blind" => "X",
        other => {
            if other.is_empty() {
                "?"
            } else {
                return format!("A{ante} {other}");
            }
        }
    };
    format!("A{ante} {blind}")
}

fn build_surplus_candles(runs: &[RunStats]) -> Vec<DistributionCandleRow> {
    if runs.is_empty() {
        return Vec::new();
    }
    let mut slots: Vec<String> = runs
        .iter()
        .flat_map(|s| s.cleared_by_slot.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    slots.sort_by_key(|a| aggregate_stats_slot_sort_key(a));

    let mut rows = Vec::new();
    for slot in slots {
        let samples: Vec<f64> = runs
            .iter()
            .filter_map(|s| {
                let clears = *s.cleared_by_slot.get(&slot)?;
                if clears == 0 {
                    return None;
                }
                let over = *s.overscore_by_slot.get(&slot).unwrap_or(&0);
                Some(over as f64 / clears as f64)
            })
            .collect();
        let Some((open, high, low, close)) = distribution_candle(&samples) else {
            continue;
        };
        rows.push(DistributionCandleRow {
            label: slot_label_compact(&slot),
            n: samples.len() as u32,
            open,
            high,
            low,
            close,
            target: None,
        });
    }
    rows
}

fn build_boss_score_candles(runs: &[RunStats], base_target: u32) -> Vec<DistributionCandleRow> {
    if runs.is_empty() {
        return Vec::new();
    }
    let max_ante = runs
        .iter()
        .flat_map(|s| s.boss_attempts_by_ante.keys().copied())
        .chain(runs.iter().map(|s| s.died_on_ante))
        .max()
        .unwrap_or(0);
    if max_ante == 0 {
        return Vec::new();
    }

    let mut rows = Vec::new();
    for ante in 1..=max_ante {
        let samples: Vec<f64> = runs
            .iter()
            .filter_map(|s| {
                let attempts = *s.boss_attempts_by_ante.get(&ante)?;
                if attempts == 0 {
                    return None;
                }
                let score = *s.boss_score_by_ante.get(&ante).unwrap_or(&0);
                Some(score as f64 / attempts as f64)
            })
            .collect();
        let Some((open, high, low, close)) = distribution_candle(&samples) else {
            continue;
        };
        rows.push(DistributionCandleRow {
            label: format!("Ante {ante}"),
            n: samples.len() as u32,
            open,
            high,
            low,
            close,
            target: Some(score_for(ante, BlindKind::Boss, base_target) as f64),
        });
    }
    rows
}

fn build_boss_blind_chart(a: &AggregateStats, base_target: u32) -> Vec<BossBlindChartRow> {
    let max_ante = a
        .max_ante_reached
        .max(a.boss_attempts_by_ante.keys().max().copied().unwrap_or(0))
        .max(a.deaths_by_ante.keys().max().copied().unwrap_or(0));
    if max_ante == 0 {
        return Vec::new();
    }

    let mut scratch: Vec<(u32, u32, f64, u32)> = Vec::new();
    let mut scale_max = 1u64;
    for ante in 1..=max_ante {
        let target = score_for(ante, BlindKind::Boss, base_target);
        let attempts = *a.boss_attempts_by_ante.get(&ante).unwrap_or(&0);
        let avg_score = if attempts > 0 {
            *a.boss_score_by_ante.get(&ante).unwrap_or(&0) as f64 / attempts as f64
        } else {
            0.0
        };
        scale_max = scale_max.max(target as u64).max(avg_score.round() as u64);
        scratch.push((ante, target, avg_score, attempts));
    }

    let scale = scale_max.max(1) as f64;
    scratch
        .into_iter()
        .map(|(ante, target, avg_score, attempts)| BossBlindChartRow {
            ante,
            target,
            avg_score,
            attempts,
            target_bar_pct: (target as f64 / scale * 100.0).min(100.0),
            avg_bar_pct: (avg_score / scale * 100.0).min(100.0),
        })
        .collect()
}

pub fn derived_from_batch(
    a: &AggregateStats,
    runs: &[RunStats],
    yaku_kind_count: usize,
    base_target: u32,
) -> BotReportDerived {
    if a.runs == 0 {
        return BotReportDerived::default();
    }
    let r = a.runs as f64;
    let rn = a.runs.max(1);
    let score_to_target = if a.total_target_score == 0 {
        0.0
    } else {
        a.total_score as f64 / a.total_target_score as f64
    };
    let relic_act: u64 = a.relic_activations.values().sum();

    let overall_win_rate_wilson_95 =
        wilson_95_pct(a.victories as u64, rn as u64).map(|(lo, hi)| WilsonCiPct { lo, hi });

    let per_run = PerRunAverages {
        win_rate_pct: a.victories as f64 * 100.0 / r,
        blinds_cleared: a.blinds_cleared_total as f64 / r,
        antes_cleared: a.antes_cleared_total as f64 / r,
        total_score: a.total_score as f64 / r,
        plays: a.total_plays as f64 / r,
        discards: a.total_discards as f64 / r,
        strategic_discards: a.total_strategic_discards as f64 / r,
        random_discards: (a.total_discards.saturating_sub(a.total_strategic_discards)) as f64 / r,
        blinds_skipped: a.total_blinds_skipped as f64 / r,
        relics_bought: a.total_relics_bought as f64 / r,
        gold_spent: a.total_gold_spent as f64 / r,
        gold_from_clears: a.total_gold_from_clears as f64 / r,
        gold_per_blind_cleared: if a.blinds_cleared_total == 0 {
            0.0
        } else {
            a.total_gold_from_clears as f64 / a.blinds_cleared_total as f64
        },
        gold_per_blind_incl_skips: if a.blinds_cleared_total == 0 {
            0.0
        } else {
            (a.total_gold_from_clears + a.total_gold_from_skip_tags) as f64
                / a.blinds_cleared_total as f64
        },
        gold_from_skip_tags: a.total_gold_from_skip_tags as f64 / r,
        skip_tag_gold_value: a.total_skip_tag_gold_value as f64 / r,
        gold_clear_base: a.total_gold_from_clear_base as f64 / r,
        gold_clear_unused_plays: a.total_gold_from_unused_plays as f64 / r,
        gold_clear_interest: a.total_gold_from_interest as f64 / r,
        gold_clear_relics: a.total_gold_from_clear_relics as f64 / r,
        gold_clear_green_luck: a.total_gold_clear_green_luck as f64 / r,
        gold_clear_gold_idol: a.total_gold_clear_gold_idol as f64 / r,
        gold_clear_jade_abacus: a.total_gold_clear_jade_abacus as f64 / r,
        gold_clear_patience: a.total_gold_clear_patience as f64 / r,
        second_wind_forfeits: a.total_second_wind_forfeits as f64 / r,
        structure_triggers: a.total_structure_triggers as f64 / r,
        structure_trigger_points_per_run: a.total_structure_trigger_points as f64 / rn as f64,
        turns: a.total_turns as f64 / r,
        peak_hand_size: a.sum_peak_hand_size as f64 / r,
        tiles_destroyed: a.total_tiles_destroyed as f64 / r,
        relic_activations: relic_act as f64 / r,
        final_gold: a.total_final_gold as f64 / r,
        target_score_faced: a.total_target_score as f64 / r,
        overscore: a.total_overscore as f64 / r,
        score_to_target_ratio: score_to_target,
    };

    let win_hint = if let Some(ref w) = overall_win_rate_wilson_95 {
        format!(
            "{} wins / {} · 95% Wilson {:.1}–{:.1}%",
            a.victories, a.runs, w.lo, w.hi
        )
    } else {
        format!("{} wins / {}", a.victories, a.runs)
    };

    let kpis = vec![
        KpiTile {
            id: "win_rate".into(),
            label: "Win rate".into(),
            value: format!("{:.1}%", per_run.win_rate_pct),
            hint: win_hint,
            highlight: true,
        },
        KpiTile {
            id: "avg_score".into(),
            label: "Avg score / run".into(),
            value: human_readable_score(per_run.total_score),
            hint: "total cleared".into(),
            highlight: false,
        },
        KpiTile {
            id: "score_target".into(),
            label: "Score ÷ target".into(),
            value: format!("{:.2}×", score_to_target),
            hint: "vs faced targets".into(),
            highlight: false,
        },
        KpiTile {
            id: "avg_blinds".into(),
            label: "Avg blinds".into(),
            value: format!("{:.2}", per_run.blinds_cleared),
            hint: "cleared per run".into(),
            highlight: false,
        },
        KpiTile {
            id: "avg_antes".into(),
            label: "Avg antes".into(),
            value: format!("{:.2}", per_run.antes_cleared),
            hint: "through run".into(),
            highlight: false,
        },
        KpiTile {
            id: "peak_blind".into(),
            label: "Peak blind".into(),
            value: human_readable_score(a.peak_blind_score as f64),
            hint: "best single blind; see Peak blind (batch max) for tiles/relics".into(),
            highlight: false,
        },
    ];

    let losses = a.runs - a.victories;
    let loss_breakdown = if losses > 0 {
        Some(LossBreakdownDerived {
            losses,
            out_of_plays: a.deaths_out_of_plays,
            out_of_plays_pct_of_losses: a.deaths_out_of_plays as f64 * 100.0 / losses as f64,
            no_actions_remaining: a.deaths_no_actions_remaining,
            no_actions_pct_of_losses: a.deaths_no_actions_remaining as f64 * 100.0 / losses as f64,
        })
    } else {
        None
    };

    let max_death_pct = a
        .deaths_by_ante
        .values()
        .map(|c| *c as f64 * 100.0 / r)
        .fold(0.0_f64, f64::max);

    let mut ante_keys: Vec<u32> = a.deaths_by_ante.keys().copied().collect();
    ante_keys.sort_unstable();
    let deaths_by_ante: Vec<DeathAnteRow> = ante_keys
        .into_iter()
        .filter_map(|ante| {
            let count = *a.deaths_by_ante.get(&ante)?;
            let pct = count as f64 * 100.0 / r;
            let text_bar = ((pct / 2.0).round() as u32).min(50);
            let dash_pct = if max_death_pct > 0.0 {
                (pct / max_death_pct) * 100.0
            } else {
                0.0
            };
            let (pct_ci_lo, pct_ci_hi) =
                wilson_95_pct(count as u64, rn as u64).unwrap_or((pct, pct));
            Some(DeathAnteRow {
                ante,
                count,
                pct_of_runs: pct,
                pct_ci_lo,
                pct_ci_hi,
                text_bar_hashes: text_bar,
                dashboard_bar_pct: dash_pct,
            })
        })
        .collect();

    let deaths_by_ante_hazard: Vec<DeathAnteHazardRow> = a
        .deaths_by_ante
        .keys()
        .max()
        .copied()
        .map(|max_ante| {
            let mut remaining = rn;
            let mut rows = Vec::new();
            for ante in 1..=max_ante {
                if remaining == 0 {
                    break;
                }
                let deaths = *a.deaths_by_ante.get(&ante).unwrap_or(&0);
                let reached = remaining;
                let hazard_pct = deaths as f64 * 100.0 / reached as f64;
                let (hazard_ci_lo, hazard_ci_hi) = wilson_95_pct(deaths as u64, reached as u64)
                    .unwrap_or((hazard_pct, hazard_pct));
                rows.push(DeathAnteHazardRow {
                    ante,
                    reached,
                    deaths,
                    hazard_pct,
                    hazard_ci_lo,
                    hazard_ci_hi,
                });
                remaining = remaining.saturating_sub(deaths);
            }
            rows
        })
        .unwrap_or_default();

    let mut blind_rows: Vec<NamedCountPct> = a
        .deaths_by_blind
        .iter()
        .map(|(name, count)| NamedCountPct {
            name: (*name).to_string(),
            count: *count,
            pct_of_runs: *count as f64 * 100.0 / r,
        })
        .collect();
    blind_rows.sort_by(|x, y| y.count.cmp(&x.count).then_with(|| x.name.cmp(&y.name)));

    let boss_blind_chart = build_boss_blind_chart(a, base_target);
    let surplus_candles = build_surplus_candles(runs);
    let boss_score_candles = build_boss_score_candles(runs, base_target);

    let mut surplus_tuples: Vec<(String, u64, f64)> = a
        .cleared_by_slot
        .iter()
        .filter_map(|(slot, clears)| {
            let clears = *clears;
            if clears == 0 {
                return None;
            }
            let overscore = *a.overscore_by_slot.get(slot).unwrap_or(&0);
            let avg = overscore as f64 / clears as f64;
            Some((slot.clone(), clears, avg))
        })
        .collect();
    surplus_tuples.sort_by(|a, b| {
        aggregate_stats_slot_sort_key(&a.0).cmp(&aggregate_stats_slot_sort_key(&b.0))
    });
    let log_max = surplus_tuples
        .iter()
        .map(|(_, _, avg)| (*avg + 1.0).ln())
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let surplus_by_slot: Vec<SurplusSlotRow> = surplus_tuples
        .into_iter()
        .map(|(slot, clears, avg)| SurplusSlotRow {
            slot,
            clears,
            avg_surplus: avg,
            bar_pct: (((avg + 1.0).ln()) / log_max * 100.0).round().min(100.0),
        })
        .collect();

    let mut avg_turns_to_clear: Vec<AvgTurnsClearRow> = a
        .cleared_by_slot
        .iter()
        .filter_map(|(slot, clears)| {
            if *clears == 0 {
                return None;
            }
            let t = *a.turns_cleared_by_slot.get(slot)? as f64;
            Some(AvgTurnsClearRow {
                slot: slot.clone(),
                avg_turns: t / *clears as f64,
            })
        })
        .collect();
    avg_turns_to_clear.sort_by(|x, y| {
        aggregate_stats_slot_sort_key(&x.slot).cmp(&aggregate_stats_slot_sort_key(&y.slot))
    });

    let mut skip_tags: Vec<NamedCountPct> = a
        .skipped_tags
        .iter()
        .map(|(tag, count)| NamedCountPct {
            name: (*tag).to_string(),
            count: *count,
            pct_of_runs: *count as f64 * 100.0 / r,
        })
        .collect();
    skip_tags.sort_by(|x, y| y.count.cmp(&x.count).then_with(|| x.name.cmp(&y.name)));

    let mut zod: Vec<NamedCount> = a
        .total_zodiacs_used
        .iter()
        .map(|(n, c)| NamedCount {
            name: (*n).to_string(),
            count: *c,
        })
        .collect();
    zod.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    zod.truncate(24);

    let mut tal: Vec<NamedCount> = a
        .total_talismans_used
        .iter()
        .map(|(n, c)| NamedCount {
            name: (*n).to_string(),
            count: *c,
        })
        .collect();
    tal.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    tal.truncate(24);

    let mut transformations_top: Vec<NamedCount> = a
        .transformations_successor
        .iter()
        .map(|(n, c)| NamedCount {
            name: (*n).to_string(),
            count: *c,
        })
        .collect();
    transformations_top.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    transformations_top.truncate(16);

    let overall_win = a.victories as f64 * 100.0 / rn as f64;
    let mut relic_pairs: Vec<(&str, u32)> = a.relics_picked.iter().map(|(n, c)| (*n, *c)).collect();
    relic_pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let relics_bought: Vec<RelicBuyRow> = relic_pairs
        .iter()
        .map(|(name, bought)| {
            let won = a.relics_picked_victories.get(*name).copied().unwrap_or(0);
            let win_pct = if *bought > 0 {
                won as f64 * 100.0 / *bought as f64
            } else {
                0.0
            };
            let (win_pct_ci_lo, win_pct_ci_hi) = if *bought > 0 {
                wilson_95_pct(won as u64, *bought as u64).unwrap_or((win_pct, win_pct))
            } else {
                (win_pct, win_pct)
            };
            let early = a.relics_picked_shop_early.get(*name).copied().unwrap_or(0);
            let late = a.relics_picked_shop_late.get(*name).copied().unwrap_or(0);
            let pct_shop_late = if *bought > 0 && early + late > 0 {
                Some(late as f64 * 100.0 / (early + late) as f64)
            } else {
                None
            };
            RelicBuyRow {
                name: (*name).to_string(),
                rarity: relic_rarity_slug(name),
                bought: *bought,
                won,
                win_pct,
                win_pct_ci_lo,
                win_pct_ci_hi,
                delta_vs_baseline_pct: win_pct - overall_win,
                pct_shop_late,
            }
        })
        .collect();

    let mut relics_by_win_rate: Vec<RelicWinRateRow> = relic_pairs
        .iter()
        .filter(|(_, b)| *b >= MIN_SAMPLES_FOR_WIN_CORR)
        .map(|(name, bought)| {
            let won = a.relics_picked_victories.get(*name).copied().unwrap_or(0);
            let win_pct = won as f64 * 100.0 / *bought as f64;
            let (win_pct_ci_lo, win_pct_ci_hi) =
                wilson_95_pct(won as u64, *bought as u64).unwrap_or((win_pct, win_pct));
            let early = a.relics_picked_shop_early.get(*name).copied().unwrap_or(0);
            let late = a.relics_picked_shop_late.get(*name).copied().unwrap_or(0);
            let pct_shop_late = if *bought > 0 && early + late > 0 {
                Some(late as f64 * 100.0 / (early + late) as f64)
            } else {
                None
            };
            RelicWinRateRow {
                name: (*name).to_string(),
                rarity: relic_rarity_slug(name),
                bought: *bought,
                won,
                win_pct,
                win_pct_ci_lo,
                win_pct_ci_hi,
                delta_vs_baseline_pct: win_pct - overall_win,
                pct_shop_late,
            }
        })
        .collect();
    relics_by_win_rate.sort_by(|a, b| {
        b.win_pct
            .partial_cmp(&a.win_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut relics_shop_timing_split: Vec<RelicShopTimingRow> = relic_pairs
        .iter()
        .filter_map(|(name, _)| {
            let early_bought = a.relics_picked_shop_early.get(*name).copied().unwrap_or(0);
            let late_bought = a.relics_picked_shop_late.get(*name).copied().unwrap_or(0);
            if early_bought < MIN_SHOP_TIMING_SPLIT_PER_BUCKET
                || late_bought < MIN_SHOP_TIMING_SPLIT_PER_BUCKET
            {
                return None;
            }
            let early_won = a
                .relics_picked_shop_early_victories
                .get(*name)
                .copied()
                .unwrap_or(0);
            let late_won = a
                .relics_picked_shop_late_victories
                .get(*name)
                .copied()
                .unwrap_or(0);
            let early_win_pct = early_won as f64 * 100.0 / early_bought as f64;
            let late_win_pct = late_won as f64 * 100.0 / late_bought as f64;
            let (early_win_pct_ci_lo, early_win_pct_ci_hi) =
                wilson_95_pct(early_won as u64, early_bought as u64)
                    .unwrap_or((early_win_pct, early_win_pct));
            let (late_win_pct_ci_lo, late_win_pct_ci_hi) =
                wilson_95_pct(late_won as u64, late_bought as u64)
                    .unwrap_or((late_win_pct, late_win_pct));
            Some(RelicShopTimingRow {
                name: (*name).to_string(),
                rarity: relic_rarity_slug(name),
                early_bought,
                early_won,
                early_win_pct,
                early_win_pct_ci_lo,
                early_win_pct_ci_hi,
                late_bought,
                late_won,
                late_win_pct,
                late_win_pct_ci_lo,
                late_win_pct_ci_hi,
                timing_gap_pct: late_win_pct - early_win_pct,
            })
        })
        .collect();
    relics_shop_timing_split.sort_by(|a, b| {
        b.timing_gap_pct
            .partial_cmp(&a.timing_gap_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    fn shop_per_run(
        m: &std::collections::BTreeMap<&'static str, u32>,
        runs: u32,
    ) -> Vec<NamedPerRun> {
        let mut v: Vec<_> = m
            .iter()
            .map(|(n, c)| NamedPerRun {
                name: (*n).to_string(),
                count: *c,
                per_run: *c as f64 / runs as f64,
            })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        v
    }

    let talismans_shop = shop_per_run(&a.talismans_picked, a.runs);
    let zodiacs_shop = shop_per_run(&a.zodiacs_picked, a.runs);
    let packs_shop = shop_per_run(&a.packs_picked, a.runs);

    let total_yaku: u64 = a.yaku_scored.values().sum();
    let yaku = if total_yaku > 0 {
        let nk = yaku_kind_count.max(1);
        let uniform = 100.0 / nk as f64;
        let mut rows: Vec<YakuRow> = a
            .yaku_scored
            .iter()
            .map(|(name, count)| {
                let c = *count;
                let share = c as f64 * 100.0 / total_yaku as f64;
                YakuRow {
                    name: (*name).to_string(),
                    awards: c,
                    per_run: c as f64 / r,
                    share_pct: share,
                    delta_uniform_pct: share - uniform,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.awards.cmp(&a.awards).then_with(|| a.name.cmp(&b.name)));
        let awarded: rustc_hash::FxHashSet<&str> = a.yaku_scored.keys().copied().collect();
        let mut never_awarded: Vec<String> = YakuKind::all()
            .iter()
            .filter(|yk| !awarded.contains(yk.name()))
            .map(|yk| yk.name().to_string())
            .collect();
        never_awarded.sort();
        Some(YakuDerived {
            total_awards: total_yaku,
            awards_per_run: total_yaku as f64 / r,
            uniform_baseline_pct: uniform,
            kind_count: nk,
            rows,
            never_awarded,
        })
    } else {
        None
    };

    let tbi = a.total_bot_issue_no_valid_hand
        + a.total_bot_issue_only_valid_unplayable
        + a.total_bot_issue_only_valid_no_score
        + a.total_bot_issue_other_stuck;
    let bot_issues = if tbi > 0 || a.total_bot_issue_lost_with_available_lines > 0 {
        Some(BotIssuesDerived {
            dead_end_total: tbi,
            no_valid: a.total_bot_issue_no_valid_hand,
            only_unplayable: a.total_bot_issue_only_valid_unplayable,
            only_no_score: a.total_bot_issue_only_valid_no_score,
            other: a.total_bot_issue_other_stuck,
            lost_with_lines: a.total_bot_issue_lost_with_available_lines,
            hottest_blinds: top_string_u32(&a.bot_issues_by_blind, 5),
            hottest_bosses: top_string_u32(&a.bot_issues_by_boss, 5),
            reasons_top: top_string_u32(&a.bot_issues_by_reason, 8),
        })
    } else {
        None
    };

    let supplementary_tables = vec![
        MapTable {
            title: "Boss faced (runs)".into(),
            rows: a
                .boss_faced
                .iter()
                .map(|(n, c)| NamedCount {
                    name: n.clone(),
                    count: *c as u64,
                })
                .collect(),
        },
        MapTable {
            title: "Boss beaten (runs)".into(),
            rows: a
                .boss_beaten
                .iter()
                .map(|(n, c)| NamedCount {
                    name: n.clone(),
                    count: *c as u64,
                })
                .collect(),
        },
        MapTable {
            title: "Deaths by ante + cause".into(),
            rows: a
                .deaths_by_ante_cause
                .iter()
                .map(|(n, c)| NamedCount {
                    name: n.clone(),
                    count: *c as u64,
                })
                .collect(),
        },
        MapTable {
            title: "Turns by blind slot".into(),
            rows: a
                .turns_by_blind_slot
                .iter()
                .map(|(n, c)| NamedCount {
                    name: n.clone(),
                    count: *c,
                })
                .collect(),
        },
        MapTable {
            title: "Discards by blind slot".into(),
            rows: a
                .discards_by_blind_slot
                .iter()
                .map(|(n, c)| NamedCount {
                    name: n.clone(),
                    count: *c,
                })
                .collect(),
        },
        MapTable {
            title: "Relic activations".into(),
            rows: a
                .relic_activations
                .iter()
                .map(|(n, c)| NamedCount {
                    name: (*n).to_string(),
                    count: *c,
                })
                .collect(),
        },
    ];

    BotReportDerived {
        per_run,
        overall_win_rate_wilson_95,
        relic_shop_timing_early_ante_max: RELIC_SHOP_TIMING_EARLY_ANTE_MAX,
        relic_shop_timing_min_per_bucket: MIN_SHOP_TIMING_SPLIT_PER_BUCKET,
        kpis,
        loss_breakdown,
        deaths_by_ante,
        deaths_by_ante_hazard,
        deaths_by_blind: blind_rows,
        boss_blind_chart,
        target_scaling: TARGET_SCALING,
        surplus_by_slot,
        surplus_candles,
        boss_score_candles,
        avg_turns_to_clear,
        skip_tags,
        consumable_zodiacs: zod,
        consumable_talismans: tal,
        transformations_top,
        relics_bought,
        relics_by_win_rate,
        relics_shop_timing_split,
        talismans_shop,
        zodiacs_shop,
        packs_shop,
        yaku,
        bot_issues,
        supplementary_tables,
    }
}
