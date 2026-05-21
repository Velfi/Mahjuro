use super::*;

impl WgpuRenderer {
    fn has_showcase_decal_atlas_for_tileset(&self, tileset_name: &str) -> bool {
        (self.showcase_decal_atlas_tileset.as_deref() == Some(tileset_name)
            && self.showcase_decal_atlas.is_some())
            || self
                .showcase_decal_atlas_cache
                .iter()
                .any(|(name, _)| name == tileset_name)
    }

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
    pub fn set_tonemap_tuning(&mut self, tuning: &crate::game::tonemap_tuning::TonemapTuning) {
        self.tonemap_exposure = tuning.exposure;
        self.tonemap_vhs_chromatic = tuning.vhs_chromatic;
        self.tonemap_vhs_scanline = tuning.vhs_scanline;
        self.tonemap_vhs_grain = tuning.vhs_grain;
        self.tonemap_vhs_vignette = tuning.vhs_vignette;
        self.tonemap_film_grain = tuning.film_grain;
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

    /// True when every player-visible tileset has a baked showcase atlas
    /// either active or parked in the renderer cache.
    pub fn showcase_decal_atlases_baked_for_all_player_tilesets(&self) -> bool {
        let tilesets = crate::asset_path::list_player_tilesets();
        if tilesets.is_empty() {
            return self.showcase_decal_atlas.is_some();
        }
        tilesets
            .iter()
            .all(|name| self.has_showcase_decal_atlas_for_tileset(name))
    }

    /// Pre-bake showcase atlases for every player tileset, keeping them in the
    /// in-memory atlas cache for hitch-free tileset cycling.
    pub fn prebake_showcase_decal_atlases_for_all_player_tilesets(
        &mut self,
        active_tileset_name: &str,
    ) {
        let _bake = crate::startup_profile::scope("splash.showcase_decal_atlases");
        let mut tilesets = crate::asset_path::list_player_tilesets();
        if !tilesets.iter().any(|n| n == active_tileset_name) {
            tilesets.push(active_tileset_name.to_string());
        }
        for tileset in &tilesets {
            self.ensure_showcase_decal_atlas(tileset);
            // Decal pre-bake can take several seconds; keep draining relic /
            // backdrop uploads so `relic_rx` does not stall behind this loop.
            self.poll_pending_texture_uploads();
        }
        // Restore active set so the first post-splash frame does not need to
        // promote from cache before draw.
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
    /// Returns true while any tile animation (spin or lift lerp) is still running.
    #[allow(dead_code)] // Was used for redraw gating; kept for diagnostics / future idle paths.
    pub fn is_spinning(&self) -> bool {
        const SPIN_SECS: f32 = 0.4;
        let spin_active = if let Some((_slot, start)) = self.focus_spin {
            start.elapsed().as_secs_f32() < SPIN_SECS
        } else {
            false
        };
        // Also keep animating while any tile's focus_t hasn't settled.
        let lerp_active = self.focus_t.iter().enumerate().any(|(i, &ft)| {
            let target = if i == self.last_focus { 1.0 } else { 0.0 };
            (ft - target).abs() > 0.001
        });
        // Keep animating while any tile is sliding into position.
        let slide_active = self.tile_anim_y.iter().any(|&y| y.abs() > 0.5)
            || self.tile_anim_x.iter().any(|&x| x.abs() > 0.01);
        spin_active || lerp_active || slide_active || !self.hand_tiles.is_empty()
    }

    /// Per-hand-tile screen-space rects after the perspective projection,
    /// captured at the end of the previous frame. Indexed by hand position.
    /// Borrow the entire projection cache for bulk access (e.g. building
    /// `DrawCtx`).
    pub fn projections(&self) -> &ProjectionCache {
        &self.proj
    }

    /// Scale factor for embedded glTF room geometry vs window height. Must match shop/hallway/archive marker math.
    pub fn set_room_gltf_height_scale(&mut self, v: f32) {
        self.room_gltf_height_scale = v;
    }

    /// Shop room tonemap + `lit_mesh` glTF punctual scale. Set each frame from app debug tuning.
    pub fn set_shop_env_render_tune(
        &mut self,
        linear_exposure: f32,
        ambient_scale: f32,
        lit_mesh_gltf_punctual_scale: f32,
        gltf_emissive_scale: f32,
    ) {
        self.shop_env_linear_exposure = linear_exposure;
        self.shop_env_ambient_scale = ambient_scale;
        self.shop_lit_mesh_gltf_punctual_scale = lit_mesh_gltf_punctual_scale;
        self.shop_gltf_emissive_scale = gltf_emissive_scale;
    }

    #[inline]
    pub(crate) fn room_gltf_height_scale(&self) -> f32 {
        self.room_gltf_height_scale
    }

    pub fn clear_smoke(&mut self) {
        self.prev_tile_world.clear();
    }
}
