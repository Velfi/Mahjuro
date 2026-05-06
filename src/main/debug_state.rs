//! Debug-only state and the arrange-mode debug feature.
//!
//! Extracted from `main.rs` to keep the trunk focused on application
//! plumbing. Field access is intentionally `pub` because the rest of `main`
//! mutates these fields directly during event handling and drawing.

use crate::debug_menu::DebugMenuBar;
use crate::debug_overlays::{
    CameraDebugOverlay, DebugVisibilityOverlay, SfxTestOverlay, ShopEnvDebugOverlay,
    TuningOverlay, VolumetricDebugOverlay,
};
use crate::render::draw_cmd::CameraParams;
use crate::render::shop_glb::ShopEnvLightingTune;

/// State for the arrange-mode debug feature. Activated via Debug > Arrange
/// Mode. The user clicks an object to select it, then uses WASD to nudge
/// position (forward/back/left/right), Q/E to nudge up/down, Shift+WASD/QE
/// to rotate in those axes. Enter confirms and copies the result to clipboard;
/// R resets the selected placement to its compiled-in default; Escape cancels.
pub struct ArrangeModeState {
    /// Name of the selected object or group. Either a click-pickable name
    /// (e.g. "Counter") or a hierarchy node name (e.g. "shop.for_sale").
    /// Group names apply their delta to every descendant leaf on save.
    pub object_name: String,
    /// Accumulated nudge in layout pixels along X (right = positive).
    /// Because world_x = pixel_x − w/2, a pixel delta maps 1:1 to world X.
    pub delta_px: f32,
    /// Accumulated nudge in layout pixels along Y (down = positive in pixel
    /// space, i.e. toward the player). world_y = h/2 − pixel_y so a positive
    /// delta_py moves the object toward the player (−world_y).
    pub delta_py: f32,
    /// Accumulated nudge in world Z (lift above the felt).
    pub delta_lift: f32,
    /// Accumulated rotation delta around Z, degrees (Shift+A/D).
    pub delta_rz_deg: f32,
    /// Accumulated rotation delta around X, degrees (Shift+W/S).
    pub delta_rx_deg: f32,
    /// Accumulated rotation delta around Y, degrees (Shift+Q/E).
    pub delta_ry_deg: f32,
    /// World-space translation of the placement at the moment it was
    /// selected — used by click-to-move so each click computes a fresh
    /// world delta from the object's original position (so repeated clicks
    /// don't accumulate).
    pub selected_world_origin: glam::Vec3,
    /// Translation step in layout pixels per key press. Toggled by pressing
    /// 1/2/3/4 (1 / 5 / 25 / 100 px) while an object is selected.
    pub trans_step_px: f32,
    /// Rotation step in degrees per key press. Toggled by pressing 1/2/3/4
    /// (1° / 15° / 45° / 90°) while an object is selected.
    pub rot_step_deg: f32,
}

/// Debug-only state: overlays, visibility toggles, FPS counter, and the
/// one-shot object-hit-test picker.
pub struct DebugState {
    pub menu: Option<DebugMenuBar>,
    pub show_fps: bool,
    pub fps_smoothed: f32,
    pub hide_tiles: bool,
    pub hide_candles: bool,
    pub hide_blind_plaque: bool,
    pub hide_scoring_placard: bool,
    pub hide_inventory: bool,
    pub visibility_overlay: Option<DebugVisibilityOverlay>,
    pub tuning_overlay: Option<TuningOverlay>,
    pub sfx_test_overlay: Option<SfxTestOverlay>,
    pub camera_debug_overlay: Option<CameraDebugOverlay>,
    pub shop_env_debug_overlay: Option<ShopEnvDebugOverlay>,
    pub volumetric_debug_overlay: Option<VolumetricDebugOverlay>,
    /// One-shot debug picker armed by the "Object Hit Test" debug menu
    /// item.
    pub object_hit_test_armed: bool,
    /// Arrange-mode state. `Some` while arrange mode is active (waiting for
    /// a click to select an object, or actively editing one). `None` when
    /// arrange mode is off.
    ///
    /// - `None` outer                 → mode is off
    /// - `Some(None)` inner           → mode on, waiting for click to select
    /// - `Some(Some(state))` inner    → object selected, editing in progress
    pub arrange_mode: Option<Option<ArrangeModeState>>,
    /// Effective 3D camera after the scene's `draw_frame` (override or table
    /// default), updated each paint — used to seed camera debug overlay.
    pub last_effective_camera: CameraParams,
    /// `Shop.glb` room scale multiplier (`window_h *` this). Debug overlay can edit live.
    pub shop_env_height_scale: f32,
    /// glTF punctual + `shop_glb` tonemap tuning. Debug overlay edits live.
    pub shop_env_lighting: ShopEnvLightingTune,
}

impl DebugState {
    pub fn new() -> Self {
        Self {
            menu: None,
            show_fps: false,
            fps_smoothed: 60.0,
            hide_tiles: false,
            hide_candles: false,
            hide_blind_plaque: false,
            hide_scoring_placard: false,
            hide_inventory: false,
            visibility_overlay: None,
            tuning_overlay: None,
            sfx_test_overlay: None,
            camera_debug_overlay: None,
            shop_env_debug_overlay: None,
            volumetric_debug_overlay: None,
            object_hit_test_armed: false,
            arrange_mode: None,
            last_effective_camera: CameraParams::default_table_camera(800.0),
            shop_env_height_scale: crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
            shop_env_lighting: ShopEnvLightingTune::SOURCE_DEFAULTS,
        }
    }

    /// Whether any debug overlay is blocking input.
    pub fn any_overlay_active(&self) -> bool {
        self.tuning_overlay.is_some()
            || self.sfx_test_overlay.is_some()
            || self.camera_debug_overlay.is_some()
            || self.shop_env_debug_overlay.is_some()
            || self.visibility_overlay.is_some()
            || self.volumetric_debug_overlay.is_some()
    }
}
