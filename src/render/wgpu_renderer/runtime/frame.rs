use super::*;

pub(super) enum RenderFrame {
    Draw(Option<wgpu::SurfaceTexture>),
    Skip,
}

impl WgpuRenderer {
    pub(super) fn apply_render_settings(
        &mut self,
        tile_material: crate::persistence::TileMaterial,
        tileset_name: &str,
    ) {
        // Encode the tile material choice into base_color_factor.w so the
        // tile_3d shader can branch on it (0 = bamboo, 1 = plastic, ...).
        self.tile_base_color_factor[3] = tile_material.shader_id();

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
        use crate::render::draw_cmd::Object3dKind;
        frame.cmds.iter().any(|cmd| match cmd {
            DrawCmd::MoonlitWater => true,
            DrawCmd::Object3d(obj) => matches!(obj.kind, Object3dKind::ShopLamp { .. }),
            DrawCmd::Object3dBatch(objs) => objs
                .iter()
                .any(|o| matches!(o.kind, Object3dKind::ShopLamp { .. })),
            _ => false,
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

        // Upload point lights for the tile shader (group 1). Scenes push
        // candle/spot lights into `frame.point_lights` in pixel-layout
        // coordinates; we map them onto the table-plane world for upload.
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &frame.point_lights,
                frame.candle_light_count,
                frame.flame_height_world,
                pl_w,
                pl_h,
                gamma,
                self.creation_time.elapsed().as_secs_f32(),
            )),
        );

        // Upload spotlights for the tile shader (group 3). Scenes push
        // directional cone lights into `frame.spot_lights`; only the tile
        // pipeline samples them (lit_mesh and the smoke lightbake don't).
        self.queue.write_buffer(
            &self.spot_lights_buffer,
            0,
            bytemuck::bytes_of(&SpotLightsBuf::from_lights(&frame.spot_lights, pl_w, pl_h)),
        );
    }
}
