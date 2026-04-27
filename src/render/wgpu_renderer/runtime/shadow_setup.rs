use super::*;

pub(super) const SHADOW_MAP_SIZE: f32 = 2048.0;

#[allow(dead_code)] // light_view_proj kept available for future per-caster passes.
pub(super) struct ShadowFrame {
    pub light_view_proj: Mat4,
    pub light_view_proj_arr: [f32; 16],
}

impl WgpuRenderer {
    /// Build the directional shadow camera and upload the shared shadow
    /// globals uniform consumed by the shadow pre-pass + lit_mesh PCF tap.
    /// Returns the light view-proj so callers can write per-caster shadow
    /// uniforms in the same frame.
    pub(super) fn setup_shadow_frame(
        &self,
        camera: &CameraFrame,
        shadows_enabled: bool,
    ) -> ShadowFrame {
        // Anchor the shadow frustum to the same key direction the lit
        // shaders use. The orthographic frustum is sized to cover the play
        // area where casters live, not the full table — most of the table
        // is empty wood and would burn shadow texels for nothing.
        let key_dir = glam::Vec3::new(0.25, 1.0, 0.35).normalize();
        // Half-extents in world units. Generous so candles + relics on the
        // sides of the play area stay inside the frustum at any window
        // aspect.
        let shadow_half_x = (camera.w * 0.6).max(camera.h * 0.6);
        let shadow_half_z = (camera.w * 0.6).max(camera.h * 0.6);
        // Light eye sits along +key_dir from the play-area center. The eye
        // distance + far plane are kept TIGHT around the scene (~80 world
        // units) so [0,1] light-space depth resolves the few units between
        // casters and the table well.
        let shadow_center = glam::Vec3::ZERO;
        let scene_height = 80.0_f32;
        let eye_dist = scene_height * 0.5;
        let shadow_eye = shadow_center + key_dir * eye_dist;
        let shadow_view = Mat4::look_at_rh(shadow_eye, shadow_center, glam::Vec3::Y);
        let shadow_proj = Mat4::orthographic_rh(
            -shadow_half_x,
            shadow_half_x,
            -shadow_half_z,
            shadow_half_z,
            0.1,
            scene_height,
        );
        let light_view_proj = shadow_proj * shadow_view;
        let light_view_proj_arr = light_view_proj.to_cols_array();
        let shadow_enabled_flag = if shadows_enabled { 1.0_f32 } else { 0.0 };
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: light_view_proj_arr,
                params: [
                    shadow_enabled_flag,
                    // Depth bias in light-space [0,1] depth. With
                    // scene_height = 80, 0.005 ≈ 0.4 world units — big
                    // enough to hide self-shadow acne, small enough that
                    // 1-unit-tall tiles still cast onto the table.
                    0.005,
                    1.0 / SHADOW_MAP_SIZE,
                    0.0,
                ],
            }),
        );
        ShadowFrame {
            light_view_proj,
            light_view_proj_arr,
        }
    }

    /// Per-instance shadow caster uniforms. Mirrors the model matrices
    /// written into the main lit-mesh + hand-tile uniforms so the shadow
    /// pre-pass can re-render the same geometry from the light's POV. Table
    /// is excluded — it's a flat receiver, not a caster. Returns nothing —
    /// writes to per-instance shadow uniform buffers via `self.queue`.
    pub(super) fn write_per_instance_shadow_casters(
        &mut self,
        frame: &UiFrame,
        camera: &CameraFrame,
        light_view_proj_arr: [f32; 16],
        tile_pick_models: &[(usize, Mat4)],
        shrine_batches: &[&[ShrinePlacement]],
    ) {
        let w = camera.w;
        let h = camera.h;
        // ── Per-instance shadow caster uniforms ─────────────────────────
        // Mirror the model matrices written into the main lit-mesh +
        // hand-tile uniforms above so the shadow pre-pass can re-render
        // the same geometry from the light's POV.
        // Table is excluded — it's a flat receiver, not a caster.
        // Candle shadows: walk Object3dKind::Candle in the cmd list.
        {
            let mut slot_i = 0usize;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::Candle {
                        scale,
                        height_scale,
                    } = o.kind
                    {
                        let Some(instances) = self.candle_instances.get(slot_i) else {
                            break;
                        };
                        let base = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let model = translate_rot_scale(
                            base,
                            mesh_y_thickness_along_local_y_to_z_up(),
                            glam::Vec3::new(scale, scale * height_scale, scale),
                        );
                        let candle_name = self.scene_path("candle");
                        let model = self.apply_arrange_override(&candle_name, model);
                        instances[0].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        instances[1].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        slot_i += 1;
                    }
                }
            }
        }
        // Shrine shadow casters (pick-blind scene). Same model as the main
        // pass: scale by extents, lift base by half-height.
        {
            let mut shrine_shadow_cursor: usize = 0;
            for batch in shrine_batches.iter() {
                for s in batch.iter() {
                    if shrine_shadow_cursor >= MAX_SHRINE_SLOTS {
                        break;
                    }
                    let slot_i = shrine_shadow_cursor;
                    shrine_shadow_cursor += 1;
                    let center = pixel_to_world(
                        w,
                        h,
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = translate_rot_scale(
                        center,
                        Mat4::IDENTITY,
                        glam::Vec3::new(s.extents[0], s.extents[1], s.extents[2]),
                    );
                    self.shrine_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Ribbon shadow casters — walk Object3dKind::ZodiacRibbon.
        {
            let mut ribbon_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::ZodiacRibbon { kind } = &o.kind {
                        if ribbon_shadow_cursor >= MAX_RIBBON_SLOTS {
                            break;
                        }
                        let anchor = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let eff_w = o.extents[0];
                        let eff_l = o.extents[1];
                        let depth = o.extents[2];
                        let base_transform =
                            translate_rot_scale(anchor, o.rotation, glam::Vec3::splat(1.0));
                        if kind.is_some() {
                            let nominal_cap = eff_w * 0.33;
                            let cap_h = if eff_l < 2.0 * nominal_cap {
                                eff_l / 2.0
                            } else {
                                nominal_cap
                            };
                            let mid_h = (eff_l - 2.0 * cap_h).max(0.0);
                            let slots_needed = if mid_h > 0.0 { 3 } else { 2 };
                            if ribbon_shadow_cursor + slots_needed > MAX_RIBBON_SLOTS {
                                break;
                            }
                            let top_model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                top_model,
                            );
                            ribbon_shadow_cursor += 1;
                            if mid_h > 0.0 {
                                let mid_model = ribbon_submesh(
                                    base_transform,
                                    -cap_h,
                                    glam::Vec3::new(eff_w, mid_h, depth),
                                );
                                self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                    &self.queue,
                                    light_view_proj_arr,
                                    mid_model,
                                );
                                ribbon_shadow_cursor += 1;
                            }
                            let bot_model = ribbon_submesh(
                                base_transform,
                                -(cap_h + mid_h),
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                bot_model,
                            );
                            ribbon_shadow_cursor += 1;
                        } else {
                            let model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, eff_l, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                model,
                            );
                            ribbon_shadow_cursor += 1;
                        }
                    }
                }
            }
        }
        // Talisman shadows: walk Object3dKind::Talisman in the cmd list.
        {
            let mut talisman_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if matches!(
                        o.kind,
                        crate::render::draw_cmd::Object3dKind::Talisman { .. }
                    ) {
                        if talisman_shadow_cursor >= MAX_TALISMAN_SLOTS {
                            break;
                        }
                        let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let sx = o.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                        let sy = o.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                        let sz = o.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                        let model =
                            translate_rot_scale(center, o.rotation, glam::Vec3::new(sx, sy, sz));
                        self.talisman_instances[talisman_shadow_cursor].write_shadow_uniform(
                            &self.queue,
                            light_view_proj_arr,
                            model,
                        );
                        talisman_shadow_cursor += 1;
                    }
                }
            }
        }
        // Primitive shadow casters: walk every `Object3dKind::Primitive`
        // in frame cmds whose `shadow_caster` flag is true, and upload
        // the shadow uniform into the matching per-shape instance slot.
        // Slot cursors must track the main dispatch's
        // `obj3d_primitive_slot` exactly so each caster maps to the
        // instance the draw-pass will select.
        {
            use crate::render::primitive::{MeshId, shape_orientation};
            let mut cursors: HashMap<MeshId, usize> = HashMap::new();
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::Primitive {
                        shape,
                        shadow_caster,
                        ..
                    } = &o.kind
                    {
                        // Step the cursor for every Primitive (matches
                        // the main dispatch). Only *write* a shadow
                        // uniform when `shadow_caster: true`.
                        let slot_i = *cursors.entry(*shape).or_insert(0);
                        *cursors.get_mut(shape).unwrap() += 1;
                        if *shadow_caster {
                            let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                            let orient = shape_orientation(*shape);
                            let model = translate_rot_scale(
                                center,
                                o.rotation * orient,
                                glam::Vec3::from(o.extents),
                            );
                            if let Some(pool) = self.primitive_instances.get_mut(shape)
                                && let Some(inst) = pool.get_mut(slot_i)
                            {
                                inst.write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                            }
                        }
                        // CabinetColumn pairs with a CabinetRails
                        // instance in the main dispatch — bump the
                        // rails cursor so shadow slots stay in sync,
                        // but don't cast rails shadows (too thin).
                        if *shape == MeshId::CabinetColumn {
                            *cursors.entry(MeshId::CabinetRails).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        // Hand tile shadow uniforms — pull each tile's model matrix from
        // `tile_pick_models` (snapshot of the per-tile model written above).
        for (i, model) in tile_pick_models.iter() {
            if let Some(htg) = self.hand_tiles.get(*i) {
                self.queue.write_buffer(
                    &htg.shadow_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    }),
                );
            }
        }
    }
}
