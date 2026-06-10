//! Memorial snapshot capture from a live run.

use crate::core::memorial_talisman::{
    MemorialJournalSnapshot, RunDefeatJournal, dominant_yaku_from_run,
};
use crate::game::run::RunState;
use mahjuro_types::GameOverReason;

pub fn snapshot_from_run(
    journal: &RunDefeatJournal,
    reason: GameOverReason,
    run: &RunState,
) -> MemorialJournalSnapshot {
    let consumables_unused = run.consumables.items.len() as u32;
    MemorialJournalSnapshot {
        journal: journal.clone(),
        loss_reason: reason,
        final_wing: run.wing,
        final_chamber: run.chamber,
        final_yen: run.yen,
        tiles_played: run.tiles_played,
        tiles_discarded: run.tiles_discarded,
        consumables_unused,
        dominant_yaku: dominant_yaku_from_run(&run.yaku_times_played),
        run_number: Some(run.run_number),
    }
}
