use super::*;
use crate::room_gpu_resident::{
    ROOM_ARCHIVE, ROOM_GAMEPLAY, ROOM_HALLWAY, ROOM_MAIN_MENU, ROOM_SHOP, ROOM_STAIRCASE,
};
use crate::scene_keys;

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

    /// Returns true when all room environments required by `key` are already on the GPU.
    ///
    /// Used by scene transitions to avoid swapping into a destination scene before its
    /// environment upload has completed.
    pub fn scene_room_gpu_ready(&self, key: &str) -> bool {
        let key = crate::scene_keys::normalize_scene_key(key);
        let loaded = |bit: u8| self.rooms_gpu_loaded & bit != 0;
        match key {
            scene_keys::MAIN_MENU => loaded(ROOM_MAIN_MENU),
            scene_keys::SHOP => loaded(ROOM_SHOP),
            scene_keys::HALLWAY => loaded(ROOM_SHOP) && loaded(ROOM_HALLWAY),
            scene_keys::STAIRWAY => loaded(ROOM_STAIRCASE),
            scene_keys::ARCHIVE => loaded(ROOM_ARCHIVE),
            scene_keys::GAMEPLAY | scene_keys::VICTORY | scene_keys::DEFEAT => {
                loaded(ROOM_GAMEPLAY)
            }
            _ => true,
        }
    }

    /// Kick background CPU decode for a hub/run room before a scene transition finishes.
    pub fn start_room_cpu_prefetch_for_scene_key(&self, key: &str) {
        let key = scene_keys::normalize_scene_key(key);
        match key {
            scene_keys::MAIN_MENU => crate::room_preload::start_main_menu_cpu_prefetch(),
            scene_keys::SHOP => crate::room_preload::start_shop_cpu_prefetch(),
            scene_keys::HALLWAY => crate::room_preload::start_hallway_cpu_prefetch(),
            scene_keys::ARCHIVE => crate::room_preload::start_archive_cpu_prefetch(),
            scene_keys::GAMEPLAY | scene_keys::VICTORY | scene_keys::DEFEAT => {
                crate::room_preload::start_gameplay_cpu_prefetch();
            }
            scene_keys::STAIRWAY => {}
            _ => {}
        }
    }

    /// Graphics preset suggested from the active adapter at renderer init.
    pub fn suggested_graphics_mode(&self) -> mahjuro_gfx_types::GraphicsMode {
        self.suggested_graphics_mode
    }

    /// Apply VRAM budget: internal render scale, room/atlas residency caps, optional resize.
    pub fn set_graphics_budget(&mut self, mode: mahjuro_gfx_types::GraphicsMode) {
        let new_scale = mode.render_scale();
        let scale_changed = (self.render_scale - new_scale).abs() > f32::EPSILON;
        let mode_changed = self.graphics_mode != mode;
        self.graphics_mode = mode;
        crate::room_preload::set_prefetch_graphics_mode(mode);
        self.render_scale = new_scale;
        if scale_changed && self.size.width > 0 && self.size.height > 0 {
            self.resize(self.size);
        }
        if mode_changed || scale_changed {
            self.trim_room_gpu_residency();
            self.trim_showcase_decal_atlas_cache();
        }
    }

    pub(super) fn trim_showcase_decal_atlas_cache(&mut self) {
        let cap = self.graphics_mode.max_showcase_decal_atlas_cache();
        while self.showcase_decal_atlas_cache.len() > cap {
            self.showcase_decal_atlas_cache.pop_back();
        }
    }

    pub(super) fn room_gpu_bit_for_scene_key(key: &str) -> Option<u8> {
        crate::room_gpu_resident::RoomGpuResidentId::bit_for_scene_key(key)
    }

    #[inline]
    pub(super) fn integrated_low_memory_gpu(&self) -> bool {
        self.integrated_gpu && self.graphics_mode == mahjuro_gfx_types::GraphicsMode::LowMemory
    }

    /// Room bits that must not be evicted: active scene plus any in-flight GPU upload.
    pub(super) fn room_gpu_evict_protected_bits(&self) -> u8 {
        let mut bits = self.poll_pinned_room_gpu_bit.unwrap_or(0);
        if self.shop_room_gpu_upload.is_some() {
            bits |= ROOM_SHOP;
        }
        if self.hallway_room_gpu_upload.is_some() {
            bits |= ROOM_HALLWAY;
        }
        if self.gameplay_room_gpu_upload.is_some() {
            bits |= ROOM_GAMEPLAY;
        }
        bits
    }

    fn effective_room_gpu_cap(&self) -> usize {
        let cap = self.graphics_mode.max_room_gpu_residents();
        if self.integrated_gpu && self.graphics_mode == mahjuro_gfx_types::GraphicsMode::LowMemory {
            1
        } else {
            cap
        }
    }

    pub(super) fn trim_room_gpu_residency(&mut self) {
        let cap = self.effective_room_gpu_cap();
        let protected = self.room_gpu_evict_protected_bits();
        while self.room_gpu_lru.len() > cap {
            let Some(bit) = self
                .room_gpu_lru
                .iter()
                .rev()
                .find(|&&b| protected & b == 0)
                .copied()
            else {
                break;
            };
            let Some(idx) = self.room_gpu_lru.iter().position(|&b| b == bit) else {
                break;
            };
            self.room_gpu_lru.remove(idx);
            self.evict_room_gpu(bit);
        }
    }

    pub(super) fn gpu_memory_pressure_snapshot(
        &self,
    ) -> crate::gpu_memory_pressure::PressureSnapshot {
        crate::gpu_memory_pressure::classify(
            &self.device,
            self.room_gpu_lru.len(),
            self.effective_room_gpu_cap(),
            self.integrated_gpu,
        )
    }

    pub(super) fn refresh_gpu_memory_pressure(&mut self) {
        let snapshot = self.gpu_memory_pressure_snapshot();
        crate::gpu_memory_pressure::log_pressure_transition(&snapshot);
        self.gpu_memory_pressure = snapshot.pressure;
    }

    /// Evict one unpinned LRU resident when at cap or under critical pressure.
    /// Returns `false` when no headroom could be freed and `allow_when_full` is false.
    pub(super) fn preflight_room_gpu_headroom_for_upload(
        &mut self,
        allow_when_full: bool,
    ) -> bool {
        let snapshot = self.gpu_memory_pressure_snapshot();
        self.gpu_memory_pressure = snapshot.pressure;
        let at_cap = snapshot.room_gpu_residents >= snapshot.max_room_gpu_residents;
        if snapshot.pressure == crate::gpu_memory_pressure::GpuMemoryPressure::Critical || at_cap {
            let before = self.room_gpu_lru.len();
            self.trim_room_gpu_residency();
            if self.room_gpu_lru.len() < before {
                return true;
            }
            // Integrated shared memory: do not stack a second room while the active scene
            // (or an in-flight upload) holds the only residency slot.
            if self.integrated_low_memory_gpu() && at_cap {
                return false;
            }
            return allow_when_full;
        }
        true
    }

    /// Drop GPU residents other than `keep_bit` so a scene transition can upload on integrated GPUs.
    pub(super) fn evict_room_gpu_residents_except(&mut self, keep_bit: u8) {
        let loaded = self.rooms_gpu_loaded;
        for bit in [
            ROOM_MAIN_MENU,
            ROOM_SHOP,
            ROOM_HALLWAY,
            ROOM_ARCHIVE,
            ROOM_STAIRCASE,
            ROOM_GAMEPLAY,
        ] {
            if bit != keep_bit && loaded & bit != 0 {
                self.evict_room_gpu_inner(bit, true);
            }
        }
    }

    pub(super) fn note_room_gpu_resident(&mut self, bit: u8) {
        if let Some(i) = self.room_gpu_lru.iter().position(|&b| b == bit) {
            self.room_gpu_lru.remove(i);
        }
        self.room_gpu_lru.push_front(bit);
        self.trim_room_gpu_residency();
    }

    pub(super) fn evict_room_gpu(&mut self, bit: u8) {
        self.evict_room_gpu_inner(bit, false);
    }

    fn evict_room_gpu_inner(&mut self, bit: u8, force: bool) {
        if self.rooms_gpu_loaded & bit == 0 {
            return;
        }
        if !force {
            let protected = self.room_gpu_evict_protected_bits();
            if protected & bit != 0 {
                return;
            }
        } else if self.room_gpu_evict_protected_bits() & bit != 0 {
            // Never tear down a room that is mid-upload during a forced transition evict.
            return;
        }
        if let Some(id) = crate::room_gpu_resident::RoomGpuResidentId::from_bit(bit) {
            self.clear_room_gpu_resident_fields(id);
        }
        self.rooms_gpu_loaded &= !bit;
        if let Some(i) = self.room_gpu_lru.iter().position(|&b| b == bit) {
            self.room_gpu_lru.remove(i);
        }
        if self.probe_gi_gpu_room.is_some() {
            self.probe_gi_gpu_room = None;
            self.probe_gi_had_room = false;
        }
        if self.graphics_mode == mahjuro_gfx_types::GraphicsMode::LowMemory {
            super::room_gpu_load::clear_room_cpu_cache_for_gpu_evict(bit);
            super::room_gpu_load::on_low_memory_room_gpu_evict_restart_prefetch(bit);
        }
        crate::gpu_memory_profile::log_room_evict(bit);
        crate::gpu_memory_profile::log_device_allocator(&self.device, "room_evict");
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

    /// True when every built-in player tileset has an offline baked atlas on disk.
    pub fn builtin_showcase_decal_atlases_ready() -> bool {
        mahjuro_assets::asset_path::list_builtin_player_tilesets()
            .iter()
            .all(|name| crate::showcase_decal_atlas::baked_showcase_decal_atlas_available(name))
    }

    /// Back-compat alias: built-in tilesets only (mods bake on demand).
    pub fn showcase_decal_atlases_baked_for_all_player_tilesets(&self) -> bool {
        Self::builtin_showcase_decal_atlases_ready()
    }

    fn active_tileset_showcase_ready(&self, active_tileset: &str) -> bool {
        if self.showcase_decal_atlas_tileset.as_deref() == Some(active_tileset)
            && self.showcase_decal_atlas.is_some()
        {
            return true;
        }
        crate::showcase_decal_atlas::baked_showcase_decal_atlas_available(active_tileset)
    }

    /// True when splash can hand off to the main-menu hub without a first-frame shadow hitch.
    pub fn splash_hub_boot_ready(&self, active_tileset: &str) -> bool {
        Self::builtin_showcase_decal_atlases_ready()
            && self.main_menu_environment.is_some()
            && self.active_tileset_showcase_ready(active_tileset)
    }

    /// Upload hub/run room GLBs (and advance CPU prefetch) while the splash plate is up.
    ///
    /// Performance/Visuals also GPU-warm every hub/run room before dismiss when possible; splash
    /// still hands off once showcase atlases and `main_menu.glb` are ready.
    pub fn tick_splash_hub_boot(&mut self) {
        crate::room_preload::try_drain_room_cpu_prefetch_threads();
        crate::room_preload::kick_eager_all_room_cpu_prefetches();
        self.ensure_main_menu_room_gpu();
        crate::room_preload::advance_hub_cpu_prefetch_chain(false);
        if self.graphics_mode != mahjuro_gfx_types::GraphicsMode::LowMemory {
            self.drive_splash_eager_room_gpu_boot();
        }
    }

    /// Upload the hub room while the splash loading screen is still up.
    pub fn prepare_splash_hub_boot(&mut self) {
        self.tick_splash_hub_boot();
    }

    /// Partial hub readiness for the unified loading progress bar (0–1).
    pub fn splash_hub_boot_progress(&self, active_tileset: &str) -> f32 {
        let mut done = 0.0f32;
        if Self::builtin_showcase_decal_atlases_ready()
            && self.active_tileset_showcase_ready(active_tileset)
        {
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
        *self.gameplay_score_roller_drive_initialized.borrow_mut() = [false; 2];
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
