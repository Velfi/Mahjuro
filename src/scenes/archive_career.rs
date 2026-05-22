//! Career stats for the Archive: Chronicle dashboard copy, folio labels, and
//! personal-record / rivalry helpers.

use crate::core::boss::{BossKind, all_bosses, final_bosses};
use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::ImageQuadSource;

/// Run-log rows: index `0` = Summary, `1..` = runs newest-first.
pub fn chronicle_list_entry_count(progress: &PlayerProgress) -> usize {
    chronicle_indices_recent_first(progress).len() + 1
}

/// `list_index` `0` → summary; `1..` → `run_history` index.
pub fn chronicle_hist_index_at_list(list_index: usize, progress: &PlayerProgress) -> Option<usize> {
    if list_index == 0 {
        return None;
    }
    chronicle_indices_recent_first(progress)
        .get(list_index - 1)
        .copied()
}

/// Display number for a run row (`1` = oldest … `N` = newest).
pub fn chronicle_display_run_number(list_index: usize, progress: &PlayerProgress) -> Option<u32> {
    let run_count = chronicle_indices_recent_first(progress).len();
    if list_index == 0 || run_count == 0 {
        return None;
    }
    let ord = list_index - 1;
    if ord >= run_count {
        return None;
    }
    Some((run_count - ord) as u32)
}

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

fn serious_records(progress: &PlayerProgress) -> impl Iterator<Item = &RunRecord> {
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

/// Title line for one run-log row (descending display number, pin glyphs).
pub fn chronicle_run_log_title(
    progress: &PlayerProgress,
    display_num: u32,
    rec: &RunRecord,
) -> String {
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
    format!("{pins}Run {display_num} — {outcome}{boss}")
}

/// Run-log row line 1: pins, number, outcome (boss on line 2 in the ledger).
pub fn chronicle_run_log_line1(
    progress: &PlayerProgress,
    display_num: u32,
    rec: &RunRecord,
) -> String {
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
    format!("{pins}Run {display_num} — {outcome}")
}

/// Full boss name for run-log row line 2.
pub fn chronicle_run_log_boss_line(rec: &RunRecord) -> Option<String> {
    rec.final_boss.map(|b| b.name().to_string())
}

/// Sample tiles for chronicle screenshots / empty career hero.
pub fn sample_signature_hand_tiles() -> Vec<Tile> {
    use crate::scenes::meld_guide::yaku_page;
    let (_desc, groups) = yaku_page(YakuKind::Toitoi);
    groups
        .iter()
        .flat_map(|g| g.tiles.iter().copied())
        .map(|t| t.display_copy())
        .collect()
}

/// Outcome color for run-log rows.
pub fn chronicle_run_outcome_color(rec: &RunRecord) -> [f32; 4] {
    use crate::render::theme::color;
    match rec.outcome {
        RunOutcome::Victory => color::alpha(color::JADE, 0.95),
        RunOutcome::Defeat { .. } => color::alpha(color::RUBY, 0.92),
    }
}

/// Dominant yaku for a single run (most played this run).
pub fn run_dominant_yaku(rec: &RunRecord) -> Option<YakuKind> {
    rec.yaku_times_played
        .iter()
        .max_by_key(|(_, c)| *c)
        .map(|(k, _)| *k)
}

/// Accent tint for yaku pills in the chronicle ledger.
pub fn yaku_pill_color(yk: YakuKind) -> [f32; 4] {
    use crate::render::theme::color;
    match yk {
        YakuKind::Tanyao | YakuKind::FullHand => color::alpha(color::CHAMPAGNE, 0.75),
        YakuKind::Honitsu | YakuKind::Chinitsu => color::alpha(color::WALNUT_SOFT, 0.9),
        YakuKind::Yakuhai | YakuKind::Toitoi | YakuKind::Chiitoitsu => {
            color::alpha(color::ANTIQUE, 0.88)
        }
        YakuKind::KokushiMusou => color::alpha(color::JADE, 0.85),
        _ => color::alpha(color::STONE, 0.82),
    }
}

/// Floor shorthand for run log (`6 · 40F` style).
pub fn chronicle_floor_shorthand(rec: &RunRecord) -> String {
    let floor = rec.final_ante.saturating_mul(5);
    format!("Ante {} · {}F", rec.final_ante, floor)
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
        format!("Tiles discarded: {}", rec.tiles_discarded),
        format!("Tiles drawn: {}", rec.times_restocked),
        format!("Gold: {}", rec.final_gold),
        format!("Plays remaining: {} / {}", rec.plays_remaining, rec.plays_max),
        format!(
            "Discards remaining: {} / {}",
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

/// Optional nemesis only when the player lost to the same boss at least 3 times.
fn nemesis_line(progress: &PlayerProgress) -> Option<(BossKind, u32, u32)> {
    const MIN_LOSSES: u32 = 3;
    let mut best: Option<(BossKind, u32, u32)> = None;

    for def in all_bosses().iter().chain(final_bosses().iter()) {
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
        let wins = progress.boss_times_defeated.get(&def.kind).copied().unwrap_or(0);
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

// ── Career aggregates ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CareerKpi {
    pub label: &'static str,
    pub value: String,
    pub detail: Option<String>,
}

pub fn career_kpi_strip(progress: &PlayerProgress) -> Vec<CareerKpi> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    if serious.is_empty() {
        return vec![CareerKpi {
            label: "Chronicle",
            value: "—".into(),
            detail: Some("Awaiting first run".into()),
        }];
    }
    let n = serious.len() as u32;
    let mut wins = 0u32;
    let mut total_score = 0u64;
    let mut max_ante = 0u32;
    let mut best_run = 0u64;
    let mut best_run_num = 0u32;
    for r in serious.iter() {
        if matches!(r.outcome, RunOutcome::Victory) {
            wins += 1;
        }
        total_score = total_score.saturating_add(r.total_score_earned);
        max_ante = max_ante.max(r.final_ante);
        if r.total_score_earned >= best_run {
            best_run = r.total_score_earned;
            best_run_num = r.run_number;
        }
    }
    let win_pct = (wins as f32 / n as f32 * 100.0).round() as u32;
    vec![
        CareerKpi {
            label: "Personal best",
            value: format!("{best_run}"),
            detail: Some(format!("Run {best_run_num}")),
        },
        CareerKpi {
            label: "Total runs",
            value: format!("{n}"),
            detail: Some("Recorded".into()),
        },
        CareerKpi {
            label: "Win rate",
            value: format!("{win_pct}%"),
            detail: Some(format!("{wins} wins")),
        },
        CareerKpi {
            label: "Highest ante",
            value: format!("{max_ante}"),
            detail: Some("Reached".into()),
        },
        CareerKpi {
            label: "Total score",
            value: format!("{total_score}"),
            detail: Some("Lifetime".into()),
        },
    ]
}

#[derive(Clone, Debug)]
pub struct ScoreBucket {
    pub label: &'static str,
    pub count: u32,
}

pub fn score_distribution_buckets(progress: &PlayerProgress) -> Vec<ScoreBucket> {
    const BUCKETS: &[(&str, u64, u64)] = &[
        ("< 10k", 0, 10_000),
        ("10k–50k", 10_000, 50_000),
        ("50k–100k", 50_000, 100_000),
        ("100k–250k", 100_000, 250_000),
        ("250k+", 250_000, u64::MAX),
    ];
    let mut counts = vec![0u32; BUCKETS.len()];
    for r in serious_records(progress) {
        let s = r.total_score_earned;
        for (i, &(_, lo, hi)) in BUCKETS.iter().enumerate() {
            if s >= lo && s < hi {
                counts[i] += 1;
                break;
            }
        }
    }
    BUCKETS
        .iter()
        .zip(counts)
        .map(|((label, _, _), count)| ScoreBucket { label, count })
        .collect()
}

#[derive(Clone, Debug)]
pub struct FooterStat {
    pub icon: &'static str,
    pub value: String,
    pub label: &'static str,
}

pub fn career_footer_stats(progress: &PlayerProgress) -> Vec<FooterStat> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    let n = serious.len();
    if n == 0 {
        return Vec::new();
    }
    let total_yaku: u32 = progress.yaku_times_scored.values().sum();
    let bosses_defeated: u32 = progress.boss_times_defeated.values().sum();
    let boss_catalog = all_bosses().len() + final_bosses().len();
    let asc_wins = serious
        .iter()
        .filter(|r| matches!(r.outcome, RunOutcome::Victory))
        .count();
    let best_streak = best_win_streak(&serious);
    let total_score: u64 = serious.iter().map(|r| r.total_score_earned).sum();
    let avg_score = total_score / n as u64;
    let avg_ante: f32 =
        serious.iter().map(|r| r.final_ante as u64).sum::<u64>() as f32 / n as f32;
    vec![
        FooterStat {
            icon: "✦",
            value: format!("{total_yaku}"),
            label: "Total yaku",
        },
        FooterStat {
            icon: "☠",
            value: format!("{bosses_defeated}/{boss_catalog}"),
            label: "Bosses",
        },
        FooterStat {
            icon: "♛",
            value: format!("{asc_wins}"),
            label: "Wins",
        },
        FooterStat {
            icon: "⚡",
            value: format!("{best_streak}"),
            label: "Best streak",
        },
        FooterStat {
            icon: "◎",
            value: format!("{avg_score}"),
            label: "Avg score",
        },
        FooterStat {
            icon: "▲",
            value: format!("{avg_ante:.1}"),
            label: "Avg ante",
        },
    ]
}

fn best_win_streak(runs: &[&RunRecord]) -> u32 {
    let mut sorted: Vec<&RunRecord> = runs.to_vec();
    sorted.sort_by_key(|r| r.timestamp_unix);
    let mut best = 0u32;
    let mut cur = 0u32;
    for r in sorted {
        if matches!(r.outcome, RunOutcome::Victory) {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Compact career tile for the Chronicle ledger (right pane).
#[derive(Clone, Debug)]
pub struct CareerTile {
    pub label: &'static str,
    pub value: String,
    pub detail: Option<String>,
}

/// Up to four career highlights for the 2×2 tile grid.
pub fn career_tiles(progress: &PlayerProgress) -> Vec<CareerTile> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    if serious.is_empty() {
        return vec![CareerTile {
            label: "Chronicle",
            value: "Awaiting first run".into(),
            detail: None,
        }];
    }

    let mut tiles = Vec::new();

    // Signature hand is shown in the wide hero row on the career pane, not here.

    if let Some((yk, n)) = progress.yaku_times_scored.iter().max_by_key(|(_, c)| *c) {
        tiles.push(CareerTile {
            label: "Favorite yaku",
            value: yk.name().into(),
            detail: Some(format!("{n} scored")),
        });
    }

    if let Some((boss, losses, wins)) = nemesis_line(progress) {
        tiles.push(CareerTile {
            label: "Nemesis",
            value: boss.name().into(),
            detail: Some(format!("{losses}L · best {wins}W")),
        });
    } else if let Some((rid, n)) = progress
        .relic_times_activated
        .iter()
        .max_by_key(|(_, c)| *c)
        && let Some(def) = crate::core::relic::all_relic_defs()
            .iter()
            .find(|d| d.id == *rid)
    {
        tiles.push(CareerTile {
            label: "Most-triggered relic",
            value: def.name.into(),
            detail: Some(format!("{n}×")),
        });
    }

    tiles.truncate(4);
    tiles
}

/// Career-wide signature hand record (for hero tile strip).
pub fn career_signature_record(progress: &PlayerProgress) -> Option<&RunRecord> {
    serious_records(progress)
        .filter(|r| r.best_structure_score > 0)
        .max_by_key(|r| r.best_structure_score)
}

// ── Run detail model ──────────────────────────────────────────────────

/// Chronicle run-detail caption for `timestamp_unix` (local time).
pub fn format_run_ended_timestamp(unix: u64) -> String {
    use chrono::{Local, TimeZone};
    if unix == 0 {
        return "Ended · time unknown".into();
    }
    let Some(dt) = Local.timestamp_opt(unix as i64, 0).single() else {
        return "Ended · time unknown".into();
    };
    format!(
        "Ended · {}",
        dt.format("%A, %B %d, %Y · %-I:%M %p")
    )
}

#[derive(Clone, Debug)]
pub struct RunDetailModel {
    pub heading: String,
    pub timestamp_line: String,
    #[allow(dead_code)] // Chronicle detail panel (concept layout) — wired when UI lands
    pub seed_line: String,
    #[allow(dead_code)]
    pub duration_line: String,
    #[allow(dead_code)]
    pub milestones: Vec<String>,
    pub signature_name: String,
    pub signature_score: u64,
    pub tiles: Vec<Tile>,
    pub tiles_representative: bool,
    pub yaku_rows: Vec<(YakuKind, u32)>,
    pub score_lines: Vec<String>,
    pub ante_scores: Vec<(u32, u64)>,
    pub timeline: Vec<(u32, String, String)>,
    pub footer: Vec<(String, String)>,
    #[allow(dead_code)]
    pub discard_by_suit: crate::core::run_chronicle::DiscardBySuit,
}

pub fn run_detail_model(progress: &PlayerProgress, display_num: u32, rec: &RunRecord) -> RunDetailModel {
    use crate::core::run_chronicle::format_run_seed;

    let heading = chronicle_run_log_title(progress, display_num, rec);
    let timestamp_line = format_run_ended_timestamp(rec.timestamp_unix);
    let seed_line = if rec.chronicle.seed != 0 {
        format_run_seed(rec.chronicle.seed)
    } else {
        "—".into()
    };
    let duration_line = format_duration(rec.duration_secs);
    let milestones = rec.chronicle.milestones.clone();

    let mut tiles = rec
        .chronicle
        .signature_hand
        .as_ref()
        .map(|s| s.tiles.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| rec.best_hand_tiles.clone());
    let mut tiles_representative = false;
    if tiles.is_empty() {
        if let Some(yk) = run_dominant_yaku(rec) {
            tiles = yaku_page_tiles(yk);
            tiles_representative = !tiles.is_empty();
        }
    }

    let mut yaku_rows: Vec<(YakuKind, u32)> = if !rec.chronicle.yaku_contributions.is_empty() {
        rec.chronicle
            .yaku_contributions
            .iter()
            .map(|(k, v)| (*k, v.han))
            .collect()
    } else {
        rec.yaku_times_played
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect()
    };
    yaku_rows.sort_by(|a, b| b.1.cmp(&a.1));

    let mut score_lines = Vec::new();
    if let Some(snap) = rec.chronicle.terminal_score.as_ref() {
        score_lines.push(format!("Base {}", snap.base_chips));
        score_lines.push(format!("Yaku +{}", snap.yaku_chips));
        score_lines.push(format!("Dora +{}", snap.dora_chips));
        if snap.relic_chips > 0 {
            score_lines.push(format!("Relics +{}", snap.relic_chips));
        }
        score_lines.push(format!(
            "Boss ×{:.2} · Stake ×{:.2}",
            snap.boss_mult_factor, snap.stake_mult_factor
        ));
        score_lines.push(format!("Total {}", snap.total));
    } else {
        score_lines.push(format!("Round {} / target {}", rec.round_score, rec.target_score));
        score_lines.push(format!("Run total {}", rec.total_score_earned));
    }
    score_lines.push(format!(
        "{} · {}",
        rec.tile_material.label(),
        rec.stake.label()
    ));
    if let Some(tier) = rec.chronicle.victory_tier {
        score_lines.push(format!("Victory tier · {}", tier.label()));
    }

    let ante_scores = if rec.score_after_ante.is_empty() {
        vec![(rec.final_ante, rec.total_score_earned)]
    } else {
        rec.score_after_ante.clone()
    };

    let timeline = encounter_timeline(rec);
    let c = &rec.chronicle;
    let footer = vec![
        ("Turns".into(), format!("{}", c.turns_total)),
        ("Drawn".into(), format!("{}", c.tiles_drawn)),
        ("Discarded".into(), format!("{}", rec.tiles_discarded)),
        ("Shops".into(), format!("{}", c.shops_visited)),
        ("Rerolls".into(), format!("{}", c.rerolls_used)),
        ("Relic triggers".into(), format!("{}", c.relic_triggers)),
        ("Gold earned".into(), format!("{}", c.gold_earned)),
        (
            "Best combo".into(),
            format!(
                "{}",
                rec.chronicle.best_combo_han.max(
                    (rec.best_structure_score / 100).min(u32::MAX as u64) as u32
                )
            ),
        ),
    ];

    RunDetailModel {
        heading,
        timestamp_line,
        seed_line,
        duration_line,
        milestones,
        signature_name: rec.best_structure_name.clone(),
        signature_score: rec.best_structure_score,
        tiles,
        tiles_representative,
        yaku_rows,
        score_lines,
        ante_scores,
        timeline,
        footer,
        discard_by_suit: c.discards_by_suit.clone(),
    }
}

fn format_duration(secs: u32) -> String {
    if secs == 0 {
        return "—".into();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else {
        format!("{m}m {s:02}s")
    }
}

fn encounter_timeline(rec: &RunRecord) -> Vec<(u32, String, String)> {
    if !rec.chronicle.encounters.is_empty() {
        return rec
            .chronicle
            .encounters
            .iter()
            .map(|e| {
                let blind = if let Some(boss) = &e.boss {
                    format!("{} ({})", e.blind_label, boss)
                } else {
                    e.blind_label.clone()
                };
                let note = if e.reward_note.is_empty() {
                    e.outcome.clone()
                } else {
                    format!("{} · {}", e.outcome, e.reward_note)
                };
                (e.ante, blind, note)
            })
            .collect();
    }
    lite_encounter_timeline(rec)
}

fn lite_encounter_timeline(rec: &RunRecord) -> Vec<(u32, String, String)> {
    let mut rows = Vec::new();
    for ante in 1..=rec.final_ante.max(1) {
        let blind = if ante == rec.final_ante {
            rec.final_blind.name().to_string()
        } else {
            "Cleared".into()
        };
        let note = if ante == rec.final_ante {
            match rec.outcome {
                RunOutcome::Victory => "Victory".into(),
                RunOutcome::Defeat { .. } => "Defeat".into(),
            }
        } else {
            "—".into()
        };
        rows.push((ante, blind, note));
    }
    rows
}

fn yaku_page_tiles(yk: YakuKind) -> Vec<Tile> {
    use crate::scenes::meld_guide::yaku_page;
    let (_desc, groups) = yaku_page(yk);
    groups
        .iter()
        .flat_map(|g| g.tiles.iter().copied())
        .map(|t| t.display_copy())
        .collect()
}

/// Default player tileset for 2D chronicle tile strips.
pub const CHRONICLE_TILESET: &str = "textures/tile_sets/original/atlas.png";

/// Atlas cell name for a tile face (`PackedAtlas` uses static str).
pub fn tile_atlas_cell(tile: &Tile) -> Option<&'static str> {
    match tile.suit {
        Suit::Souzu if (1..=9).contains(&tile.rank) => {
            Some(["B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9"][tile.rank as usize - 1])
        }
        Suit::Manzu if (1..=9).contains(&tile.rank) => {
            Some(["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9"][tile.rank as usize - 1])
        }
        Suit::Pinzu if (1..=9).contains(&tile.rank) => {
            Some(["D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9"][tile.rank as usize - 1])
        }
        Suit::Wind => match tile.rank {
            1 => Some("EWind"),
            2 => Some("SWind"),
            3 => Some("WWind"),
            4 => Some("NWind"),
            _ => None,
        },
        Suit::Dragon => match tile.rank {
            1 => Some("DRed"),
            2 => Some("DGreen"),
            3 => Some("DWhite"),
            _ => None,
        },
        Suit::Flower if (1..=4).contains(&tile.rank) => {
            Some(["Flower1", "Flower2", "Flower3", "Flower4"][tile.rank as usize - 1])
        }
        Suit::Season if (1..=4).contains(&tile.rank) => {
            Some(["Season1", "Season2", "Season3", "Season4"][tile.rank as usize - 1])
        }
        _ => None,
    }
}

pub fn tile_image_source(tile: &Tile) -> Option<ImageQuadSource> {
    let name = tile_atlas_cell(tile)?;
    Some(ImageQuadSource::PackedAtlas {
        sheet: CHRONICLE_TILESET,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::format_run_ended_timestamp;

    #[test]
    fn run_ended_timestamp_is_human_readable() {
        let s = format_run_ended_timestamp(1_779_453_032);
        assert!(s.starts_with("Ended · "));
        assert!(!s.contains("unix"));
        assert!(s.contains("2026"));
    }

    #[test]
    fn run_ended_timestamp_unknown_when_zero() {
        assert_eq!(
            format_run_ended_timestamp(0),
            "Ended · time unknown"
        );
    }
}

pub fn format_score(n: u64) -> String {
    let s = n.to_string();
    if s.len() <= 3 {
        return s;
    }
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
