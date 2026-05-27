//! Relic balance telemetry: shop funnel, bot valuations, score attribution, depth splits.

use crate::core::relic::all_relic_defs;
use crate::core::scoring::{ScoreBreakdown, ScoreStep, StepKind};

use super::stats::{AggregateStats, RunStats};

/// Marginal / hold value histogram bucket labels (score units).
pub const VALUE_BUCKET_LABELS: &[&str] = &["<0", "0", "1–499", "500–999", "1k–5k", "5k+"];

pub fn value_bucket(v: i32) -> &'static str {
    match v {
        ..0 => "<0",
        0 => "0",
        1..500 => "1–499",
        500..1000 => "500–999",
        1000..5000 => "1k–5k",
        _ => "5k+",
    }
}

fn bucket_map_key(name: &str, bucket: &str) -> String {
    format!("{name}\0{bucket}")
}

pub fn match_step_source_to_relic(source: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for def in all_relic_defs() {
        if source == def.name || source.starts_with(def.name) {
            let len = def.name.len();
            if best.map(|(_, bl)| len > bl).unwrap_or(true) {
                best = Some((def.name, len));
            }
        }
    }
    best.map(|(n, _)| n)
}

pub fn record_shop_offer(stats: &mut RunStats, relic_name: &'static str) {
    *stats.relic_shop_offers.entry(relic_name).or_insert(0) += 1;
}

pub fn record_marginal_buy(stats: &mut RunStats, relic_name: &'static str, marginal: i32) {
    *stats.relic_marginal_buy_sum.entry(relic_name).or_insert(0) += marginal as i64;
    *stats
        .relic_marginal_buy_count
        .entry(relic_name)
        .or_insert(0) += 1;
    stats
        .relic_marginal_buy_min
        .entry(relic_name)
        .and_modify(|m| *m = (*m).min(marginal))
        .or_insert(marginal);
    stats
        .relic_marginal_buy_max
        .entry(relic_name)
        .and_modify(|m| *m = (*m).max(marginal))
        .or_insert(marginal);
    let bucket = value_bucket(marginal);
    *stats
        .relic_marginal_buy_buckets
        .entry(relic_name)
        .or_default()
        .entry(bucket)
        .or_insert(0) += 1;
}

pub fn record_hold_sell(stats: &mut RunStats, relic_name: &'static str, hold: i32) {
    *stats.relic_hold_sell_sum.entry(relic_name).or_insert(0) += hold as i64;
    *stats.relic_hold_sell_count.entry(relic_name).or_insert(0) += 1;
    stats
        .relic_hold_sell_min
        .entry(relic_name)
        .and_modify(|m| *m = (*m).min(hold))
        .or_insert(hold);
    stats
        .relic_hold_sell_max
        .entry(relic_name)
        .and_modify(|m| *m = (*m).max(hold))
        .or_insert(hold);
    let bucket = value_bucket(hold);
    *stats
        .relic_hold_sell_buckets
        .entry(relic_name)
        .or_default()
        .entry(bucket)
        .or_insert(0) += 1;
}

fn step_score_delta(prev_total: u64, step: &ScoreStep) -> u64 {
    step.running_total.saturating_sub(prev_total)
}

pub fn record_score_breakdown(stats: &mut RunStats, breakdown: &ScoreBreakdown) {
    let mut prev_total = 0u64;
    for step in breakdown.base_steps.iter().chain(breakdown.steps.iter()) {
        if matches!(step.kind, StepKind::Final) {
            continue;
        }
        let Some(relic) = match_step_source_to_relic(&step.source) else {
            prev_total = step.running_total;
            continue;
        };
        let delta = step_score_delta(prev_total, step);
        prev_total = step.running_total;
        if delta == 0 && !matches!(step.kind, StepKind::Yen) {
            continue;
        }
        *stats.relic_score_triggers.entry(relic).or_insert(0) += 1;
        *stats.relic_score_points.entry(relic).or_insert(0) += delta;
        match step.kind {
            StepKind::Chips => {
                *stats.relic_score_chips.entry(relic).or_insert(0) += delta;
            }
            StepKind::Mult => {
                *stats.relic_score_mult_pts.entry(relic).or_insert(0) += delta;
            }
            // Yen steps do not move `running_total`; economy relics use `yen_clear_*` counters.
            StepKind::Yen | StepKind::Final => {}
        }
    }
}

pub fn record_run_depth_split(agg: &mut AggregateStats, run: &RunStats) {
    let picked: std::collections::BTreeSet<&str> = run.relics_picked.keys().copied().collect();
    for def in all_relic_defs() {
        let name = def.name;
        if picked.contains(name) {
            *agg.relic_depth_with_antes_sum.entry(name).or_insert(0) += run.wings_cleared as u64;
            *agg.relic_depth_with_runs.entry(name).or_insert(0) += 1;
        } else {
            *agg.relic_depth_without_antes_sum.entry(name).or_insert(0) += run.wings_cleared as u64;
            *agg.relic_depth_without_runs.entry(name).or_insert(0) += 1;
        }
    }
}

pub fn merge_run_relic_analytics(agg: &mut AggregateStats, run: &RunStats) {
    record_run_depth_split(agg, run);

    for (name, count) in &run.relic_shop_offers {
        *agg.relic_shop_offers.entry(name).or_insert(0) += *count as u64;
    }
    for (name, sum) in &run.relic_marginal_buy_sum {
        *agg.relic_marginal_buy_sum.entry(name).or_insert(0) += *sum;
    }
    for (name, count) in &run.relic_marginal_buy_count {
        *agg.relic_marginal_buy_count.entry(name).or_insert(0) += *count as u64;
    }
    for (name, v) in &run.relic_marginal_buy_min {
        agg.relic_marginal_buy_min
            .entry(name)
            .and_modify(|m| *m = (*m).min(*v))
            .or_insert(*v);
    }
    for (name, v) in &run.relic_marginal_buy_max {
        agg.relic_marginal_buy_max
            .entry(name)
            .and_modify(|m| *m = (*m).max(*v))
            .or_insert(*v);
    }
    for (name, buckets) in &run.relic_marginal_buy_buckets {
        for (bucket, count) in buckets {
            *agg.relic_marginal_buy_bucket_totals
                .entry(bucket_map_key(name, bucket))
                .or_insert(0) += *count as u64;
        }
    }

    for (name, sum) in &run.relic_hold_sell_sum {
        *agg.relic_hold_sell_sum.entry(name).or_insert(0) += *sum;
    }
    for (name, count) in &run.relic_hold_sell_count {
        *agg.relic_hold_sell_count.entry(name).or_insert(0) += *count as u64;
    }
    for (name, v) in &run.relic_hold_sell_min {
        agg.relic_hold_sell_min
            .entry(name)
            .and_modify(|m| *m = (*m).min(*v))
            .or_insert(*v);
    }
    for (name, v) in &run.relic_hold_sell_max {
        agg.relic_hold_sell_max
            .entry(name)
            .and_modify(|m| *m = (*m).max(*v))
            .or_insert(*v);
    }
    for (name, buckets) in &run.relic_hold_sell_buckets {
        for (bucket, count) in buckets {
            *agg.relic_hold_sell_bucket_totals
                .entry(bucket_map_key(name, bucket))
                .or_insert(0) += *count as u64;
        }
    }

    for (name, pts) in &run.relic_score_points {
        *agg.relic_score_points.entry(name).or_insert(0) += *pts;
    }
    for (name, pts) in &run.relic_score_chips {
        *agg.relic_score_chips.entry(name).or_insert(0) += *pts;
    }
    for (name, pts) in &run.relic_score_mult_pts {
        *agg.relic_score_mult_pts.entry(name).or_insert(0) += *pts;
    }
    for (name, gold) in &run.relic_score_yen {
        *agg.relic_score_yen.entry(name).or_insert(0) += *gold;
    }
    for (name, n) in &run.relic_score_triggers {
        *agg.relic_score_triggers.entry(name).or_insert(0) += *n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_buckets_cover_range() {
        assert_eq!(value_bucket(-1), "<0");
        assert_eq!(value_bucket(0), "0");
        assert_eq!(value_bucket(499), "1–499");
        assert_eq!(value_bucket(5000), "5k+");
    }

    #[test]
    fn tea_ceremony_substeps_map_to_relic() {
        assert_eq!(
            match_step_source_to_relic("Tea Ceremony · Harmony"),
            Some("Tea Ceremony")
        );
    }
}
