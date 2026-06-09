//! Career stats for the Archive: Chronicle dashboard copy, folio labels, and
//! personal-record / rivalry helpers.

use crate::core::OrdealKindExt;
use crate::core::ordeal::{OrdealKind, all_ordeals, final_ordeals};
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

/// Best signature-hand score across non-tutorial runs.
fn max_structure_score_serious(progress: &PlayerProgress) -> Option<u64> {
    serious_records(progress)
        .map(|r| r.best_structure_score)
        .max()
}

/// Mahjong red-dragon tile (Noto Emoji block U+1F004) — career-best signature hand.
const BEST_HAND_PIN: &str = "\u{1F004} ";

/// 0 = 1st … 2 = 3rd by total score among serious runs.
pub fn chronicle_run_score_rank(progress: &PlayerProgress, rec: &RunRecord) -> Option<u8> {
    let mut ranked: Vec<&RunRecord> = serious_records(progress).collect();
    if ranked.len() <= 1 {
        return None;
    }
    ranked.sort_by(|a, b| {
        b.total_score_earned
            .cmp(&a.total_score_earned)
            .then(b.timestamp_unix.cmp(&a.timestamp_unix))
    });
    ranked
        .iter()
        .position(|r| std::ptr::eq(*r, rec))
        .and_then(|i| u8::try_from(i).ok())
        .filter(|&i| i < 3)
}

/// Highlight for a top-three run score (`0` = gold … `2` = bronze).
pub fn chronicle_score_rank_color(rank: u8) -> [f32; 4] {
    use crate::render::theme::color;
    match rank {
        0 => color::GOLD,
        1 => color::alpha(color::CHAMPAGNE, 0.88),
        _ => color::alpha(color::BRASS, 0.96),
    }
}

/// Default chips magnitude tint for non-podium scores.
pub fn chronicle_chips_color() -> [f32; 4] {
    use crate::render::theme::color;
    color::alpha(color::score_cascade::CHIPS, 0.94)
}

/// Wing column tint in the run log.
pub fn chronicle_wing_color() -> [f32; 4] {
    use crate::render::theme::color;
    color::alpha(color::keyword::WIND, 0.92)
}

/// Run-detail footer value tint by stat label.
pub fn chronicle_footer_value_color(label: &str) -> [f32; 4] {
    use crate::render::theme::color;
    match label {
        "Yen earned" => color::alpha(color::keyword::GOLD, 0.96),
        "Best combo" => color::alpha(color::keyword::TRIGGER, 0.96),
        _ => color::CHAMPAGNE,
    }
}

/// Medal emoji when `rec` ranks 1st–3rd among serious runs by total score.
fn chronicle_run_score_rank_medal(
    progress: &PlayerProgress,
    rec: &RunRecord,
) -> Option<&'static str> {
    match chronicle_run_score_rank(progress, rec)? {
        0 => Some("🥇 "),
        1 => Some("🥈 "),
        2 => Some("🥉 "),
        _ => None,
    }
}

fn chronicle_run_best_hand_pin(progress: &PlayerProgress, rec: &RunRecord) -> Option<&'static str> {
    if serious_records(progress).count() <= 1 {
        return None;
    }
    if rec.best_structure_score > 0
        && Some(rec.best_structure_score) == max_structure_score_serious(progress)
    {
        Some(BEST_HAND_PIN)
    } else {
        None
    }
}

/// Title pins + body for one run-log row (medals / best-hand pin, then run line).
pub fn chronicle_run_log_title_parts(
    progress: &PlayerProgress,
    display_num: u32,
    rec: &RunRecord,
) -> (String, String) {
    let mut pins = String::new();
    if let Some(medal) = chronicle_run_score_rank_medal(progress, rec) {
        pins.push_str(medal);
    }
    if let Some(hand_pin) = chronicle_run_best_hand_pin(progress, rec) {
        pins.push_str(hand_pin);
    }
    let outcome = match rec.outcome {
        RunOutcome::Victory => "Victory",
        RunOutcome::Defeat { .. } => "Defeat",
    };
    let boss = rec
        .final_ordeal
        .map(|b| format!(" — {}", b.name()))
        .unwrap_or_default();
    (pins, format!("Run {display_num} — {outcome}{boss}"))
}

/// Title line for one run-log row (descending display number, score-rank medals, best-hand pin).
pub fn chronicle_run_log_title(
    progress: &PlayerProgress,
    display_num: u32,
    rec: &RunRecord,
) -> String {
    let (pins, body) = chronicle_run_log_title_parts(progress, display_num, rec);
    format!("{pins}{body}")
}

/// Full boss name for run-log row line 2.
pub fn chronicle_run_log_ordeal_line(rec: &RunRecord) -> Option<String> {
    rec.final_ordeal.map(|b| b.name().to_string())
}

/// Sample tiles for chronicle screenshots / empty career hero.
pub fn sample_signature_hand_tiles() -> Vec<Tile> {
    use crate::scenes::guide::yaku_page;
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
        RunOutcome::Victory => color::alpha(color::chart::POSITIVE, 0.95),
        RunOutcome::Defeat { .. } => color::alpha(color::chart::NEGATIVE, 0.92),
    }
}

/// Dominant yaku for a single run (most played this run).
pub fn run_dominant_yaku(rec: &RunRecord) -> Option<YakuKind> {
    rec.yaku_times_played
        .iter()
        .max_by_key(|(_, c)| *c)
        .map(|(k, _)| *k)
}

/// Bone-tablet face for chronicle yaku name pills (porcelain cream, like gameplay tablets).
pub fn yaku_pill_face() -> [f32; 4] {
    use crate::render::theme::color;
    color::PORCELAIN_AGED
}

/// Label ink on yaku pills (walnut on cream, readable on the ledger).
pub fn yaku_pill_ink() -> [f32; 4] {
    use crate::render::theme::color;
    color::WALNUT_INK
}

/// Bottom rim on yaku pills — subtle antique line like carved bone edge shadow.
pub fn yaku_pill_rim() -> [f32; 4] {
    use crate::render::theme::color;
    color::alpha(color::ANTIQUE, 0.42)
}

/// Multi-line plaque for inspect / description.
pub fn chronicle_run_description(rec: &RunRecord) -> String {
    let outcome = match rec.outcome {
        RunOutcome::Victory => "Victory".into(),
        RunOutcome::Defeat { reason } => format!("Defeat ({reason:?})"),
    };
    let boss = rec
        .final_ordeal
        .map(|b| format!("{}\n", b.name()))
        .unwrap_or_default();
    format!(
        "{outcome}\n{boss}Wing {} · {} chamber\nRound score {} / target {}\nRun total score {}\nBest hand: {} ({})\nMaterial {} · season {}",
        rec.final_wing,
        rec.final_chamber.name(),
        format_score(rec.round_score),
        format_score(rec.target_score as u64),
        format_score(rec.total_score_earned),
        rec.best_structure_name,
        format_score(rec.best_structure_score),
        rec.tile_material.label(),
        rec.season.label(),
    )
}

/// Optional nemesis only when the player lost to the same boss at least 3 times.
fn nemesis_line(progress: &PlayerProgress) -> Option<(OrdealKind, u32, u32)> {
    const MIN_LOSSES: u32 = 3;
    let mut best: Option<(OrdealKind, u32, u32)> = None;

    for def in all_ordeals().iter().chain(final_ordeals().iter()) {
        let losses = progress
            .run_history
            .iter()
            .filter(|r| {
                !r.tutorial_run
                    && matches!(r.outcome, RunOutcome::Defeat { .. })
                    && r.final_ordeal == Some(def.kind)
            })
            .count() as u32;
        if losses < MIN_LOSSES {
            continue;
        }
        let wins = progress
            .ordeal_times_defeated
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

// ── Career aggregates ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CareerKpi {
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ScoreHistoryPoint {
    pub score: u64,
    pub victory: bool,
}

#[derive(Clone, Debug)]
pub struct OrdealRecordRow {
    pub ordeal: OrdealKind,
    pub wins: u32,
    pub losses: u32,
    pub best_score: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct WingOutcomeCell {
    pub wing: u32,
    pub wins: u32,
    pub losses: u32,
}

#[derive(Clone, Debug)]
pub struct SignatureHandStats {
    pub yaku_tags: Vec<YakuKind>,
    pub times_used: u32,
    pub avg_score: u64,
}

pub fn career_score_history_points(progress: &PlayerProgress) -> Vec<ScoreHistoryPoint> {
    let mut runs: Vec<&RunRecord> = serious_records(progress).collect();
    runs.sort_by_key(|r| r.timestamp_unix);
    runs.into_iter()
        .map(|r| ScoreHistoryPoint {
            score: r.total_score_earned,
            victory: matches!(r.outcome, RunOutcome::Victory),
        })
        .collect()
}

pub fn career_average_score(progress: &PlayerProgress) -> u64 {
    let runs: Vec<&RunRecord> = serious_records(progress).collect();
    if runs.is_empty() {
        return 0;
    }
    runs.iter().map(|r| r.total_score_earned).sum::<u64>() / runs.len() as u64
}

pub fn career_ordeal_records(progress: &PlayerProgress) -> Vec<OrdealRecordRow> {
    use std::collections::HashMap;
    let mut by_ordeal: HashMap<OrdealKind, (u32, u32, u64)> = HashMap::new();
    for r in serious_records(progress) {
        let Some(boss) = r.final_ordeal else {
            continue;
        };
        let e = by_ordeal.entry(boss).or_insert((0, 0, 0));
        match r.outcome {
            RunOutcome::Victory => e.0 += 1,
            RunOutcome::Defeat { .. } => e.1 += 1,
        }
        if r.total_score_earned > e.2 {
            e.2 = r.total_score_earned;
        }
    }
    let mut rows: Vec<OrdealRecordRow> = by_ordeal
        .into_iter()
        .map(|(ordeal, (wins, losses, best_score))| OrdealRecordRow {
            ordeal,
            wins,
            losses,
            best_score,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.losses
            .cmp(&a.losses)
            .then_with(|| b.wins.cmp(&a.wins))
            .then_with(|| a.ordeal.name().cmp(b.ordeal.name()))
    });
    rows.truncate(6);
    rows
}

pub fn career_ante_outcome_matrix(progress: &PlayerProgress) -> Vec<WingOutcomeCell> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for r in serious_records(progress) {
        let e = map.entry(r.final_wing).or_insert((0, 0));
        match r.outcome {
            RunOutcome::Victory => e.0 += 1,
            RunOutcome::Defeat { .. } => e.1 += 1,
        }
    }
    map.into_iter()
        .map(|(wing, (wins, losses))| WingOutcomeCell { wing, wins, losses })
        .collect()
}

pub fn career_top_yaku(progress: &PlayerProgress, limit: usize) -> Vec<(YakuKind, u32)> {
    let mut yaku: Vec<(YakuKind, u32)> = progress
        .yaku_times_scored
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    yaku.sort_by(|a, b| b.1.cmp(&a.1));
    yaku.truncate(limit);
    yaku
}

pub fn signature_hand_yaku_tags(rec: &RunRecord) -> Vec<YakuKind> {
    let mut tags: Vec<(YakuKind, u32)> = rec
        .yaku_times_played
        .iter()
        .map(|(k, c)| (*k, *c))
        .collect();
    if tags.is_empty() && !rec.best_structure_name.is_empty() {
        return Vec::new();
    }
    tags.sort_by(|a, b| b.1.cmp(&a.1));
    tags.into_iter().take(4).map(|(k, _)| k).collect()
}

pub fn signature_hand_stats(progress: &PlayerProgress, rec: &RunRecord) -> SignatureHandStats {
    let name = rec.best_structure_name.as_str();
    let mut times_used = 0u32;
    let mut score_sum = 0u64;
    for r in serious_records(progress) {
        if r.best_structure_name == name && r.best_structure_score > 0 {
            times_used += 1;
            score_sum += r.best_structure_score;
        }
    }
    let avg_score = if times_used > 0 {
        score_sum / times_used as u64
    } else {
        rec.best_structure_score
    };
    SignatureHandStats {
        yaku_tags: signature_hand_yaku_tags(rec),
        times_used,
        avg_score,
    }
}

pub fn chronicle_run_outcome_short(rec: &RunRecord) -> &'static str {
    match rec.outcome {
        RunOutcome::Victory => "Win",
        RunOutcome::Defeat { .. } => "Loss",
    }
}

pub fn career_kpi_strip(progress: &PlayerProgress) -> Vec<CareerKpi> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    if serious.is_empty() {
        return vec![CareerKpi {
            label: "Chronicle",
            value: "No runs yet".into(),
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
        max_ante = max_ante.max(r.final_wing);
        if r.total_score_earned >= best_run {
            best_run = r.total_score_earned;
            best_run_num = r.run_number;
        }
    }
    let win_pct = (wins as f32 / n as f32 * 100.0).round() as u32;
    vec![
        CareerKpi {
            label: "Best run",
            value: format!("{} · #{best_run_num}", format_chips_compact(best_run)),
        },
        CareerKpi {
            label: "Win rate",
            value: format!("{win_pct}% · {wins}/{n}"),
        },
        CareerKpi {
            label: "Peak wing",
            value: format_wing(max_ante),
        },
        CareerKpi {
            label: "Total score",
            value: format_chips_compact(total_score),
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
        ("< 10k cp", 0, 10_000),
        ("10k–50k cp", 10_000, 50_000),
        ("50k–100k cp", 50_000, 100_000),
        ("100k–250k cp", 100_000, 250_000),
        ("250k+ cp", 250_000, u64::MAX),
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

/// Compact career tile for the Chronicle ledger (right pane).
#[derive(Clone, Debug)]
pub struct CareerTile {
    pub label: &'static str,
    pub value: String,
    pub detail: Option<String>,
    /// When set, the value line is drawn as a bone-tablet pill instead of plain text.
    pub yaku: Option<YakuKind>,
}

/// Up to four career highlights for the 2×2 tile grid.
pub fn career_tiles(progress: &PlayerProgress) -> Vec<CareerTile> {
    let serious: Vec<&RunRecord> = serious_records(progress).collect();
    if serious.is_empty() {
        return vec![CareerTile {
            label: "Chronicle",
            value: "Awaiting first run".into(),
            detail: None,
            yaku: None,
        }];
    }

    let mut tiles = Vec::new();

    // Signature hand is shown in the wide hero row on the career pane, not here.

    if let Some((yk, n)) = progress.yaku_times_scored.iter().max_by_key(|(_, c)| *c) {
        tiles.push(CareerTile {
            label: "Favorite yaku",
            value: String::new(),
            detail: Some(format!("{n} scored")),
            yaku: Some(*yk),
        });
    }

    if let Some((boss, losses, wins)) = nemesis_line(progress) {
        tiles.push(CareerTile {
            label: "Nemesis",
            value: boss.name().into(),
            detail: Some(format!("{losses}L · best {wins}W")),
            yaku: None,
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
            yaku: None,
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
    format!("Ended · {}", dt.format("%A, %B %d, %Y · %-I:%M %p"))
}

#[derive(Clone, Debug)]
pub struct RunDetailModel {
    pub title_pins: String,
    pub title_rest: String,
    pub timestamp_line: String,
    pub signature_name: String,
    pub signature_score: u64,
    pub tiles: Vec<Tile>,
    pub tiles_representative: bool,
    pub yaku_rows: Vec<(YakuKind, u32)>,
    /// Bar value suffix for [`Self::yaku_rows`] (`" han"` or `"×"`).
    pub yaku_value_suffix: &'static str,
    pub score_lines: Vec<String>,
    pub wing_scores: Vec<(u32, u64)>,
    pub timeline: Vec<(u32, String, String)>,
    pub footer: Vec<(String, String)>,
}

pub fn run_detail_model(
    progress: &PlayerProgress,
    display_num: u32,
    rec: &RunRecord,
) -> RunDetailModel {
    let (title_pins, title_rest) = chronicle_run_log_title_parts(progress, display_num, rec);
    let timestamp_line = format_run_ended_timestamp(rec.timestamp_unix);

    let mut tiles = rec
        .chronicle
        .signature_hand
        .as_ref()
        .map(|s| s.tiles.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| rec.best_hand_tiles.clone());
    let mut tiles_representative = false;
    if tiles.is_empty()
        && let Some(yk) = run_dominant_yaku(rec)
    {
        tiles = yaku_page_tiles(yk);
        tiles_representative = !tiles.is_empty();
    }

    let (mut yaku_rows, yaku_value_suffix): (Vec<(YakuKind, u32)>, &str) =
        if !rec.chronicle.yaku_contributions.is_empty() {
            (
                rec.chronicle
                    .yaku_contributions
                    .iter()
                    .map(|(k, v)| (*k, v.han))
                    .collect(),
                " han",
            )
        } else {
            (
                rec.yaku_times_played
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect(),
                "×",
            )
        };
    yaku_rows.sort_by(|a, b| b.1.cmp(&a.1));

    let mut score_lines = Vec::new();
    if let Some(snap) = rec.chronicle.terminal_score.as_ref() {
        score_lines.push(format!(
            "Base {} chips",
            format_score(snap.base_chips as u64)
        ));
        score_lines.push(format!(
            "Yaku +{} chips",
            format_score(snap.yaku_chips as u64)
        ));
        score_lines.push(format!(
            "Dora +{} chips",
            format_score(snap.dora_chips as u64)
        ));
        if snap.relic_chips > 0 {
            score_lines.push(format!(
                "Relics +{} chips",
                format_score(snap.relic_chips as u64)
            ));
        }
        score_lines.push(format!(
            "Boss mult ×{:.2} · Season mult ×{:.2}",
            snap.boss_mult_factor, snap.season_mult_factor
        ));
        score_lines.push(format!("Total {} chips", format_score(snap.total)));
    } else {
        score_lines.push(format!(
            "Round {} / {} chips target",
            format_score(rec.round_score),
            format_score(rec.target_score as u64),
        ));
        score_lines.push(format_chips(rec.total_score_earned));
    }
    score_lines.push(format!(
        "{} · {}",
        rec.tile_material.label(),
        rec.season.label()
    ));
    if let Some(tier) = rec.chronicle.victory_tier {
        score_lines.push(format!("Victory tier · {}", tier.label()));
    }

    let wing_scores = if rec.score_after_wing.is_empty() {
        vec![(rec.final_wing, rec.total_score_earned)]
    } else {
        rec.score_after_wing.clone()
    };

    let timeline = encounter_timeline(rec);
    let c = &rec.chronicle;
    let footer = vec![
        ("Turns".into(), format!("{}", c.turns_total)),
        ("Drawn".into(), format!("{}", c.tiles_drawn)),
        ("Discarded".into(), format!("{}", rec.tiles_discarded)),
        ("Shops".into(), format!("{}", c.shops_visited)),
        ("Restocks".into(), format!("{}", c.restocks_used)),
        ("Relic triggers".into(), format!("{}", c.relic_triggers)),
        ("Yen earned".into(), format!("¥{}", c.yen_earned)),
        (
            "Best combo".into(),
            format_han(
                rec.chronicle
                    .best_combo_han
                    .max((rec.best_structure_score / 100).min(u32::MAX as u64) as u32),
            ),
        ),
    ];

    RunDetailModel {
        title_pins,
        title_rest,
        timestamp_line,
        signature_name: rec.best_structure_name.clone(),
        signature_score: rec.best_structure_score,
        tiles,
        tiles_representative,
        yaku_rows,
        yaku_value_suffix,
        score_lines,
        wing_scores,
        timeline,
        footer,
    }
}

fn encounter_timeline(rec: &RunRecord) -> Vec<(u32, String, String)> {
    if !rec.chronicle.encounters.is_empty() {
        return rec
            .chronicle
            .encounters
            .iter()
            .map(|e| {
                let blind = if let Some(boss) = &e.ordeal_name {
                    format!("{} ({})", e.chamber_label, boss)
                } else {
                    e.chamber_label.clone()
                };
                let note = if e.reward_note.is_empty() {
                    e.outcome.clone()
                } else {
                    format!("{} · {}", e.outcome, e.reward_note)
                };
                (e.wing, blind, note)
            })
            .collect();
    }
    lite_encounter_timeline(rec)
}

fn lite_encounter_timeline(rec: &RunRecord) -> Vec<(u32, String, String)> {
    let mut rows = Vec::new();
    for ante in 1..=rec.final_wing.max(1) {
        let blind = if ante == rec.final_wing {
            rec.final_chamber.name().to_string()
        } else {
            "Cleared".into()
        };
        let note = if ante == rec.final_wing {
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
    use crate::scenes::guide::yaku_page;
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

pub use crate::ui::score_format::format_score;

/// Score with unit for Chronicle copy (career ledger, run detail).
pub fn format_chips(n: u64) -> String {
    format!("{} chips", format_score(n))
}

/// Compact score magnitude for narrow Chronicle columns and chart callouts.
///
/// Values over 999 use the three most significant digits with a `k` / `M` / `B` suffix
/// (e.g. `1_000` → `1.00 k`, `1_000_000` → `1.00 M`).
fn format_score_magnitude(n: u64) -> String {
    if n <= 999 {
        return n.to_string();
    }

    let (divisor, suffix) = if n >= 1_000_000_000 {
        (1_000_000_000.0, "B")
    } else if n >= 1_000_000 {
        (1_000_000.0, "M")
    } else {
        (1_000.0, "k")
    };

    let v = n as f64 / divisor;
    if v >= 100.0 {
        format!("{:.0} {suffix}", v.round())
    } else if v >= 10.0 {
        format!("{:.1} {suffix}", v)
    } else {
        format!("{:.2} {suffix}", v)
    }
}

/// Compact score for the narrow run-log column (magnitude + `cp` chips abbrev).
pub fn format_run_log_score(n: u64) -> String {
    format!("{} cp", format_score_magnitude(n))
}

/// Compact score callout for chart stat panels and ordeal rows.
pub fn format_chips_compact(n: u64) -> String {
    format!("{} cp", format_score_magnitude(n))
}

/// Wing index with a clear prefix (`W4`).
pub fn format_wing(wing: u32) -> String {
    format!("W{wing}")
}

/// Han count with unit (`206 han`).
pub fn format_han(han: u32) -> String {
    format!("{han} han")
}

#[cfg(test)]
mod tests {
    use super::{career_ordeal_records, chronicle_run_log_title, format_run_ended_timestamp};
    use crate::core::ordeal::OrdealKind;
    use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
    use crate::core::rules::ChamberKind;
    use crate::core::season::Season;
    use crate::game::event_bus::GameOverReason;
    use crate::persistence::TileMaterial;

    fn defeat_against(boss: OrdealKind, score: u64) -> RunRecord {
        defeat_against_hand(boss, score, 0)
    }

    fn defeat_against_hand(boss: OrdealKind, score: u64, best_hand: u64) -> RunRecord {
        RunRecord {
            timestamp_unix: 0,
            run_number: 1,
            outcome: RunOutcome::Defeat {
                reason: GameOverReason::OutOfPlays,
            },
            final_wing: 4,
            final_chamber: ChamberKind::Ordeal,
            final_ordeal: Some(boss),
            round_score: score,
            target_score: 500,
            total_score_earned: score,
            final_yen: 0,
            plays_remaining: 0,
            discards_remaining: 0,
            plays_max: 4,
            discards_max: 4,
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: best_hand,
            best_structure_name: String::new(),
            yaku_times_played: Default::default(),
            relics_owned: vec![],
            consumables_owned: vec![],
            tile_material: TileMaterial::Bamboo,
            season: Season::Spring,
            tutorial_run: false,
            memorial_kind: None,
            best_hand_tiles: vec![],
            score_after_wing: vec![],
            chronicle: crate::core::run_chronicle::RunChronicle::default(),
            duration_secs: 0,
        }
    }

    #[test]
    fn career_ordeal_records_breaks_wl_ties_by_name() {
        let mut progress = PlayerProgress::new();
        for _ in 0..5 {
            progress
                .run_history
                .push(defeat_against(OrdealKind::Whisper, 52_371));
        }
        for _ in 0..3 {
            progress
                .run_history
                .push(defeat_against(OrdealKind::Gate, 34_047));
            progress
                .run_history
                .push(defeat_against(OrdealKind::Drought, 8_769));
        }

        let rows = career_ordeal_records(&progress);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].ordeal, OrdealKind::Whisper);
        assert_eq!(rows[1].ordeal, OrdealKind::Drought);
        assert_eq!(rows[2].ordeal, OrdealKind::Gate);
    }

    #[test]
    fn chronicle_score_formatters_include_units() {
        assert_eq!(super::format_chips(40_368), "40,368 chips");
        assert_eq!(super::format_run_log_score(999), "999 cp");
        assert_eq!(super::format_run_log_score(1_000), "1.00 k cp");
        assert_eq!(super::format_run_log_score(72_000), "72.0 k cp");
        assert_eq!(super::format_run_log_score(347_000), "347 k cp");
        assert_eq!(super::format_chips_compact(1_500_000), "1.50 M cp");
        assert_eq!(super::format_run_log_score(1_000_000), "1.00 M cp");
        assert_eq!(super::format_wing(4), "W4");
        assert_eq!(super::format_han(206), "206 han");
    }

    #[test]
    fn chronicle_run_log_title_skips_medals_for_lone_run() {
        let mut progress = PlayerProgress::new();
        progress
            .run_history
            .push(defeat_against(OrdealKind::Whisper, 10_000));
        let rec = &progress.run_history[0];
        let title = chronicle_run_log_title(&progress, 1, rec);
        assert!(!title.contains("🥇"));
        assert!(!title.contains("🥈"));
        assert!(!title.contains("🥉"));
        assert_eq!(title, "Run 1 — Defeat — The Whisper");
    }

    #[test]
    fn chronicle_best_hand_pin_uses_mahjong_red_dragon_glyph() {
        let font = crate::render::decal::load_noto_emoji_font().expect("Noto Emoji");
        assert!(
            font.has_glyph('\u{1F004}'),
            "expected Noto Emoji to cover mahjong red-dragon tile for best-hand pin"
        );
    }

    #[test]
    fn chronicle_run_log_title_shows_best_hand_pin_on_signature_pr() {
        let mut progress = PlayerProgress::new();
        progress
            .run_history
            .push(defeat_against_hand(OrdealKind::Whisper, 10_000, 50_000));
        progress
            .run_history
            .push(defeat_against_hand(OrdealKind::Gate, 20_000, 30_000));
        let pr_hand = &progress.run_history[0];
        let title = chronicle_run_log_title(&progress, 2, pr_hand);
        assert!(
            title.contains('\u{1F004}'),
            "expected best-hand pin: {title}"
        );
        let other = &progress.run_history[1];
        let other_title = chronicle_run_log_title(&progress, 1, other);
        assert!(
            !other_title.contains('\u{1F004}'),
            "only the PR hand run should get the pin: {other_title}"
        );
    }

    #[test]
    fn chronicle_run_log_title_shows_score_rank_medals_for_top_three() {
        let mut progress = PlayerProgress::new();
        progress
            .run_history
            .push(defeat_against(OrdealKind::Whisper, 10_000));
        progress
            .run_history
            .push(defeat_against(OrdealKind::Gate, 5_000));
        progress
            .run_history
            .push(defeat_against(OrdealKind::Drought, 1_000));
        let best = &progress.run_history[0];
        let title = chronicle_run_log_title(&progress, 3, best);
        assert!(
            title.starts_with("🥇 "),
            "expected 1st-place medal: {title}"
        );
        assert!(title.contains("Run 3 — Defeat"));
        let second = &progress.run_history[1];
        let second_title = chronicle_run_log_title(&progress, 2, second);
        assert!(
            second_title.starts_with("🥈 "),
            "expected 2nd-place medal: {second_title}"
        );
        let third = &progress.run_history[2];
        let third_title = chronicle_run_log_title(&progress, 1, third);
        assert!(
            third_title.starts_with("🥉 "),
            "expected 3rd-place medal: {third_title}"
        );
    }

    #[test]
    fn run_ended_timestamp_is_human_readable() {
        let s = format_run_ended_timestamp(1_779_453_032);
        assert!(s.starts_with("Ended · "));
        assert!(!s.contains("unix"));
        assert!(s.contains("2026"));
    }

    #[test]
    fn run_ended_timestamp_unknown_when_zero() {
        assert_eq!(format_run_ended_timestamp(0), "Ended · time unknown");
    }
}
