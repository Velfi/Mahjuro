use std::io::Write;

use super::*;
use crate::bot::reporting::human_readable_score;
use crate::core::relic::RelicId;
use crate::core::yaku::YakuKind;
use crate::core::zodiac::YakuLevels;
use crate::game::event_bus::GameOverReason;
use crate::game::run::RunState;

/// Categorize why a non-victory run ended, for balance-plot bucketing.
/// Returns one of: "no-legal-hand", "only-unplayable", "only-no-score",
/// "stuck-other", or "target-miss" (scoring failure without systemic block).
fn classify_run_death_cause(s: &RunStats) -> &'static str {
    if s.bot_issue_no_valid_hand > 0 {
        "no-legal-hand"
    } else if s.bot_issue_only_valid_unplayable > 0 {
        "only-unplayable"
    } else if s.bot_issue_only_valid_no_score > 0 {
        "only-no-score"
    } else if s.bot_issue_other_stuck > 0 {
        "stuck-other"
    } else {
        "target-miss"
    }
}

pub(crate) fn aggregate_stats_slot_sort_key(slot: &str) -> (u32, u32) {
    let (ante_str, rest) = slot.split_once('-').unwrap_or((slot, ""));
    let ante = ante_str.parse::<u32>().unwrap_or(99);
    let blind_order = match rest {
        "Small Blind" => 0,
        "Big Blind" => 1,
        "Boss Blind" => 2,
        _ => 99,
    };
    (ante, blind_order)
}

#[derive(Debug, Clone, Serialize)]
pub struct RunStats {
    pub blinds_cleared: u32,
    pub antes_cleared: u32,
    pub victory: bool,
    pub died_on_ante: u32,
    pub died_on_blind: BlindKind,
    pub total_score: u64,
    pub plays_used: u32,
    pub discards_used: u32,
    pub strategic_discards: u32,
    pub final_gold: i32,
    pub blinds_skipped: u32,
    pub relics_bought: u32,
    pub gold_spent: u32,
    pub gold_from_clears: u32,
    pub gold_from_clear_base: u32,
    pub gold_from_unused_plays: u32,
    pub gold_from_interest: u32,
    pub gold_from_clear_relics: u32,
    pub gold_from_skip_tags: u32,
    pub skip_tag_gold_value: u32,
    pub total_target_score: u64,
    pub total_overscore: u64,
    pub peak_blind_score: u64,
    pub skipped_tags: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked: std::collections::BTreeMap<&'static str, u32>,
    pub talismans_picked: std::collections::BTreeMap<&'static str, u32>,
    pub zodiacs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub packs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub bot_issue_no_valid_hand: u32,
    pub bot_issue_only_valid_unplayable: u32,
    pub bot_issue_only_valid_no_score: u32,
    pub bot_issue_other_stuck: u32,
    pub bot_issue_lost_with_available_lines: u32,
    pub bot_issues_by_reason: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_blind: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_boss: std::collections::BTreeMap<String, u32>,
    pub overscore_by_slot: std::collections::BTreeMap<String, u64>,
    pub cleared_by_slot: std::collections::BTreeMap<String, u32>,
    pub boss_faced: std::collections::BTreeMap<String, u8>,
    pub boss_beaten: std::collections::BTreeMap<String, u8>,
    /// Count of [`GameEvent::YakuScored`] per committed scoring action (one play may award several).
    pub yaku_scored: std::collections::BTreeMap<&'static str, u32>,
    /// Zodiac **consumes** from the dish during blinds (not shop purchases; see `zodiacs_picked`).
    pub zodiacs_used: std::collections::BTreeMap<&'static str, u32>,
    /// Talisman uses from dish + any [`GameEvent::TalismanUsed`] seen on the scoring bus.
    pub talismans_used: std::collections::BTreeMap<&'static str, u32>,
    pub structure_triggers: u32,
    pub structure_trigger_points: u64,
    pub second_wind_forfeits: u32,
    /// Set when the run ends on [`RunState::round_failure_reason`]; absent for victory / odd losses.
    pub death_reason: Option<GameOverReason>,
    pub gold_clear_green_luck: u32,
    pub gold_clear_gold_idol: u32,
    pub gold_clear_jade_abacus: u32,
    pub gold_clear_patience: u32,
    pub turns_total: u32,
    pub turns_by_blind_slot: std::collections::BTreeMap<String, u32>,
    /// Turn count for blinds that **cleared** only (denominator-friendly with `cleared_by_slot`).
    pub turns_cleared_by_slot: std::collections::BTreeMap<String, u32>,
    pub discards_by_blind_slot: std::collections::BTreeMap<String, u32>,
    pub peak_hand_size: u32,
    pub relic_activations: std::collections::BTreeMap<&'static str, u32>,
    pub tiles_destroyed: u32,
    pub transformations_successor: std::collections::BTreeMap<&'static str, u32>,
    /// Active relic display names in tray order at run end.
    pub final_relics: Vec<String>,
    /// Remaining consumable labels at run end.
    pub final_consumables: Vec<String>,
    pub final_yaku_levels: YakuLevels,
}

impl Default for RunStats {
    fn default() -> Self {
        Self {
            blinds_cleared: 0,
            antes_cleared: 0,
            victory: false,
            died_on_ante: 1,
            died_on_blind: BlindKind::Small,
            total_score: 0,
            plays_used: 0,
            discards_used: 0,
            strategic_discards: 0,
            final_gold: 0,
            blinds_skipped: 0,
            relics_bought: 0,
            gold_spent: 0,
            gold_from_clears: 0,
            gold_from_clear_base: 0,
            gold_from_unused_plays: 0,
            gold_from_interest: 0,
            gold_from_clear_relics: 0,
            gold_from_skip_tags: 0,
            skip_tag_gold_value: 0,
            total_target_score: 0,
            total_overscore: 0,
            peak_blind_score: 0,
            skipped_tags: std::collections::BTreeMap::new(),
            relics_picked: std::collections::BTreeMap::new(),
            talismans_picked: std::collections::BTreeMap::new(),
            zodiacs_picked: std::collections::BTreeMap::new(),
            packs_picked: std::collections::BTreeMap::new(),
            bot_issue_no_valid_hand: 0,
            bot_issue_only_valid_unplayable: 0,
            bot_issue_only_valid_no_score: 0,
            bot_issue_other_stuck: 0,
            bot_issue_lost_with_available_lines: 0,
            bot_issues_by_reason: std::collections::BTreeMap::new(),
            bot_issues_by_blind: std::collections::BTreeMap::new(),
            bot_issues_by_boss: std::collections::BTreeMap::new(),
            overscore_by_slot: std::collections::BTreeMap::new(),
            cleared_by_slot: std::collections::BTreeMap::new(),
            boss_faced: std::collections::BTreeMap::new(),
            boss_beaten: std::collections::BTreeMap::new(),
            yaku_scored: std::collections::BTreeMap::new(),
            zodiacs_used: std::collections::BTreeMap::new(),
            talismans_used: std::collections::BTreeMap::new(),
            structure_triggers: 0,
            structure_trigger_points: 0,
            second_wind_forfeits: 0,
            death_reason: None,
            gold_clear_green_luck: 0,
            gold_clear_gold_idol: 0,
            gold_clear_jade_abacus: 0,
            gold_clear_patience: 0,
            turns_total: 0,
            turns_by_blind_slot: std::collections::BTreeMap::new(),
            turns_cleared_by_slot: std::collections::BTreeMap::new(),
            discards_by_blind_slot: std::collections::BTreeMap::new(),
            peak_hand_size: 0,
            relic_activations: std::collections::BTreeMap::new(),
            tiles_destroyed: 0,
            transformations_successor: std::collections::BTreeMap::new(),
            final_relics: Vec::new(),
            final_consumables: Vec::new(),
            final_yaku_levels: YakuLevels::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct AggregateStats {
    pub runs: u32,
    pub blinds_cleared_total: u64,
    pub antes_cleared_total: u64,
    pub victories: u32,
    pub max_ante_reached: u32,
    pub total_score: u64,
    pub total_plays: u64,
    pub total_discards: u64,
    pub total_strategic_discards: u64,
    pub total_blinds_skipped: u64,
    pub total_relics_bought: u64,
    pub total_gold_spent: u64,
    pub total_final_gold: i64,
    pub total_gold_from_clears: u64,
    pub total_gold_from_clear_base: u64,
    pub total_gold_from_unused_plays: u64,
    pub total_gold_from_interest: u64,
    pub total_gold_from_clear_relics: u64,
    pub total_gold_from_skip_tags: u64,
    pub total_skip_tag_gold_value: u64,
    pub total_target_score: u64,
    pub total_overscore: u64,
    pub peak_blind_score: u64,
    pub total_bot_issue_no_valid_hand: u64,
    pub total_bot_issue_only_valid_unplayable: u64,
    pub total_bot_issue_only_valid_no_score: u64,
    pub total_bot_issue_other_stuck: u64,
    pub total_bot_issue_lost_with_available_lines: u64,
    pub bot_issues_by_reason: std::collections::BTreeMap<String, u32>,
    pub deaths_by_ante: std::collections::BTreeMap<u32, u32>,
    pub deaths_by_blind: std::collections::BTreeMap<&'static str, u32>,
    pub skipped_tags: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_victories: std::collections::BTreeMap<&'static str, u32>,
    pub talismans_picked: std::collections::BTreeMap<&'static str, u32>,
    pub zodiacs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub packs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub bot_issues_by_blind: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_boss: std::collections::BTreeMap<String, u32>,
    pub overscore_by_slot: std::collections::BTreeMap<String, u64>,
    pub cleared_by_slot: std::collections::BTreeMap<String, u64>,
    pub boss_faced: std::collections::BTreeMap<String, u32>,
    pub boss_beaten: std::collections::BTreeMap<String, u32>,
    pub deaths_by_ante_cause: std::collections::BTreeMap<String, u32>,
    pub yaku_scored: std::collections::BTreeMap<&'static str, u64>,
    pub total_zodiacs_used: std::collections::BTreeMap<&'static str, u64>,
    pub total_talismans_used: std::collections::BTreeMap<&'static str, u64>,
    pub total_structure_triggers: u64,
    pub total_structure_trigger_points: u64,
    pub total_second_wind_forfeits: u64,
    pub deaths_out_of_plays: u32,
    pub deaths_no_actions_remaining: u32,
    pub total_gold_clear_green_luck: u64,
    pub total_gold_clear_gold_idol: u64,
    pub total_gold_clear_jade_abacus: u64,
    pub total_gold_clear_patience: u64,
    pub total_turns: u64,
    pub sum_peak_hand_size: u64,
    pub turns_by_blind_slot: std::collections::BTreeMap<String, u64>,
    pub turns_cleared_by_slot: std::collections::BTreeMap<String, u64>,
    pub discards_by_blind_slot: std::collections::BTreeMap<String, u64>,
    pub relic_activations: std::collections::BTreeMap<&'static str, u64>,
    pub total_tiles_destroyed: u64,
    pub transformations_successor: std::collections::BTreeMap<&'static str, u64>,
}

impl AggregateStats {
    pub(crate) fn record(&mut self, s: &RunStats) {
        self.runs += 1;
        self.blinds_cleared_total += s.blinds_cleared as u64;
        self.antes_cleared_total += s.antes_cleared as u64;
        if s.victory {
            self.victories += 1;
        }
        self.max_ante_reached = self.max_ante_reached.max(s.died_on_ante);
        self.total_score += s.total_score;
        self.total_plays += s.plays_used as u64;
        self.total_discards += s.discards_used as u64;
        self.total_strategic_discards += s.strategic_discards as u64;
        self.total_blinds_skipped += s.blinds_skipped as u64;
        self.total_relics_bought += s.relics_bought as u64;
        self.total_gold_spent += s.gold_spent as u64;
        self.total_final_gold += s.final_gold as i64;
        self.total_gold_from_clears += s.gold_from_clears as u64;
        self.total_gold_from_clear_base += s.gold_from_clear_base as u64;
        self.total_gold_from_unused_plays += s.gold_from_unused_plays as u64;
        self.total_gold_from_interest += s.gold_from_interest as u64;
        self.total_gold_from_clear_relics += s.gold_from_clear_relics as u64;
        self.total_gold_from_skip_tags += s.gold_from_skip_tags as u64;
        self.total_skip_tag_gold_value += s.skip_tag_gold_value as u64;
        self.total_gold_clear_green_luck += s.gold_clear_green_luck as u64;
        self.total_gold_clear_gold_idol += s.gold_clear_gold_idol as u64;
        self.total_gold_clear_jade_abacus += s.gold_clear_jade_abacus as u64;
        self.total_gold_clear_patience += s.gold_clear_patience as u64;
        self.total_target_score += s.total_target_score;
        self.total_overscore += s.total_overscore;
        self.peak_blind_score = self.peak_blind_score.max(s.peak_blind_score);
        self.total_bot_issue_no_valid_hand += s.bot_issue_no_valid_hand as u64;
        self.total_bot_issue_only_valid_unplayable += s.bot_issue_only_valid_unplayable as u64;
        self.total_bot_issue_only_valid_no_score += s.bot_issue_only_valid_no_score as u64;
        self.total_bot_issue_other_stuck += s.bot_issue_other_stuck as u64;
        self.total_bot_issue_lost_with_available_lines +=
            s.bot_issue_lost_with_available_lines as u64;
        for (reason, count) in &s.bot_issues_by_reason {
            *self.bot_issues_by_reason.entry(reason.clone()).or_insert(0) += *count;
        }
        if !s.victory {
            *self.deaths_by_ante.entry(s.died_on_ante).or_insert(0) += 1;
            *self
                .deaths_by_blind
                .entry(s.died_on_blind.name())
                .or_insert(0) += 1;
            match s.death_reason {
                Some(GameOverReason::OutOfPlays) => self.deaths_out_of_plays += 1,
                Some(GameOverReason::NoActionsRemaining) => self.deaths_no_actions_remaining += 1,
                None => {}
            }
        }
        for (tag, count) in &s.skipped_tags {
            *self.skipped_tags.entry(tag).or_insert(0) += *count;
        }
        for (name, count) in &s.relics_picked {
            *self.relics_picked.entry(name).or_insert(0) += *count;
            if s.victory {
                *self.relics_picked_victories.entry(name).or_insert(0) += *count;
            }
        }
        for (name, count) in &s.talismans_picked {
            *self.talismans_picked.entry(name).or_insert(0) += *count;
        }
        for (name, count) in &s.zodiacs_picked {
            *self.zodiacs_picked.entry(name).or_insert(0) += *count;
        }
        for (name, count) in &s.packs_picked {
            *self.packs_picked.entry(name).or_insert(0) += *count;
        }
        for (blind, count) in &s.bot_issues_by_blind {
            *self.bot_issues_by_blind.entry(blind.clone()).or_insert(0) += *count;
        }
        for (boss, count) in &s.bot_issues_by_boss {
            *self.bot_issues_by_boss.entry(boss.clone()).or_insert(0) += *count;
        }
        for (slot, overscore) in &s.overscore_by_slot {
            *self.overscore_by_slot.entry(slot.clone()).or_insert(0) += *overscore;
        }
        for (slot, count) in &s.cleared_by_slot {
            *self.cleared_by_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for boss in s.boss_faced.keys() {
            *self.boss_faced.entry(boss.clone()).or_insert(0) += 1;
        }
        for boss in s.boss_beaten.keys() {
            *self.boss_beaten.entry(boss.clone()).or_insert(0) += 1;
        }
        if !s.victory {
            let cause = classify_run_death_cause(s);
            let key = format!("{}|{}", s.died_on_ante, cause);
            *self.deaths_by_ante_cause.entry(key).or_insert(0) += 1;
        }
        self.total_structure_triggers += s.structure_triggers as u64;
        self.total_structure_trigger_points += s.structure_trigger_points;
        self.total_second_wind_forfeits += s.second_wind_forfeits as u64;
        self.total_turns += s.turns_total as u64;
        self.sum_peak_hand_size += s.peak_hand_size as u64;
        self.total_tiles_destroyed += s.tiles_destroyed as u64;
        for (name, count) in &s.zodiacs_used {
            *self.total_zodiacs_used.entry(name).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.talismans_used {
            *self.total_talismans_used.entry(name).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.turns_by_blind_slot {
            *self.turns_by_blind_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.turns_cleared_by_slot {
            *self.turns_cleared_by_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.discards_by_blind_slot {
            *self.discards_by_blind_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.relic_activations {
            *self.relic_activations.entry(name).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.transformations_successor {
            *self
                .transformations_successor
                .entry(name)
                .or_insert(0) += *count as u64;
        }
        for (name, count) in &s.yaku_scored {
            *self.yaku_scored.entry(name).or_insert(0) += *count as u64;
        }
    }

    pub fn to_aggregate_v2(&self) -> crate::bot::export_schema::BotAggregateV2 {
        crate::bot::stats_derived::aggregate_to_v2(self)
    }

    pub fn to_derived(&self, yaku_kind_count: usize) -> crate::bot::export_schema::BotReportDerived {
        crate::bot::stats_derived::derived_from_aggregate(self, yaku_kind_count)
    }

    pub fn write_summary(&self, w: &mut dyn Write) -> std::io::Result<()> {
        macro_rules! out {
            ($($t:tt)*) => {
                writeln!(w, $($t)*)?;
            };
        }

        out!("\n=== Bot Stats ({} runs) ===", self.runs);
        if self.runs == 0 {
            return Ok(());
        }
        let d = self.to_derived(YakuKind::all().len());
        let pr = &d.per_run;
        out!(
            "victories:           {} / {} ({:.1}%)",
            self.victories, self.runs, pr.win_rate_pct
        );
        out!("avg blinds cleared:  {:.2}", pr.blinds_cleared);
        out!("avg antes cleared:   {:.2}", pr.antes_cleared);
        out!("max ante reached:    {}", self.max_ante_reached);
        out!("avg total score:     {}", human_readable_score(pr.total_score));
        out!("avg plays used:      {:.2}", pr.plays);
        out!(
            "avg discards used:   {:.2} ({:.2} strategic, {:.2} random)",
            pr.discards, pr.strategic_discards, pr.random_discards
        );
        out!("avg blinds skipped:  {:.2}", pr.blinds_skipped);
        out!(
            "avg relics bought:   {:.2} (avg gold spent: {:.1})",
            pr.relics_bought, pr.gold_spent
        );
        out!(
            "avg gold earned:     {:.1} clears + {:.1} skip-tags = {:.1}",
            pr.gold_from_clears,
            pr.gold_from_skip_tags,
            pr.gold_from_clears + pr.gold_from_skip_tags
        );
        out!(
            "avg clear payout:    {:.1} base + {:.1} plays + {:.1} interest + {:.1} relics",
            pr.gold_clear_base,
            pr.gold_clear_unused_plays,
            pr.gold_clear_interest,
            pr.gold_clear_relics
        );
        out!(
            "  (relic slice:      {:.1} green luck + {:.1} gold idol + {:.1} jade abacus + {:.1} patience)",
            pr.gold_clear_green_luck,
            pr.gold_clear_gold_idol,
            pr.gold_clear_jade_abacus,
            pr.gold_clear_patience
        );
        out!("avg second-wind forfeits: {:.2}", pr.second_wind_forfeits);
        out!(
            "avg structure cash-ins: {:.2} ({:.0} pts/run from structure)",
            pr.structure_triggers,
            pr.structure_trigger_points_per_run
        );
        out!(
            "avg turns / run:     {:.2} (avg peak hand size {:.2})",
            pr.turns, pr.peak_hand_size
        );
        out!(
            "tiles destroyed:     {:.2}/run | relic activations: {:.0}/run",
            pr.tiles_destroyed, pr.relic_activations
        );
        out!(
            "avg final gold:      {:.1} (avg skip-tag value taken: {:.1})",
            pr.final_gold, pr.skip_tag_gold_value
        );
        out!(
            "avg targets faced:   {} (score/target {:.2}x, avg overscore {}, peak blind score {})",
            human_readable_score(pr.target_score_faced),
            pr.score_to_target_ratio,
            human_readable_score(pr.overscore),
            human_readable_score(self.peak_blind_score as f64)
        );

        if let Some(ref lb) = d.loss_breakdown {
            out!("\nloss — engine stop reason (round exhausted; excludes odd failures):");
            out!(
                "  out of plays: {:>4} ({:.1}% of losses) | no actions: {:>4} ({:.1}%)",
                lb.out_of_plays,
                lb.out_of_plays_pct_of_losses,
                lb.no_actions_remaining,
                lb.no_actions_pct_of_losses
            );
        }

        out!("\ndeaths by ante:");
        for row in &d.deaths_by_ante {
            let bar = "#".repeat(row.text_bar_hashes as usize);
            out!(
                "  ante {:>2}: {:>4} ({:>5.1}%) {}",
                row.ante, row.count, row.pct_of_runs, bar
            );
        }

        out!("\ndeaths by blind:");
        for row in &d.deaths_by_blind {
            out!(
                "  {:<12} {:>4} ({:>5.1}%)",
                row.name, row.count, row.pct_of_runs
            );
        }

        if !d.surplus_by_slot.is_empty() {
            out!("\navg score surplus per blind (log-scaled bars):");
            for row in &d.surplus_by_slot {
                let bar_len = ((row.bar_pct / 100.0) * 40.0).round() as usize;
                let bar = "#".repeat(bar_len.min(40));
                out!(
                    "  {:<18} (n={:>5}) {:>10}  {}",
                    row.slot,
                    row.clears,
                    human_readable_score(row.avg_surplus),
                    bar
                );
            }
        }

        if !d.avg_turns_to_clear.is_empty() {
            out!("\navg turns to clear (cleared blinds only):");
            for row in &d.avg_turns_to_clear {
                out!("  {:<18} {:>6.2}", row.slot, row.avg_turns);
            }
        }

        if !d.skip_tags.is_empty() {
            out!("\nskip tags taken:");
            for row in &d.skip_tags {
                out!(
                    "  {:<16} {:>4} ({:>5.1}%)",
                    row.name, row.count, row.pct_of_runs
                );
            }
        }

        if !d.consumable_zodiacs.is_empty() || !d.consumable_talismans.is_empty() {
            out!("\nconsumables used (in-round + scoring events, not shop buys):");
            if !d.consumable_zodiacs.is_empty() {
                let line: String = d
                    .consumable_zodiacs
                    .iter()
                    .take(12)
                    .map(|x| format!("{} x{}", x.name, x.count))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  zodiacs: {line}");
            }
            if !d.consumable_talismans.is_empty() {
                let line: String = d
                    .consumable_talismans
                    .iter()
                    .take(12)
                    .map(|x| format!("{} x{}", x.name, x.count))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  talismans: {line}");
            }
        }

        if !d.transformations_top.is_empty() {
            out!("\ntransformation successors discovered:");
            for row in d.transformations_top.iter().take(16) {
                out!("  {:<24} {:>5}", row.name, row.count);
            }
        }

        if !d.relics_bought.is_empty() {
            const MIN_SAMPLES_FOR_WIN_CORR: u32 = 20;
            let overall_win_rate = pr.win_rate_pct;
            out!("\nrelics bought (sorted by total purchases):");
            out!("  {:<22} {:>7} {:>7} {:>8}", "relic", "bought", "won", "win%");
            for row in &d.relics_bought {
                out!(
                    "  {:<22} {:>7} {:>7} {:>7.1}%",
                    row.name, row.bought, row.won, row.win_pct
                );
            }
            if !d.relics_by_win_rate.is_empty() {
                out!(
                    "\nrelics by win-rate (≥{} samples, baseline {:.1}%):",
                    MIN_SAMPLES_FOR_WIN_CORR, overall_win_rate
                );
                out!(
                    "  {:<22} {:>7} {:>7} {:>8} {:>8}",
                    "relic", "bought", "won", "win%", "Δ"
                );
                for row in &d.relics_by_win_rate {
                    out!(
                        "  {:<22} {:>7} {:>7} {:>7.1}% {:>+7.1}",
                        row.name, row.bought, row.won, row.win_pct, row.delta_vs_baseline_pct
                    );
                }
            }
        }

        if !d.talismans_shop.is_empty() {
            out!("\ntalismans bought:");
            for row in &d.talismans_shop {
                out!("  {:<22} {:>5} ({:.2}/run)", row.name, row.count, row.per_run);
            }
        }

        if !d.zodiacs_shop.is_empty() {
            out!("\nzodiacs acquired:");
            for row in &d.zodiacs_shop {
                out!("  {:<22} {:>5} ({:.2}/run)", row.name, row.count, row.per_run);
            }
        }

        if let Some(ref y) = d.yaku {
            out!("\nyaku scored (each committed play may count multiple patterns):");
            out!(
                "  total awards: {} ({:.2}/run); uniform baseline {:.1}% per kind ({} kinds)",
                y.total_awards,
                y.awards_per_run,
                y.uniform_baseline_pct,
                y.kind_count
            );
            out!(
                "  {:<22} {:>8} {:>8} {:>8} {:>8}",
                "yaku", "awards", "/run", "share%", "Δuni%"
            );
            for row in &y.rows {
                out!(
                    "  {:<22} {:>8} {:>8.2} {:>7.1}% {:>+7.1}%",
                    row.name,
                    row.awards,
                    row.per_run,
                    row.share_pct,
                    row.delta_uniform_pct
                );
            }
            if !y.never_awarded.is_empty() {
                out!(
                    "\n  no awards (underused: bot never triggered these, {} kinds):",
                    y.never_awarded.len()
                );
                for name in &y.never_awarded {
                    out!("    {}", name);
                }
            }
        }

        if !d.packs_shop.is_empty() {
            out!("\npacks bought:");
            for row in &d.packs_shop {
                out!("  {:<22} {:>5} ({:.2}/run)", row.name, row.count, row.per_run);
            }
        }

        if let Some(ref bi) = d.bot_issues {
            out!("\nbot terminal issues:");
            if bi.dead_end_total > 0 {
                out!("  dead-ends:");
                out!(
                    "  total {:>6} | no-valid {} | only-valid-unplayable {} | only-valid-no-score {} | other {}",
                    bi.dead_end_total,
                    bi.no_valid,
                    bi.only_unplayable,
                    bi.only_no_score,
                    bi.other
                );
            }
            if bi.lost_with_lines > 0 {
                out!(
                    "  lost with lines available: {}",
                    bi.lost_with_lines
                );
            }
            if !bi.hottest_blinds.is_empty() {
                let s = bi
                    .hottest_blinds
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  hottest blinds: {s}");
            }
            if !bi.hottest_bosses.is_empty() {
                let s = bi
                    .hottest_bosses
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  hottest bosses: {s}");
            }
            if !bi.reasons_top.is_empty() {
                let s = bi
                    .reasons_top
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  reasons: {s}");
            }
        }

        Ok(())
    }


    pub fn print_summary(&self) {
        let _ = self.write_summary(&mut std::io::stdout().lock());
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ClearPayoutBreakdown {
    pub base_reward: u32,
    pub unused_play_bonus: u32,
    pub interest: u32,
    pub green_luck_bonus: u32,
    pub gold_idol_bonus: u32,
    pub jade_abacus_bonus: u32,
    pub patience_bonus: u32,
    pub relic_bonus: u32,
    pub total: u32,
}

pub(crate) fn clear_payout_breakdown(run: &RunState) -> ClearPayoutBreakdown {
    let base_reward = run.blind.clear_reward();
    let unused_play_bonus = run.plays_remaining;
    let interest = (run.gold.max(0) as u32 / 5).min(3);
    let green_luck_bonus = if run.relics.has(RelicId::GreenLuck) && !run.honors_scored_this_round {
        4
    } else {
        0
    };
    let gold_idol_bonus = if run.relics.has(RelicId::GoldIdol) {
        3
    } else {
        0
    };
    let jade_abacus_bonus = if run.relics.has(RelicId::JadeAbacus) {
        (run.gold.max(0) as u32 / 4).min(4)
    } else {
        0
    };
    let patience_bonus = if run.relics.has(RelicId::Patience) {
        2 * run.discards_remaining
    } else {
        0
    };
    let relic_bonus = green_luck_bonus + gold_idol_bonus + jade_abacus_bonus + patience_bonus;
    let total = base_reward + unused_play_bonus + interest + relic_bonus;
    ClearPayoutBreakdown {
        base_reward,
        unused_play_bonus,
        interest,
        green_luck_bonus,
        gold_idol_bonus,
        jade_abacus_bonus,
        patience_bonus,
        relic_bonus,
        total,
    }
}
