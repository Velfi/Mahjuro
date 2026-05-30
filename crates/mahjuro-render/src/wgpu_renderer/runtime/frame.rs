use super::*;

pub(super) enum RenderFrame {
    Draw(Option<wgpu::SurfaceTexture>),
    Skip,
}

impl WgpuRenderer {
    #[inline]
    pub(super) fn active_tile_mesh(&self) -> &TileMeshGpuSet {
        &self.tile_meshes[crate::tile_glb::tile_material_index(self.active_tile_material)]
    }

    fn push_active_showcase_decal_atlas_to_cache(&mut self) {
        let (Some(tileset), Some(atlas)) = (
            self.showcase_decal_atlas_tileset.take(),
            self.showcase_decal_atlas.take(),
        ) else {
            return;
        };
        if let Some(pos) = self
            .showcase_decal_atlas_cache
            .iter()
            .position(|(name, _)| name == &tileset)
        {
            let _ = self.showcase_decal_atlas_cache.remove(pos);
        }
        self.showcase_decal_atlas_cache.push_front((tileset, atlas));
    }

    fn activate_cached_showcase_decal_atlas(&mut self, tileset_name: &str) -> bool {
        let Some(pos) = self
            .showcase_decal_atlas_cache
            .iter()
            .position(|(name, _)| name == tileset_name)
        else {
            return false;
        };
        let Some((name, atlas)) = self.showcase_decal_atlas_cache.remove(pos) else {
            return false;
        };
        self.showcase_decal_atlas = Some(atlas);
        self.showcase_decal_atlas_tileset = Some(name);
        true
    }

    pub(crate) fn ensure_showcase_decal_atlas(&mut self, tileset_name: &str) {
        if self.showcase_decal_atlas_tileset.as_deref() == Some(tileset_name)
            && self.showcase_decal_atlas.is_some()
        {
            return;
        }
        self.push_active_showcase_decal_atlas_to_cache();
        if self.activate_cached_showcase_decal_atlas(tileset_name) {
            return;
        }
        let atlas = crate::showcase_decal_atlas::load_showcase_decal_atlas(
            &self.device,
            &self.queue,
            tileset_name,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        self.showcase_decal_atlas = Some(atlas);
        self.showcase_decal_atlas_tileset = Some(tileset_name.to_string());
    }

    pub(super) fn apply_render_settings(
        &mut self,
        tile_material: mahjuro_gfx_types::TileMaterial,
        _effects_quality: mahjuro_gfx_types::EffectsQuality,
        tileset_name: &str,
    ) {
        // Showcase decal atlas is loaded lazily in `run_showcase_tiles_placement` when
        // the frame actually draws `ShowcaseTileBatch`.
        // `tile_3d.wgsl` reads `base_color_factor.w`: procedural kinds 0–2 for the legacy
        // procedural mesh; 4 = shop env (base map only); 5 = `tile.glb` + projected decal.
        // Imported tile meshes must use kind 5 — procedural 0–2 assumes authored local frame
        // (front-face + ivory band) and reads nearly black on GLB geometry (e.g. pack reveal).
        self.tile_base_color_factor[3] = if self.active_tile_mesh().primitives.is_empty() {
            crate::tile_body::TileBodyShaderKind::resolve(tile_material).id()
        } else {
            crate::tile_body::TEXTURED_TILE_GAMEPLAY_BODY_KIND
        };

        if self.active_tile_material != tile_material {
            self.active_tile_material = tile_material;
            self.hand_tiles.clear();
            self.showcase_tiles.clear();
        }

        // Swap tilesets: if the user picked a different set in Options, update
        // the active name and blow the per-tile decal caches so the next frame
        // re-rasterizes against the new set's PNGs.
        if self.tile_set.as_deref() != Some(tileset_name) {
            self.push_active_showcase_decal_atlas_to_cache();
            self.tile_set = Some(tileset_name.to_owned());
            self.tile_face_overlays.clear();
            self.hand_tiles.clear();
            self.showcase_tiles.clear();
            let _ = self.activate_cached_showcase_decal_atlas(tileset_name);
        }
    }

    pub(super) fn acquire_render_frame(&self) -> anyhow::Result<RenderFrame> {
        match &self.target {
            RenderTarget::Surface(surface) => {
                // When acquisition fails we skip the whole frame (no `present()`). On Metal
                // that often reads as a persistent black window after launch unfocused /
                // occlusion, or right after `configure` — poll + extra acquire attempts usually
                // land a drawable the same tick.
                let telemetry = &self.acquire_telemetry;
                let try_once = |s: &wgpu::Surface| match s.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t) => {
                        telemetry.record_attempt(super::AcquireOutcome::Success);
                        Some(t)
                    }
                    wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                        telemetry.record_attempt(super::AcquireOutcome::Suboptimal);
                        Some(t)
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        telemetry.record_attempt(super::AcquireOutcome::TimeoutOrOccluded);
                        None
                    }
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        telemetry.record_attempt(super::AcquireOutcome::Outdated);
                        s.configure(&self.device, &self.config);
                        None
                    }
                    wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                        telemetry.record_attempt(super::AcquireOutcome::Lost);
                        log::warn!(
                            "swapchain surface lost or invalid — reconfiguring (next frame should recover)"
                        );
                        s.configure(&self.device, &self.config);
                        None
                    }
                };

                let start = Instant::now();
                let outcome = {
                    if let Some(t) = try_once(surface) {
                        Ok(RenderFrame::Draw(Some(t)))
                    } else {
                        let _ = self.device.poll(wgpu::PollType::Poll);
                        if let Some(t) = try_once(surface) {
                            Ok(RenderFrame::Draw(Some(t)))
                        } else {
                            std::thread::yield_now();
                            let _ = self.device.poll(wgpu::PollType::Poll);
                            if let Some(t) = try_once(surface) {
                                Ok(RenderFrame::Draw(Some(t)))
                            } else {
                                Ok(RenderFrame::Skip)
                            }
                        }
                    }
                };
                let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
                let frame_drawn = matches!(&outcome, Ok(RenderFrame::Draw(Some(_))));
                telemetry.record_frame(elapsed_ms, frame_drawn);
                outcome
            }
            RenderTarget::Offscreen { .. } => Ok(RenderFrame::Draw(None)),
        }
    }

    pub(super) fn bloom_is_active(frame: &UiFrame) -> bool {
        frame.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::MoonlitWater
                    | DrawCmd::ShopEnvironment
                    | DrawCmd::HallwayEnvironment
                    | DrawCmd::StaircaseEnvironment
                    | DrawCmd::ArchiveEnvironment
                    // Main menu stars / emissive sky read too hot with full HDR bloom.
                    | DrawCmd::EmberDrift
            )
        })
    }

    /// Linear HDR gain for embedded room env passes (matches `write_gltf_room_env_uniforms`).
    pub(super) fn room_env_linear_hdr_gain(
        frame: &UiFrame,
        shop_env_linear_exposure: f32,
    ) -> Option<f32> {
        use crate::room_glb::ROOM_GLB_LINEAR_EXPOSURE_BASE;

        let has_room_env = frame.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::ShopEnvironment
                    | DrawCmd::HallwayEnvironment
                    | DrawCmd::StaircaseEnvironment
                    | DrawCmd::ArchiveEnvironment
                    | DrawCmd::MainMenuEnvironment
                    | DrawCmd::GameplayEnvironment
            )
        });
        if !has_room_env && !frame.uses_room_glb_shader() {
            return None;
        }

        let hallway = frame
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::HallwayEnvironment));
        let staircase = frame
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::StaircaseEnvironment));
        let archive = frame
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::ArchiveEnvironment));
        let main_menu = frame
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::MainMenuEnvironment));

        let mut gain = shop_env_linear_exposure * ROOM_GLB_LINEAR_EXPOSURE_BASE;
        if hallway {
            gain *= crate::hallway_glb::HALLWAY_ENV_LINEAR_EXPOSURE_MUL;
        }
        if staircase {
            gain *= crate::staircase_glb::STAIRCASE_ENV_LINEAR_EXPOSURE_MUL;
        }
        if archive {
            gain *= crate::archive_glb::ARCHIVE_ENV_LINEAR_EXPOSURE_MUL;
        }
        if main_menu {
            gain *= crate::main_menu_glb::MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL;
        }
        Some(gain)
    }

    /// `(threshold, composite_strength, extract_scale)` for the bloom passes.
    pub(super) fn bloom_render_tuning(
        frame: &UiFrame,
        shop_env_linear_exposure: f32,
    ) -> (f32, f32, f32) {
        if !Self::bloom_is_active(frame) {
            return (9999.0, 0.0, 0.0);
        }

        const THRESHOLD: f32 = 1.30;
        const EXTRACT_SCALE: f32 = 0.85;
        const STRENGTH_NON_ROOM: f32 = 0.28;

        let strength =
            if let Some(gain) = Self::room_env_linear_hdr_gain(frame, shop_env_linear_exposure) {
                // glTF emissive is absolute HDR; lit surfaces use `tile_seed` exposure
                // (often ≪ 1). Scale bloom down in crushed rooms so candle halos stay
                // tight instead of fogging the frame.
                let ev = gain.max(1e-8).log2();
                ((ev + 8.5) * 0.045 + 0.14).clamp(0.12, 0.36)
            } else {
                STRENGTH_NON_ROOM
            };

        (THRESHOLD, strength, EXTRACT_SCALE)
    }

    pub(super) fn advance_frame_timers(
        &mut self,
        draw_settle_speed: f32,
        sort_settle_speed: f32,
    ) -> f32 {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.05);
        self.last_frame = now;
        // Cache for downstream prep loops (bowl/mirror hover envelopes,
        // etc.) so they don't have to recompute or re-clamp the timestamp.
        self.frame_dt = dt;

        for y in self.tile_anim_y.iter_mut() {
            *y *= (-draw_settle_speed * dt).exp(); // exponential ease-out
            if y.abs() < 0.5 {
                *y = 0.0;
            }
        }
        for x in self.tile_anim_x.iter_mut() {
            *x *= (-sort_settle_speed * dt).exp();
            if x.abs() < 0.01 {
                *x = 0.0;
            }
        }

        dt
    }

    pub(super) fn upload_frame_uniforms(
        &self,
        frame: &UiFrame,
        effects_quality: mahjuro_gfx_types::EffectsQuality,
        cascade_effects_quality: mahjuro_gfx_types::EffectsQuality,
        gamma: f32,
    ) {
        // Update globals with current time for animated shaders.
        let w_f = self.size.width as f32;
        let h_f = self.size.height as f32;
        let (cx, cy) = frame.cursor_pos.unwrap_or((w_f * 0.5, h_f * 0.5));
        let cascade_quality_level = if frame.transition_progress > 0.0
            && frame.transition_progress < 1.0
        {
            cascade_effects_quality.quality_level_f32()
        } else {
            0.0
        };
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [w_f, h_f],
                time: self.creation_time.elapsed().as_secs_f32(),
                gamma: gamma.max(0.01),
                cursor_pos: [cx, cy],
                transition_progress: frame.transition_progress,
                quality_level: effects_quality.quality_level_f32(),
                moon_phase: current_moon_phase(),
                _globals_pad: [
                    cascade_quality_level,
                    if crate::main_menu_glb::main_menu_pride_rainbow_active(
                        self.main_menu_pride_rainbow_debug,
                    ) {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                ],
            }),
        );
        // Gameplay / artist-style point lights (group 1 for tiles + lit_mesh).
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        let time_s = self.creation_time.elapsed().as_secs_f32();
        let lit_mesh_inv_scale = if frame.scene_lighting.embedded_gltf_punctual {
            self.active_frame_env().lit_mesh_gltf_punctual_scale
        } else {
            1.0
        };
        let punctual_bake = PunctualLightBakeParams {
            src: &frame.scene_lighting.punctual,
            candle_count: frame.candle_light_count,
            flame_height_world: frame.flame_height_world,
            lit_mesh_punctual_intensity_scale: lit_mesh_inv_scale,
            screen_w: pl_w,
            screen_h: pl_h,
            gamma,
            time: time_s,
        };
        let shop_camera_punctual = |cam: &crate::draw_cmd::CameraParams| {
            PointLightsBuf::from_scene_punctual_shop_camera(&PunctualLightBakeShopCameraParams {
                bake: &punctual_bake,
                cam,
            })
        };
        let use_ray_plane = frame
            .showcase_render_hints
            .layout_uses_ray_plane(self.active_scene_key);
        let point_lights_buf = match (use_ray_plane, frame.camera_override.as_ref()) {
            (true, Some(cam)) => shop_camera_punctual(cam),
            _ => PointLightsBuf::from_scene_punctual(&punctual_bake),
        };
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&point_lights_buf),
        );

        // Upload spotlights (tile + `lit_mesh` group 3).
        let spot_cam = if use_ray_plane {
            frame.camera_override.as_ref()
        } else {
            None
        };
        self.queue.write_buffer(
            &self.spot_lights_buffer,
            0,
            bytemuck::bytes_of(&SpotLightsBuf::from_lights(
                &frame.scene_lighting.spot_lights,
                pl_w,
                pl_h,
                spot_cam,
            )),
        );
    }
}
