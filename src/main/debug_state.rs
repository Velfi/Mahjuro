//! Debug-only state.
//!
//! Extracted from `main.rs` to keep the trunk focused on application
//! plumbing. Field access is intentionally `pub` because the rest of `main`
//! mutates these fields directly during event handling and drawing.

#[cfg(debug_menu_enabled)]
use crate::debug_menu::DebugMenuBar;
use crate::debug_overlays::{
    CameraDebugOverlay, DebugVisibilityOverlay, HallwayDistortionDebugOverlay,
    SceneLookDebugOverlay, SfxTestOverlay, TuningOverlay,
};
use crate::render::draw_cmd::CameraParams;
use crate::render::flame_debug_overlay::FlameDebugOverlay;
use crate::render::rain_debug_overlay::RainDebugOverlay;
use crate::trailer_mode::TrailerMode;

/// Debug-only state: overlays, visibility toggles, FPS counter, and the
/// one-shot object-hit-test picker.
pub struct DebugState {
    #[cfg(debug_menu_enabled)]
    pub menu: Option<DebugMenuBar>,
    pub show_fps: bool,
    /// Strip all 2D draw commands (HUD, modals, debug panels) so only the 3D
    /// scene is visible. Toggled from Debug → Overlays → Hide 2D UI.
    pub hide_2d_ui: bool,
    pub fps_smoothed: f32,
    pub visibility: crate::scenes::DebugVisibility,
    pub visibility_overlay: Option<DebugVisibilityOverlay>,
    pub tuning_overlay: Option<TuningOverlay>,
    pub sfx_test_overlay: Option<SfxTestOverlay>,
    pub camera_debug_overlay: Option<CameraDebugOverlay>,
    /// Per-scene tonemap / post-FX + room GLB lighting (right panel).
    pub scene_look_debug_overlay: Option<SceneLookDebugOverlay>,
    /// Pick-blind hallway vertex warp tuning (left panel).
    pub hallway_distortion_debug_overlay: Option<HallwayDistortionDebugOverlay>,
    /// Rapid seed/tint / camera cycling for capture reels (Debug → Trigger Trailer Mode).
    pub trailer_mode: Option<TrailerMode>,
    pub rain_debug_overlay: Option<RainDebugOverlay>,
    pub flame_debug_overlay: Option<FlameDebugOverlay>,
    /// Main menu moon tab — pride rainbow on moon / stars (defaults on in June).
    pub main_menu_pride_rainbow_debug: bool,
    /// Main menu moon tab — live calendar vs forced synodic phase.
    pub main_menu_moon_phase_debug: crate::render::main_menu_moon_tuning::MainMenuMoonPhaseDebug,
    /// Effective 3D camera after the scene's `draw_frame` (override or table
    /// default), updated each paint — used to seed camera debug overlay.
    pub last_effective_camera: CameraParams,
}

impl DebugState {
    pub fn new() -> Self {
        Self {
            #[cfg(debug_menu_enabled)]
            menu: None,
            show_fps: false,
            hide_2d_ui: false,
            fps_smoothed: 60.0,
            visibility: crate::scenes::DebugVisibility::default(),
            visibility_overlay: None,
            tuning_overlay: None,
            sfx_test_overlay: None,
            camera_debug_overlay: None,
            scene_look_debug_overlay: None,
            hallway_distortion_debug_overlay: None,
            trailer_mode: None,
            rain_debug_overlay: None,
            flame_debug_overlay: None,
            main_menu_pride_rainbow_debug:
                crate::render::main_menu_glb::main_menu_pride_rainbow_default_enabled(),
            main_menu_moon_phase_debug:
                crate::render::main_menu_moon_tuning::MainMenuMoonPhaseDebug::default(),
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
            || self.flame_debug_overlay.is_some()
            || self.visibility_overlay.is_some()
    }
}
