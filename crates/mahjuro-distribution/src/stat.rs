//! Profile stat mirrors for distribution backends.

use mahjuro_core::core::progression::{PlayerProgress, RunOutcome, RunRecord};

/// Integer stats mirrored from [`PlayerProgress`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileStat {
    RunsFinished,
    RunsWon,
    BestEndingRoundScore,
}

impl ProfileStat {
    pub fn steam_api_name(self) -> &'static str {
        self.partner_id()
    }

    pub fn game_center_leaderboard_id(self) -> &'static str {
        self.partner_id()
    }

    pub fn xbox_stat_name(self) -> &'static str {
        self.partner_id()
    }

    fn partner_id(self) -> &'static str {
        match self {
            Self::RunsFinished => "RUNS_FINISHED",
            Self::RunsWon => "RUNS_WON",
            Self::BestEndingRoundScore => "BEST_ENDING_ROUND_SCORE",
        }
    }
}

#[inline]
fn u64_to_i32_saturating(n: u64) -> i32 {
    n.min(i32::MAX as u64) as i32
}

fn serious_run_records<'a>(
    progress: &'a PlayerProgress,
) -> impl Iterator<Item = &'a RunRecord> + 'a {
    progress.run_history.iter().filter(|r| !r.tutorial_run)
}

/// Snapshot of profile stats for distribution backends (all non-negative `i32`).
pub fn profile_stat_snapshot(progress: &PlayerProgress) -> [(ProfileStat, i32); 3] {
    let runs_finished = progress.runs_completed.min(i32::MAX as u32) as i32;

    let runs_won = serious_run_records(progress)
        .filter(|r| matches!(r.outcome, RunOutcome::Victory))
        .count()
        .min(i32::MAX as usize) as i32;

    let best_ending_round = progress
        .high_scores
        .first()
        .copied()
        .map(u64_to_i32_saturating)
        .unwrap_or(0);

    [
        (ProfileStat::RunsFinished, runs_finished),
        (ProfileStat::RunsWon, runs_won),
        (ProfileStat::BestEndingRoundScore, best_ending_round),
    ]
}
