//! Per-frame data assembly for the Wall screen.

use std::collections::HashMap;

use crate::core::tile::Suit;
use crate::game::run::RunState;
use crate::game::wall_ledger::{
    WallLedgerFaceGroup, WallLedgerMode, WallLedgerReadModel, read_wall_ledger,
};
use crate::game::wall_stats::{WallStats, compute_wall_stats_for_run};

pub struct WallFrameContext {
    pub ledger: WallLedgerReadModel,
    pub stats: WallStats,
}

pub fn build_frame_context(run: &RunState, mode: WallLedgerMode) -> WallFrameContext {
    let ledger = read_wall_ledger(run, mode);
    let stats = compute_wall_stats_for_run(&ledger, run);
    WallFrameContext { ledger, stats }
}

pub fn groups_by_face<'a>(
    ledger: &'a WallLedgerReadModel,
) -> HashMap<(Suit, u8), &'a WallLedgerFaceGroup> {
    let mut groups = HashMap::new();
    for g in ledger.standard_groups.iter().chain(&ledger.pack_groups) {
        groups.insert((g.suit, g.rank), g);
    }
    groups
}
