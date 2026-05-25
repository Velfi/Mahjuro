//! Debug-only state.
//!
//! Extracted from `main.rs` to keep the trunk focused on application
//! plumbing. Field access is intentionally `pub` because the rest of `main`
//! mutates these fields directly during event handling and drawing.

use crate::debug_menu::DebugMenuBar;
use crate::debug_overlays::{
    CameraDebugOverlay, DebugVisibilityOverlay, HallwayDistortionDebugOverlay,
    SceneLookDebugOverlay, SfxTestOverlay, TuningOverlay,
};
use crate::render::draw_cmd::CameraParams;
use crate::render::rain_debug_overlay::RainDebugOverlay;

/// Debug-only state: overlays, visibility toggles, FPS counter, and the
/// one-shot object-hit-test picker.
pub struct DebugState {
    pub menu: Option<DebugMenuBar>,
    pub show_fps: bool,
    pub fps_smoothed: f32,
    pub hide_tiles: bool,
    pub hide_candles: bool,
    pub hide_chamber_plaque: bool,
    pub hide_scoring_placard: bool,
    pub hide_inventory: bool,
    pub visibility_overlay: Option<DebugVisibilityOverlay>,
    pub tuning_overlay: Option<TuningOverlay>,
    pub sfx_test_overlay: Option<SfxTestOverlay>,
    pub camera_debug_overlay: Option<CameraDebugOverlay>,
    /// Per-scene tonemap / post-FX + room GLB lighting (right panel).
    pub scene_look_debug_overlay: Option<SceneLookDebugOverlay>,
    /// Pick-blind hallway vertex warp tuning (left panel).
    pub hallway_distortion_debug_overlay: Option<HallwayDistortionDebugOverlay>,
    pub rain_debug_overlay: Option<RainDebugOverlay>,
    /// Effective 3D camera after the scene's `draw_frame` (override or table
    /// default), updated each paint — used to seed camera debug overlay.
    pub last_effective_camera: CameraParams,
}

impl DebugState {
    pub fn new() -> Self {
        Self {
            menu: None,
            show_fps: false,
            fps_smoothed: 60.0,
            hide_tiles: false,
            hide_candles: false,
            hide_chamber_plaque: false,
            hide_scoring_placard: false,
            hide_inventory: false,
            visibility_overlay: None,
            tuning_overlay: None,
            sfx_test_overlay: None,
            camera_debug_overlay: None,
            scene_look_debug_overlay: None,
            hallway_distortion_debug_overlay: None,
            rain_debug_overlay: None,
            last_effective_camera: CameraParams::default_table_camera(800.0),
        }
    }

    /// Whether any debug overlay is blocking input.
    pub fn any_overlay_active(&self) -> bool {
        self.tuning_overlay.is_some()
            || self.sfx_test_overlay.is_some()
            || self.camera_debug_overlay.is_some()
            || self.scene_look_debug_overlay.is_some()
            || self.hallway_distortion_debug_overlay.is_some()
            || self.rain_debug_overlay.is_some()
            || self.visibility_overlay.is_some()
    }
}
