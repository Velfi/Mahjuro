use crate::table_transform::{rot_euler_xyz_rad, translate_rot_scale};

use super::*;

struct PendingShowcaseTile {
    instance: Tile3dInstance,
    translucent: bool,
    casts_shadow: bool,
    shadow_model: [f32; 16],
}

impl WgpuRenderer {
    /// Showcase tile placement pre-pass: build instanced tile + shadow buffers,
    /// pick rects, and outline instances for all `ShowcaseTileBatch` draws.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_showcase_tiles_placement(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        tile_basis: Mat4,
        tile_preset: mahjuro_gfx_types::TilePreset,
        _dt: f32,
        _light_view_proj_arr: [f32; 16],
        showcase_tile_batches: &[&[crate::draw_cmd::ShowcaseTilePlacement]],
        tile_3d_rects: &mut Vec<(usize, [f32; 4])>,
        tile_pick_models: &mut Vec<(usize, Mat4)>,
        tile_glows: &mut Vec<GpuInstance>,
        shadow_uniforms_changed: &mut bool,
    ) {
        let mut hdr_tonemap = self.tile_hdr_tonemap(frame);
        hdr_tonemap[3] = self.room_punctual_inv_doc_scale(
            camera,
            frame.foreground_scene_lighting().embedded_gltf_punctual,
        );
        let punctual_tuning = self.tile_punctual_tuning(frame);
        self.queue.write_buffer(
            &self.tile_frame_uniform_buffer,
            0,
            bytemuck::bytes_of(&TileFrameUniform {
                view_proj: camera.view_proj_arr,
                cam_pos: camera.cam_pos.to_array(),
                _pad0: 0.0,
                tile_post_params: hdr_tonemap,
                tile_punctual_params: punctual_tuning,
            }),
        );
        self.queue.write_buffer(
            &self.tile_outline_frame_uniform_buffer,
            0,
            bytemuck::bytes_of(&super::super::TileOutlineFrameUniform {
                view_proj: camera.view_proj_arr,
                hdr_tonemap,
            }),
        );

        self.tile_3d_instances_staging.clear();
        self.tile_3d_batch_ranges.clear();
        self.tile_3d_batch_blend_ranges.clear();
        self.tile_shadow_instances_staging.clear();
        self.tile_shadow_batch_ranges.clear();
        self.tile_outline_instances_staging.clear();
        self.tile_outline_batch_ranges.clear();
        self.coin_3d_batch_range = None;
        self.coin_shadow_batch_range = None;

        let total_showcase: usize = showcase_tile_batches
            .iter()
            .map(|b| b.len())
            .sum::<usize>()
            .min(MAX_SHOWCASE_TILE_SLOTS);

        if total_showcase > 0 || !self.coin_3d_draw_state.is_empty() {
            let tileset_owned = self.tile_set.clone().expect(
                "tile_set must be set by apply_render_settings before drawing showcase tiles",
            );
            self.ensure_showcase_decal_atlas(&tileset_owned);
            self.refresh_tile_material_bind_groups();
        }

        let cam_pos = camera.cam_pos;
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
        let gameplay_tile_z_flip = self.active_scene_key == Some("gameplay");

        let mut dora_tile_bounds: Option<[f32; 4]> = None;
        let mut round_wind_tile_bounds: Option<[f32; 4]> = None;
        let grow_bounds = |bounds: &mut Option<[f32; 4]>, rect: [f32; 4]| {
            let x0 = rect[0];
            let y0 = rect[1];
            let x1 = rect[0] + rect[2];
            let y1 = rect[1] + rect[3];
            match bounds.as_mut() {
                Some(b) => {
                    b[0] = b[0].min(x0);
                    b[1] = b[1].min(y0);
                    b[2] = b[2].max(x1);
                    b[3] = b[3].max(y1);
                }
                None => *bounds = Some([x0, y0, x1, y1]),
            }
        };

        let use_ray_plane = frame
            .showcase_render_hints
            .showcase_tiles_use_ray_plane(self.active_scene_key);

        for batch in showcase_tile_batches {
            let batch_3d_start = self.tile_3d_instances_staging.len() as u32;
            let batch_shadow_start = self.tile_shadow_instances_staging.len() as u32;
            let outline_batch_start = self.tile_outline_instances_staging.len() as u32;
            let mut pending = Vec::with_capacity(batch.len());

            for p in batch.iter() {
                if pending.len() >= MAX_SHOWCASE_TILE_SLOTS {
                    break;
                }
                let center = crate::world_space::layout_anchor_to_world(
                    w,
                    h,
                    frame.foreground_camera(),
                    p.center_pos[0],
                    p.center_pos[1],
                    p.center_pos[2],
                    use_ray_plane,
                );
                let tile_short_px = p.size_px * 0.85;
                let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();
                let scale = glam::Vec3::new(
                    tile_long_px / LOCAL_X_EXTENT,
                    tile_thickness_px / LOCAL_Y_EXTENT,
                    tile_short_px / LOCAL_Z_EXTENT,
                ) * p.scale;

                let rotation_z = p.rotation[2]
                    + if gameplay_tile_z_flip {
                        std::f32::consts::PI
                    } else {
                        0.0
                    };
                let base_rotation = rot_euler_xyz_rad(p.rotation[0], p.rotation[1], rotation_z);
                let oriented = base_rotation * tile_basis;
                let model = translate_rot_scale(center, oriented, scale);

                let mut sc_min_x = f32::INFINITY;
                let mut sc_min_y = f32::INFINITY;
                let mut sc_max_x = f32::NEG_INFINITY;
                let mut sc_max_y = f32::NEG_INFINITY;
                for &corner in &[
                    glam::Vec3::new(-0.5, -0.5, -0.5),
                    glam::Vec3::new(0.5, -0.5, -0.5),
                    glam::Vec3::new(-0.5, 0.5, -0.5),
                    glam::Vec3::new(0.5, 0.5, -0.5),
                    glam::Vec3::new(-0.5, -0.5, 0.5),
                    glam::Vec3::new(0.5, -0.5, 0.5),
                    glam::Vec3::new(-0.5, 0.5, 0.5),
                    glam::Vec3::new(0.5, 0.5, 0.5),
                ] {
                    let world_c = model.transform_point3(corner);
                    let (px, py) = project_to_screen(world_c);
                    sc_min_x = sc_min_x.min(px);
                    sc_min_y = sc_min_y.min(py);
                    sc_max_x = sc_max_x.max(px);
                    sc_max_y = sc_max_y.max(py);
                }
                let overlay_w = (sc_max_x - sc_min_x).max(16.0);
                let overlay_h = (sc_max_y - sc_min_y).max(16.0);
                let overlay_x = sc_min_x;
                let overlay_y = sc_min_y;

                use crate::draw_cmd::TileOverlayRectGroup;
                match p.overlay_rect_group {
                    Some(TileOverlayRectGroup::DoraTiles) => grow_bounds(
                        &mut dora_tile_bounds,
                        [overlay_x, overlay_y, overlay_w, overlay_h],
                    ),
                    Some(TileOverlayRectGroup::RoundWindTiles) => grow_bounds(
                        &mut round_wind_tile_bounds,
                        [overlay_x, overlay_y, overlay_w, overlay_h],
                    ),
                    None => {}
                }

                if let Some(pick_id) = p.pick_id {
                    let uid = p.tile.id;
                    self.prev_tile_world.insert(uid, center);
                    tile_3d_rects.push((pick_id, [overlay_x, overlay_y, overlay_w, overlay_h]));
                    tile_pick_models.push((pick_id, model));

                    if p.glow {
                        let gw = overlay_w * 1.50;
                        let gh = overlay_h * 1.55;
                        let gx = overlay_x + (overlay_w - gw) * 0.5;
                        let gy = overlay_y + (overlay_h - gh) * 0.5;
                        tile_glows.push(GpuInstance {
                            rect: [gx, gy, gw, gh],
                            color: p.glow_color.unwrap_or([1.00, 0.38, 0.05, 0.62]),
                            user: 0,
                        });
                    }
                }

                let mut sc_bcf = self.tile_base_color_factor;
                sc_bcf[0] = p.brightness;
                sc_bcf[1] = if p.hovered {
                    0.5
                } else if p.selected {
                    1.0
                } else {
                    0.0
                };
                sc_bcf[2] = p.tile.enhancement.map_or(0.0, |e| e.shader_id());

                let opacity = p.opacity.clamp(0.0, 1.0);
                let translucent = opacity < 0.999;
                let casts_shadow = !frame.showcase_render_hints.zodiac_celebration_no_shadow
                    && !translucent;
                pending.push(PendingShowcaseTile {
                    instance: Tile3dInstance {
                        model: model.to_cols_array(),
                        tile_visual_params: sc_bcf,
                        tile_decal_atlas_uv: self.decal_atlas_uv_for(&p.tile),
                        tile_material_seed: p.tile.id as f32,
                        tile_opacity: opacity,
                        _pad: [0.0; 2],
                    },
                    translucent,
                    casts_shadow,
                    shadow_model: model.to_cols_array(),
                });

                if p.outline {
                    const OUTLINE_GROW: f32 = 1.11;
                    let outline_scale = scale * OUTLINE_GROW;
                    let outline_model = translate_rot_scale(center, oriented, outline_scale);
                    let mut outline_bcf = sc_bcf;
                    if p.hovered && p.selected {
                        outline_bcf[1] = 1.5;
                    }
                    self.tile_outline_instances_staging.push(TileOutlineInstance {
                        model: outline_model.to_cols_array(),
                        base_color_factor: outline_bcf,
                    });
                }
            }

            let opaque_start = self.tile_3d_instances_staging.len() as u32;
            for tile in pending.iter().filter(|t| !t.translucent) {
                self.tile_3d_instances_staging.push(tile.instance);
                if tile.casts_shadow {
                    self.tile_shadow_instances_staging.push(TileShadowInstance {
                        model: tile.shadow_model,
                    });
                }
            }
            let opaque_n = self.tile_3d_instances_staging.len() as u32 - opaque_start;

            let blend_start = self.tile_3d_instances_staging.len() as u32;
            for tile in pending.iter().filter(|t| t.translucent) {
                self.tile_3d_instances_staging.push(tile.instance);
            }
            let blend_n = self.tile_3d_instances_staging.len() as u32 - blend_start;

            self.tile_3d_batch_ranges.push((opaque_start, opaque_n));
            self.tile_3d_batch_blend_ranges.push((blend_start, blend_n));

            let batch_shadow_n =
                self.tile_shadow_instances_staging.len() as u32 - batch_shadow_start;
            self.tile_shadow_batch_ranges.push((batch_shadow_start, batch_shadow_n));
            let outline_n =
                self.tile_outline_instances_staging.len() as u32 - outline_batch_start;
            self.tile_outline_batch_ranges
                .push((outline_batch_start, outline_n));

            let _ = batch_3d_start;
        }

        // Append coin instances after showcase batches.
        if !self.coin_3d_draw_state.is_empty() {
            let coin_3d_start = self.tile_3d_instances_staging.len() as u32;
            let coin_shadow_start = self.tile_shadow_instances_staging.len() as u32;
            for coin in &self.coin_3d_draw_state {
                self.tile_3d_instances_staging.push(coin.instance);
                if coin.casts_shadow {
                    self.tile_shadow_instances_staging.push(TileShadowInstance {
                        model: coin.instance.model,
                    });
                }
            }
            let coin_3d_n = self.tile_3d_instances_staging.len() as u32 - coin_3d_start;
            let coin_shadow_n =
                self.tile_shadow_instances_staging.len() as u32 - coin_shadow_start;
            self.coin_3d_batch_range = Some((coin_3d_start, coin_3d_n));
            self.coin_shadow_batch_range = Some((coin_shadow_start, coin_shadow_n));
        }

        if !self.tile_3d_instances_staging.is_empty() {
            self.queue.write_buffer(
                &self.tile_3d_instance_buffer,
                0,
                bytemuck::cast_slice(&self.tile_3d_instances_staging),
            );
        }
        if !self.tile_shadow_instances_staging.is_empty() {
            *shadow_uniforms_changed = true;
            self.queue.write_buffer(
                &self.tile_shadow_instance_buffer,
                0,
                bytemuck::cast_slice(&self.tile_shadow_instances_staging),
            );
        }
        if !self.tile_outline_instances_staging.is_empty() {
            self.queue.write_buffer(
                &self.tile_outline_instance_buffer,
                0,
                bytemuck::cast_slice(&self.tile_outline_instances_staging),
            );
        }

        self.proj.dora_tile_rect = dora_tile_bounds
            .map(|b| [b[0], b[1], (b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0)]);
        self.proj.round_wind_tile_rect = round_wind_tile_bounds
            .map(|b| [b[0], b[1], (b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0)]);

        self.proj.hand_rects.clone_from(tile_3d_rects);
        self.last_pick_models.clone_from(tile_pick_models);
        self.last_pick_camera = Some(PickCamera {
            inv_view_proj: camera.view_proj.inverse(),
            viewport_w: w,
            viewport_h: h,
        });

        self.proj.relic_rects.clear();
        for (model, _rid) in &self.last_relic_models {
            self.proj.relic_rects.push(camera.project_unit_cube_rect(*model));
        }
        self.proj.ribbon_rects.clear();
        for model in &self.last_ribbon_models {
            self.proj
                .ribbon_rects
                .push(camera.project_aabb_rect(*model, [0.5, 0.5, 0.05], 0.0));
        }
        self.proj.talisman_rects.clear();
        for model in &self.last_talisman_models {
            self.proj
                .talisman_rects
                .push(camera.project_aabb_rect(*model, TALISMAN_LOCAL_HALF, 0.0));
        }
        for (rect, pick_id) in &self.proj.pack_rects {
            self.proj.aux_dish_rects.push((*pick_id, *rect));
        }

        let _ = (cam_pos, view_proj_arr);
    }
}
