use super::*;

impl WgpuRenderer {
    /// Walk all `Object3d` batches and the wall-stack placements and write
    /// uniforms into the appropriate per-kind instance pools, filling in
    /// `object3d_draw_list` and patching the start/end ranges of the
    /// corresponding `RenderOp::Object3dBatch` entries in `ops`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_object3d_placement(
        &mut self,
        camera: &CameraFrame,
        object3d_cmds: &[&[crate::render::draw_cmd::Object3d]],
        wall_stack_cmds: &[&WallStackPlacement],
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        ops: &mut Vec<RenderOp>,
        relic_glows: &mut Vec<GpuInstance>,
    ) {
        let cam_pos = camera.cam_pos;
        let look_target = camera.look_target;
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen = |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
        let project_unit_cube_rect = |model: Mat4| -> [f32; 4] { camera.project_unit_cube_rect(model) };
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

            let mut obj3d_primitive_slot: HashMap<crate::render::primitive::MeshId, usize> =
                HashMap::new();
            let mut obj3d_yaku_slot: usize = 0;
            let mut obj3d_wood_slot: usize = 0;
            let mut obj3d_relic_slot: usize = 0;
            let mut obj3d_pack_slot: usize = 0;
            let mut obj3d_talisman_slot: usize = 0;
            let mut obj3d_ribbon_slot: usize = 0;
            let mut obj3d_shrine_slot: usize = 0;
            let mut obj3d_dora_plinth_slot: usize = 0;
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
                    use crate::render::draw_cmd::Object3dKind;
                    let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                    let model = translate_rot_scale(
                        center,
                        obj.rotation, // Mat4 set directly by the scene
                        glam::Vec3::from(obj.extents),
                    );

                    match &obj.kind {
                        Object3dKind::Primitive {
                            shape,
                            material,
                            pick_id,
                            shadow_caster: _,
                            silhouette,
                        } => {
                            self.place_object3d_primitive(
                                &camera,
                                obj,
                                shape,
                                material,
                                pick_id,
                                silhouette,
                                &mut obj3d_primitive_slot,
                                object3d_draw_list,
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
                            let yaku_name = "gameplay.hand.yaku_tablet";
                            let model = self.apply_arrange_override(yaku_name, model);
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
                            self.last_debug_pickables.push((
                                yaku_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::YakuTablet, slot_i));
                        }
                        Object3dKind::WoodTablet { label, pick_id } => {
                            let slot_i = obj3d_wood_slot;
                            obj3d_wood_slot += 1;
                            if slot_i >= MAX_WOOD_TABLET_SLOTS {
                                continue;
                            }
                            // Explicit `arrange_name` wins; otherwise
                            // fall back to the legacy gameplay-slot
                            // convention so saved arrange overrides for
                            // the action bar keep loading.
                            let wood_name = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else {
                                match slot_i {
                                    0 => "gameplay.action_bar.tablet_sort_suit".to_string(),
                                    1 => "gameplay.action_bar.tablet_sort_rank".to_string(),
                                    2 => "gameplay.action_bar.tablet_cash_in".to_string(),
                                    3 => "gameplay.action_bar.tablet_journal".to_string(),
                                    _ => "gameplay.action_bar.tablet".to_string(),
                                }
                            };
                            let model = self.apply_arrange_override(&wood_name, model);
                            let label_hash = tablet_label_hash(label, 512, 192);
                            let inst = &mut self.wood_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
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
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                self.wood_tablet_mesh.default_material,
                                true,
                            );
                            self.proj
                                .wood_tablet_rects
                                .push(project_unit_cube_rect(model));
                            self.last_wood_tablet_models.push(model);
                            self.last_debug_pickables.push((
                                wood_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
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
                            object3d_draw_list.push((DrawKind::WoodTablet, slot_i));
                        }
                        Object3dKind::Relic {
                            relic_id,
                            glow,
                            silhouette,
                            pick_id,
                        } => {
                            if obj3d_relic_slot >= MAX_RELIC_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_relic_slot;
                            obj3d_relic_slot += 1;
                            // Object3dKind::Relic fires for shop for-sale relics
                            // (single column Placement) and gameplay relics
                            // (single sidebar Placement).
                            let relic_arr_name = match self.active_scene_key {
                                Some("shop") => "shop.for_sale.relics".to_string(),
                                Some("gameplay") => "gameplay.relic_col".to_string(),
                                _ => format!("relic[{slot_i}]"),
                            };
                            let model = self.apply_arrange_override(&relic_arr_name, model);
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
                                self.relic_instances[slot_i].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
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
                            self.last_debug_pickables.push((
                                relic_arr_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
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
                                });
                            }
                            object3d_draw_list.push((DrawKind::Relic, slot_i));
                        }
                        Object3dKind::Pack { kind, pick_id } => {
                            if obj3d_pack_slot >= self.pack_instances.len() {
                                continue;
                            }
                            let slot_i = obj3d_pack_slot;
                            obj3d_pack_slot += 1;
                            let _ = slot_i;
                            let pack_arr_name = obj.arrange_name.unwrap_or("shop.for_sale.packs");
                            let model = self.apply_arrange_override(pack_arr_name, model);
                            let material = MaterialParams {
                                kind: MaterialKind::Foil,
                                base_color: obj.color,
                                specular_strength: 0.70,
                                specular_power: 48.0,
                            };
                            self.pack_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
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
                            self.proj
                                .pack_rects
                                .push((project_unit_cube_rect(model), *pick_id));
                            self.last_debug_pickables.push((
                                pack_arr_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Pack, slot_i));
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
                            // Default to the for-sale stall arrange group, but
                            // let the caller opt into a different group (e.g.
                            // owned-inventory talismans, which shouldn't share
                            // the shop's Rx/Ry/Rz arrange rotation).
                            let talisman_name =
                                obj.arrange_name.unwrap_or("shop.for_sale.talismans");
                            let talisman_center_arr = self.apply_arrange_override(
                                talisman_name,
                                translate_rot_scale(
                                    center,
                                    obj.rotation,
                                    glam::Vec3::new(sx, sy, sz),
                                ),
                            );
                            // Re-decompose center after possible override; simpler: re-derive center from matrix.
                            let talisman_model = talisman_center_arr;
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
                            self.last_talisman_models.push(talisman_model);
                            self.proj
                                .talisman_rects
                                .push(project_unit_cube_rect(talisman_model));
                            self.last_debug_pickables.push((
                                talisman_name.to_string(),
                                talisman_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Talisman, slot_i));
                        }
                        Object3dKind::ZodiacRibbon { kind } => {
                            self.place_object3d_ribbon(
                                &camera,
                                obj,
                                center,
                                kind,
                                &mut obj3d_ribbon_slot,
                                object3d_draw_list,
                            );
                        }
                        Object3dKind::Shrine { glow } => {
                            if obj3d_shrine_slot >= MAX_SHRINE_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_shrine_slot;
                            obj3d_shrine_slot += 1;
                            // Shrines are pick-blind only; one placement per slot.
                            let shrine_name = match slot_i {
                                0 => "pick_blind.shrine[0]",
                                1 => "pick_blind.shrine[1]",
                                2 => "pick_blind.shrine[2]",
                                _ => "pick_blind.shrine",
                            };
                            // Shrine center is lifted by half-height; scene passes base pos.
                            let shrine_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            // The shrine mesh is built Y-up; rotate into Z-up world so it
                            // stands upright rather than lying flat. Compose with any
                            // scene-level obj.rotation (e.g. arrange-mode overrides).
                            let shrine_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let shrine_model = self.apply_arrange_override(
                                shrine_name,
                                translate_rot_scale(
                                    shrine_center,
                                    shrine_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 1.05, 0.95, obj.color[3]];
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
                                kind: MaterialKind::Plain,
                                base_color,
                                specular_strength: 0.06,
                                specular_power: 8.0,
                            };
                            self.shrine_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                shrine_model,
                                material,
                            );
                            // Project AABB for shrine_rects (label anchoring).
                            let shrine_world_center = shrine_model.w_axis.truncate();
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
                                            shrine_world_center + glam::Vec3::new(cx, cy, cz);
                                        let (px, py) = project_to_screen(world);
                                        mn_x = mn_x.min(px);
                                        mn_y = mn_y.min(py);
                                        mx_x = mx_x.max(px);
                                        mx_y = mx_y.max(py);
                                    }
                                }
                            }
                            self.proj
                                .shrine_rects
                                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            self.last_debug_pickables.push((
                                shrine_name.to_string(),
                                shrine_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Shrine, slot_i));
                        }
                        Object3dKind::DoraPlinth { glow } => {
                            if obj3d_dora_plinth_slot >= MAX_DORA_PLINTH_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_dora_plinth_slot;
                            obj3d_dora_plinth_slot += 1;
                            let plinth_name = "gameplay.dora_plinth";
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
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let plinth_model = self.apply_arrange_override(
                                plinth_name,
                                translate_rot_scale(
                                    plinth_center,
                                    plinth_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
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
                            self.dora_plinth_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                plinth_model,
                                material,
                            );
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
                            self.proj.dora_plinth_rect =
                                Some([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            self.last_debug_pickables.push((
                                plinth_name.to_string(),
                                plinth_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::DoraPlinth, slot_i));
                        }
                        Object3dKind::SellTray { pick_id } => {
                            // Round dish mesh is built Y-up; rotate local Y
                            // into world Z so the rim sits flat on the table
                            // and `extents[1]` (rim) becomes vertical
                            // thickness. Compose with any scene rotation.
                            let oriented = mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let model = translate_rot_scale(
                                center,
                                oriented,
                                glam::Vec3::from(obj.extents),
                            );
                            let model = self.apply_arrange_override("shop.shelf.sell_tray", model);
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.3,
                                specular_power: 16.0,
                            };
                            self.sell_tray_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            if let Some(pid) = pick_id {
                                self.last_sell_tray_model = Some((model, *pid));
                            }
                            // Folded "SELL" tent card sits in the recess when
                            // the tray is focused (any control method). The
                            // shop scene encodes focus state via hover_target
                            // (≥0.5 = focused/hovered).
                            if obj.hover_target >= 0.5 {
                                if !self.sell_card_decal_ready {
                                    let rgba = crate::render::decal::rasterize_tablet_label_decal(
                                        "SELL",
                                        self.ui_font.as_ref(),
                                        self.emoji_font.as_ref(),
                                        256,
                                        128,
                                        [0.62, 0.18, 0.14, 1.0],
                                    );
                                    self.sell_card_instance.set_decal(
                                        crate::render::lit_mesh::DecalUploadCtx {
                                            device: &self.device,
                                            queue: &self.queue,
                                            layout: &self.lit_mesh_material_layout,
                                            sampler: &self.tile_sampler,
                                            relief_view: &self.lit_mesh_relief_default_view,
                                        },
                                        &rgba,
                                        256,
                                        128,
                                    );
                                    self.sell_card_decal_ready = true;
                                }
                                // Build the card model matrix anchored to the
                                // tray. Local card extents: x=-0.5..0.5,
                                // y=0..0.5, z=-0.5..0.5. The tray is a unit
                                // box with rim top at +0.5 and recess at +0.2;
                                // we shrink the card to fit inside the rim and
                                // sit on the recessed floor.
                                let (scale, rot, trans) = model.to_scale_rotation_translation();
                                // Card footprint: 60% of rim diameter, height
                                // ~70% of rim depth.
                                // Card height is decoupled from the (very
                                // shallow) rim thickness so it stays readable
                                // on the flat plate; sized off the plate
                                // footprint instead.
                                let footprint = scale.x.min(scale.z);
                                let card_scale = glam::Vec3::new(
                                    scale.x * 0.55,
                                    footprint * 0.55,
                                    scale.z * 0.55,
                                );
                                // Sit the card just above the rim top
                                // (local y=+0.5) so it doesn't poke through
                                // the shallow plate. Nudged back along local
                                // -z (world +y, deeper into scene) so the
                                // card stands toward the rear of the dish
                                // instead of centered in the recess.
                                let local_floor = glam::Vec3::new(0.0, 0.55, -0.15);
                                let world_floor = trans + rot * (local_floor * scale);
                                // Yaw the card 100° around world +Z so the
                                // crease faces the camera at a slight angle.
                                let yaw = glam::Quat::from_rotation_z(100.0_f32.to_radians());
                                let card_rot = yaw * rot;
                                let card_model = Mat4::from_scale_rotation_translation(
                                    card_scale,
                                    card_rot,
                                    world_floor,
                                );
                                let card_material = MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: [0.96, 0.93, 0.84, 1.0],
                                    specular_strength: 0.10,
                                    specular_power: 8.0,
                                };
                                self.sell_card_instance.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    card_model,
                                    card_material,
                                    true,
                                );
                                self.last_sell_card_model = Some(card_model);
                            }
                            self.last_debug_pickables.push((
                                "shop.shelf.sell_tray".to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::SellTray, 0));
                        }
                        Object3dKind::ShopLamp { glow } => {
                            // Lamp mesh is in world-space Z-up convention: no corrective
                            // rotation needed. pos is the apex/cord-attachment point (high Z).
                            // The shade rim (wide, open end) hangs below at lower Z. ✓
                            let lamp_center =
                                pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let lamp_model = self.apply_arrange_override(
                                "shop.props.lamp",
                                translate_rot_scale(
                                    lamp_center,
                                    obj.rotation,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            // Body — brass Metal material.
                            self.lamp_body_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                self.lamp_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::LampBody, 0));
                            // Bulb — Glass material. Push brightness well above
                            // 1.0 when glow is active so the HDR bulb color
                            // crosses the bloom extract threshold and glares.
                            let g = glow.clamp(0.0, 1.0);
                            let dm = &self.lamp_bulb_mesh.default_material;
                            let bulb_mat = MaterialParams {
                                kind: crate::render::lit_mesh::MaterialKind::Glass,
                                base_color: [
                                    dm.base_color[0] * (1.0 + g * 1.4),
                                    dm.base_color[1] * (1.0 + g * 1.0),
                                    dm.base_color[2] * (1.0 + g * 0.5),
                                    1.0,
                                ],
                                specular_strength: dm.specular_strength,
                                specular_power: dm.specular_power,
                            };
                            self.lamp_bulb_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                bulb_mat,
                            );
                            object3d_draw_list.push((DrawKind::LampBulb, 0));
                            // Trimesh pick: AABB of extents [w,h,w] is a bad
                            // silhouette for a lamp (thin cord on top of a wide
                            // shade) and invites accidental grabs on empty air
                            // above the shade. Ray-cast against the actual cord
                            // + cone triangles so the pick region matches what
                            // the player sees.
                            self.last_debug_trimesh_pickables.push((
                                "shop.props.lamp".to_string(),
                                lamp_model,
                                TrimeshRef::LampBody,
                            ));
                        }
                        Object3dKind::Bug {
                            slot,
                            flap_rad,
                            live_wing_alpha,
                            blur_alpha,
                        } => {
                            let slot = *slot;
                            if slot >= MAX_BUG_SLOTS {
                                continue;
                            }
                            let bug_model = translate_rot_scale(
                                center,
                                obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            self.bug_body_instances[slot].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                self.bug_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::BugBody, slot));
                            // Live wing model matrices: the mesh lives in +Y,
                            // so the left wing is the identity and the
                            // right wing flips Y (mirror across body).
                            // Flap rotates about mesh +X, which is the
                            // body axis — the right wing uses -flap so
                            // the two counter-sweep like a moth's.
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
                            object3d_draw_list.push((DrawKind::BugWingL, slot));
                            self.bug_wing_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_r,
                                wing_mat,
                                live_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingR, slot));
                            // Blur fans — the swept-volume mesh is drawn once per
                            // side with no flap rotation (the mesh itself is the
                            // full sweep). The right side reuses the same mesh
                            // with a Y-mirror transform, matching how the live
                            // wing pair is built.
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
                            object3d_draw_list.push((DrawKind::BugWingBlurL, slot));
                            self.bug_wing_blur_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0)),
                                blur_mat,
                                blur_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingBlurR, slot));
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
                            object3d_draw_list.push((DrawKind::Orb, slot_i));
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
                                rot_rz_rx_deg(tilt_deg, *rotation_z_deg),
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.mirror", hover_model);
                            self.mirror_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.mirror_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.mirror_rect = Some(project_aabb_rect(
                                    hover_model,
                                    MIRROR_LOCAL_HALF,
                                    MIRROR_LOCAL_CENTER_Y,
                                ));
                                self.last_mirror_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.mirror".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    MIRROR_LOCAL_HALF[0],
                                    MIRROR_LOCAL_HALF[1],
                                    MIRROR_LOCAL_HALF[2],
                                ),
                                MIRROR_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Mirror, slot_i));
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
                            if !self.extruded_glyph_meshes.contains_key(label) {
                                if let Some(cpu) = self.glyph_cpu_cache.mesh_for(label) {
                                    let gpu = LitMeshGpu::new(
                                        &self.device,
                                        cpu,
                                        &format!("glyph-{}", label),
                                    );
                                    self.extruded_glyph_meshes.insert(label.clone(), gpu);
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
                            let glyph_model =
                                self.apply_arrange_override("gameplay.score_popup", glyph_model);
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
                            self.last_debug_pickables.push((
                                "gameplay.score_popup".to_string(),
                                glyph_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::ExtrudedGlyph, slot_i));
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
                            let cascade_token_name = match ck {
                                CascadeTokenKind::Chips => "gameplay.cascade_token.chips",
                                CascadeTokenKind::Mult => "gameplay.cascade_token.mult",
                            };
                            let model = translate_rot_scale(
                                center,
                                Mat4::IDENTITY,
                                glam::Vec3::new(
                                    obj.extents[0] * pulse_scale,
                                    obj.extents[1] * pulse_scale,
                                    obj.extents[2] * pulse_scale,
                                ),
                            );
                            let model = self.apply_arrange_override(cascade_token_name, model);
                            let base = match ck {
                                CascadeTokenKind::Chips => [0.55, 0.78, 1.00, 1.0],
                                CascadeTokenKind::Mult => [0.85, 0.32, 0.42, 1.0],
                            };
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
                            self.last_debug_pickables.push((
                                cascade_token_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::CascadeToken, slot_i));
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
                            let candle_name = self.scene_path("candle");
                            let candle_model =
                                self.apply_arrange_override(&candle_name, candle_model);
                            self.candle_instances[slot_i][0].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wax_mesh.default_material,
                            );
                            self.candle_instances[slot_i][1].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wick_mesh.default_material,
                            );
                            self.last_debug_pickables.push((
                                candle_name,
                                candle_model,
                                glam::Vec3::new(0.36, 0.305, 0.36),
                                0.305,
                            ));
                            object3d_draw_list.push((DrawKind::CandleWax, slot_i));
                            object3d_draw_list.push((DrawKind::CandleWick, slot_i));
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
                            let arrange_name = match fan_kind {
                                TallyFanKind::Draws => "gameplay.counter.draws_fan",
                                TallyFanKind::Discards => "gameplay.counter.discards_fan",
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
                            for (stick_i, &k) in visible_slots.iter().enumerate() {
                                if obj3d_tally_stick_cursor + 1 >= MAX_TALLY_STICK_SLOTS * 2 {
                                    break;
                                }
                                let angle = slot_angle(k);
                                let rot = fan_yaw * Mat4::from_rotation_y(angle) * base_orient;
                                let model = translate_rot_scale(pivot, rot, base_scale);
                                let model = self.apply_arrange_override(arrange_name, model);
                                if stick_i == 0 {
                                    self.last_debug_pickables.push((
                                        arrange_name.to_string(),
                                        model,
                                        glam::Vec3::new(0.5, 0.5, 0.5),
                                        0.0,
                                    ));
                                }
                                self.tally_stick_instances[obj3d_tally_stick_cursor].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    base_material,
                                );
                                self.tally_stick_instances[obj3d_tally_stick_cursor + 1]
                                    .write_uniform(&self.queue, view_proj_arr, model, tip_material);
                                object3d_draw_list
                                    .push((DrawKind::TallyStickBase, obj3d_tally_stick_cursor));
                                object3d_draw_list
                                    .push((DrawKind::TallyStickTip, obj3d_tally_stick_cursor + 1));
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
                            // Scene passes rotation_x_deg via obj.rotation (Mat4::from_rotation_x).
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
                                glam::Mat4::from_rotation_x(tilt) * obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.bowl", hover_model);
                            self.bowl_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.bowl_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.bowl_rect = Some(project_aabb_rect(
                                    hover_model,
                                    BOWL_LOCAL_HALF,
                                    BOWL_LOCAL_CENTER_Y,
                                ));
                                self.last_bowl_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.bowl".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    BOWL_LOCAL_HALF[0],
                                    BOWL_LOCAL_HALF[1],
                                    BOWL_LOCAL_HALF[2],
                                ),
                                BOWL_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Bowl, slot_i));
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
                let model = self.apply_arrange_override("WallTile", model);
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
                // Wall tiles aren't arrangeable — keep the legacy name so
                // the hit-test debug overlay still identifies them.
                self.last_debug_pickables.push((
                    "gameplay.wall_tile".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
                wall_tile_slot_cursor += 1;
            }
        }
    }
}
