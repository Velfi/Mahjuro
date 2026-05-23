//! Archive catalog "seen" state — which unlocked entries the player has
//! focused since they appeared in the Collection grids.

use crate::core::boss::{BossKind, all_bosses, final_bosses};
use crate::core::progression::{is_transformation_successor_relic, PlayerProgress};
use crate::core::relic::{RelicId, all_relic_defs};
use crate::core::talisman::TalismanKind;
use crate::core::yaku::YakuKind;

/// Archive section tabs (matches [`crate::scenes::collection::Tab`] order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveTab {
    Relics = 0,
    Talismans = 1,
    Yaku = 2,
    Bosses = 3,
    Chronicle = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveSeenMark {
    Relic(RelicId),
    Yaku(YakuKind),
    Boss(BossKind),
    Talisman(TalismanKind),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchiveNewCounts {
    pub relics: usize,
    pub talismans: usize,
    pub yaku: usize,
    pub bosses: usize,
    pub chronicle_runs: usize,
}

impl ArchiveNewCounts {
    pub fn total_catalog(&self) -> usize {
        self.relics + self.talismans + self.yaku + self.bosses
    }

    pub fn any(&self) -> bool {
        self.total_catalog() > 0 || self.chronicle_runs > 0
    }

    pub fn for_tab(self, tab: ArchiveTab) -> usize {
        match tab {
            ArchiveTab::Relics => self.relics,
            ArchiveTab::Talismans => self.talismans,
            ArchiveTab::Yaku => self.yaku,
            ArchiveTab::Bosses => self.bosses,
            ArchiveTab::Chronicle => self.chronicle_runs,
        }
    }
}

pub fn archive_new_counts(
    progress: &PlayerProgress,
    chronicle_last_seen_run_len: u32,
) -> ArchiveNewCounts {
    ArchiveNewCounts {
        relics: visible_archive_relics(progress)
            .iter()
            .filter(|id| !progress.archive_seen_relics.contains(id))
            .count(),
        talismans: visible_archive_talismans(progress)
            .iter()
            .filter(|id| !progress.archive_seen_talismans.contains(id))
            .count(),
        yaku: visible_archive_yaku(progress)
            .iter()
            .filter(|id| !progress.archive_seen_yaku.contains(id))
            .count(),
        bosses: visible_archive_bosses(progress)
            .iter()
            .filter(|id| !progress.archive_seen_bosses.contains(id))
            .count(),
        chronicle_runs: chronicle_unseen_run_count(progress, chronicle_last_seen_run_len),
    }
}

pub fn archive_has_any_new(
    progress: &PlayerProgress,
    chronicle_last_seen_run_len: u32,
) -> bool {
    archive_new_counts(progress, chronicle_last_seen_run_len).any()
}

pub fn chronicle_unseen_run_count(
    progress: &PlayerProgress,
    chronicle_last_seen_run_len: u32,
) -> usize {
    progress
        .run_history
        .len()
        .saturating_sub(chronicle_last_seen_run_len as usize)
}

pub fn chronicle_run_is_new(run_history_index: usize, chronicle_last_seen_run_len: u32) -> bool {
    run_history_index >= chronicle_last_seen_run_len as usize
}

pub fn visible_archive_relics(progress: &PlayerProgress) -> Vec<RelicId> {
    let available = progress.available_relics();
    all_relic_defs()
        .iter()
        .filter(|d| progress.transformation_successor_visible(d.id))
        .filter(|d| available.contains(&d.id) || is_transformation_successor_relic(d.id))
        .map(|d| d.id)
        .collect()
}

pub fn visible_archive_yaku(progress: &PlayerProgress) -> Vec<YakuKind> {
    YakuKind::all()
        .iter()
        .copied()
        .filter(|yk| progress.yaku_times_scored.contains_key(yk))
        .collect()
}

pub fn visible_archive_bosses(progress: &PlayerProgress) -> Vec<BossKind> {
    all_bosses()
        .iter()
        .chain(final_bosses().iter())
        .map(|def| def.kind)
        .filter(|kind| progress.boss_times_encountered.contains_key(kind))
        .collect()
}

pub fn visible_archive_talismans(progress: &PlayerProgress) -> Vec<TalismanKind> {
    TalismanKind::all()
        .iter()
        .copied()
        .filter(|tk| progress.talisman_times_purchased.contains_key(&tk))
        .collect()
}

pub fn archive_seen_needs_migration_seed(progress: &PlayerProgress) -> bool {
    progress.runs_completed > 0
        && progress.archive_seen_relics.is_empty()
        && progress.archive_seen_yaku.is_empty()
        && progress.archive_seen_bosses.is_empty()
        && progress.archive_seen_talismans.is_empty()
}

pub fn archive_seen_migration_seed(progress: &mut PlayerProgress) {
    if !archive_seen_needs_migration_seed(progress) {
        return;
    }
    progress
        .archive_seen_relics
        .extend(visible_archive_relics(progress));
    progress
        .archive_seen_yaku
        .extend(visible_archive_yaku(progress));
    progress
        .archive_seen_bosses
        .extend(visible_archive_bosses(progress));
    progress
        .archive_seen_talismans
        .extend(visible_archive_talismans(progress));
}

impl PlayerProgress {
    pub fn mark_archive_seen(&mut self, mark: ArchiveSeenMark) -> bool {
        match mark {
            ArchiveSeenMark::Relic(id) => self.archive_seen_relics.insert(id),
            ArchiveSeenMark::Yaku(yk) => self.archive_seen_yaku.insert(yk),
            ArchiveSeenMark::Boss(bk) => self.archive_seen_bosses.insert(bk),
            ArchiveSeenMark::Talisman(tk) => self.archive_seen_talismans.insert(tk),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::boss::BossKind;
    use crate::core::relic::RelicId;
    use crate::core::yaku::YakuKind;

    fn sample_run_record(run_number: u32) -> crate::core::progression::RunRecord {
        crate::core::progression::RunRecord {
            timestamp_unix: 0,
            run_number,
            outcome: crate::core::progression::RunOutcome::Defeat {
                reason: crate::game::event_bus::GameOverReason::OutOfPlays,
            },
            final_ante: 1,
            final_blind: crate::core::rules::BlindKind::Small,
            final_boss: None,
            round_score: 0,
            target_score: 100,
            total_score_earned: 0,
            final_gold: 0,
            plays_remaining: 0,
            discards_remaining: 0,
            plays_max: 4,
            discards_max: 4,
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            yaku_times_played: rustc_hash::FxHashMap::default(),
            relics_owned: vec![],
            consumables_owned: vec![],
            tile_material: crate::persistence::TileMaterial::Bamboo,
            stake: crate::core::stake::Stake::Spring,
            tutorial_run: false,
            memorial_kind: None,
            best_hand_tiles: Vec::new(),
            score_after_ante: Vec::new(),
            chronicle: crate::core::run_chronicle::RunChronicle::default(),
            duration_secs: 0,
        }
    }

    #[test]
    fn migration_seed_prevents_false_new_on_legacy_profile() {
        let mut p = PlayerProgress::new();
        p.runs_completed = 3;
        p.unlocked_relics.insert(RelicId::MultiplierMaster);
        p.yaku_times_scored.insert(YakuKind::Tanyao, 1);
        p.boss_times_encountered.insert(BossKind::House, 1);
        assert!(archive_seen_needs_migration_seed(&p));
        archive_seen_migration_seed(&mut p);
        assert!(!archive_seen_needs_migration_seed(&p));
        assert_eq!(archive_new_counts(&p, 0).relics, 0);
        assert_eq!(archive_new_counts(&p, 0).yaku, 0);
        assert_eq!(archive_new_counts(&p, 0).bosses, 0);
    }

    #[test]
    fn new_catalog_entry_after_seen_seed() {
        let mut p = PlayerProgress::new();
        p.runs_completed = 1;
        p.yaku_times_scored.insert(YakuKind::Tanyao, 1);
        archive_seen_migration_seed(&mut p);
        p.yaku_times_scored.insert(YakuKind::Toitoi, 1);
        assert_eq!(archive_new_counts(&p, 0).yaku, 1);
        p.mark_archive_seen(ArchiveSeenMark::Yaku(YakuKind::Toitoi));
        assert_eq!(archive_new_counts(&p, 0).yaku, 0);
    }

    #[test]
    fn chronicle_unseen_runs() {
        let mut p = PlayerProgress::new();
        p.run_history.push(sample_run_record(1));
        p.run_history.push(sample_run_record(2));
        assert_eq!(chronicle_unseen_run_count(&p, 0), 2);
        assert_eq!(chronicle_unseen_run_count(&p, 1), 1);
        assert_eq!(chronicle_unseen_run_count(&p, 2), 0);
        assert!(chronicle_run_is_new(1, 1));
        assert!(!chronicle_run_is_new(0, 1));
    }
}
