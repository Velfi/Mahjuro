use super::*;
use std::sync::OnceLock;

impl WgpuRenderer {
    pub fn queue_screenshot(&self, path: std::path::PathBuf) {
        self.pending_screenshot.set(Some(path));
    }

    /// Begin a GPU pass timing capture for the next `frames` frames. The
    /// debug menu binds this to the "Profile GPU…" entry. Results are
    /// emitted via `log::debug!` once the capture finishes; if the adapter
    /// lacks `TIMESTAMP_QUERY` support a warning is logged instead.
    pub fn start_gpu_profile(&mut self, frames: u32) {
        self.gpu_profiler
            .start(frames, self.size.width, self.size.height);
    }

    /// Returns `true` exactly once, on the frame after a GPU profile session
    /// finishes and emits its report. Polled from the app loop to play a
    /// confirmation SFX so the player knows the capture is done.
    pub fn take_gpu_profile_just_completed(&mut self) -> bool {
        self.gpu_profiler.take_just_completed()
    }

    pub fn set_active_scene(&mut self, key: Option<&'static str>) {
        self.active_scene_key = key;
    }

    /// Push the per-scene tonemap + VHS tuning the next `render` call should
    /// upload. The renderer keeps the values across frames; the Options
    /// "VHS overlay" gate (passed via `RenderSettings.vhs_enabled`) can
    /// hard-mute the VHS branch without zeroing the per-amount values
    /// here, so toggling the option restores the previously tuned look.
    pub fn set_tonemap_tuning(&mut self, tuning: &crate::tuning::tonemap::TonemapTuning) {
        self.tonemap_exposure = tuning.exposure;
        self.tonemap_vhs_chromatic = tuning.vhs_chromatic;
        self.tonemap_vhs_scanline = tuning.vhs_scanline;
        self.tonemap_vhs_grain = tuning.vhs_grain;
        self.tonemap_vhs_vignette = tuning.vhs_vignette;
        // Per-scene "VHS on" comes from any non-zero amplitude — a scene
        // that tunes every amount to 0 reads as "VHS effectively off"
        // here even when the Options master toggle is on. This lets
        // per-scene saves disable the look without players having to
        // toggle the global.
        self.tonemap_vhs_enabled = tuning.vhs_chromatic > 0.0
            || tuning.vhs_scanline > 0.0
            || tuning.vhs_grain > 0.0
            || tuning.vhs_vignette > 0.0;
    }

    /// True when every player-visible tileset has an offline baked atlas on disk.
    pub fn showcase_decal_atlases_baked_for_all_player_tilesets(&self) -> bool {
        static SHOWCASE_ATLAS_ALL_READY: OnceLock<bool> = OnceLock::new();
        *SHOWCASE_ATLAS_ALL_READY.get_or_init(|| {
            let tilesets = mahjuro_assets::asset_path::list_player_tilesets();
            tilesets
                .iter()
                .all(|name| crate::showcase_decal_atlas::baked_showcase_decal_atlas_available(name))
        })
    }

    /// True when splash can hand off to the main-menu hub without a first-frame shadow hitch.
    pub fn splash_hub_boot_ready(&self) -> bool {
        self.showcase_decal_atlases_baked_for_all_player_tilesets()
            && self.main_menu_environment.is_some()
    }

    /// Upload hub `main_menu.glb` (and advance CPU prefetch) while the splash plate is up.
    pub fn tick_splash_hub_boot(&mut self) {
        crate::room_preload::try_drain_room_cpu_prefetch_threads();
        self.ensure_main_menu_room_gpu();
        crate::room_preload::advance_hub_cpu_prefetch_chain();
    }

    /// Upload the hub room while the splash loading screen is still up.
    pub fn prepare_splash_hub_boot(&mut self) {
        self.tick_splash_hub_boot();
    }

    /// Partial hub readiness for the unified loading progress bar (0–1).
    pub fn splash_hub_boot_progress(&self) -> f32 {
        let mut done = 0.0f32;
        if self.showcase_decal_atlases_baked_for_all_player_tilesets() {
            done += 1.0;
        }
        if self.main_menu_environment.is_some() {
            done += 1.0;
        }
        done / 2.0
    }

    /// Load the active tileset showcase atlas from its baked PNG.
    pub fn ensure_active_showcase_decal_atlas(&mut self, active_tileset_name: &str) {
        self.ensure_showcase_decal_atlas(active_tileset_name);
        if self.tile_set.as_deref() != Some(active_tileset_name) {
            self.tile_set = Some(active_tileset_name.to_owned());
        }
    }

    /// Returns `true` while boot-time async decode threads (relic images and/or
    /// full-screen backdrop plates) still have GPU uploads pending. Used by the
    /// headless screenshot harness; **splash** does not gate on this (see `frame_tick`).
    pub fn is_loading(&self) -> bool {
        self.relic_rx.is_some() || self.background_rx.is_some()
    }

    /// Drain relic / background decode queues and queue GPU uploads. Normally
    /// invoked from [`Self::render`]; also called while the SDL window is
    /// backgrounded so boot-time loading keeps progressing without simulating
    /// gameplay.
    pub fn poll_pending_texture_uploads(&mut self) {
        self.poll_relic_textures();
        self.poll_background_textures();
    }

    /// Per-hand-tile screen-space rects after the perspective projection,
    /// captured at the end of the previous frame. Indexed by hand position.
    /// Borrow the entire projection cache for bulk access (e.g. building
    /// `DrawCtx`).
    pub fn projections(&self) -> &ProjectionCache {
        &self.proj
    }

    /// Upload per-scene room GLB tuning for this frame. `active_key` selects
    /// [`WgpuRenderer::active_frame_env`] (tiles / bloom / active-scene lit_mesh).
    pub fn set_frame_scene_env_tunes(
        &mut self,
        active_key: Option<&str>,
        tunes: &[(&'static str, crate::room_glb::RoomEnvFrameTune)],
    ) {
        
        self.frame_env_tunes.clear();
        for (key, tune) in tunes {
            self.frame_env_tunes.insert(*key, *tune);
        }
        self.active_frame_env = active_key
            .and_then(|k| self.frame_env_tunes.get(k).copied())
            .unwrap_or_default();
    }

    #[inline]
    pub(crate) fn env_tune_for(&self, scene_key: &str) -> crate::room_glb::RoomEnvFrameTune {
        *self
            .frame_env_tunes
            .get(scene_key)
            .unwrap_or(&self.active_frame_env)
    }

    #[inline]
    pub(crate) fn active_frame_env(&self) -> crate::room_glb::RoomEnvFrameTune {
        self.active_frame_env
    }

    pub fn clear_smoke(&mut self) {
        self.prev_tile_world.clear();
    }

    /// Snap gameplay score/target odometer rollers to the next frame's HUD values
    /// instead of animating from the previous round's drive state.
    pub fn snap_gameplay_score_rollers(&self) {
        *self
            .gameplay_score_roller_drive_initialized
            .borrow_mut() = [false; 2];
        *self.gameplay_score_roller_roll_elapsed.borrow_mut() = 0.0;
    }

    /// True while either score/target odometer bank is still catching up to
    /// `score` / `target` (used to drive the rollers spinning loop SFX).
    pub fn gameplay_score_rollers_spinning(&self, score: u64, target: u64) -> bool {
        let drive_values = self.gameplay_score_roller_drive_values.borrow();
        let initialized = self.gameplay_score_roller_drive_initialized.borrow();
        let goal = [score as f64, target as f64];
        runtime::shop_environment::gameplay_score_roller_bank_moving(
            &initialized,
            &drive_values,
            &goal,
        )
    }

    /// Playback speed for the rollers loop SFX (gentler ramp than the visual drive).
    pub fn gameplay_score_roller_loop_speed(&self) -> f32 {
        let elapsed = *self.gameplay_score_roller_roll_elapsed.borrow();
        runtime::shop_environment::gameplay_score_roller_loop_speed_multiplier(elapsed) as f32
    }
}
