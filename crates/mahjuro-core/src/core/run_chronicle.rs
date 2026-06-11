//! Per-run analytics captured for the Archive Chronicle ledger.
//!
//! [`RunChronicle`] accumulates on [`crate::game::run::RunState`] during play and
//! is frozen into [`crate::core::progression::RunRecord`] when a run ends.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::core::chamber_target::FINAL_WING;
use crate::core::consumable::Consumable;
use crate::core::rules::ChamberKind;
use crate::core::scoring::{ScoreBreakdown, StepKind};
use crate::core::season::Season;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;

/// Suit buckets for the discard distribution chart.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscardBySuit {
    pub manzu: u32,
    pub pinzu: u32,
    pub souzu: u32,
    pub honors: u32,
}

/// One wing/chamber row in the encounter history table.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunEncounterRecord {
    #[serde(alias = "ante")]
    pub wing: u32,
    #[serde(alias = "blind_label")]
    pub chamber_label: String,
    #[serde(default)]
    #[serde(alias = "boss")]
    pub ordeal_name: Option<String>,
    pub outcome: String,
    #[serde(default)]
    pub reward_note: String,
}

/// Final or best hand snapshot for the signature-hand panel.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignatureHandRecord {
    pub tiles: Vec<Tile>,
    #[serde(default)]
    pub yaku: Vec<YakuKind>,
    /// Display "han" total — sum of yaku mult bonuses × 2 (UI convention).
    #[serde(default)]
    pub yaku_han_total: u32,
    #[serde(default)]
    pub dora_count: u32,
    #[serde(default)]
    pub aka_dora_count: u32,
    #[serde(default)]
    pub ura_dora_count: u32,
}

/// Score breakdown lines for the Chronicle detail panel.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ChronicleScoreSnapshot {
    pub base_chips: i32,
    pub yaku_chips: i32,
    pub dora_chips: i32,
    pub relic_chips: i32,
    pub boss_mult_factor: f64,
    pub season_mult_factor: f64,
    pub streak_mult_factor: f64,
    pub total: u64,
}

/// Per-yaku contribution across the run (counts + display han from mult).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct YakuRunContribution {
    pub times: u32,
    pub han: u32,
}

/// Victory quality tier for Chronicle headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VictoryTier {
    #[default]
    Standard,
    High,
    Exceptional,
}

impl VictoryTier {
    pub fn label(self) -> &'static str {
        match self {
            VictoryTier::Standard => "Standard",
            VictoryTier::High => "High",
            VictoryTier::Exceptional => "Exceptional",
        }
    }
}

/// Live per-run chronicle accumulator (persisted on save, copied to [`RunRecord`]).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunChronicle {
    #[serde(default)]
    pub started_unix: u64,
    /// RNG seed for this run (display via [`format_run_seed`]).
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub turns_total: u32,
    #[serde(default)]
    pub tiles_drawn: u32,
    #[serde(default)]
    pub shops_visited: u32,
    #[serde(default, alias = "rerolls_used")]
    pub restocks_used: u32,
    #[serde(default)]
    pub relic_triggers: u32,
    /// Total yen gained during the run (not ending balance).
    #[serde(default)]
    pub yen_earned: u32,
    #[serde(default)]
    pub discards_by_suit: DiscardBySuit,
    /// Per-face discard counts (`Tile::label()` keys) for rarity coloring.
    #[serde(default)]
    pub discards_by_face: BTreeMap<String, u32>,
    #[serde(default)]
    pub yaku_contributions: HashMap<YakuKind, YakuRunContribution>,
    #[serde(default)]
    pub encounters: Vec<RunEncounterRecord>,
    #[serde(default)]
    pub consumables_used: Vec<Consumable>,
    #[serde(default)]
    pub signature_hand: Option<SignatureHandRecord>,
    #[serde(default)]
    pub terminal_score: Option<ChronicleScoreSnapshot>,
    /// Peak display-han from a single scoring action this run.
    #[serde(default)]
    pub best_combo_han: u32,
    #[serde(default)]
    pub milestones: Vec<String>,
    #[serde(default)]
    pub victory_tier: Option<VictoryTier>,
    /// Round score when each blind ended (cleared, skipped, or failed).
    #[serde(default)]
    pub chamber_scores: Vec<u64>,
}

impl RunChronicle {
    pub fn new_run(seed: u64, started_unix: u64) -> Self {
        Self {
            seed,
            started_unix,
            ..Self::default()
        }
    }

    pub fn note_turn(&mut self) {
        self.turns_total = self.turns_total.saturating_add(1);
    }

    pub fn note_tiles_drawn(&mut self, n: u32) {
        self.tiles_drawn = self.tiles_drawn.saturating_add(n);
    }

    pub fn note_shop_visit(&mut self) {
        self.shops_visited = self.shops_visited.saturating_add(1);
    }

    pub fn note_restock(&mut self) {
        self.restocks_used = self.restocks_used.saturating_add(1);
    }

    pub fn note_relic_trigger(&mut self) {
        self.relic_triggers = self.relic_triggers.saturating_add(1);
    }

    pub fn note_yen_earned(&mut self, delta: i32) {
        if delta > 0 {
            self.yen_earned = self.yen_earned.saturating_add(delta as u32);
        }
    }

    pub fn note_discarded_tile(&mut self, tile: &Tile) {
        match tile.suit {
            Suit::Manzu => {
                self.discards_by_suit.manzu = self.discards_by_suit.manzu.saturating_add(1)
            }
            Suit::Pinzu => {
                self.discards_by_suit.pinzu = self.discards_by_suit.pinzu.saturating_add(1)
            }
            Suit::Souzu => {
                self.discards_by_suit.souzu = self.discards_by_suit.souzu.saturating_add(1)
            }
            Suit::Wind | Suit::Dragon | Suit::Flower | Suit::Season => {
                self.discards_by_suit.honors = self.discards_by_suit.honors.saturating_add(1);
            }
        }
        *self
            .discards_by_face
            .entry(tile.label().to_string())
            .or_insert(0) += 1;
    }

    pub fn note_consumable_used(&mut self, item: Consumable) {
        self.consumables_used.push(item);
    }

    pub fn record_chamber_cleared(
        &mut self,
        ante: u32,
        blind: ChamberKind,
        boss: Option<&str>,
        reward_note: String,
        round_score: u64,
    ) {
        self.chamber_scores.push(round_score);
        self.encounters.push(RunEncounterRecord {
            wing: ante,
            chamber_label: blind.name().to_string(),
            ordeal_name: boss.map(str::to_string),
            outcome: "Cleared".into(),
            reward_note,
        });
    }

    pub fn record_chamber_skipped(&mut self, ante: u32, blind: ChamberKind, reward_note: String) {
        self.chamber_scores.push(0);
        self.encounters.push(RunEncounterRecord {
            wing: ante,
            chamber_label: blind.name().to_string(),
            ordeal_name: None,
            outcome: "Skipped".into(),
            reward_note,
        });
    }

    pub fn record_run_end_defeat(
        &mut self,
        ante: u32,
        blind: ChamberKind,
        boss: Option<&str>,
        round_score: u64,
    ) {
        if self
            .encounters
            .last()
            .is_some_and(|e| e.wing == ante && e.outcome == "Defeated")
        {
            return;
        }
        self.chamber_scores.push(round_score);
        self.encounters.push(RunEncounterRecord {
            wing: ante,
            chamber_label: blind.name().to_string(),
            ordeal_name: boss.map(str::to_string),
            outcome: "Defeated".into(),
            reward_note: String::new(),
        });
    }

    pub fn absorb_scoring(
        &mut self,
        breakdown: &ScoreBreakdown,
        tiles: &[Tile],
        yaku_levels: &crate::core::zodiac::YakuLevels,
    ) {
        let hand_han: u32 = breakdown
            .detected_yaku
            .iter()
            .map(|&y| yaku_display_han(y, yaku_levels.level_of(y)))
            .sum();
        self.best_combo_han = self.best_combo_han.max(hand_han);

        for &y in &breakdown.detected_yaku {
            let level = yaku_levels.level_of(y);
            let entry = self.yaku_contributions.entry(y).or_default();
            entry.times = entry.times.saturating_add(1);
            entry.han = entry.han.saturating_add(yaku_display_han(y, level));
        }

        let sig = signature_from_breakdown(breakdown, tiles);
        if self
            .signature_hand
            .as_ref()
            .map(|s| sig.yaku_han_total > s.yaku_han_total)
            .unwrap_or(true)
        {
            self.signature_hand = Some(sig);
        }
    }

    pub fn set_terminal_breakdown(&mut self, breakdown: &ScoreBreakdown, season: Season) {
        self.terminal_score = Some(score_snapshot_from_breakdown(breakdown, season));
    }

    pub fn finalize_for_outcome(
        &mut self,
        victory: bool,
        total_score: u64,
        final_wing: u32,
        plays_remaining: u32,
    ) {
        if victory {
            self.victory_tier = Some(victory_tier_for(total_score, final_wing));
            self.milestones = compute_milestones(self, total_score, final_wing, plays_remaining);
        }
    }
}

/// Four-group seed string for the Chronicle header (`4KF7-2N8J-Q9D1` style).
pub fn format_run_seed(seed: u64) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut n = seed;
    if n == 0 {
        n = 1;
    }
    let mut chars = Vec::with_capacity(12);
    for _ in 0..12 {
        let idx = (n % 32) as usize;
        chars.push(ALPHABET[idx] as char);
        n /= 32;
    }
    chars.reverse();
    format!(
        "{}-{}-{}",
        chars[0..4].iter().collect::<String>(),
        chars[4..8].iter().collect::<String>(),
        chars[8..12].iter().collect::<String>(),
    )
}

fn yaku_display_han(y: YakuKind, level: u32) -> u32 {
    (y.mult_bonus_at(level) * 2.0).round().max(1.0) as u32
}

fn count_dora_in_source(source: &str) -> u32 {
    source
        .strip_prefix("Dora ×")
        .and_then(|s| s.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

pub fn signature_from_breakdown(breakdown: &ScoreBreakdown, tiles: &[Tile]) -> SignatureHandRecord {
    let mut dora_count = 0u32;
    for step in &breakdown.steps {
        if step.source.starts_with("Dora ×") {
            dora_count = dora_count.saturating_add(count_dora_in_source(&step.source));
        }
    }
    SignatureHandRecord {
        tiles: tiles.iter().map(|t| t.display_copy()).collect(),
        yaku: breakdown.detected_yaku.clone(),
        yaku_han_total: breakdown
            .detected_yaku
            .iter()
            .map(|&y| yaku_display_han(y, 1))
            .sum(),
        dora_count,
        aka_dora_count: 0,
        ura_dora_count: 0,
    }
}

fn score_snapshot_from_breakdown(
    breakdown: &ScoreBreakdown,
    season: Season,
) -> ChronicleScoreSnapshot {
    let mut yaku_chips = 0i32;
    let mut dora_chips = 0i32;
    let mut relic_chips = 0i32;
    let mut prev_chips = breakdown.base_chips;
    let mut boss_mult = 1.0f64;
    let mut streak_mult = 1.0f64;

    for step in &breakdown.steps {
        let delta_chips = step.running_chips - prev_chips;
        if step.source.starts_with("Dora") {
            dora_chips += delta_chips;
        } else if breakdown
            .detected_yaku
            .iter()
            .any(|y| step.source.starts_with(y.name()))
        {
            yaku_chips += delta_chips;
        } else if step.kind == StepKind::Chips && delta_chips != 0 {
            relic_chips += delta_chips;
        }
        if (step.source.contains("Boss") || step.source.contains("boss"))
            && step.kind == StepKind::Mult
            && delta_chips == 0
        {
            let mult_delta = step.running_mult / prev_chips.max(1) as f64;
            if mult_delta > 1.0 {
                boss_mult = mult_delta;
            }
        }
        if step.source.contains("Chain") || step.source.contains("Streak") {
            streak_mult = step.running_mult;
        }
        prev_chips = step.running_chips;
    }

    ChronicleScoreSnapshot {
        base_chips: breakdown.base_chips,
        yaku_chips,
        dora_chips,
        relic_chips,
        boss_mult_factor: boss_mult,
        season_mult_factor: season.base_target_mult() as f64,
        streak_mult_factor: streak_mult.max(1.0),
        total: breakdown.total,
    }
}

fn victory_tier_for(total_score: u64, final_wing: u32) -> VictoryTier {
    if final_wing >= FINAL_WING && total_score >= 50_000 {
        VictoryTier::Exceptional
    } else if total_score >= 25_000 || final_wing >= 6 {
        VictoryTier::High
    } else {
        VictoryTier::Standard
    }
}

fn compute_milestones(
    chronicle: &RunChronicle,
    total_score: u64,
    final_wing: u32,
    plays_remaining: u32,
) -> Vec<String> {
    let mut tags = Vec::new();
    if total_score >= 30_000 {
        tags.push("High Score".into());
    }
    if final_wing >= FINAL_WING && plays_remaining >= 2 {
        tags.push("Speed Clear".into());
    }
    if chronicle.best_combo_han >= 12 {
        tags.push("High Han".into());
    }
    if chronicle.discards_by_suit.honors == 0
        && chronicle.discards_by_suit.manzu
            + chronicle.discards_by_suit.pinzu
            + chronicle.discards_by_suit.souzu
            > 0
    {
        tags.push("No Honors Discarded".into());
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chamber_scores_record_on_clear_and_defeat() {
        use crate::core::rules::ChamberKind;
        let mut c = RunChronicle::default();
        c.record_chamber_cleared(1, ChamberKind::Small, None, String::new(), 420);
        c.record_chamber_cleared(1, ChamberKind::Big, None, String::new(), 880);
        c.record_run_end_defeat(1, ChamberKind::Ordeal, Some("The Whisper"), 210);
        assert_eq!(c.chamber_scores, vec![420, 880, 210]);
    }

    #[test]
    fn format_run_seed_is_grouped() {
        let s = format_run_seed(0xDEAD_BEEF_CAFE);
        assert_eq!(s.len(), 14);
        assert_eq!(s.as_bytes()[4], b'-');
        assert_eq!(s.as_bytes()[9], b'-');
    }

    #[test]
    fn discard_tally_buckets_suits() {
        let mut c = RunChronicle::default();
        c.note_discarded_tile(&Tile::new(Suit::Manzu, 3, 1));
        c.note_discarded_tile(&Tile::new(Suit::Dragon, 1, 2));
        assert_eq!(c.discards_by_suit.manzu, 1);
        assert_eq!(c.discards_by_suit.honors, 1);
    }
}
