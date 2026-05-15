//! Steam Stats API names — must match the partner backend exactly
//! (`Stats & Achievements` for app 4636490). See project maintainer docs or
//! the PR/chat summary for the Steamworks table.

use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};

/// Integer stats mirrored from [`PlayerProgress`] (and `run_history`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SteamStat {
    /// `PlayerProgress::runs_completed` — every run that reached the
    /// victory/defeat screen (includes tutorial losses that level meta).
    RunsFinished,
    /// Full victories on non-tutorial runs (`run_history` filter).
    RunsWon,
    /// Best final **round** score kept in `high_scores` (same ordering as
    /// `record_score` — highest first).
    BestEndingRoundScore,
}

impl SteamStat {
    pub fn api_name(self) -> &'static str {
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

/// Snapshot of profile stats for Steam (all non-negative `i32`).
pub fn profile_stat_snapshot(progress: &PlayerProgress) -> [(SteamStat, i32); 3] {
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
        (SteamStat::RunsFinished, runs_finished),
        (SteamStat::RunsWon, runs_won),
        (SteamStat::BestEndingRoundScore, best_ending_round),
    ]
}
