use super::*;

impl WgpuRenderer {
    /// Showcase tile placement pre-pass: grow / update the showcase tile pool
    /// so each tile in every `ShowcaseTileBatch` has a ready-to-draw
    /// `ShowcaseTileGpu` slot with the correct decal and up-to-date model
    /// matrix. Also runs the HandStrip arrange-mode pre-pass.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_showcase_tiles_placement(
        &mut self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        tile_basis: Mat4,
        tile_preset: crate::persistence::TilePreset,
        _dt: f32,
        light_view_proj_arr: [f32; 16],
        showcase_tile_batches: &[&[crate::render::draw_cmd::ShowcaseTilePlacement]],
        tile_3d_rects: &mut Vec<(usize, [f32; 4])>,
        tile_pick_models: &mut Vec<(usize, Mat4)>,
        tile_glows: &mut Vec<GpuInstance>,
        shadow_uniforms_changed: &mut bool,
    ) {
        let hdr_tonemap = self.tile_hdr_tonemap(frame);
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
                        primitives: &self.tile_primitives,
                        decal_atlas,
                        distortion_placeholder: &self.tile_env_distortion_placeholder,
                    };
                    let stg = make_showcase_tile_gpu(&ctx, self.tile_base_color_factor, tile);
                    self.showcase_tiles.push(stg);
                } else {
                    break;
                }
            }

            // ── HandStrip arrange-mode pre-pass ────────────────────────────
            // When a "HandStrip" arrange override is active, compute the
            // strip's world-space pivot (centroid of all hand tiles — those
            // with a pick_id) and build a delta-rotation matrix so each
            // tile's center is rotated around that pivot before the
            // translation offset is added.
            let hand_strip_arrange: Option<(glam::Vec3, Mat4, glam::Vec3)> = {
                if let Some(ref ov) = self.debug_arrange_override {
                    if ov.name == "HandStrip" {
                        // Collect world centers of hand tiles (pick_id = Some).
                        let hand_centers: Vec<glam::Vec3> = showcase_tile_batches
                            .iter()
                            .flat_map(|b| b.iter())
                            .filter(|p| p.pick_id.is_some())
                            .map(|p| {
                                pixel_to_world(
                                    w,
                                    h,
                                    p.center_pos[0],
                                    p.center_pos[1],
                                    p.center_pos[2],
                                )
                            })
                            .collect();
                        if !hand_centers.is_empty() {
                            let count = hand_centers.len() as f32;
                            let pivot =
                                hand_centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                            // Delta rotation applied around the pivot in world space.
                            let r_delta = Mat4::from_rotation_z(ov.delta_rz_deg.to_radians())
                                * Mat4::from_rotation_y(ov.delta_ry_deg.to_radians())
                                * Mat4::from_rotation_x(ov.delta_rx_deg.to_radians());
                            // Translation offset: pixel_x → +world_x, pixel_y → -world_y.
                            let translation =
                                glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
                            Some((pivot, r_delta, translation))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            // Track hand-tile world centers for the HandStrip debug pickable
            // (registered after the loop).
            let mut hand_strip_centers: Vec<glam::Vec3> = Vec::new();

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
                            primitives: &self.tile_primitives,
                            decal_atlas,
                            distortion_placeholder: &self.tile_env_distortion_placeholder,
                        };
                        self.showcase_tiles[slot_cursor] =
                            make_showcase_tile_gpu(&ctx, self.tile_base_color_factor, &p.tile);
                    }

                    // Build model matrix from the placement's explicit 3D transform.
                    // Shop uses a perspective camera; layout `(px, py, lift)` must match the same
                    // ray → `plane_z` hit as `Object3d` anchors (`world_on_camera_ray_plane_z`),
                    // not flat `pixel_to_world`, or celebration tiles miss the frustum.
                    let mut center = match (
                        self.active_scene_key,
                        frame.camera_override.as_ref(),
                    ) {
                        (Some("shop") | Some("tile_pack_celebration"), Some(cam)) => {
                            crate::render::world_space::world_on_camera_ray_plane_z(
                                w,
                                h,
                                cam,
                                p.center_pos[0],
                                p.center_pos[1],
                                p.center_pos[2],
                            )
                        }
                        (Some("showcase"), Some(cam))
                            if frame
                                .showcase_render_hints
                                .showcase_tiles_use_camera_ray_plane_z =>
                        {
                            crate::render::world_space::world_on_camera_ray_plane_z(
                                w,
                                h,
                                cam,
                                p.center_pos[0],
                                p.center_pos[1],
                                p.center_pos[2],
                            )
                        }
                        _ => {
                            pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2])
                        }
                    };
                    let tile_short_px = p.size_px * 0.85;
                    let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                    let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ) * p.scale;

                    let mut base_rotation =
                        rot_euler_xyz_rad(p.rotation[0], p.rotation[1], p.rotation[2]);

                    // Apply HandStrip arrange override: rotate each hand tile's
                    // center around the strip pivot, then add the translation.
                    if let (true, Some((pivot, r_delta, translation))) =
                        (p.pick_id.is_some(), &hand_strip_arrange)
                    {
                        let offset = center - *pivot;
                        let rotated_offset = r_delta.transform_vector3(offset);
                        center = *pivot + rotated_offset + *translation;
                        // Also rotate the tile's own orientation so the face
                        // tracks the strip rotation (e.g. ry spins tiles in
                        // place as well as revolving their centers).
                        base_rotation = *r_delta * base_rotation;
                        hand_strip_centers.push(center);
                    }

                    let oriented = base_rotation * tile_basis;
                    let model = translate_rot_scale(center, oriented, scale);

                    if let Some(pick_id) = p.pick_id {
                        let uid = p.tile.id;
                        self.prev_tile_world.insert(uid, center);

                        // Project the tile's 8 corners for the screen AABB,
                        // used for pick tracking and glow rect sizing.
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
                        }),
                    );
                    if p.outline {
                        const OUTLINE_GROW: f32 = 1.07;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(center, oriented, outline_scale);
                        self.tile_outline_instances_staging.push(
                            super::super::TileOutlineInstance {
                                model: outline_model.to_cols_array(),
                                base_color_factor: sc_bcf,
                            },
                        );
                    }
                    let su = ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    };
                    {
                        let stg_mut = &mut self.showcase_tiles[slot_cursor];
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

            if !self.tile_outline_instances_staging.is_empty() {
                self.queue.write_buffer(
                    &self.tile_outline_instance_buffer,
                    0,
                    bytemuck::cast_slice(&self.tile_outline_instances_staging),
                );
            }

            // Register the hand strip as a single debug-pickable so arrange
            // mode can select it by clicking any tile. The pickable is an AABB
            // that encloses all hand-tile centers (or their arrange-moved
            // positions when an override is already active).
            if !hand_strip_centers.is_empty() || {
                // Fallback: compute from batch placements when the override is
                // not yet active (first click selection).

                showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .any(|p| p.pick_id.is_some())
            } {
                // Use the centers we collected (post-override) if available,
                // otherwise derive directly from placements.
                let centers: Vec<glam::Vec3> = if !hand_strip_centers.is_empty() {
                    hand_strip_centers.clone()
                } else {
                    showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .filter(|p| p.pick_id.is_some())
                        .map(|p| {
                            pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2])
                        })
                        .collect()
                };
                if !centers.is_empty() {
                    let count = centers.len() as f32;
                    let centroid = centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                    // Build half-extents that encompass all tile centers plus
                    // one tile-width of padding so clicking the end tiles works.
                    let tile_half = showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .find(|p| p.pick_id.is_some())
                        .map(|p| p.size_px * 0.5)
                        .unwrap_or(40.0);
                    let mut hx = tile_half;
                    let mut hy = tile_half;
                    let mut hz = tile_half;
                    for c in &centers {
                        let d = (*c - centroid).abs();
                        hx = hx.max(d.x + tile_half);
                        hy = hy.max(d.y + tile_half);
                        hz = hz.max(d.z + tile_half);
                    }
                    let strip_model =
                        translate_rot_scale(centroid, Mat4::IDENTITY, glam::Vec3::new(hx, hy, hz));
                    self.last_debug_pickables.push((
                        "gameplay.hand.strip".to_string(),
                        strip_model,
                        glam::Vec3::splat(0.5),
                        0.0,
                    ));
                }
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
        // Talismans: local mesh AABB is `TALISMAN_LOCAL_HALF` (y=0.7, z=0.09),
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

        // Tile occluder buffer — analytic AABBs for the per-fragment ray
        // occlusion test that gives the candle pools their tile shadows.
        // Each tile contributes a single conservative world-space AABB
        // built from the 8 transformed local corners of its mesh extent.
        // Limited to MAX_TILE_OCCLUDERS so the uniform stays bounded.
        //
        // After collecting per-tile boxes we inflate adjacent tiles toward
        // each other so their AABBs touch along the row axis. Without this,
        // the back candles sit high above the table and their light threads
        // through the visible gaps between hand tiles, painting sharp
        // specular streaks on the table in front of the row (the row is
        // visually contiguous but physically gappy). The inflation per side
        // is half the gap to the nearest neighbour, so distant tiles never
        // smear into each other.
        {
            let hx = LOCAL_X_EXTENT * 0.5;
            let hy = LOCAL_Y_EXTENT * 0.5;
            let hz = LOCAL_Z_EXTENT * 0.5;
            let local_corners = [
                glam::Vec3::new(-hx, -hy, -hz),
                glam::Vec3::new(hx, -hy, -hz),
                glam::Vec3::new(-hx, hy, -hz),
                glam::Vec3::new(hx, hy, -hz),
                glam::Vec3::new(-hx, -hy, hz),
                glam::Vec3::new(hx, -hy, hz),
                glam::Vec3::new(-hx, hy, hz),
                glam::Vec3::new(hx, hy, hz),
            ];
            let mut tiles: Vec<(glam::Vec3, glam::Vec3)> =
                Vec::with_capacity(tile_pick_models.len().min(MAX_TILE_OCCLUDERS));
            for (_, model) in tile_pick_models.iter() {
                if tiles.len() >= MAX_TILE_OCCLUDERS {
                    break;
                }
                let mut lo = glam::Vec3::splat(f32::INFINITY);
                let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                for c in local_corners.iter() {
                    let w = model.transform_point3(*c);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
                tiles.push(((lo + hi) * 0.5, (hi - lo) * 0.5));
            }

            // Pick the dominant horizontal axis (X or Y on the felt; Z is up) by
            // comparing the spread of tile centers. The hand is laid out
            // along screen X — that's world X after `pixel_to_world` — but
            // detecting it from the data keeps this robust if the layout
            // ever rotates.
            if tiles.len() >= 2 {
                let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
                let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);
                for (c, _) in &tiles {
                    min_x = min_x.min(c.x);
                    max_x = max_x.max(c.x);
                    min_z = min_z.min(c.z);
                    max_z = max_z.max(c.z);
                }
                let row_axis_x = (max_x - min_x) >= (max_z - min_z);

                let mut order: Vec<usize> = (0..tiles.len()).collect();
                order.sort_by(|&a, &b| {
                    let ka = if row_axis_x {
                        tiles[a].0.x
                    } else {
                        tiles[a].0.z
                    };
                    let kb = if row_axis_x {
                        tiles[b].0.x
                    } else {
                        tiles[b].0.z
                    };
                    ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                });
                for win in order.windows(2) {
                    let (a, b) = (win[0], win[1]);
                    let (ca, cb) = (tiles[a].0, tiles[b].0);
                    let (ha, hb) = (tiles[a].1, tiles[b].1);
                    let gap = if row_axis_x {
                        (cb.x - ca.x) - (ha.x + hb.x)
                    } else {
                        (cb.z - ca.z) - (ha.z + hb.z)
                    };
                    if gap > 0.0 {
                        let pad = gap * 0.5;
                        if row_axis_x {
                            tiles[a].1.x += pad;
                            tiles[b].1.x += pad;
                        } else {
                            tiles[a].1.z += pad;
                            tiles[b].1.z += pad;
                        }
                    }
                }
            }

            let mut occ = TileOccludersBuf::empty();
            for (i, (center, half)) in tiles.iter().enumerate() {
                occ.boxes[i] = TileOccluderGpu {
                    center: [center.x, center.y, center.z, 0.0],
                    half_extents: [half.x, half.y, half.z, 0.0],
                };
            }
            occ.count[0] = tiles.len() as u32;
            self.queue
                .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
        }
    }
}
