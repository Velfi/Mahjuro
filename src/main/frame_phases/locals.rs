use std::time::Instant;

use crate::scenes::{OverlayRequest, SceneIntent};
use crate::ui::input::{RumbleLabOp, UiAction};

/// Per-frame scratch state passed between frame pipeline phases.
pub struct FrameLocals {
    pub now: Instant,
    pub actions: Vec<UiAction>,
    pub button_clicks: Vec<u32>,
    pub quit_requested: bool,
    pub switch_profile: Option<usize>,
    pub delete_profile: Option<usize>,
    pub complete_onboarding: bool,
    pub overlay_request: Option<OverlayRequest>,
    pub rumble_lab_ops: Vec<RumbleLabOp>,
    pub bump_archive_chronicle_seen: Option<u32>,
    pub seed_archive_seen: bool,
    pub update_result: Option<SceneIntent>,
    pub hide_cursor: bool,
    pub updated_overlay: bool,
}

impl Default for FrameLocals {
    fn default() -> Self {
        Self {
            now: Instant::now(),
            actions: Vec::new(),
            button_clicks: Vec::new(),
            quit_requested: false,
            switch_profile: None,
            delete_profile: None,
            complete_onboarding: false,
            overlay_request: None,
            rumble_lab_ops: Vec::new(),
            bump_archive_chronicle_seen: None,
            seed_archive_seen: false,
            update_result: None,
            hide_cursor: false,
            updated_overlay: false,
        }
    }
}

impl FrameLocals {
    pub fn clear_input(&mut self) {
        self.actions.clear();
        self.button_clicks.clear();
    }
}
