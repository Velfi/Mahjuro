use crate::table_transform::{rot_euler_xyz_rad, translate_rot_scale};

use super::*;

impl WgpuRenderer {
    /// Showcase tile placement pre-pass: grow / update the showcase tile pool
    /// so each tile in every `ShowcaseTileBatch` has a ready-to-draw
    /// `ShowcaseTileGpu` slot with the correct decal and up-to-date model
    /// matrix. Also runs the HandStrip arrange-mode pre-pass.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_showcase_tiles_placement(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        tile_basis: Mat4,
        tile_preset: mahjuro_gfx_types::TilePreset,
        _dt: f32,
        light_view_proj_arr: [f32; 16],
        showcase_tile_batches: &[&[crate::draw_cmd::ShowcaseTilePlacement]],
        tile_3d_rects: &mut Vec<(usize, [f32; 4])>,
        tile_pick_models: &mut Vec<(usize, Mat4)>,
        tile_glows: &mut Vec<GpuInstance>,
        shadow_uniforms_changed: &mut bool,
    ) {
        let mut hdr_tonemap = self.tile_hdr_tonemap(frame);
        hdr_tonemap[3] =
            self.room_punctual_inv_doc_scale(camera, frame.scene_lighting.embedded_gltf_punctual);
        let punctual_tuning = self.tile_punctual_tuning(frame);
        self.queue.write_buffer(
            &self.tile_outline_frame_uniform_buffer,
            0,
            bytemuck::bytes_of(&super::super::TileOutlineFrameUniform {
                view_proj: camera.view_proj_arr,
                hdr_tonemap,
            }),
        );
        self.tile_outline_instances_staging.clear();
        self.tile_outline_batch_ranges.clear();
        let cam_pos = camera.cam_pos;
        let view_proj = camera.view_proj;
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
        let project_unit_cube_rect =
            |model: Mat4| -> [f32; 4] { camera.project_unit_cube_rect(model) };
        let project_aabb_rect = |model: Mat4, half: [f32; 3], center_y: f32| -> [f32; 4] {
            camera.project_aabb_rect(model, half, center_y)
        };
        // Gameplay hand/structure tiles use GLB-authored rotation (often identity) and need
        // a π Z correction here. Tutorial showcase tiles already bake π into placement
        // (`TUTORIAL_TILE_ROTATION`); applying the same flip would double-invert them.
        let gameplay_tile_z_flip = self.active_scene_key == Some("gameplay");
        // ── Showcase tile GPU resources + uniforms ────────────────────────
        // Grow or update the pool so each tile in every ShowcaseTileBatch has
        // a ready-to-draw ShowcaseTileGpu slot with the correct decal and
        // up-to-date model matrix.
        {
            let total_showcase: usize = showcase_tile_batches
                .iter()
                .map(|b| b.len())
                .sum::<usize>()
                .min(MAX_SHOWCASE_TILE_SLOTS);

            if total_showcase > 0 {
                let tileset_owned = self.tile_set.clone().expect(
                    "tile_set must be set by apply_render_settings before drawing showcase tiles",
                );
                self.ensure_showcase_decal_atlas(&tileset_owned);
            }

            // Ensure we have enough slots.
            while self.showcase_tiles.len() < total_showcase {
                // Placeholder — will be rebuilt immediately below if tile_id
                // doesn't match, but we need *something* to hold the GPU
                // resources. Use the first tile from the first batch.
                let placeholder_tile = showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .next()
                    .map(|p| &p.tile);
                if let Some(tile) = placeholder_tile {
                    let decal_atlas = self
                        .showcase_decal_atlas
                        .as_ref()
                        .expect("showcase decal atlas must be built when showcase tiles are drawn");
                    let ctx = ShowcaseTileCtx {
                        device: &self.device,
                        layout: &self.tile_material_layout,
                        shadow_caster_layout: &self.shadow_caster_layout,
                        primitives: self.active_tile_mesh().primitives.as_slice(),
                        decal_atlas,
                        distortion_placeholder: &self.tile_env_distortion_placeholder,
                    };
                    let stg = make_showcase_tile_gpu(&ctx, self.tile_base_color_factor, tile);
                    self.showcase_tiles.push(stg);
                } else {
                    break;
                }
            }

            // Track hand-tile world centers for the hand-strip debug pickable
            // (registered after the loop).
            let mut dora_tile_bounds: Option<[f32; 4]> = None; // [min_x, min_y, max_x, max_y]
            let mut round_wind_tile_bounds: Option<[f32; 4]> = None; // [min_x, min_y, max_x, max_y]
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

            let mut slot_cursor = 0usize;
            for batch in showcase_tile_batches.iter() {
                let outline_batch_start = self.tile_outline_instances_staging.len() as u32;
                for p in batch.iter() {
                    if slot_cursor >= MAX_SHOWCASE_TILE_SLOTS {
                        break;
                    }
                    let wanted_id = (
                        p.tile.suit,
                        p.tile.rank,
                        p.tile.enhancement,
                        p.tile.debuffed_visual,
                    );
                    // Re-rasterise decal if the tile identity changed.
                    if self.showcase_tiles[slot_cursor].tile_id != wanted_id {
                        let decal_atlas = self.showcase_decal_atlas.as_ref().expect(
                            "showcase decal atlas must be built when showcase tiles are drawn",
                        );
                        let ctx = ShowcaseTileCtx {
                            device: &self.device,
                            layout: &self.tile_material_layout,
                            shadow_caster_layout: &self.shadow_caster_layout,
                            primitives: self.active_tile_mesh().primitives.as_slice(),
                            decal_atlas,
                            distortion_placeholder: &self.tile_env_distortion_placeholder,
                        };
                        self.showcase_tiles[slot_cursor] =
                            make_showcase_tile_gpu(&ctx, self.tile_base_color_factor, &p.tile);
                    }

                    let use_ray_plane = frame
                        .showcase_render_hints
                        .showcase_tiles_use_ray_plane(self.active_scene_key);
                    let center = crate::world_space::layout_anchor_to_world(
                        w,
                        h,
                        frame.camera_override.as_ref(),
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
                    // Project the tile's 8 corners to get a tight-ish screen AABB.
                    let lx = tile_long_px * 0.5;
                    let ly = tile_thickness_px * 0.5;
                    let lz = tile_short_px * 0.5;
                    let sc_corners = [
                        glam::Vec3::new(-lx, -ly, -lz),
                        glam::Vec3::new(lx, -ly, -lz),
                        glam::Vec3::new(-lx, ly, -lz),
                        glam::Vec3::new(lx, ly, -lz),
                        glam::Vec3::new(-lx, -ly, lz),
                        glam::Vec3::new(lx, -ly, lz),
                        glam::Vec3::new(-lx, ly, lz),
                        glam::Vec3::new(lx, ly, lz),
                    ];
                    let mut sc_min_x = f32::INFINITY;
                    let mut sc_min_y = f32::INFINITY;
                    let mut sc_max_x = f32::NEG_INFINITY;
                    let mut sc_max_y = f32::NEG_INFINITY;
                    for c in sc_corners {
                        let world_c = center + oriented.transform_point3(c);
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

                    let stg = &self.showcase_tiles[slot_cursor];
                    let mut sc_bcf = self.tile_base_color_factor;
                    sc_bcf[0] = p.brightness;
                    // 1.0 = selected (gold rim), 0.5 = hovered (cool rim),
                    // 0.0 = none. Hovered supersedes selected.
                    sc_bcf[1] = if p.hovered {
                        0.5
                    } else if p.selected {
                        1.0
                    } else {
                        0.0
                    };
                    sc_bcf[2] = p.tile.enhancement.map_or(0.0, |e| e.shader_id());
                    // Per-tile procedural variation in tile_3d.wgsl (e.g.
                    // tortoise shell mottling) is seeded from the tile's
                    // unique run-scoped id so a given tile keeps the same
                    // pattern across draws, shuffles, and reorders.
                    let tile_seed = p.tile.id as f32;
                    self.queue.write_buffer(
                        &stg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: sc_bcf,
                            cam_pos: cam_pos.to_array(),
                            tile_seed,
                            decal_atlas_uv: stg.decal_atlas_uv,
                            hdr_tonemap,
                            punctual_tuning,
                        }),
                    );
                    if p.outline {
                        const OUTLINE_GROW: f32 = 1.07;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(center, oriented, outline_scale);
                        let mut outline_bcf = sc_bcf;
                        // 1.5 = combined hover+selected perimeter alternation mode.
                        if p.hovered && p.selected {
                            outline_bcf[1] = 1.5;
                        }
                        self.tile_outline_instances_staging.push(
                            super::super::TileOutlineInstance {
                                model: outline_model.to_cols_array(),
                                base_color_factor: outline_bcf,
                            },
                        );
                    }
                    let su = ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    };
                    {
                        let stg_mut = &mut self.showcase_tiles[slot_cursor];
                        stg_mut.casts_shadow = true;
                        if stg_mut.cached_shadow_caster != su {
                            stg_mut.cached_shadow_caster = su;
                            self.queue.write_buffer(
                                &stg_mut.shadow_uniform_buffer,
                                0,
                                bytemuck::bytes_of(&su),
                            );
                            *shadow_uniforms_changed = true;
                        }
                    }

                    slot_cursor += 1;
                }
                let outline_n =
                    self.tile_outline_instances_staging.len() as u32 - outline_batch_start;
                self.tile_outline_batch_ranges
                    .push((outline_batch_start, outline_n));
            }

            self.proj.dora_tile_rect = dora_tile_bounds
                .map(|b| [b[0], b[1], (b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0)]);
            self.proj.round_wind_tile_rect = round_wind_tile_bounds
                .map(|b| [b[0], b[1], (b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0)]);

            if !self.tile_outline_instances_staging.is_empty() {
                self.queue.write_buffer(
                    &self.tile_outline_instance_buffer,
                    0,
                    bytemuck::cast_slice(&self.tile_outline_instances_staging),
                );
            }
        }

        // Snapshot projected tile rects and pick models now that both the hand
        // pre-pass and showcase pre-pass have had a chance to push entries.
        self.proj.hand_rects = tile_3d_rects.clone();
        self.last_pick_models = tile_pick_models.clone();
        self.last_pick_camera = Some(PickCamera {
            inv_view_proj: view_proj.inverse(),
            viewport_w: w,
            viewport_h: h,
        });

        // Rebuild projected screen rects for relics/ribbons/talismans from
        // the authoritative `last_*_models` lists. Keeping this as a single
        // bulk step — instead of per-site pushes paired with each model
        // push — means mouse pick (model list) and focus nav (rect list)
        // always see the same set of items; a new draw path can't add a
        // model without a matching rect.
        self.proj.relic_rects.clear();
        for (model, _rid) in &self.last_relic_models {
            self.proj.relic_rects.push(project_unit_cube_rect(*model));
        }
        // Ribbons: mesh local AABB is x/y ∈ [-0.5, 0.5], z ∈ [-0.05, 0.05].
        self.proj.ribbon_rects.clear();
        for model in &self.last_ribbon_models {
            self.proj
                .ribbon_rects
                .push(project_aabb_rect(*model, [0.5, 0.5, 0.05], 0.0));
        }
        // Talismans: local mesh AABB is `TALISMAN_LOCAL_HALF` (normalized cap half-extent 0.5, z=0.045),
        // not ±0.5. The model already bakes the world scale (see sx/sy/sz
        // derivations against `TALISMAN_LOCAL_HALF * 2`), so we must project
        // the real local bounds — unit-cube projection clips ~30% off height
        // and 5.5× overstates depth.
        self.proj.talisman_rects.clear();
        for model in &self.last_talisman_models {
            self.proj
                .talisman_rects
                .push(project_aabb_rect(*model, TALISMAN_LOCAL_HALF, 0.0));
        }

        // Append pack rects to `aux_dish_rects` for focus nav (pack placements
        // also flow through Object3d paths into `pack_rects`).
        for (rect, pick_id) in &self.proj.pack_rects {
            self.proj.aux_dish_rects.push((*pick_id, *rect));
        }
    }
}
