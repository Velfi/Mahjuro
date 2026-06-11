//! UI state for the strategic Wall screen.

use crate::game::wall_stats::FaceKey;

pub struct WallScreenState {
    pub selected: FaceKey,
}

impl WallScreenState {
    pub fn face_visible(&self, _entry: &crate::game::wall_stats::TileLedgerEntry) -> bool {
        true
    }
}
