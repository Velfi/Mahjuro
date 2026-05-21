use glam::Mat4;

use crate::render::{
    draw_cmd::{TallyFanKind, WallStackPlacement},
    lit_mesh::{LitMeshGpu, MaterialKind, MaterialParams},
    mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF},
    river_mesh::{RIVER_LOCAL_CENTER_Y, RIVER_LOCAL_HALF},
    table_transform::{
        mesh_y_thickness_along_local_y_to_z_up, rot_fixed_axes_deg_matrix,
        score_popup_glyph_rot_rad, translate_rot_scale,
    },
    talisman_mesh::{TALISMAN_LOCAL_HALF, talisman_material},
    wgpu_renderer::{
        GpuInstance, MAX_BOOK_SLOTS, MAX_BOSS_ICON_SLOTS, MAX_BOWL_SLOTS, MAX_CASCADE_TOKEN_SLOTS,
        MAX_EXTRUDED_GLYPH_SLOTS, MAX_MIRROR_SLOTS, MAX_ORB_SLOTS, MAX_PLINTH_SLOTS,
        MAX_RELIC_SLOTS, MAX_TALISMAN_SLOTS, MAX_TALLY_FAN_SLOTS, MAX_TALLY_STICK_SLOTS,
        MAX_WALL_TILE_SLOTS, MAX_WOOD_TABLET_SLOTS, MAX_YAKU_TABLET_SLOTS, WgpuRenderer,
        boss_icon_material_params, relic_material_params,
        runtime::{CameraFrame, DrawKind, RenderOp},
        tablet_label_hash,
    },
    world_space::pixel_to_world,
};

impl WgpuRenderer {
    #[inline]
    pub(super) fn push_object3d_draw(
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        kind: DrawKind,
        slot: usize,
    ) {
        object3d_draw_list.push((kind, slot));
    }

    /// Walk all `Object3d` batches and the wall-stack placements and write
    /// uniforms into the appropriate per-kind instance pools, filling in
    /// `object3d_draw_list` and patching the start/end ranges of the
    /// corresponding `RenderOp::Object3dBatch` entries in `ops`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_object3d_placement(
        &mut self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        object3d_cmds: &[&[crate::render::draw_cmd::Object3d]],
        wall_stack_cmds: &[&WallStackPlacement],
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        ops: &mut [RenderOp],
        relic_glows: &mut Vec<GpuInstance>,
        relic_debuff_markers: &mut Vec<GpuInstance>,
        mut shadow: Option<&mut super::shadow_setup::Object3dShadowCtx<'_>>,
    ) {
        self.reset_shop_inspect_shadow_slot();
        let cam_pos = camera.cam_pos;
        let look_target = camera.look_target;
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
        // ── Object3d general-purpose placement pre-pass ──────────────────
        // Walk all Object3d batches and write uniforms into the appropriate
        // per-kind instance pools. Each kind uses its own slot cursor so
        // Object3d instances don't collide with legacy placement instances.
        // Also fills in the start/end range in the corresponding RenderOp.
        {
            let _camera_pitch_deg = {
                let look = look_target - cam_pos;
                look.z.atan2(look.y.abs()).to_degrees() + 180.0
            };

            let mut obj3d_primitive_slot: rustc_hash::FxHashMap<
                crate::render::primitive::MeshId,
                usize,
            > = rustc_hash::FxHashMap::default();
            let mut obj3d_yaku_slot: usize = 0;
            let mut obj3d_wood_slot: usize = 0;
            let mut obj3d_book_slot: usize = 0;
            let mut obj3d_relic_slot: usize = 0;
            let mut obj3d_boss_icon_slot: usize = 0;
            let mut obj3d_pack_slot: usize = 0;
            let mut obj3d_talisman_slot: usize = 0;
            let mut obj3d_ribbon_slot: usize = 0;
            let mut obj3d_plinth_slot: usize = 0;
            let mut obj3d_orb_slot: usize = 0;
            let mut obj3d_bowl_slot: usize = 0;
            let mut obj3d_mirror_slot: usize = 0;
            let mut obj3d_tally_fan_idx: usize = 0;
            let mut obj3d_tally_stick_cursor: usize = 0;
            let mut obj3d_candle_slot: usize = 0;
            let mut obj3d_cascade_token_slot: usize = 0;
            let mut obj3d_glyph_slot: usize = 0;

            // Find the RenderOp::Object3dBatch ops to patch their start/end.
            let mut op_batch_idx: usize = 0;
            let mut obj3d_cmd_idx: usize = 0;

            for batch in object3d_cmds.iter() {
                let batch_start = object3d_draw_list.len();

                for obj in batch.iter() {
                    self.shadow_placement_anim_id = obj.anim_id;
                    use crate::render::draw_cmd::Object3dKind;
                    let use_ray_plane =
                        match (self.active_scene_key, frame.camera_override.as_ref()) {
                            (Some("tile_pack_celebration"), Some(_)) => true,
                            (Some("showcase"), Some(_)) => {
                                frame.showcase_render_hints.object3d_use_camera_ray_plane_z
                            }
                            _ => false,
                        };
                    let center = if use_ray_plane && let Some(cam) = frame.camera_override.as_ref()
                    {
                        crate::render::world_space::world_on_camera_ray_plane_z(
                            w, h, cam, obj.pos[0], obj.pos[1], obj.pos[2],
                        )
                    } else {
                        pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2])
                    };
                    let model = translate_rot_scale(
                        center,
                        obj.rotation_matrix(),
                        glam::Vec3::from(obj.extents),
                    );
                    // Frustum cull — skip work for kinds that aren't
                    // shadow casters or part of slot-shared shadow walks.
                    // `Candle` / `Ribbon` / `Talisman` / `Primitive`
                    // shadows in `passes/shadow.rs` re-walk `frame.cmds`
                    // with their own per-shape cursors and would draw
                    // stale uniforms if we culled here without also
                    // updating that walk; the easier safe move is to
                    // leave them alone for now (Steam Deck baseline
                    // disables shadows anyway). Local extent of 1.5x
                    // the unit cube gives generous slack against
                    // arrange-mode nudges and per-kind y-offsets so we
                    // never pop a barely-on-screen object.
                    let cull_eligible = !matches!(
                        obj.kind,
                        Object3dKind::Candle { .. }
                            | Object3dKind::ZodiacRibbon { .. }
                            | Object3dKind::Pack { .. }
                            | Object3dKind::Talisman { .. }
                            | Object3dKind::Primitive { .. }
                            | Object3dKind::WoodTablet { .. }
                    );
                    if cull_eligible && camera.aabb_outside_frustum(model, [1.5, 1.5, 1.5]) {
                        continue;
                    }
                    match &obj.kind {
                        Object3dKind::Primitive {
                            shape,
                            material,
                            pick_id,
                            shadow_caster: _,
                            silhouette,
                        } => {
                            self.place_object3d_primitive(
                                frame,
                                camera,
                                obj,
                                shape,
                                material,
                                pick_id,
                                silhouette,
                                &mut obj3d_primitive_slot,
                                object3d_draw_list,
                                &mut shadow,
                            );
                        }
                        Object3dKind::YakuTablet {
                            label,
                            active,
                            hover,
                        } => {
                            let slot_i = obj3d_yaku_slot;
                            obj3d_yaku_slot += 1;
                            if slot_i >= MAX_YAKU_TABLET_SLOTS {
                                continue;
                            }
                            let base = if *active {
                                [1.00_f32, 0.92, 0.72, 1.0]
                            } else {
                                [0.93_f32, 0.89, 0.78, 1.0]
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.30 + 0.20 * hover.clamp(0.0, 1.0),
                                specular_power: 32.0,
                            };
                            // All slots share one placement (gameplay.hand.yaku_tablet).
                            let _ = slot_i;
                            let label_hash = tablet_label_hash(label, 256, 96);
                            let inst = &mut self.yaku_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
                                    self.emoji_font.as_ref(),
                                );
                                inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.lit_mesh_relief_default_view,
                                    },
                                    &rgba,
                                    256,
                                    96,
                                );
                                inst.decal_label_hash = label_hash;
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                true,
                            );
                            self.register_placement_shadow_slot(DrawKind::YakuTablet, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.yaku_tablet_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::YakuTablet,
                                slot_i,
                            );
                        }
                        Object3dKind::WoodTablet { label, pick_id } => {
                            let slot_i = obj3d_wood_slot;
                            obj3d_wood_slot += 1;
                            if slot_i >= MAX_WOOD_TABLET_SLOTS {
                                continue;
                            }
                            let label_hash = tablet_label_hash(label, 512, 192);
                            let has_decal = !label.is_empty();
                            let inst = &mut self.wood_tablet_instances[slot_i];
                            if has_decal
                                && (inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash)
                            {
                                let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
                                );
                                inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.lit_mesh_relief_default_view,
                                    },
                                    &rgba,
                                    512,
                                    192,
                                );
                                inst.decal_label_hash = label_hash;
                            } else if !has_decal && inst.decal_label_hash != label_hash {
                                inst.decal_label_hash = label_hash;
                            }
                            if has_decal {
                                inst.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    self.wood_tablet_mesh.default_material,
                                    true,
                                );
                            } else {
                                inst.write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    self.wood_tablet_mesh.default_material,
                                );
                            }
                            self.write_lit_mesh_shadow(
                                &mut shadow,
                                &self.wood_tablet_instances[slot_i],
                                model,
                                self.wood_tablet_mesh.default_material.kind,
                            );
                            self.proj
                                .wood_tablet_rects
                                .push(project_unit_cube_rect(model));
                            self.last_wood_tablet_models.push(model);
                            // When a scene routes this tablet's click
                            // via `ShopHit::Dish(pid)` (shop journal
                            // button), publish the rect + model into
                            // the primitive-pick channels.
                            if let Some(pid) = pick_id {
                                self.proj
                                    .aux_dish_rects
                                    .push((Some(*pid), project_unit_cube_rect(model)));
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::WoodTablet,
                                slot_i,
                            );
                        }
                        Object3dKind::Book {
                            spine_label,
                            pick_id,
                            open_amount,
                        } => {
                            let slot_i = obj3d_book_slot;
                            obj3d_book_slot += 1;
                            if slot_i >= MAX_BOOK_SLOTS {
                                continue;
                            }
                            // ── Body instance (back cover, page block,
                            // page-content surface, spine, ribbons). The
                            // page-content surface samples the journal
                            // page target via the leather shader's
                            // `uv.x > 3.5` sentinel, so we bind that
                            // texture at slot 3 (`relief_tex`). The
                            // body has no calligraphy decal — that
                            // moved to the cover instance.
                            let body_inst = &mut self.book_instances[slot_i];
                            // First-time-only allocation of a 1×1 white
                            // decal stub so the bind group has a valid
                            // slot-1 binding even though the body
                            // doesn't sample a decal. Re-runs whenever
                            // `journal_scene_view_generation` advances
                            // (resize destroys the previous view, and
                            // `set_decal` rebuilds the bind group with
                            // the fresh view at slot 3).
                            let current_gen = self.journal_scene_view_generation;
                            let needs_rebind = body_inst.decal_texture.is_none()
                                || body_inst.relief_view_generation != current_gen;
                            if needs_rebind {
                                body_inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.journal_scene_view,
                                    },
                                    &[0xff, 0xff, 0xff, 0x00],
                                    1,
                                    1,
                                );
                                body_inst.decal_label_hash = 0;
                                body_inst.relief_view_generation = current_gen;
                            }
                            // Override the body's base_color.a with
                            // `open_amount` so the leather shader's
                            // page-content branch can discard page
                            // fragments while the cover still
                            // occludes them. Other body faces
                            // (back cover, page block, spine) ignore
                            // alpha and render normally.
                            let mut body_material = self.book_mesh.default_material;
                            body_material.base_color[3] = *open_amount;
                            body_inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                body_material,
                                false,
                            );
                            self.register_placement_shadow_slot(DrawKind::Book, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.book_instances[slot_i],
                                    model,
                                    body_material.kind,
                                );
                            }
                            if let Some(pid) = pick_id {
                                self.proj
                                    .aux_dish_rects
                                    .push((Some(*pid), project_unit_cube_rect(model)));
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Book,
                                slot_i,
                            );

                            // ── Cover instance: rotate the front cover
                            // around the local spine axis (X = -0.5).
                            // open_amount = 0 keeps the cover flush over
                            // the page surface; open_amount = 1 swings
                            // it ~170° to expose the page.
                            let cover_inst = &mut self.book_cover_instances[slot_i];
                            let label_hash = tablet_label_hash(spine_label, 512, 192);
                            if cover_inst.decal_texture.is_none()
                                || cover_inst.decal_label_hash != label_hash
                            {
                                let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                    spine_label,
                                    self.ui_font.as_ref(),
                                );
                                cover_inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.lit_mesh_relief_default_view,
                                    },
                                    &rgba,
                                    512,
                                    192,
                                );
                                cover_inst.decal_label_hash = label_hash;
                            }
                            // Hinge: rotate around the local Z axis
                            // (the spine runs Z-axially) by
                            // +open_amount * 170° so the fore-edge of
                            // the cover lifts away from the page
                            // surface and the cover lays open on the
                            // camera-right side.
                            //
                            // The rotation is around the local axis at
                            // X = -0.5, Y = 0 (mid-cover). To rotate
                            // around a non-origin axis we translate
                            // vertices so the axis goes through origin,
                            // rotate, then translate back. As an
                            // affine transform: T(+spine) * R * T(-spine).
                            let spine_x = crate::render::book_mesh::SPINE_X;
                            let cover_y_mid = 0.5
                                * (crate::render::book_mesh::FRONT_COVER_Y_LO
                                    + crate::render::book_mesh::FRONT_COVER_Y_HI);
                            let theta = open_amount * 170.0_f32.to_radians();
                            let hinge = glam::Mat4::from_translation(glam::Vec3::new(
                                spine_x,
                                cover_y_mid,
                                0.0,
                            )) * glam::Mat4::from_rotation_z(theta)
                                * glam::Mat4::from_translation(glam::Vec3::new(
                                    -spine_x,
                                    -cover_y_mid,
                                    0.0,
                                ));
                            let cover_model = model * hinge;
                            cover_inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                cover_model,
                                self.book_cover_mesh.default_material,
                                true,
                            );
                            self.register_placement_shadow_slot(DrawKind::BookCover, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.book_cover_instances[slot_i],
                                    cover_model,
                                    self.book_cover_mesh.default_material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BookCover,
                                slot_i,
                            );
                        }
                        Object3dKind::Relic {
                            relic_id,
                            glow,
                            silhouette,
                            debuffed,
                            pick_id,
                        } => {
                            if obj3d_relic_slot >= MAX_RELIC_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_relic_slot;
                            obj3d_relic_slot += 1;
                            // obj.rotation already encodes pitch/roll; extents are full.
                            let g = if *silhouette {
                                0.0
                            } else {
                                glow.clamp(0.0, 1.0)
                            };
                            let base_color = if *silhouette {
                                // Silhouette tint: the scene controls
                                // this via `obj.color`. Collection scene
                                // passes a muted rarity accent so locked
                                // relics still read as "earned-worth-
                                // chasing" rather than pure-black dots.
                                // Any caller that wants the old solid
                                // matte can pass `[0.04, 0.04, 0.05, 1]`
                                // explicitly — which is now the accent
                                // math's identity for near-zero tint.
                                obj.color
                            } else if g > 0.0 {
                                let target = [1.55, 1.32, 0.78, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = if *silhouette {
                                crate::render::lit_mesh::MaterialParams {
                                    kind: crate::render::lit_mesh::MaterialKind::Plain,
                                    base_color,
                                    specular_strength: 0.0,
                                    specular_power: 1.0,
                                }
                            } else {
                                relic_material_params(*relic_id, base_color, g)
                            };
                            if *silhouette {
                                self.relic_instances[slot_i].write_uniform_tinted(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                    base_color,
                                );
                            } else {
                                self.relic_instances[slot_i].write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                    false,
                                );
                            }
                            self.register_placement_shadow_slot(DrawKind::Relic, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.relic_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            // Silhouette pass skips the relic albedo/relief
                            // texture — we want the shape only, not the
                            // engraved artwork.
                            let want_tex = if *silhouette {
                                None
                            } else if self.relic_textures.contains_key(relic_id) {
                                Some(*relic_id)
                            } else {
                                None
                            };
                            if self.relic_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.relic_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("relic-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.relic_slot_texture[slot_i] = want_tex;
                            }
                            self.last_relic_models.push((model, *relic_id));
                            if let Some(pid) = pick_id {
                                self.last_pickable_relic_models
                                    .push((*pid, model, *relic_id));
                            }
                            let projected_rect = project_unit_cube_rect(model);
                            self.proj.relic_rects.push(projected_rect);
                            if g > 0.0 {
                                // Activation halo: champagne bloom inflated past
                                // the projected rect so the additive falloff
                                // spills out past the relic silhouette.
                                let [rx, ry, rw, rh] = projected_rect;
                                let pad_x = rw * 0.85;
                                let pad_y = rh * 0.95;
                                relic_glows.push(GpuInstance {
                                    rect: [
                                        rx - pad_x,
                                        ry - pad_y,
                                        rw + pad_x * 2.0,
                                        rh + pad_y * 2.0,
                                    ],
                                    color: [1.00, 0.82, 0.36, 1.20 * g],
                                    user: 0,
                                });
                            }
                            if *debuffed && !*silhouette {
                                let [rx, ry, rw, rh] = projected_rect;
                                let side = (rw.min(rh) * 0.42).max(14.0).min(rw.min(rh) * 0.92);
                                let cx = rx + rw * 0.5;
                                let cy = ry + rh * 0.48;
                                relic_debuff_markers.push(GpuInstance {
                                    rect: [cx - side * 0.5, cy - side * 0.5, side, side],
                                    color: [1.0, 1.0, 1.0, 1.0],
                                    user: 0,
                                });
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Relic,
                                slot_i,
                            );
                        }
                        Object3dKind::BossIcon {
                            kind,
                            glow,
                            pick_id,
                        } => {
                            if obj3d_boss_icon_slot >= MAX_BOSS_ICON_SLOTS {
                                continue;
                            }
                            self.ensure_boss_icon_gpu(*kind);
                            let slot_i = obj3d_boss_icon_slot;
                            obj3d_boss_icon_slot += 1;
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.55, 1.32, 0.78, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = boss_icon_material_params(base_color, g);
                            self.boss_icon_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                false,
                            );
                            self.register_placement_shadow_slot(DrawKind::BossIcon, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.boss_icon_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            let want_tex =
                                self.boss_icon_textures.contains_key(kind).then_some(*kind);
                            if self.boss_icon_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(bk) => &self.boss_icon_textures[&bk].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(bk) => &self.boss_icon_textures[&bk].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.boss_icon_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("boss-icon-bg"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.boss_icon_slot_texture[slot_i] = want_tex;
                            }
                            if g > 0.0 {
                                let projected_rect = project_unit_cube_rect(model);
                                let [rx, ry, rw, rh] = projected_rect;
                                let pad_x = rw * 0.85;
                                let pad_y = rh * 0.95;
                                relic_glows.push(GpuInstance {
                                    rect: [
                                        rx - pad_x,
                                        ry - pad_y,
                                        rw + pad_x * 2.0,
                                        rh + pad_y * 2.0,
                                    ],
                                    color: [1.0, 0.88, 0.62, 0.85 * g],
                                    user: 0,
                                });
                            }
                            let _ = pick_id;
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BossIcon,
                                slot_i,
                            );
                        }
                        Object3dKind::Pack { kind, pick_id } => {
                            if obj3d_pack_slot >= self.pack_instances.len() {
                                continue;
                            }
                            let slot_i = obj3d_pack_slot;
                            obj3d_pack_slot += 1;
                            let _ = slot_i;
                            // Hover glow: hover_target ramps 0..1 as the
                            // player focuses/cursor-hovers the pack. Lift
                            // the foil tint toward a warm bloom and push
                            // an additive halo (parallel to relic
                            // activation glow) so the wrapper visibly
                            // rewards looking at it.
                            let hover_g = obj.hover_target.clamp(0.0, 1.0);
                            let base_color = if hover_g > 0.0 {
                                let target = [1.35, 1.18, 0.78, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * hover_g * 0.55,
                                    obj.color[1] + (target[1] - obj.color[1]) * hover_g * 0.55,
                                    obj.color[2] + (target[2] - obj.color[2]) * hover_g * 0.55,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Foil,
                                base_color,
                                specular_strength: 0.70,
                                specular_power: 48.0,
                            };
                            // Foil packs must keep `material_params.w == 0` so the
                            // shader composites the cover decal and streak/holo
                            // bands (`w > 0.5` is the talisman-foil path).
                            // Showcase pack celebrations already disable the
                            // directional shadow map in `shadow_setup.rs`.
                            self.pack_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                false,
                            );
                            self.register_placement_shadow_slot(DrawKind::Pack, slot_i);
                            if self.placement_shadow_writes(frame)
                                && crate::render::lit_mesh::lit_mesh_casts_directional_shadow(
                                    material.kind,
                                    0.0,
                                )
                            {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.pack_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            let want_tex = if self.pack_textures.contains_key(kind) {
                                Some(*kind)
                            } else {
                                None
                            };
                            if self.pack_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.pack_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("pack-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.pack_slot_texture[slot_i] = want_tex;
                            }
                            // Project the 8 unit-cube corners via the model matrix to get
                            // the screen-space bounding rect. This feeds focus-nav and
                            // controller selection via aux_dish_rects (appended below).
                            let projected_rect = project_unit_cube_rect(model);
                            self.proj.pack_rects.push((projected_rect, *pick_id));
                            if hover_g > 0.0 {
                                // Hover halo: an additive bloom inflated
                                // past the projected rect so the falloff
                                // spills out around the wrapper —
                                // matches the activation halo on
                                // Object3dKind::Relic above. Slightly
                                // tighter pad than relics because packs
                                // are smaller and a wider spill reads as
                                // an unfocused glow. Halo color blends
                                // the per-kind seal color into a warm
                                // gold so the rim picks up the wax tone
                                // without losing the candlelit feel.
                                let [rx, ry, rw, rh] = projected_rect;
                                let pad_x = rw * 0.55;
                                let pad_y = rh * 0.65;
                                let seal = kind.seal_color();
                                let mix_t = 0.35;
                                let halo_r = 1.00 + (seal[0] - 1.00) * mix_t;
                                let halo_g = 0.86 + (seal[1] - 0.86) * mix_t;
                                let halo_b = 0.46 + (seal[2] - 0.46) * mix_t;
                                relic_glows.push(GpuInstance {
                                    rect: [
                                        rx - pad_x,
                                        ry - pad_y,
                                        rw + pad_x * 2.0,
                                        rh + pad_y * 2.0,
                                    ],
                                    color: [halo_r, halo_g, halo_b, 0.85 * hover_g],
                                    user: 0,
                                });
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Pack,
                                slot_i,
                            );
                        }
                        Object3dKind::Talisman { kind } => {
                            if obj3d_talisman_slot >= MAX_TALISMAN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_talisman_slot;
                            obj3d_talisman_slot += 1;
                            // extents already encode full size; scale to mesh local half-extents.
                            let sx = obj.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                            let sy = obj.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                            let sz = obj.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                            let _ = slot_i;
                            let talisman_model = translate_rot_scale(
                                center,
                                obj.rotation_matrix(),
                                glam::Vec3::new(sx, sy, sz),
                            );
                            let material = talisman_material(*kind, obj.color);
                            let kind_idx = crate::core::talisman::TalismanKind::all()
                                .iter()
                                .position(|&k| k == *kind)
                                .unwrap_or(0) as u8;
                            if self.talisman_slot_kind[slot_i] != Some(kind_idx) {
                                self.talisman_instances[slot_i].rebind_texture(
                                    &self.device,
                                    &self.lit_mesh_material_layout,
                                    &self.talisman_height_views[kind_idx as usize],
                                    &self.lit_mesh_relief_default_view,
                                    &self.tile_sampler,
                                );
                                self.talisman_slot_kind[slot_i] = Some(kind_idx);
                            }
                            self.talisman_instances[slot_i].write_uniform_raw_w(
                                &self.queue,
                                view_proj_arr,
                                talisman_model,
                                material,
                                kind_idx as f32,
                            );
                            self.register_placement_shadow_slot(DrawKind::Talisman, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.talisman_instances[slot_i],
                                    talisman_model,
                                    material.kind,
                                );
                            }
                            self.last_talisman_models.push(talisman_model);
                            self.proj
                                .talisman_rects
                                .push(project_unit_cube_rect(talisman_model));
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Talisman,
                                slot_i,
                            );
                        }
                        Object3dKind::ZodiacRibbon { kind } => {
                            self.place_object3d_ribbon(
                                frame,
                                camera,
                                obj,
                                center,
                                kind,
                                &mut obj3d_ribbon_slot,
                                object3d_draw_list,
                                &mut shadow,
                            );
                        }
                        Object3dKind::Plinth { glow, role } => {
                            if obj3d_plinth_slot >= MAX_PLINTH_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_plinth_slot;
                            obj3d_plinth_slot += 1;
                            // Mesh is built Y-up centered; lift the world position
                            // by half-height so `obj.pos` describes the plinth's
                            // base sitting on the table felt.
                            let plinth_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            let plinth_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation_matrix();
                            let plinth_model = translate_rot_scale(
                                plinth_center,
                                plinth_rot,
                                glam::Vec3::from(obj.extents),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 0.95, 0.55, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Metal,
                                base_color,
                                specular_strength: 0.85,
                                specular_power: 64.0,
                            };
                            self.plinth_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                plinth_model,
                                material,
                                false,
                            );
                            self.register_placement_shadow_slot(DrawKind::Plinth, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.plinth_instances[slot_i],
                                    plinth_model,
                                    material.kind,
                                );
                            }
                            // Project AABB → screen rect for hover/focus.
                            let plinth_world_center = plinth_model.w_axis.truncate();
                            let [hx, hy, hz] = [
                                obj.extents[0] * 0.5,
                                obj.extents[1] * 0.5,
                                obj.extents[2] * 0.5,
                            ];
                            let (mut mn_x, mut mn_y, mut mx_x, mut mx_y) = (
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            );
                            for cx in [-hx, hx] {
                                for cy in [-hy, hy] {
                                    for cz in [-hz, hz] {
                                        let world =
                                            plinth_world_center + glam::Vec3::new(cx, cy, cz);
                                        let (px, py) = project_to_screen(world);
                                        mn_x = mn_x.min(px);
                                        mn_y = mn_y.min(py);
                                        mx_x = mx_x.max(px);
                                        mx_y = mx_y.max(py);
                                    }
                                }
                            }
                            let rect = [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y];
                            // Local +Y is plinth height; +0.36 is the top platform
                            // where tiles/icons rest (vs. the decorative crown).
                            let platform_world =
                                plinth_model.transform_point3(glam::Vec3::new(0.0, 0.36, 0.0));
                            let (platform_px, platform_py) = project_to_screen(platform_world);
                            use crate::render::draw_cmd::PlinthRole;
                            match role {
                                PlinthRole::RoundWind => {
                                    self.proj.round_wind_plinth_rect = Some(rect);
                                }
                                PlinthRole::Boss => {
                                    self.proj.boss_plinth_rect = Some(rect);
                                    self.proj.boss_plinth_platform_px =
                                        Some([platform_px, platform_py]);
                                }
                                PlinthRole::Dora => {
                                    self.proj.plinth_rect = Some(rect);
                                }
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Plinth,
                                slot_i,
                            );
                        }
                        Object3dKind::Bug {
                            slot,
                            flap_rad,
                            live_wing_alpha,
                            blur_alpha,
                        } => {
                            let slot = *slot;
                            if slot >= crate::render::wgpu_renderer::MAX_BUG_SLOTS {
                                continue;
                            }
                            let bug_model = translate_rot_scale(
                                center,
                                obj.rotation_matrix(),
                                glam::Vec3::from(obj.extents),
                            );
                            self.bug_body_instances[slot].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                self.bug_body_mesh.default_material,
                                false,
                            );
                            self.register_placement_shadow_slot(DrawKind::BugBody, slot);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.bug_body_instances[slot],
                                    bug_model,
                                    self.bug_body_mesh.default_material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BugBody,
                                slot,
                            );
                            let flap_l = glam::Mat4::from_rotation_x(*flap_rad);
                            let flap_r = glam::Mat4::from_rotation_x(-*flap_rad)
                                * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0));
                            let live_a = live_wing_alpha.clamp(0.0, 1.0);
                            let wing_mat = self.bug_wing_mesh.default_material;
                            let live_tint = [
                                wing_mat.base_color[0],
                                wing_mat.base_color[1],
                                wing_mat.base_color[2],
                                wing_mat.base_color[3] * live_a,
                            ];
                            self.bug_wing_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_l,
                                wing_mat,
                                live_tint,
                            );
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BugWingL,
                                slot,
                            );
                            self.bug_wing_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_r,
                                wing_mat,
                                live_tint,
                            );
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BugWingR,
                                slot,
                            );
                            let blur_a = blur_alpha.clamp(0.0, 1.0);
                            let blur_mat = self.bug_wing_blur_mesh.default_material;
                            let blur_tint = [
                                blur_mat.base_color[0],
                                blur_mat.base_color[1],
                                blur_mat.base_color[2],
                                blur_mat.base_color[3] * blur_a,
                            ];
                            self.bug_wing_blur_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                blur_mat,
                                blur_tint,
                            );
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BugWingBlurL,
                                slot,
                            );
                            self.bug_wing_blur_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0)),
                                blur_mat,
                                blur_tint,
                            );
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::BugWingBlurR,
                                slot,
                            );
                        }
                        Object3dKind::MaterialOrb { material } => {
                            if obj3d_orb_slot >= MAX_ORB_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_orb_slot;
                            obj3d_orb_slot += 1;
                            self.orb_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                *material,
                            );
                            self.register_placement_shadow_slot(DrawKind::Orb, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.orb_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Orb,
                                slot_i,
                            );
                        }
                        Object3dKind::Mirror {
                            rotation_x_deg,
                            rotation_z_deg,
                        } => {
                            if obj3d_mirror_slot >= MAX_MIRROR_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_mirror_slot;
                            obj3d_mirror_slot += 1;
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 2 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            let tilt_deg = *rotation_x_deg + anim * 22.0;
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                rot_fixed_axes_deg_matrix(
                                    Mat4::from_rotation_z((*rotation_z_deg).to_radians())
                                        * Mat4::from_rotation_x(tilt_deg.to_radians()),
                                ),
                                glam::Vec3::from(obj.extents),
                            );
                            self.mirror_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.mirror_mesh.default_material,
                            );
                            self.write_lit_mesh_shadow(
                                &mut shadow,
                                &self.mirror_instances[slot_i],
                                hover_model,
                                self.mirror_mesh.default_material.kind,
                            );
                            if slot_i == 0 {
                                self.proj.mirror_rect = Some(project_aabb_rect(
                                    hover_model,
                                    MIRROR_LOCAL_HALF,
                                    MIRROR_LOCAL_CENTER_Y,
                                ));
                                self.last_mirror_model = Some(hover_model);
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Mirror,
                                slot_i,
                            );
                        }
                        Object3dKind::ExtrudedGlyph {
                            scale: g_scale,
                            rotation_x: g_rx,
                            rotation_y: g_ry,
                            label,
                            emissive,
                            material: g_mat,
                        } => {
                            if obj3d_glyph_slot >= MAX_EXTRUDED_GLYPH_SLOTS {
                                continue;
                            }
                            if !self.extruded_glyph_meshes.contains_key(label.as_ref()) {
                                if let Some(cpu) = self.glyph_cpu_cache.mesh_for(label) {
                                    let gpu = LitMeshGpu::new(
                                        &self.device,
                                        cpu,
                                        &format!("glyph-{}", label),
                                    );
                                    // One-time alloc when a new glyph string is first seen;
                                    // subsequent frames hit the cache and skip this branch.
                                    self.extruded_glyph_meshes.insert(label.to_string(), gpu);
                                } else {
                                    continue;
                                }
                            }
                            let slot_i = obj3d_glyph_slot;
                            obj3d_glyph_slot += 1;
                            let g_center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let glyph_model = translate_rot_scale(
                                g_center,
                                score_popup_glyph_rot_rad(
                                    *g_ry,
                                    -std::f32::consts::FRAC_PI_2 + *g_rx,
                                ),
                                glam::Vec3::splat(*g_scale),
                            );
                            let material = match g_mat {
                                crate::render::draw_cmd::GlyphMaterial::Metal => MaterialParams {
                                    kind: MaterialKind::Metal,
                                    base_color: obj.color,
                                    specular_strength: 1.0,
                                    specular_power: 128.0,
                                },
                                crate::render::draw_cmd::GlyphMaterial::Polychrome => {
                                    MaterialParams {
                                        kind: MaterialKind::Polychrome,
                                        base_color: obj.color,
                                        specular_strength: 0.85,
                                        specular_power: 48.0,
                                    }
                                }
                                crate::render::draw_cmd::GlyphMaterial::Plain => MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: obj.color,
                                    specular_strength: 0.35 + 0.20 * emissive.clamp(0.0, 1.0),
                                    specular_power: 96.0,
                                },
                            };
                            self.extruded_glyph_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                glyph_model,
                                material,
                            );
                            self.register_placement_shadow_slot(DrawKind::ExtrudedGlyph, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.extruded_glyph_instances[slot_i],
                                    glyph_model,
                                    material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::ExtrudedGlyph,
                                slot_i,
                            );
                        }
                        Object3dKind::CascadeToken { kind: ck, pulse } => {
                            if obj3d_cascade_token_slot >= MAX_CASCADE_TOKEN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_cascade_token_slot;
                            obj3d_cascade_token_slot += 1;
                            let pulse_f = pulse.clamp(0.0, 1.0);
                            let pulse_scale = 1.0 + 0.18 * pulse_f;
                            let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let model = translate_rot_scale(
                                center,
                                Mat4::IDENTITY,
                                glam::Vec3::new(
                                    obj.extents[0] * pulse_scale,
                                    obj.extents[1] * pulse_scale,
                                    obj.extents[2] * pulse_scale,
                                ),
                            );
                            let base = ck.color();
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.40 + 0.30 * pulse_f,
                                specular_power: 48.0,
                            };
                            self.cascade_token_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            self.register_placement_shadow_slot(DrawKind::CascadeToken, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.cascade_token_instances[slot_i],
                                    model,
                                    material.kind,
                                );
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::CascadeToken,
                                slot_i,
                            );
                        }
                        Object3dKind::Candle {
                            scale,
                            height_scale,
                        } => {
                            let slot_i = obj3d_candle_slot;
                            obj3d_candle_slot += 1;
                            if self.candle_instances.get(slot_i).is_none() {
                                continue;
                            }
                            let base = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let s = *scale;
                            let candle_model = translate_rot_scale(
                                base,
                                mesh_y_thickness_along_local_y_to_z_up(),
                                glam::Vec3::new(s, s * *height_scale, s),
                            );
                            self.candle_instances[slot_i][0].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wax_mesh.default_material,
                            );
                            self.register_placement_shadow_slot(DrawKind::CandleWax, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.write_lit_mesh_shadow(
                                    &mut shadow,
                                    &self.candle_instances[slot_i][0],
                                    candle_model,
                                    self.candle_wax_mesh.default_material.kind,
                                );
                            }
                            self.candle_instances[slot_i][1].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wick_mesh.default_material,
                            );
                            self.register_placement_shadow_slot(DrawKind::CandleWax, slot_i);
                            if self.placement_shadow_writes(frame) {
                                self.register_placement_shadow_slot(DrawKind::CandleWick, slot_i);
                                if self.placement_shadow_writes(frame) {
                                    self.write_lit_mesh_shadow(
                                        &mut shadow,
                                        &self.candle_instances[slot_i][1],
                                        candle_model,
                                        self.candle_wick_mesh.default_material.kind,
                                    );
                                }
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::CandleWax,
                                slot_i,
                            );
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::CandleWick,
                                slot_i,
                            );
                        }
                        Object3dKind::TallyFan {
                            stick_len,
                            stick_wide,
                            stick_thickness,
                            count,
                            max_count,
                            spread_deg,
                            tip_color,
                            rotation_y_deg,
                            placement_rot_deg,
                            kind: fan_kind,
                        } => {
                            let fan_i = obj3d_tally_fan_idx;
                            obj3d_tally_fan_idx += 1;
                            if fan_i >= MAX_TALLY_FAN_SLOTS {
                                continue;
                            }
                            let max_c: u32 = (*max_count).max(1);
                            let count_usize = (*count).min(max_c) as usize;
                            let spread_rad = spread_deg.to_radians();
                            let slot_angle = |k: u32| -> f32 {
                                if max_c <= 1 {
                                    0.0
                                } else {
                                    -spread_rad * 0.5
                                        + (k as f32) * (spread_rad / (max_c as f32 - 1.0))
                                }
                            };
                            let pivot = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let base_orient = mesh_y_thickness_along_local_y_to_z_up();
                            let fan_yaw = Mat4::from_rotation_z(rotation_y_deg.to_radians());
                            let base_scale =
                                glam::Vec3::new(*stick_wide, *stick_len, *stick_thickness);
                            let base_material = self.tally_stick_base_mesh.default_material;
                            let tip_material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: *tip_color,
                                specular_strength: 0.40,
                                specular_power: 42.0,
                            };
                            let missing = (max_c as usize).saturating_sub(count_usize);
                            let mut visible_slots: Vec<u32> = (0..max_c).collect();
                            for trim in 0..missing {
                                if trim % 2 == 0 {
                                    visible_slots.pop();
                                } else {
                                    visible_slots.remove(0);
                                }
                            }
                            for (_stick_i, &k) in visible_slots.iter().enumerate() {
                                if obj3d_tally_stick_cursor + 1 >= MAX_TALLY_STICK_SLOTS * 2 {
                                    break;
                                }
                                let angle = slot_angle(k);
                                let rot = fan_yaw * Mat4::from_rotation_y(angle) * base_orient;
                                let model =
                                    crate::render::table_transform::apply_rotation_deg_to_model(
                                        translate_rot_scale(pivot, rot, base_scale),
                                        *placement_rot_deg,
                                    );
                                self.tally_stick_instances[obj3d_tally_stick_cursor].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    base_material,
                                );
                                self.register_placement_shadow_slot(
                                    DrawKind::TallyStickBase,
                                    obj3d_tally_stick_cursor,
                                );
                                if self.placement_shadow_writes(frame) {
                                    self.write_lit_mesh_shadow(
                                        &mut shadow,
                                        &self.tally_stick_instances[obj3d_tally_stick_cursor],
                                        model,
                                        base_material.kind,
                                    );
                                }
                                self.tally_stick_instances[obj3d_tally_stick_cursor + 1]
                                    .write_uniform(&self.queue, view_proj_arr, model, tip_material);
                                self.register_placement_shadow_slot(
                                    DrawKind::TallyStickTip,
                                    obj3d_tally_stick_cursor + 1,
                                );
                                if self.placement_shadow_writes(frame) {
                                    self.write_lit_mesh_shadow(
                                        &mut shadow,
                                        &self.tally_stick_instances[obj3d_tally_stick_cursor + 1],
                                        model,
                                        tip_material.kind,
                                    );
                                }
                                WgpuRenderer::push_object3d_draw(
                                    object3d_draw_list,
                                    DrawKind::TallyStickBase,
                                    obj3d_tally_stick_cursor,
                                );
                                WgpuRenderer::push_object3d_draw(
                                    object3d_draw_list,
                                    DrawKind::TallyStickTip,
                                    obj3d_tally_stick_cursor + 1,
                                );
                                obj3d_tally_stick_cursor += 2;
                            }
                            let fan_width =
                                *stick_len * (spread_rad * 0.5).sin() * 2.0 + *stick_wide;
                            let fan_height = *stick_len + *stick_wide * 0.5;
                            let fan_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + *stick_len * 0.5,
                            );
                            let fan_model = translate_rot_scale(
                                fan_center,
                                fan_yaw,
                                glam::Vec3::new(fan_width, *stick_thickness * 2.0, fan_height),
                            );
                            let slot_idx = match fan_kind {
                                TallyFanKind::Draws => 0,
                                TallyFanKind::Discards => 1,
                            };
                            self.proj.peg_rects[slot_idx] = Some(project_unit_cube_rect(fan_model));
                        }
                        Object3dKind::Bowl => {
                            if obj3d_bowl_slot >= MAX_BOWL_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_bowl_slot;
                            obj3d_bowl_slot += 1;
                            // Ease hover in-place (can't call self.ease_hover mid-borrow).
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 1 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            // Recompute model with hover lift + tilt baked in.
                            // Scene passes base euler in `obj.rotation`; hover adds pitch on the left.
                            let tilt = anim * 18.0_f32.to_radians();
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                glam::Mat4::from_rotation_x(tilt) * obj.rotation_matrix(),
                                glam::Vec3::from(obj.extents),
                            );
                            self.bowl_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.bowl_mesh.default_material,
                            );
                            self.write_lit_mesh_shadow(
                                &mut shadow,
                                &self.bowl_instances[slot_i],
                                hover_model,
                                self.bowl_mesh.default_material.kind,
                            );
                            if slot_i == 0 {
                                self.proj.bowl_rect = Some(project_aabb_rect(
                                    hover_model,
                                    RIVER_LOCAL_HALF,
                                    RIVER_LOCAL_CENTER_Y,
                                ));
                                self.proj.bowl_model = Some(hover_model);
                                self.last_bowl_model = Some(hover_model);
                            }
                            WgpuRenderer::push_object3d_draw(
                                object3d_draw_list,
                                DrawKind::Bowl,
                                slot_i,
                            );
                        }
                    }
                }

                let batch_end = object3d_draw_list.len();
                // Patch the placeholder RenderOp that was pushed during the cmd walk.
                // Find the correct Object3dBatch op by scanning from op_batch_idx.
                while op_batch_idx < ops.len() {
                    if let RenderOp::Object3dBatch { start, end } = &mut ops[op_batch_idx]
                        && *start == 0
                        && *end == 0
                    {
                        *start = batch_start;
                        *end = batch_end;
                        op_batch_idx += 1;
                        break;
                    }
                    op_batch_idx += 1;
                }
                obj3d_cmd_idx += 1;
            }
            let _ = obj3d_cmd_idx;
        }

        // Wall stack: facedown tiles laid out in a row at the back of the
        // table, height growing slightly as the stack thickens. Phase 1 uses
        // the bone tablet mesh (a plain box) — phase 7 may swap to the real
        // tile mesh.
        let mut wall_tile_slot_cursor: usize = 0;
        for w_cmd in wall_stack_cmds.iter() {
            let row_len = w_cmd.row_len.max(1);
            let total = w_cmd.remaining.min(MAX_WALL_TILE_SLOTS as u32);
            let tile_w = w_cmd.tile_extents[0];
            let tile_h = w_cmd.tile_extents[1];
            let tile_d = w_cmd.tile_extents[2];
            for k in 0..total {
                if wall_tile_slot_cursor >= MAX_WALL_TILE_SLOTS {
                    break;
                }
                let col = k % row_len;
                let layer = k / row_len;
                let px = w_cmd.world_pos[0] + col as f32 * tile_w;
                let py = w_cmd.world_pos[1] + layer as f32 * tile_d;
                let pz = w_cmd.world_pos[2] + tile_h * 0.5;
                let center = pixel_to_world(w, h, px, py, pz);
                let model = translate_rot_scale(
                    center,
                    Mat4::IDENTITY,
                    glam::Vec3::new(tile_w, tile_h, tile_d),
                );
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: [0.86, 0.81, 0.69, 1.0],
                    specular_strength: 0.20,
                    specular_power: 24.0,
                };
                self.wall_tile_instances[wall_tile_slot_cursor].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                self.write_lit_mesh_shadow(
                    &mut shadow,
                    &self.wall_tile_instances[wall_tile_slot_cursor],
                    model,
                    material.kind,
                );
                wall_tile_slot_cursor += 1;
            }
        }
    }
}
