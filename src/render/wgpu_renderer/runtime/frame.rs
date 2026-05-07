use super::*;

pub(super) enum RenderFrame {
    Draw(Option<wgpu::SurfaceTexture>),
    Skip,
}

impl WgpuRenderer {
    fn ensure_showcase_decal_atlas(&mut self, tileset_name: &str) {
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
        self.ensure_showcase_decal_atlas(tileset_name);
        // `tile_3d.wgsl` reads `base_color_factor.w`: procedural kinds 0–2, shop env 4,
        // imported Tile.glb (per-primitive albedo + face decal) 5 — see `tile_body.rs`.
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

        // Cap shell-fluff draws by the effects-quality knob. Shells are
        // the dominant felt-mode perf cost — each one is a full-screen-
        // fragment-bound extra draw of the table mesh, so this is the
        // single most important felt knob.
        self.active_felt_shell_count = effects_quality.felt_shell_count().min(10);
        self.felt_shader_lod = effects_quality.felt_shader_lod();

        // Swap tilesets: if the user picked a different set in Options, update
        // the active name and blow the per-tile decal caches so the next frame
        // re-rasterizes against the new set's PNGs.
        if self.tile_set.as_deref() != Some(tileset_name) {
            self.tile_set = Some(tileset_name.to_owned());
            self.tile_face_overlays.clear();
            self.hand_tiles.clear();
            self.showcase_tiles.clear();
        }
    }

    pub(super) fn acquire_render_frame(&self) -> anyhow::Result<RenderFrame> {
        match &self.target {
            RenderTarget::Surface(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) => Ok(RenderFrame::Draw(Some(t))),
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => Ok(RenderFrame::Draw(Some(t))),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    Ok(RenderFrame::Skip)
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(&self.device, &self.config);
                    Ok(RenderFrame::Skip)
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                    Ok(RenderFrame::Skip)
                }
            },
            RenderTarget::Offscreen { .. } => Ok(RenderFrame::Draw(None)),
        }
    }

    pub(super) fn bloom_is_active(frame: &UiFrame) -> bool {
        frame.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::MoonlitWater | DrawCmd::ShopEnvironment | DrawCmd::EmberDrift
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
        let point_lights_buf = match (
            self.active_scene_key.as_deref(),
            frame.camera_override.as_ref(),
        ) {
            (Some("shop") | Some("tile_pack_celebration"), Some(cam))
                if frame.cmds.iter().any(|c| {
                    matches!(
                        c,
                        crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(_)
                    )
                }) =>
            {
                PointLightsBuf::from_lights_shop_camera(
                    &frame.point_lights,
                    cam,
                    frame.candle_light_count,
                    frame.flame_height_world,
                    1.0,
                    pl_w,
                    pl_h,
                    gamma,
                    time_s,
                )
            }
            _ => PointLightsBuf::from_lights(
                &frame.point_lights,
                frame.candle_light_count,
                frame.flame_height_world,
                1.0,
                pl_w,
                pl_h,
                gamma,
                time_s,
            ),
        };
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&point_lights_buf),
        );

        // Shop `KHR_lights_punctual` uploads — `shop_glb` binding 0 / `lit_mesh` binding 2, inverse-square.
        // `extras.w` dims `lit_mesh` props only; `shop_glb.wgsl` does not read it.
        let shop_gltf_lit_mesh_scale = if frame.shop_env_gltf_punctual {
            self.shop_lit_mesh_gltf_punctual_scale
        } else {
            1.0
        };
        self.queue.write_buffer(
            &self.shop_gltf_point_lights_buffer,
            0,
            bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &frame.shop_gltf_point_lights,
                0,
                frame.flame_height_world,
                shop_gltf_lit_mesh_scale,
                pl_w,
                pl_h,
                gamma,
                time_s,
            )),
        );

        // Upload spotlights for the tile shader (group 3). Scenes push
        // directional cone lights into `frame.spot_lights`; only the tile
        // pipeline samples them (not `lit_mesh`).
        self.queue.write_buffer(
            &self.spot_lights_buffer,
            0,
            bytemuck::bytes_of(&SpotLightsBuf::from_lights(&frame.spot_lights, pl_w, pl_h)),
        );
    }
}
