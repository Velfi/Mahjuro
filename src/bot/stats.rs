use super::*;
use crate::bot::reporting::human_readable_score;

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
    }

    #[allow(dead_code)]
    pub(crate) fn merge_in(&mut self, other: AggregateStats) {
        self.runs += other.runs;
        self.blinds_cleared_total += other.blinds_cleared_total;
        self.antes_cleared_total += other.antes_cleared_total;
        self.victories += other.victories;
        self.max_ante_reached = self.max_ante_reached.max(other.max_ante_reached);
        self.total_score += other.total_score;
        self.total_plays += other.total_plays;
        self.total_discards += other.total_discards;
        self.total_strategic_discards += other.total_strategic_discards;
        self.total_blinds_skipped += other.total_blinds_skipped;
        self.total_relics_bought += other.total_relics_bought;
        self.total_gold_spent += other.total_gold_spent;
        self.total_final_gold += other.total_final_gold;
        self.total_gold_from_clears += other.total_gold_from_clears;
        self.total_gold_from_clear_base += other.total_gold_from_clear_base;
        self.total_gold_from_unused_plays += other.total_gold_from_unused_plays;
        self.total_gold_from_interest += other.total_gold_from_interest;
        self.total_gold_from_clear_relics += other.total_gold_from_clear_relics;
        self.total_gold_from_skip_tags += other.total_gold_from_skip_tags;
        self.total_skip_tag_gold_value += other.total_skip_tag_gold_value;
        self.total_target_score += other.total_target_score;
        self.total_overscore += other.total_overscore;
        self.peak_blind_score = self.peak_blind_score.max(other.peak_blind_score);
        self.total_bot_issue_no_valid_hand += other.total_bot_issue_no_valid_hand;
        self.total_bot_issue_only_valid_unplayable += other.total_bot_issue_only_valid_unplayable;
        self.total_bot_issue_only_valid_no_score += other.total_bot_issue_only_valid_no_score;
        self.total_bot_issue_other_stuck += other.total_bot_issue_other_stuck;
        self.total_bot_issue_lost_with_available_lines +=
            other.total_bot_issue_lost_with_available_lines;
        for (reason, count) in other.bot_issues_by_reason {
            *self.bot_issues_by_reason.entry(reason).or_insert(0) += count;
        }
        for (ante, count) in other.deaths_by_ante {
            *self.deaths_by_ante.entry(ante).or_insert(0) += count;
        }
        for (blind, count) in other.deaths_by_blind {
            *self.deaths_by_blind.entry(blind).or_insert(0) += count;
        }
        for (tag, count) in other.skipped_tags {
            *self.skipped_tags.entry(tag).or_insert(0) += count;
        }
        for (name, count) in other.relics_picked {
            *self.relics_picked.entry(name).or_insert(0) += count;
        }
        for (name, count) in other.relics_picked_victories {
            *self.relics_picked_victories.entry(name).or_insert(0) += count;
        }
        for (name, count) in other.talismans_picked {
            *self.talismans_picked.entry(name).or_insert(0) += count;
        }
        for (name, count) in other.zodiacs_picked {
            *self.zodiacs_picked.entry(name).or_insert(0) += count;
        }
        for (name, count) in other.packs_picked {
            *self.packs_picked.entry(name).or_insert(0) += count;
        }
        for (blind, count) in other.bot_issues_by_blind {
            *self.bot_issues_by_blind.entry(blind).or_insert(0) += count;
        }
        for (boss, count) in other.bot_issues_by_boss {
            *self.bot_issues_by_boss.entry(boss).or_insert(0) += count;
        }
        for (slot, overscore) in other.overscore_by_slot {
            *self.overscore_by_slot.entry(slot).or_insert(0) += overscore;
        }
        for (slot, count) in other.cleared_by_slot {
            *self.cleared_by_slot.entry(slot).or_insert(0) += count;
        }
    }

    pub fn print_summary(&self) {
        fn slot_sort_key(slot: &str) -> (u32, u32) {
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

        println!("\n=== Bot Stats ({} runs) ===", self.runs);
        if self.runs == 0 {
            return;
        }
        let avg_blinds = self.blinds_cleared_total as f64 / self.runs as f64;
        let avg_antes = self.antes_cleared_total as f64 / self.runs as f64;
        let avg_score = self.total_score as f64 / self.runs as f64;
        let avg_plays = self.total_plays as f64 / self.runs as f64;
        let avg_discards = self.total_discards as f64 / self.runs as f64;
        let avg_strategic = self.total_strategic_discards as f64 / self.runs as f64;
        let win_rate = self.victories as f64 * 100.0 / self.runs as f64;
        let avg_final_gold = self.total_final_gold as f64 / self.runs as f64;
        let avg_gold_from_clears = self.total_gold_from_clears as f64 / self.runs as f64;
        let avg_gold_from_skip_tags = self.total_gold_from_skip_tags as f64 / self.runs as f64;
        let avg_skip_tag_gold_value = self.total_skip_tag_gold_value as f64 / self.runs as f64;
        let avg_target_score = self.total_target_score as f64 / self.runs as f64;
        let avg_overscore = self.total_overscore as f64 / self.runs as f64;
        let score_to_target = if self.total_target_score == 0 {
            0.0
        } else {
            self.total_score as f64 / self.total_target_score as f64
        };
        println!(
            "victories:           {} / {} ({:.1}%)",
            self.victories, self.runs, win_rate
        );
        println!("avg blinds cleared:  {:.2}", avg_blinds);
        println!("avg antes cleared:   {:.2}", avg_antes);
        println!("max ante reached:    {}", self.max_ante_reached);
        println!("avg total score:     {}", human_readable_score(avg_score));
        println!("avg plays used:      {:.2}", avg_plays);
        println!(
            "avg discards used:   {:.2} ({:.2} strategic, {:.2} random)",
            avg_discards,
            avg_strategic,
            avg_discards - avg_strategic
        );
        println!(
            "avg blinds skipped:  {:.2}",
            self.total_blinds_skipped as f64 / self.runs as f64
        );
        println!(
            "avg relics bought:   {:.2} (avg gold spent: {:.1})",
            self.total_relics_bought as f64 / self.runs as f64,
            self.total_gold_spent as f64 / self.runs as f64
        );
        println!(
            "avg gold earned:     {:.1} clears + {:.1} skip-tags = {:.1}",
            avg_gold_from_clears,
            avg_gold_from_skip_tags,
            avg_gold_from_clears + avg_gold_from_skip_tags
        );
        println!(
            "avg clear payout:    {:.1} base + {:.1} plays + {:.1} interest + {:.1} relics",
            self.total_gold_from_clear_base as f64 / self.runs as f64,
            self.total_gold_from_unused_plays as f64 / self.runs as f64,
            self.total_gold_from_interest as f64 / self.runs as f64,
            self.total_gold_from_clear_relics as f64 / self.runs as f64
        );
        println!(
            "avg final gold:      {:.1} (avg skip-tag value taken: {:.1})",
            avg_final_gold, avg_skip_tag_gold_value
        );
        println!(
            "avg targets faced:   {} (score/target {:.2}x, avg overscore {}, peak blind score {})",
            human_readable_score(avg_target_score),
            score_to_target,
            human_readable_score(avg_overscore),
            human_readable_score(self.peak_blind_score as f64)
        );
        println!("\ndeaths by ante:");
        for (ante, count) in &self.deaths_by_ante {
            let pct = *count as f64 * 100.0 / self.runs as f64;
            let bar = "#".repeat(((pct / 2.0).round() as usize).min(50));
            println!("  ante {:>2}: {:>4} ({:>5.1}%) {}", ante, count, pct, bar);
        }
        println!("\ndeaths by blind:");
        for (blind, count) in &self.deaths_by_blind {
            let pct = *count as f64 * 100.0 / self.runs as f64;
            println!("  {:<12} {:>4} ({:>5.1}%)", blind, count, pct);
        }
        if !self.cleared_by_slot.is_empty() {
            println!("\navg score surplus per blind (log-scaled bars):");
            let mut slots: Vec<(&String, u64, u64)> = self
                .cleared_by_slot
                .iter()
                .map(|(slot, count)| {
                    let overscore = *self.overscore_by_slot.get(slot).unwrap_or(&0);
                    (slot, *count, overscore)
                })
                .collect();
            slots.sort_by_key(|(slot, _, _)| slot_sort_key(slot));
            let avgs: Vec<f64> = slots
                .iter()
                .map(|(_, count, overscore)| {
                    if *count == 0 {
                        0.0
                    } else {
                        *overscore as f64 / *count as f64
                    }
                })
                .collect();
            let log_max = avgs
                .iter()
                .copied()
                .map(|v| (v + 1.0).ln())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for ((slot, count, _), avg) in slots.iter().zip(avgs.iter()) {
                let bar_len = ((((*avg + 1.0).ln()) / log_max) * 40.0).round() as usize;
                let bar = "#".repeat(bar_len.min(40));
                println!(
                    "  {:<18} (n={:>5}) {:>10}  {}",
                    slot,
                    count,
                    human_readable_score(*avg),
                    bar
                );
            }
        }
        if !self.skipped_tags.is_empty() {
            println!("\nskip tags taken:");
            for (tag, count) in &self.skipped_tags {
                let pct = *count as f64 * 100.0 / self.runs as f64;
                println!("  {:<16} {:>4} ({:>5.1}%)", tag, count, pct);
            }
        }
        if !self.relics_picked.is_empty() {
            const MIN_SAMPLES_FOR_WIN_CORR: u32 = 20;
            let overall_win_rate = self.victories as f64 * 100.0 / self.runs.max(1) as f64;

            println!("\nrelics bought (sorted by total purchases):");
            let mut rows: Vec<(&&str, &u32)> = self.relics_picked.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            println!(
                "  {:<22} {:>7} {:>7} {:>8}",
                "relic", "bought", "won", "win%"
            );
            for (name, count) in &rows {
                let won = self
                    .relics_picked_victories
                    .get(*name)
                    .copied()
                    .unwrap_or(0);
                let win_pct = if **count > 0 {
                    won as f64 * 100.0 / **count as f64
                } else {
                    0.0
                };
                println!("  {:<22} {:>7} {:>7} {:>7.1}%", name, count, won, win_pct);
            }

            let mut ranked: Vec<(&&str, &u32, u32, f64)> = rows
                .iter()
                .filter_map(|(name, count)| {
                    if **count < MIN_SAMPLES_FOR_WIN_CORR {
                        return None;
                    }
                    let won = self
                        .relics_picked_victories
                        .get(*name)
                        .copied()
                        .unwrap_or(0);
                    let rate = won as f64 * 100.0 / **count as f64;
                    Some((*name, *count, won, rate))
                })
                .collect();
            if !ranked.is_empty() {
                ranked.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
                println!(
                    "\nrelics by win-rate (≥{} samples, baseline {:.1}%):",
                    MIN_SAMPLES_FOR_WIN_CORR, overall_win_rate
                );
                println!(
                    "  {:<22} {:>7} {:>7} {:>8} {:>8}",
                    "relic", "bought", "won", "win%", "Δ"
                );
                for (name, count, won, rate) in ranked {
                    let delta = rate - overall_win_rate;
                    println!(
                        "  {:<22} {:>7} {:>7} {:>7.1}% {:>+7.1}",
                        name, count, won, rate, delta
                    );
                }
            }
        }
        if !self.talismans_picked.is_empty() {
            println!("\ntalismans bought:");
            let mut rows: Vec<(&&str, &u32)> = self.talismans_picked.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            for (name, count) in rows {
                let per_run = *count as f64 / self.runs as f64;
                println!("  {:<22} {:>5} ({:.2}/run)", name, count, per_run);
            }
        }
        if !self.zodiacs_picked.is_empty() {
            println!("\nzodiacs acquired:");
            let mut rows: Vec<(&&str, &u32)> = self.zodiacs_picked.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            for (name, count) in rows {
                let per_run = *count as f64 / self.runs as f64;
                println!("  {:<22} {:>5} ({:.2}/run)", name, count, per_run);
            }
        }
        if !self.packs_picked.is_empty() {
            println!("\npacks bought:");
            let mut rows: Vec<(&&str, &u32)> = self.packs_picked.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            for (name, count) in rows {
                let per_run = *count as f64 / self.runs as f64;
                println!("  {:<22} {:>5} ({:.2}/run)", name, count, per_run);
            }
        }
        let total_bot_issues = self.total_bot_issue_no_valid_hand
            + self.total_bot_issue_only_valid_unplayable
            + self.total_bot_issue_only_valid_no_score
            + self.total_bot_issue_other_stuck;
        let total_bot_losses_with_lines = self.total_bot_issue_lost_with_available_lines;
        if total_bot_issues > 0 || total_bot_losses_with_lines > 0 {
            println!("\nbot terminal issues:");
        }
        if total_bot_issues > 0 {
            println!("  dead-ends:");
            println!(
                "  total {:>6} | no-valid {} | only-valid-unplayable {} | only-valid-no-score {} | other {}",
                total_bot_issues,
                self.total_bot_issue_no_valid_hand,
                self.total_bot_issue_only_valid_unplayable,
                self.total_bot_issue_only_valid_no_score,
                self.total_bot_issue_other_stuck
            );
        }
        if total_bot_losses_with_lines > 0 {
            println!(
                "  lost with lines available: {}",
                total_bot_losses_with_lines
            );
        }
        if total_bot_issues > 0 || total_bot_losses_with_lines > 0 {
            if let Some(blinds) = format_top_counts(&self.bot_issues_by_blind, 5) {
                println!("  hottest blinds: {blinds}");
            }
            if let Some(bosses) = format_top_counts(&self.bot_issues_by_boss, 5) {
                println!("  hottest bosses: {bosses}");
            }
            if let Some(reasons) = format_top_counts(&self.bot_issues_by_reason, 8) {
                println!("  reasons: {reasons}");
            }
        }
    }
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
        relic_bonus,
        total,
    }
}
