//! UI state for the strategic Wall screen.

use crate::game::wall_stats::{FaceKey, WallCountView};

pub struct WallScreenState {
    pub view: WallCountView,
    pub selected: FaceKey,
}

impl WallScreenState {
    pub fn face_visible(&self, _entry: &crate::game::wall_stats::TileLedgerEntry) -> bool {
        true
    }
}

#[allow(dead_code)]
pub type WallViewMode = WallCountView;
