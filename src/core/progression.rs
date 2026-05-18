//! Meta progression and unlocks.

use std::collections::{BTreeMap, BTreeSet};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::{Deserialize, Serialize};

use crate::core::boss::BossKind;
use crate::core::consumable::Consumable;
use crate::core::relic::RelicId;
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::stake::Stake;
use crate::core::talisman::TalismanKind;
use crate::core::yaku::YakuKind;
use crate::game::event_bus::GameOverReason;
use crate::persistence::TileMaterial;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerProgress {
    pub unlocked_relics: HashSet<RelicId>,
    pub unlocked_rules: HashSet<RuleModifier>,
    pub high_scores: Vec<u64>,
    /// Total runs completed (drives level progression).
    #[serde(default)]
    pub runs_completed: u32,
    /// Extra plays per round from permanent upgrades.
    #[serde(default)]
    pub bonus_plays: u32,
    /// Extra starting relics at run start.
    #[serde(default)]
    pub starting_relic_slots: u32,
    /// Whether the player has completed (or skipped) the tutorial.
    #[serde(default)]
    pub tutorial_completed: bool,
    /// Whether the player has ever won a run (defeated the final boss).
    /// Unlocks the Plastic tile material.
    #[serde(default)]
    pub has_won: bool,
    /// Times each boss was selected via PlayBlind on a Boss blind. Keys
    /// present in this map are "encountered" and appear in the Collection's
    /// Bosses tab; unseen bosses stay hidden.
    #[serde(default)]
    pub boss_times_encountered: HashMap<BossKind, u32>,
    /// Times each boss was defeated (blind cleared with
    /// `reached_target: true` while `self.blind == Boss`).
    #[serde(default)]
    pub boss_times_defeated: HashMap<BossKind, u32>,
    /// Times each talisman was purchased from the shop. Pack-acquired or
    /// granted talismans don't count. Keys gate the Collection's
    /// Talismans tab.
    #[serde(default)]
    pub talisman_times_purchased: HashMap<TalismanKind, u32>,
    /// Times each talisman was consumed (used from the consumable dish).
    /// Strict subset of purchased in practice, but kept separate because
    /// "times used" is the more interesting stat to display.
    #[serde(default)]
    pub talisman_times_used: HashMap<TalismanKind, u32>,
    /// Times each yaku was scored on a committed hand (cash-in). Persisted on
    /// the profile — once non-zero, that yaku stays **name-revealed** in the
    /// journal and on gameplay tablets for all later runs. Also keys the
    /// Collection's Yaku tab.
    #[serde(default)]
    pub yaku_times_scored: HashMap<YakuKind, u32>,
    /// Times each relic activated (fired its effect). Drives the Relic
    /// stats plaque; does not gate visibility (relics still use the
    /// level-gated `available_relics()` path for the Collection grid).
    #[serde(default)]
    pub relic_times_activated: HashMap<RelicId, u32>,
    /// Append-only log of every run that reached the defeat or victory
    /// screen. One row per finished run — rage-quits and still-in-progress
    /// runs do not land here. Source of truth for run-outcome analytics;
    /// aggregate rollups (total victories, deaths_by_blind, etc.) can be
    /// computed from this on demand.
    #[serde(default)]
    pub run_history: Vec<RunRecord>,
    /// Successor relics unlocked only after a fragile primary burns (Silk Moth,
    /// Taotie, Geese, Rakuware, Stone Lantern, Monarch Butterfly). Shops, runs, and Collection consult this.
    #[serde(default)]
    pub discovered_transformation_successors: HashSet<RelicId>,
    /// Per-material ladder of cleared stakes. `Spring` is implicitly unlocked
    /// for every material (never written to this map); higher stakes require
    /// a full victory on the previous tier *with that material*. So beating
    /// Summer on Bamboo unlocks Autumn for Bamboo only.
    ///
    /// Backfill logic in `backfill_stakes_from_history` re-derives entries
    /// from `run_history` on load, so older profiles start with the right
    /// unlocks for any victories already in the log.
    #[serde(default)]
    pub unlocked_stakes: BTreeMap<TileMaterial, BTreeSet<Stake>>,
}

/// Relics that never appear in meta level-up and stay out of Collection until a
/// fragile primary burns (or legacy unlock — see [`PlayerProgress::transformation_successor_visible`]).
pub fn transformation_successor_relic_ids() -> &'static [RelicId] {
    &[
        RelicId::SilkMoth,
        RelicId::Taotie,
        RelicId::Geese,
        RelicId::Rakuware,
        RelicId::StoneLantern,
        RelicId::MonarchButterfly,
    ]
}

#[inline]
pub fn is_transformation_successor_relic(id: RelicId) -> bool {
    transformation_successor_relic_ids().contains(&id)
}

/// Terminal outcome of a single run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunOutcome {
    Victory,
    Defeat { reason: GameOverReason },
}

/// Snapshot of a finished run at the moment the defeat or victory screen
/// opened. Schema mirrors `bot::RunStats` where the data exists on
/// `RunState`; append-only so older profiles remain readable (all new
/// fields carry `#[serde(default)]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    /// Seconds since the Unix epoch when the run ended. 0 if the system
    /// clock is unavailable (should never happen in practice).
    #[serde(default)]
    pub timestamp_unix: u64,
    pub run_number: u32,
    pub outcome: RunOutcome,
    // ── Where the run ended ──────────────────────────────────────────
    /// Ante the run ended on. Victory => the cleared final ante;
    /// defeat => the ante the player died on.
    pub final_ante: u32,
    /// Blind the run ended on. Victory => the final Boss blind;
    /// defeat => the blind the player failed to clear.
    pub final_blind: BlindKind,
    /// Specific boss facing the player when the run ended, if any.
    /// Only meaningful when `final_blind == Boss`.
    #[serde(default)]
    pub final_boss: Option<BossKind>,
    // ── Score / resources at end of run ──────────────────────────────
    /// Score on the final round (the losing or winning round).
    pub round_score: u64,
    pub target_score: u32,
    /// Cumulative score earned across the whole run.
    pub total_score_earned: u64,
    pub final_gold: i32,
    pub plays_remaining: u32,
    pub discards_remaining: u32,
    pub plays_max: u32,
    pub discards_max: u32,
    // ── Run-wide counters ────────────────────────────────────────────
    pub tiles_played: u32,
    pub tiles_discarded: u32,
    pub times_restocked: u32,
    pub best_structure_score: u64,
    pub best_structure_name: String,
    /// Per-yaku play counts for this run. Snapshot, not cumulative
    /// across runs (that lives on `PlayerProgress::yaku_times_scored`).
    #[serde(default)]
    pub yaku_times_played: HashMap<YakuKind, u32>,
    // ── Loadout at end of run ────────────────────────────────────────
    pub relics_owned: Vec<RelicId>,
    pub consumables_owned: Vec<Consumable>,
    pub tile_material: TileMaterial,
    /// Stake the run was played on. Older records (pre-stake feature) default
    /// to `Spring` via `#[serde(default)]`, which is the right assumption —
    /// those runs had no higher tiers available.
    #[serde(default)]
    pub stake: Stake,
    /// Whether this run was a tutorial run. Tutorials reach the
    /// defeat/victory screen too, but analytics usually wants to filter
    /// them out.
    #[serde(default)]
    pub tutorial_run: bool,
}

impl PlayerProgress {
    /// Kokushi Musō has been **cashed in** at least once: `GameEvent::YakuScored`
    /// only fires on structure cash-in, so this matches the first successful
    /// Kokushi score (not hand previews or undiscovered state).
    pub fn kokushi_musou_discovered(&self) -> bool {
        self.yaku_times_scored
            .get(&YakuKind::KokushiMusou)
            .copied()
            .unwrap_or(0)
            > 0
    }

    /// Shop Qilin ribbon — same gate as [`Self::kokushi_musou_discovered`].
    pub fn qilin_ribbon_unlocked(&self) -> bool {
        self.kokushi_musou_discovered()
    }

    pub fn new() -> Self {
        Self {
            unlocked_relics: HashSet::default(),
            unlocked_rules: HashSet::default(),
            high_scores: Vec::new(),
            runs_completed: 0,
            bonus_plays: 0,
            starting_relic_slots: 0,
            tutorial_completed: false,
            has_won: false,
            boss_times_encountered: HashMap::default(),
            boss_times_defeated: HashMap::default(),
            talisman_times_purchased: HashMap::default(),
            talisman_times_used: HashMap::default(),
            yaku_times_scored: HashMap::default(),
            relic_times_activated: HashMap::default(),
            run_history: Vec::new(),
            unlocked_stakes: BTreeMap::new(),
            discovered_transformation_successors: HashSet::default(),
        }
    }

    /// Stakes cleared (with a full victory) on the given tile material.
    /// `Spring` is always considered available but is NOT returned here — this
    /// reflects the *earned* ladder, not the *playable* ladder. Callers who
    /// need "is this stake playable?" should use [`stake_unlocked_for`].
    pub fn stakes_cleared_for(&self, material: TileMaterial) -> &BTreeSet<Stake> {
        static EMPTY: BTreeSet<Stake> = BTreeSet::new();
        self.unlocked_stakes.get(&material).unwrap_or(&EMPTY)
    }

    /// Is `stake` playable on `material`? Spring is always available; each
    /// higher stake requires the previous one to have been cleared on this
    /// material (full victory, not just reaching ante 8).
    pub fn stake_unlocked_for(&self, material: TileMaterial, stake: Stake) -> bool {
        match stake {
            Stake::Spring => true,
            _ => {
                let cleared = self.stakes_cleared_for(material);
                // Every stake lower than `stake` must appear in `cleared`.
                Stake::ALL
                    .iter()
                    .take_while(|&&s| s < stake)
                    .skip(1) // skip Spring; it's implicit
                    .all(|s| cleared.contains(s))
                    && {
                        // And the immediately-previous stake, explicitly.
                        stake
                            .previous()
                            .map(|p| cleared.contains(&p))
                            .unwrap_or(true)
                    }
            }
        }
    }

    /// Record a full victory at `stake` on `material` — the gate that unlocks
    /// the next stake for that material. Idempotent; returns `Some(next)` if
    /// a new tier was just freed up, `None` otherwise (already unlocked, or
    /// Winter has no successor).
    pub fn record_stake_victory(&mut self, material: TileMaterial, stake: Stake) -> Option<Stake> {
        let entry = self.unlocked_stakes.entry(material).or_default();
        let newly_inserted = entry.insert(stake);
        // The unlocked ladder advances by one on a successful win of `stake`,
        // so the *next* stake becomes playable. We report it when the victory
        // was new; repeat wins don't re-surface the notification.
        match (newly_inserted, stake.next()) {
            (true, Some(next)) => Some(next),
            _ => None,
        }
    }

    /// Backfill `unlocked_stakes` from `run_history`. Called once on profile
    /// load so older saves that pre-date the Stakes feature surface the right
    /// ladder: every Victory in history populates its (material, stake) pair.
    ///
    /// Non-victory runs are ignored, and Spring victories are still recorded
    /// so downstream "highest stake cleared" readouts can display Spring.
    pub fn backfill_stakes_from_history(&mut self) {
        for record in &self.run_history {
            if matches!(record.outcome, RunOutcome::Victory) {
                self.unlocked_stakes
                    .entry(record.tile_material)
                    .or_default()
                    .insert(record.stake);
            }
        }
    }

    pub fn record_score(&mut self, score: u64) {
        self.high_scores.push(score);
        self.high_scores.sort_by(|a, b| b.cmp(a));
        self.high_scores.truncate(10);
    }

    /// Current progression level (1–14) based on runs completed.
    /// New unlocks land every run for the first six runs, then every other run
    /// until the level-14 catch-all kicks in at 21 runs.
    pub fn current_level(&self) -> u32 {
        match self.runs_completed {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 4,
            4 => 5,
            5..=6 => 6,
            7..=8 => 7,
            9..=10 => 8,
            11..=12 => 9,
            13..=14 => 10,
            15..=16 => 11,
            17..=18 => 12,
            19..=20 => 13,
            _ => 14,
        }
    }

    /// Check if a new level was reached and apply unlocks.
    /// Returns details of what was unlocked, or `None` if nothing new.
    pub fn check_level_up(&mut self) -> Option<LevelUpResult> {
        let level = self.current_level();
        let unlocks = unlocks_for_level(level);

        let mut changed = false;
        let mut new_relics = Vec::new();
        for relic in unlocks.relics {
            if is_transformation_successor_relic(relic) {
                continue;
            }
            if self.unlocked_relics.insert(relic) {
                new_relics.push(relic);
                changed = true;
            }
        }
        let mut new_rules = Vec::new();
        for rule in unlocks.rules {
            if self.unlocked_rules.insert(rule) {
                new_rules.push(rule);
                changed = true;
            }
        }

        // Yaku and dora are level-gated (not tracked in a HashSet), so they
        // are always "new" for their unlock level.
        if !unlocks.yaku.is_empty() || unlocks.dora {
            changed = true;
        }

        if changed {
            Some(LevelUpResult {
                new_level: level,
                relics: new_relics,
                rules: new_rules,
            })
        } else {
            None
        }
    }

    /// Relics available for this player's progression level (shop / run stock).
    /// Transformation successors (Silk Moth, Taotie, Geese, Rakuware, Stone Lantern)
    /// are omitted — they only enter the shop after a primary burns on the
    /// current run; see [`crate::game::run::relic_eligible_for_shop_stock`].
    pub fn available_relics(&self) -> Vec<RelicId> {
        let level = self.current_level();
        let mut available = Vec::new();
        for l in 1..=level {
            for r in unlocks_for_level(l).relics {
                if is_transformation_successor_relic(r) && !self.transformation_successor_visible(r)
                {
                    continue;
                }
                available.push(r);
            }
        }
        available
    }

    /// Collection / shop visibility for a relic id (successors stay hidden until discovered).
    pub fn transformation_successor_visible(&self, id: RelicId) -> bool {
        !is_transformation_successor_relic(id)
            || self.discovered_transformation_successors.contains(&id)
            || (id == RelicId::Geese && self.unlocked_relics.contains(&RelicId::Geese))
            || (id == RelicId::Rakuware && self.unlocked_relics.contains(&RelicId::Rakuware))
            || (id == RelicId::MonarchButterfly
                && self.unlocked_relics.contains(&RelicId::MonarchButterfly))
    }

    /// Record that the player has seen a successor (Collection reveal). Returns true if new.
    pub fn note_transformation_successor_discovered(&mut self, id: RelicId) -> bool {
        if !is_transformation_successor_relic(id) {
            return false;
        }
        self.discovered_transformation_successors.insert(id)
    }

    /// Debug / cheat: reveal every transformation successor in the Collection
    /// and raise `runs_completed` to at least level 11 so all fragile
    /// primaries (through Chrysalis) appear in [`Self::available_relics`].
    /// Does not call [`Self::check_level_up`] (avoids level-up modals).
    pub fn cheat_unlock_all_transformation_chains_meta(&mut self) {
        for &id in transformation_successor_relic_ids() {
            self.discovered_transformation_successors.insert(id);
        }
        // Minimum `runs_completed` for `current_level() >= 11` (Chrysalis tier).
        const MIN_RUNS_FOR_ALL_FRAGILE_PRIMARIES: u32 = 15;
        if self.runs_completed < MIN_RUNS_FOR_ALL_FRAGILE_PRIMARIES {
            self.runs_completed = MIN_RUNS_FOR_ALL_FRAGILE_PRIMARIES;
        }
    }

    /// Rules available for this player's progression level.
    pub fn available_rules(&self) -> Vec<RuleModifier> {
        let level = self.current_level();
        let mut available = Vec::new();
        for l in 1..=level {
            available.extend(unlocks_for_level(l).rules);
        }
        available
    }

    /// Yaku the run may surface in previews and reference UI.
    ///
    /// Kokushi Musō stays **secret** until the first time it is cashed in and
    /// recorded in [`Self::yaku_times_scored`]. Scoring still awards it when
    /// the hand qualifies — the scorer always applies Kokushi when detected,
    /// even if this list omits it.
    pub fn available_yaku(&self) -> Vec<YakuKind> {
        YakuKind::all()
            .iter()
            .copied()
            .filter(|&y| {
                if y == YakuKind::KokushiMusou && !self.kokushi_musou_discovered() {
                    return false;
                }
                true
            })
            .collect()
    }

    /// Whether the Plastic tile material is unlocked (requires first victory).
    pub fn plastic_unlocked(&self) -> bool {
        self.has_won
    }

    /// Whether dora tiles are enabled at this level.
    pub fn dora_enabled(&self) -> bool {
        let level = self.current_level();
        (1..=level).any(|l| unlocks_for_level(l).dora)
    }
}

impl RunRecord {
    /// Capture a run's terminal state into a `RunRecord`. Called from
    /// the App layer when either the victory screen or the defeat screen
    /// is about to open — whichever branch fires first wins.
    pub fn from_run(run: &crate::game::run::RunState, outcome: RunOutcome) -> Self {
        let timestamp_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let final_boss = if run.blind == BlindKind::Boss {
            run.boss.upcoming
        } else {
            None
        };
        Self {
            timestamp_unix,
            run_number: run.run_number,
            outcome,
            final_ante: run.ante,
            final_blind: run.blind,
            final_boss,
            round_score: run.round_score,
            target_score: run.target_score,
            total_score_earned: run.total_score_earned,
            final_gold: run.gold,
            plays_remaining: run.plays_remaining,
            discards_remaining: run.discards_remaining,
            plays_max: run.plays_max,
            discards_max: run.discards_max,
            tiles_played: run.tiles_played,
            tiles_discarded: run.tiles_discarded,
            times_restocked: run.times_restocked,
            best_structure_score: run.best_structure_score,
            best_structure_name: run.best_structure_name.clone(),
            yaku_times_played: run.yaku_times_played.clone(),
            relics_owned: run.relics.active.clone(),
            consumables_owned: run.consumables.items.clone(),
            tile_material: run.mode.tile_material,
            stake: run.mode.stake,
            tutorial_run: false,
        }
    }
}

struct LevelUnlocks {
    relics: Vec<RelicId>,
    rules: Vec<RuleModifier>,
    yaku: Vec<YakuKind>,
    dora: bool,
}

/// What was unlocked when the player leveled up.
pub struct LevelUpResult {
    pub new_level: u32,
    pub relics: Vec<RelicId>,
    pub rules: Vec<RuleModifier>,
}

/// Every relic defined in `relics.json` that no earlier level unlocks. Keeps
/// level 14 a catch-all so newly added relics are reachable without editing
/// this file.
fn level_14_relics() -> Vec<RelicId> {
    let earlier: rustc_hash::FxHashSet<RelicId> =
        (1..=13).flat_map(|l| unlocks_for_level(l).relics).collect();
    crate::core::relic::all_relic_defs()
        .iter()
        .map(|d| d.id)
        .filter(|id| !earlier.contains(id) && !is_transformation_successor_relic(*id))
        .collect()
}

fn unlocks_for_level(level: u32) -> LevelUnlocks {
    match level {
        // ── L1: Set scoring intro + base rule modifiers ──────────────────
        1 => LevelUnlocks {
            relics: vec![
                RelicId::TripletBoost,
                RelicId::SequenceSurge,
                RelicId::PairPower,
            ],
            rules: vec![
                RuleModifier::PairDoubleScore,
                RuleModifier::SequenceWrap,
                RuleModifier::NoSequenceBonus,
                RuleModifier::HonorTripleScore,
            ],
            yaku: vec![],
            dora: false,
        },
        // ── L2: Suit & rank chips (all three Serpents together) ──────────
        2 => LevelUnlocks {
            relics: vec![
                RelicId::JadeSerpent,
                RelicId::RubySerpent,
                RelicId::LapisSerpent,
                RelicId::LowTide,
                RelicId::HighTide,
                RelicId::CrackedTile,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L3: Basic economy + first scoring yaku ───────────────────────
        3 => LevelUnlocks {
            relics: vec![
                RelicId::GoldIdol,
                RelicId::JadeAbacus,
                RelicId::NestEgg,
                RelicId::Patience,
                RelicId::NoHonorButWealth,
                RelicId::Sweepstakes,
                RelicId::GreenLuck,
            ],
            rules: vec![],
            yaku: vec![YakuKind::Toitoi, YakuKind::Tanyao],
            dora: false,
        },
        // ── L4: Tempo & shop + flower intro ──────────────────────────────
        4 => LevelUnlocks {
            relics: vec![
                RelicId::QuickDraw,
                RelicId::MerchantsEye,
                RelicId::IGotAGuy,
                RelicId::Momentum,
                RelicId::Hanami,
                RelicId::GardenKeeper,
                RelicId::GlassCannon,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L5: Set composition + Ikebana flower payoff + Iipeikou/Honitsu
        5 => LevelUnlocks {
            relics: vec![
                RelicId::MultiplierMaster,
                RelicId::SetMagnet,
                RelicId::EdgeRunner,
                RelicId::ChainReaction,
                RelicId::WindReader,
                RelicId::TurtleShell,
                RelicId::Ikebana,
            ],
            rules: vec![],
            yaku: vec![YakuKind::Iipeikou, YakuKind::Honitsu],
            dora: false,
        },
        // ── L6: Dora opens + honors/dragons + Lotus Bloom flower scaler ──
        6 => LevelUnlocks {
            relics: vec![
                RelicId::DoraCrown,
                RelicId::HonorFury,
                RelicId::WhiteDragonsHush,
                RelicId::DragonRage,
                RelicId::StrengthInNumbers,
                RelicId::WallWeaver,
                RelicId::LotusBloom,
            ],
            rules: vec![],
            yaku: vec![YakuKind::Chinitsu, YakuKind::Chiitoitsu],
            dora: true,
        },
        // ── L7: Late-economy uncommons + hand-size modifiers ─────────────
        7 => LevelUnlocks {
            relics: vec![
                RelicId::KongCollector,
                RelicId::BeggarsCup,
                RelicId::Cosmopolitan,
                RelicId::Heirloom,
                RelicId::Tourist,
                RelicId::BigHands,
                RelicId::TinyHands,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L8: Mid scaling specialists + Sanshoku/Honroutou ─────────────
        8 => LevelUnlocks {
            relics: vec![RelicId::Humility, RelicId::Bonfire, RelicId::StarTile],
            rules: vec![],
            yaku: vec![YakuKind::SanshokuDoujun, YakuKind::Honroutou],
            dora: false,
        },
        // ── L9: Specialist rares + Junchan/Ittsu ─────────────────────────
        9 => LevelUnlocks {
            relics: vec![
                RelicId::LuckySeven,
                RelicId::Minimalist,
                RelicId::AntTrail,
                RelicId::SolitarySage,
                RelicId::RiverRunner,
                RelicId::GhostHand,
            ],
            rules: vec![],
            yaku: vec![YakuKind::Junchan, YakuKind::Ittsu],
            dora: false,
        },
        // ── L10: Fragile transforms (burn-into-successor relics) ─────────
        10 => LevelUnlocks {
            relics: vec![
                RelicId::PaperLantern,
                RelicId::MeltingIce,
                RelicId::SilkThread,
                RelicId::TeaCeremony,
                RelicId::RustlingGooseEgg,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L11: Retriggers & polish ─────────────────────────────────────
        11 => LevelUnlocks {
            relics: vec![
                RelicId::TilePolisher,
                RelicId::LastBreath,
                RelicId::VoiceOfThePeople,
                RelicId::VoiceOfTheElite,
                RelicId::Chrysalis,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L12: Conditional ×mult "Way of" relics + tough rules ─────────
        12 => LevelUnlocks {
            relics: vec![
                RelicId::WayOfPurity,
                RelicId::WayOfPairs,
                RelicId::WayOfTriplets,
                RelicId::WayOfSequences,
                RelicId::ClosedGate,
                RelicId::Disgust,
            ],
            rules: vec![RuleModifier::NoSequences, RuleModifier::ReducedPlays],
            yaku: vec![],
            dora: false,
        },
        // ── L13: Late rares (utility / scaling / chaos) ──────────────────
        13 => LevelUnlocks {
            relics: vec![
                RelicId::SecondWind,
                RelicId::GoldenEngine,
                RelicId::Snowball,
                RelicId::Obsession,
                RelicId::KanDrum,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        // ── L14: Capstones (legendaries + the most chaotic rares) ────────
        14 => LevelUnlocks {
            relics: level_14_relics(),
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        _ => LevelUnlocks {
            relics: vec![],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kokushi_omitted_from_available_yaku_until_first_cash_in() {
        use crate::core::yaku::YakuKind;
        let mut p = PlayerProgress::new();
        let ay = p.available_yaku();
        assert!(!ay.contains(&YakuKind::KokushiMusou));
        assert_eq!(ay.len(), YakuKind::all().len() - 1);
        *p.yaku_times_scored
            .entry(YakuKind::KokushiMusou)
            .or_insert(0) += 1;
        let ay2 = p.available_yaku();
        assert!(ay2.contains(&YakuKind::KokushiMusou));
        assert_eq!(ay2.len(), YakuKind::all().len());
    }

    #[test]
    fn level_progression() {
        let mut p = PlayerProgress::new();
        assert_eq!(p.current_level(), 1);
        p.runs_completed = 1;
        assert_eq!(p.current_level(), 2);
        p.runs_completed = 4;
        assert_eq!(p.current_level(), 5);
        p.runs_completed = 6;
        assert_eq!(p.current_level(), 6);
        p.runs_completed = 13;
        assert_eq!(p.current_level(), 10);
        p.runs_completed = 21;
        assert_eq!(p.current_level(), 14);
        p.runs_completed = 100;
        assert_eq!(p.current_level(), 14);
    }

    #[test]
    fn level_up_unlocks_relics() {
        let mut p = PlayerProgress::new();
        // Hop from L1 to L5 in one bump: collect all intervening unlocks.
        p.runs_completed = 4;
        let result = p.check_level_up();
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.new_level, 5);
        assert!(result.relics.contains(&RelicId::MultiplierMaster));
        assert!(p.unlocked_relics.contains(&RelicId::MultiplierMaster));
    }

    #[test]
    fn available_relics_grow_with_level() {
        let mut p = PlayerProgress::new();
        let l1 = p.available_relics().len();
        p.runs_completed = 6;
        let l6 = p.available_relics().len();
        assert!(l6 > l1);
    }

    #[test]
    fn transformation_successors_hidden_until_discovered() {
        let mut p = PlayerProgress::new();
        p.runs_completed = 100;
        assert!(!p.available_relics().contains(&RelicId::SilkMoth));
        assert!(!p.transformation_successor_visible(RelicId::SilkMoth));
        assert!(p.note_transformation_successor_discovered(RelicId::SilkMoth));
        assert!(
            !p.available_relics().contains(&RelicId::SilkMoth),
            "successors are not meta pool — only in-shop after a run-time burn",
        );
        assert!(p.transformation_successor_visible(RelicId::SilkMoth));
        assert!(!p.note_transformation_successor_discovered(RelicId::SilkMoth));
    }

    #[test]
    fn cheat_unlock_all_transformation_chains_meta_discovers_and_lists_primaries() {
        let mut p = PlayerProgress::new();
        p.cheat_unlock_all_transformation_chains_meta();
        assert_eq!(p.runs_completed, 15);
        for &id in transformation_successor_relic_ids() {
            assert!(
                p.transformation_successor_visible(id),
                "expected successor {id:?} visible after cheat",
            );
        }
        let avail = p.available_relics();
        for prim in [
            RelicId::PaperLantern,
            RelicId::SilkThread,
            RelicId::MeltingIce,
            RelicId::RustlingGooseEgg,
            RelicId::TeaCeremony,
            RelicId::Chrysalis,
        ] {
            assert!(
                avail.contains(&prim),
                "expected fragile primary {prim:?} in available_relics",
            );
        }
    }

    #[test]
    fn cheat_unlock_transformation_meta_does_not_lower_runs_completed() {
        let mut p = PlayerProgress::new();
        p.runs_completed = 100;
        p.cheat_unlock_all_transformation_chains_meta();
        assert_eq!(p.runs_completed, 100);
    }

    #[test]
    fn every_active_relic_is_level_or_transformation_successor() {
        use crate::core::relic::all_relic_defs;
        use rustc_hash::FxHashSet;
        let unlocked: FxHashSet<RelicId> =
            (1..=14).flat_map(|l| unlocks_for_level(l).relics).collect();
        let successors: FxHashSet<RelicId> = transformation_successor_relic_ids()
            .iter()
            .copied()
            .collect();
        let missing: Vec<RelicId> = all_relic_defs()
            .iter()
            .map(|d| d.id)
            .filter(|id| !unlocked.contains(id) && !successors.contains(id))
            .collect();
        assert!(
            missing.is_empty(),
            "relics neither level-unlocked nor transformation-successor: {missing:?}",
        );
    }

    #[test]
    fn no_relic_appears_in_two_levels() {
        use rustc_hash::FxHashMap;
        let mut seen: FxHashMap<RelicId, u32> = FxHashMap::default();
        for l in 1..=14u32 {
            for r in unlocks_for_level(l).relics {
                if let Some(prev) = seen.insert(r, l) {
                    panic!("relic {:?} appears in level {} and level {}", r, prev, l);
                }
            }
        }
    }

    #[test]
    fn dora_unlocks_at_level_6() {
        let mut p = PlayerProgress::new();
        assert!(!p.dora_enabled());
        p.runs_completed = 5;
        assert_eq!(p.current_level(), 6);
        assert!(p.dora_enabled());
    }

    #[test]
    fn spring_always_playable() {
        let p = PlayerProgress::new();
        for m in [
            TileMaterial::Bamboo,
            TileMaterial::Plastic,
            TileMaterial::TortoiseShell,
        ] {
            assert!(p.stake_unlocked_for(m, Stake::Spring));
        }
    }

    #[test]
    fn stake_ladder_is_per_material() {
        let mut p = PlayerProgress::new();
        // Clear Spring on Bamboo → Summer unlocks on Bamboo only.
        let next = p.record_stake_victory(TileMaterial::Bamboo, Stake::Spring);
        assert_eq!(next, Some(Stake::Summer));
        assert!(p.stake_unlocked_for(TileMaterial::Bamboo, Stake::Summer));
        assert!(!p.stake_unlocked_for(TileMaterial::Plastic, Stake::Summer));
    }

    #[test]
    fn record_victory_is_idempotent() {
        let mut p = PlayerProgress::new();
        let first = p.record_stake_victory(TileMaterial::Bamboo, Stake::Spring);
        let second = p.record_stake_victory(TileMaterial::Bamboo, Stake::Spring);
        assert_eq!(first, Some(Stake::Summer));
        assert_eq!(second, None, "second win shouldn't reannounce");
    }

    #[test]
    fn winter_requires_full_chain() {
        let mut p = PlayerProgress::new();
        p.record_stake_victory(TileMaterial::Bamboo, Stake::Spring);
        assert!(!p.stake_unlocked_for(TileMaterial::Bamboo, Stake::Winter));
        p.record_stake_victory(TileMaterial::Bamboo, Stake::Summer);
        p.record_stake_victory(TileMaterial::Bamboo, Stake::Autumn);
        assert!(p.stake_unlocked_for(TileMaterial::Bamboo, Stake::Winter));
    }

    #[test]
    fn backfill_rebuilds_unlocks_from_history() {
        let mut p = PlayerProgress::new();
        // Synthesize a RunRecord as if an old profile had a Summer win on
        // Bamboo but no `unlocked_stakes` map on disk.
        p.run_history.push(RunRecord {
            timestamp_unix: 0,
            run_number: 1,
            outcome: RunOutcome::Victory,
            final_ante: 8,
            final_blind: BlindKind::Boss,
            final_boss: None,
            round_score: 1000,
            target_score: 500,
            total_score_earned: 5000,
            final_gold: 0,
            plays_remaining: 0,
            discards_remaining: 0,
            plays_max: 4,
            discards_max: 4,
            tiles_played: 50,
            tiles_discarded: 10,
            times_restocked: 0,
            best_structure_score: 400,
            best_structure_name: "Pair".to_string(),
            yaku_times_played: HashMap::default(),
            relics_owned: vec![],
            consumables_owned: vec![],
            tile_material: TileMaterial::Bamboo,
            stake: Stake::Summer,
            tutorial_run: false,
        });
        assert!(p.unlocked_stakes.is_empty());
        p.backfill_stakes_from_history();
        assert!(
            p.stakes_cleared_for(TileMaterial::Bamboo)
                .contains(&Stake::Summer)
        );
        assert!(p.stake_unlocked_for(TileMaterial::Bamboo, Stake::Autumn));
    }
}
