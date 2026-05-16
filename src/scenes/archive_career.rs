//! Career stats for the Archive: Chronicle dashboard copy, folio labels, and
//! personal-record / rivalry helpers.

use crate::core::boss::BossKind;
use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
use crate::core::yaku::YakuKind;
use crate::persistence::TileMaterial;

/// Indices into `progress.run_history`, most recent finished run first.
pub fn chronicle_indices_recent_first(progress: &PlayerProgress) -> Vec<usize> {
    let mut v: Vec<usize> = progress
        .run_history
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.tutorial_run)
        .map(|(i, _)| i)
        .collect();
    v.sort_by_key(|&idx| std::cmp::Reverse(progress.run_history[idx].timestamp_unix));
    v
}

fn serious_records<'a>(progress: &'a PlayerProgress) -> impl Iterator<Item = &'a RunRecord> {
    progress.run_history.iter().filter(|r| !r.tutorial_run)
}

/// Best total score across non-tutorial runs (for PR pins).
pub fn max_total_score_serious(progress: &PlayerProgress) -> Option<u64> {
    serious_records(progress)
        .map(|r| r.total_score_earned)
        .max()
}

fn max_structure_score_serious(progress: &PlayerProgress) -> Option<u64> {
    serious_records(progress)
        .map(|r| r.best_structure_score)
        .max()
}

/// Title line for one folio in the Chronicle grid (includes pin glyphs).
pub fn chronicle_folio_title(progress: &PlayerProgress, rec: &RunRecord) -> String {
    let mut pins = String::new();
    if matches!(rec.outcome, RunOutcome::Victory) {
        pins.push_str("★ ");
    }
    if Some(rec.total_score_earned) == max_total_score_serious(progress) {
        pins.push_str("⌁ ");
    }
    if Some(rec.best_structure_score) == max_structure_score_serious(progress)
        && rec.best_structure_score > 0
    {
        pins.push_str("◆ ");
    }
    let outcome = match rec.outcome {
        RunOutcome::Victory => "Victory",
        RunOutcome::Defeat { .. } => "Defeat",
    };
    let boss = rec
        .final_boss
        .map(|b| format!(" — {}", b.name()))
        .unwrap_or_default();
    format!("{}Run {} — {}{}", pins, rec.run_number, outcome, boss)
}

/// Multi-line plaque for inspect / description.
pub fn chronicle_run_description(rec: &RunRecord) -> String {
    let outcome = match rec.outcome {
        RunOutcome::Victory => "Victory".into(),
        RunOutcome::Defeat { reason } => format!("Defeat ({reason:?})"),
    };
    let boss = rec
        .final_boss
        .map(|b| format!("{}\n", b.name()))
        .unwrap_or_default();
    format!(
        "{outcome}\n{boss}Ante {} · {} blind\nRound score {} / target {}\nRun total score {}\nBest hand: {} ({})\nMaterial {} · stake {}",
        rec.final_ante,
        rec.final_blind.name(),
        rec.round_score,
        rec.target_score,
        rec.total_score_earned,
        rec.best_structure_name,
        rec.best_structure_score,
        rec.tile_material.label(),
        rec.stake.label(),
    )
}

/// Stats block for Chronicle inspect plaque (run snapshot).
pub fn chronicle_run_stats(rec: &RunRecord) -> String {
    let mut lines = vec![
        format!("Tiles played: {}", rec.tiles_played),
        format!("Discarded: {}", rec.tiles_discarded),
        format!("Restocks: {}", rec.times_restocked),
        format!("Gold: {}", rec.final_gold),
        format!("Plays left: {} / {}", rec.plays_remaining, rec.plays_max),
        format!(
            "Discards left: {} / {}",
            rec.discards_remaining, rec.discards_max
        ),
    ];
    if !rec.yaku_times_played.is_empty() {
        let mut pairs: Vec<(YakuKind, u32)> = rec
            .yaku_times_played
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in pairs.into_iter().take(6) {
            lines.push(format!("{}: {}", k.name(), n));
        }
    }
    if !rec.relics_owned.is_empty() {
        let names: Vec<String> = rec
            .relics_owned
            .iter()
            .filter_map(|id| {
                crate::core::relic::all_relic_defs()
                    .iter()
                    .find(|d| d.id == *id)
                    .map(|d| d.name.to_string())
            })
            .collect();
        lines.push(format!("Relics: {}", names.join(", ")));
    }
    lines.join("\n")
}

/// One primary frieze line + optional secondary lines (keep short for layout).
pub fn career_frieze_lines(progress: &PlayerProgress) -> Vec<String> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    if serious.is_empty() {
        return vec!["Your chronicle begins after your first run.".into()];
    }

    let mut out = Vec::new();

    // Primary: personal best run total score (or high_scores board).
    let best_run = serious
        .iter()
        .map(|r| r.total_score_earned)
        .max()
        .unwrap_or(0);
    let board_best = progress.high_scores.first().copied().unwrap_or(0);
    let primary_score = best_run.max(board_best);
    out.push(format!(
        "Personal best — {} pts (run total / board)",
        primary_score
    ));

    // Signature structure
    let sig = serious
        .iter()
        .max_by_key(|r| r.best_structure_score)
        .map(|r| (r.best_structure_score, r.best_structure_name.as_str()));
    if let Some((sc, name)) = sig
        && sc > 0
    {
        out.push(format!("Signature hand — {} ({})", name, sc));
    }

    // Signature yaku (lifetime)
    if let Some((yk, n)) = progress.yaku_times_scored.iter().max_by_key(|(_, c)| *c) {
        out.push(format!("Favorite yaku — {} ({})", yk.name(), n));
    }

    // Signature relic activations
    if let Some((rid, n)) = progress
        .relic_times_activated
        .iter()
        .max_by_key(|(_, c)| *c)
        && let Some(def) = crate::core::relic::all_relic_defs()
            .iter()
            .find(|d| d.id == *rid)
    {
        out.push(format!("Most-triggered relic — {} ({})", def.name, n));
    }

    // Nemesis (guardrails: ≥3 losses on that boss; pair with wins)
    if let Some((boss, losses, wins)) = nemesis_line(progress) {
        out.push(format!(
            "Rivalry — {name}: {losses} losses, {wins} wins",
            name = boss.name()
        ));
    }

    // Stakes teaser
    let stake_bits: Vec<String> = [
        TileMaterial::Bamboo,
        TileMaterial::Plastic,
        TileMaterial::TortoiseShell,
    ]
    .iter()
    .filter_map(|m| {
        let cleared = progress.stakes_cleared_for(*m);
        cleared
            .iter()
            .max()
            .map(|s| format!("{}: {}", m.label(), s.label()))
    })
    .collect();
    if !stake_bits.is_empty() {
        out.push(format!("Stakes — {}", stake_bits.join(" · ")));
    }

    out
}

/// Optional nemesis only when the player lost to the same boss at least 3 times.
fn nemesis_line(progress: &PlayerProgress) -> Option<(BossKind, u32, u32)> {
    const MIN_LOSSES: u32 = 3;
    let mut best: Option<(BossKind, u32, u32)> = None;

    for def in crate::core::boss::all_bosses()
        .iter()
        .chain(crate::core::boss::final_bosses().iter())
    {
        let losses = progress
            .run_history
            .iter()
            .filter(|r| {
                !r.tutorial_run
                    && matches!(r.outcome, RunOutcome::Defeat { .. })
                    && r.final_boss == Some(def.kind)
            })
            .count() as u32;
        if losses < MIN_LOSSES {
            continue;
        }
        let wins = progress
            .boss_times_defeated
            .get(&def.kind)
            .copied()
            .unwrap_or(0);
        let replace = match best {
            None => true,
            Some((_, prev_losses, _)) => losses > prev_losses,
        };
        if replace {
            best = Some((def.kind, losses, wins));
        }
    }
    best
}

/// Explain Chronicle omission in UI copy.
pub const CHRONICLE_TUTORIAL_NOTE: &str = "Practice tutorial runs are not listed in the Chronicle.";
