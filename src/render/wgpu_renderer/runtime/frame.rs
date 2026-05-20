use super::*;

pub(super) enum RenderFrame {
    Draw(Option<wgpu::SurfaceTexture>),
    Skip,
}

impl WgpuRenderer {
    pub(crate) fn ensure_showcase_decal_atlas(&mut self, tileset_name: &str) {
        if self.showcase_decal_atlas_tileset.as_deref() == Some(tileset_name)
            && self.showcase_decal_atlas.is_some()
        {
            return;
        }
        let atlas = crate::render::showcase_decal_atlas::build_showcase_decal_atlas_texture(
            &self.device,
            &self.queue,
            self.ui_font.as_ref(),
            self.emoji_font.as_ref(),
            Some(tileset_name),
        );
        self.showcase_decal_atlas = Some(atlas);
        self.showcase_decal_atlas_tileset = Some(tileset_name.to_string());
    }

    pub(super) fn apply_render_settings(
        &mut self,
        tile_material: crate::persistence::TileMaterial,
        surface_kind: crate::persistence::SurfaceKind,
        effects_quality: crate::persistence::EffectsQuality,
        tileset_name: &str,
    ) {
        // Showcase decal atlas is built lazily in `run_showcase_tiles_placement` when
        // the frame actually draws `ShowcaseTileBatch` — avoids 336 CPU raster passes
        // at startup and on scenes that never use showcase tiles.
        // `tile_3d.wgsl` reads `base_color_factor.w`: procedural kinds 0–2 for the legacy
        // procedural mesh; 4 = shop env (base map only); 5 = `tile.glb` + projected decal.
        // Imported tile meshes must use kind 5 — procedural 0–2 assumes authored local frame
        // (front-face + ivory band) and reads nearly black on GLB geometry (e.g. pack reveal).
        self.tile_base_color_factor[3] = if self.tile_primitives.is_empty() {
            crate::render::tile_body::TileBodyShaderKind::resolve(tile_material).id()
        } else {
            crate::render::tile_body::TEXTURED_TILE_GAMEPLAY_BODY_KIND
        };

        // Pick which procedural surface the table mesh routes through. The
        // shader branch is selected by `material_params.x` (the kind), so
        // swapping the params is enough — no pipeline / mesh rebuild.
        self.table_material = match surface_kind {
            crate::persistence::SurfaceKind::Walnut => {
                crate::render::lit_mesh::MaterialParams::lacquered_wood()
            }
            crate::persistence::SurfaceKind::GreenFelt => {
                crate::render::lit_mesh::MaterialParams::felt_green()
            }
        };

        self.felt_shader_lod = effects_quality.felt_shader_lod();

        // Swap tilesets: if the user picked a different set in Options, update
        // the active name and blow the per-tile decal caches so the next frame
        // re-rasterizes against the new set's PNGs.
        if self.tile_set.as_deref() != Some(tileset_name) {
            self.tile_set = Some(tileset_name.to_owned());
            self.tile_face_overlays.clear();
            self.hand_tiles.clear();
            self.showcase_tiles.clear();
            self.showcase_decal_atlas = None;
            self.showcase_decal_atlas_tileset = None;
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
                    | DrawCmd::ArchiveEnvironment
                    // Main menu stars / emissive sky read too hot with full HDR bloom.
                    | DrawCmd::EmberDrift
            )
        })
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

        for tile in self.departing_tiles.iter_mut() {
            tile.elapsed += dt;
        }
        self.departing_tiles.retain(|t| t.elapsed < t.lifetime);

        dt
    }

    pub(super) fn upload_frame_uniforms(
        &self,
        frame: &UiFrame,
        effects_quality: crate::persistence::EffectsQuality,
        gamma: f32,
    ) {
        // Update globals with current time for animated shaders.
        let w_f = self.size.width as f32;
        let h_f = self.size.height as f32;
        let (cx, cy) = frame.cursor_pos.unwrap_or((w_f * 0.5, h_f * 0.5));
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
                _globals_pad: [0.0; 3],
            }),
        );
        // Gameplay / artist-style point lights (group 1 for tiles + lit_mesh).
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        let time_s = self.creation_time.elapsed().as_secs_f32();
        let has_showcase_tiles = frame
            .cmds
            .iter()
            .any(|c| matches!(c, crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(_)));
        let h = &frame.showcase_render_hints;
        let lit_mesh_inv_scale = if frame.scene_lighting.embedded_gltf_punctual {
            self.shop_lit_mesh_gltf_punctual_scale
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
        let shop_camera_punctual = |cam: &crate::render::draw_cmd::CameraParams| {
            PointLightsBuf::from_scene_punctual_shop_camera(&PunctualLightBakeShopCameraParams {
                bake: &punctual_bake,
                cam,
            })
        };
        let point_lights_buf = match (self.active_scene_key, frame.camera_override.as_ref()) {
            // Pack closeup has no showcase tiles; lights must still use the same
            // ray → plane_z mapping as perspective `Object3d` / showcase placement.
            (Some("tile_pack_celebration"), Some(cam)) => shop_camera_punctual(cam),
            (Some("showcase"), Some(cam))
                if h.object3d_use_camera_ray_plane_z
                    || (h.showcase_tiles_use_camera_ray_plane_z && has_showcase_tiles) =>
            {
                shop_camera_punctual(cam)
            }
            (Some("shop"), Some(cam)) if has_showcase_tiles => shop_camera_punctual(cam),
            // Pick-blind always uses a perspective `camera_override` (hallway GLB).
            // Smooth fills must use the same ray → plane_z mapping as the env mesh;
            // `pixel_to_world` is not the inverse of that projection (see `world_space.rs`).
            (Some("pick_blind"), Some(cam)) => shop_camera_punctual(cam),
            (Some("main_menu_exterior"), Some(cam)) => shop_camera_punctual(cam),
            _ => PointLightsBuf::from_scene_punctual(&punctual_bake),
        };
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&point_lights_buf),
        );

        // Upload spotlights (tile + `lit_mesh` group 3).
        let spot_cam = match self.active_scene_key {
            Some("tile_pack_celebration") => frame.camera_override.as_ref(),
            Some("showcase") if frame.showcase_render_hints.object3d_use_camera_ray_plane_z => {
                frame.camera_override.as_ref()
            }
            _ => None,
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
