use super::*;

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

    /// True when the showcase decal atlas for the active tileset is built and
    /// resident on the GPU. The atlas is the single biggest CPU cost in the
    /// renderer (≈3 s of `image::resize` + PNG decode for 336 face decals on
    /// first use); the splash scene gates its dismissal on this so the bake
    /// happens behind the splash plate rather than freezing the first
    /// gameplay frame.
    pub fn showcase_decal_atlas_baked(&self) -> bool {
        self.showcase_decal_atlas.is_some()
    }

    /// Pre-bake the showcase decal atlas for `tileset_name`, blocking until
    /// the GPU upload is queued. Idempotent for an already-baked tileset.
    /// Called from the splash scene's tick so the cost is amortised behind
    /// the splash plate; subsequent renders skip the lazy bake in
    /// `runtime/showcase_tiles.rs`.
    ///
    /// Also seeds `tile_set` so `apply_render_settings` doesn't immediately
    /// invalidate the atlas on the first real `render()` call (its
    /// "tileset changed" check compares against this field).
    pub fn prebake_showcase_decal_atlas(&mut self, tileset_name: &str) {
        if self.showcase_decal_atlas.is_some()
            && self.showcase_decal_atlas_tileset.as_deref() == Some(tileset_name)
        {
            return;
        }
        // Seed tile_set so the next apply_render_settings does not see a
        // mismatch and clear the atlas we are about to build.
        if self.tile_set.as_deref() != Some(tileset_name) {
            self.tile_set = Some(tileset_name.to_owned());
        }
        self.ensure_showcase_decal_atlas(tileset_name);
    }

    /// Returns `true` while boot-time async decode threads (relic images and/or
    /// full-screen backdrop plates) still have GPU uploads pending. Used by the
    /// headless screenshot harness; **splash** does not gate on this (see `frame_tick`).
    pub fn is_loading(&self) -> bool {
        self.relic_rx.is_some() || self.background_rx.is_some()
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
        let departing_active = !self.departing_tiles.is_empty();
        spin_active
            || lerp_active
            || slide_active
            || departing_active
            || !self.hand_tiles.is_empty()
    }

    /// Per-hand-tile screen-space rects after the perspective projection,
    /// captured at the end of the previous frame. Indexed by hand position.
    /// Borrow the entire projection cache for bulk access (e.g. building
    /// `DrawCtx`).
    pub fn projections(&self) -> &ProjectionCache {
        &self.proj
    }

    /// Set (or clear) the arrange-mode model-matrix override. Called each frame
    /// from `App` when arrange mode has a selected object. Pass `None` to clear.
    pub fn set_arrange_override(&mut self, ov: Option<DebugArrangeOverride>) {
        self.debug_arrange_override = ov;
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

    /// Push art-direction knobs for the procedural mountain-haze shader
    /// into its uniform buffer. Called once per frame from `main.rs` so
    /// debug-overlay edits take effect immediately.
    ///
    /// When `wall_half_width_uv` is `0`, fog spans the full screen horizontally
    /// (legacy horizon wash). Gameplay passes a positive half-width for a
    /// vertical fog slab centered at `wall_center_x`.
    pub fn set_haze_tuning(
        &self,
        density: f32,
        r: f32,
        g: f32,
        b: f32,
        horizon_y: f32,
        drift_speed: f32,
        wall_center_x: f32,
        wall_half_width_uv: f32,
    ) {
        let uniform = HazeUniform {
            color_density: [r, g, b, density.max(0.0)],
            params: [
                horizon_y.clamp(0.0, 1.0),
                drift_speed.max(0.0),
                wall_center_x.clamp(0.0, 1.0),
                wall_half_width_uv.max(0.0),
            ],
        };
        self.queue
            .write_buffer(&self.haze_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Replace the committed-rotation map (used to apply each Placement's
    /// `rx/ry/rz_deg` to its matching arrange-tagged draw). Called each frame
    /// from `App` with the active scene's entries.
    pub fn set_committed_arrange_rotations(
        &mut self,
        rotations: std::collections::HashMap<String, [f32; 3]>,
    ) {
        self.committed_arrange_rotations = rotations;
    }
    pub fn clear_smoke(&mut self) {
        self.prev_tile_world.clear();
    }
}
