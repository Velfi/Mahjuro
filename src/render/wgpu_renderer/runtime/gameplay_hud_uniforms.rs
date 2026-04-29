use super::*;

impl WgpuRenderer {
    /// Skeuomorphic gameplay HUD uniform writes (phase 1) — plaque, ofuda,
    /// tablets, bowl, peg block, wall stack. Walks the cmd list, writes
    /// per-instance uniforms, and projects screen-space rects for hit-test.
    pub(super) fn write_gameplay_hud_uniforms(
        &mut self,
        camera: &CameraFrame,
        yaku_tablet_batches: &[&[YakuTabletPlacement]],
    ) {
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
        // ── Skeuomorphic gameplay HUD uniform writes (phase 1) ─────────
        //
        // The new HUD meshes (plaque, ofuda, tablets, bowl, peg block, wall
        // stack) all share the lit-mesh pipeline. Each gets its
        // own slot pool above; per-frame we walk the cmds, write the
        // per-instance uniform, and (where the scene needs it for hit
        // testing in later phases) project the AABB to a screen-space rect.
        self.proj.yaku_tablet_rects.clear();
        self.proj.wood_tablet_rects.clear();
        self.proj.plaque_rects.clear();
        self.proj.bowl_rect = None;
        self.proj.mirror_rect = None;
        self.proj.dora_plinth_rect = None;
        self.proj.peg_rects = [None, None];
        self.proj.aux_dish_rects.clear();
        self.last_aux_dish_aabbs.clear();
        self.last_yaku_tablet_models.clear();
        self.last_wood_tablet_models.clear();
        self.last_bowl_model = None;
        self.last_mirror_model = None;

        // Helper closure: project the unit-cube AABB transformed by `model`
        // into a screen-space rect. Used by tablets/bowl for hit testing.
        let project_unit_cube_rect = |model: Mat4| -> [f32; 4] {
            let corners = [
                glam::Vec3::new(-0.5, -0.5, -0.5),
                glam::Vec3::new(0.5, -0.5, -0.5),
                glam::Vec3::new(-0.5, 0.5, -0.5),
                glam::Vec3::new(0.5, 0.5, -0.5),
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in corners {
                let w = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
        };

        // Plaques (single instance per cmd).
        // Yaku tablet batches.
        let mut yaku_tablet_slot_cursor: usize = 0;
        for batch in yaku_tablet_batches.iter() {
            for t in batch.iter() {
                if yaku_tablet_slot_cursor >= MAX_YAKU_TABLET_SLOTS {
                    break;
                }
                let slot_i = yaku_tablet_slot_cursor;
                yaku_tablet_slot_cursor += 1;
                // Hover lift along world Y so the tablet rises off the tray.
                let lift = t.hover.clamp(0.0, 1.0) * t.extents[1] * 0.4;
                let center = pixel_to_world(
                    w,
                    h,
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift,
                );
                // Tilt top face toward the camera (same Rx sign as wood tablets).
                let tilt_deg = -25.0_f32;
                let model = translate_rot_scale(
                    center,
                    rot_rz_rx_deg(tilt_deg, t.rotation_z_deg),
                    glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]),
                );
                let model = self.apply_arrange_override("gameplay.hand.yaku_tablet", model);
                // Active tablets warm up to a champagne tint; dim ones stay
                // bone. The decal pass (phase 2) will paint the engraved name
                // on top via a per-instance albedo texture.
                // Porcelain: bright cool-white when idle; a warmer cream
                // cast when this yaku is the selected target so it still
                // reads as "active" against the row.
                let base = if t.active {
                    [1.00, 0.97, 0.90, 1.0]
                } else {
                    [0.97, 0.96, 0.94, 1.0]
                };
                let material = MaterialParams {
                    kind: MaterialKind::Porcelain,
                    base_color: base,
                    // Shiny-chiclet glaze: strong pinpoint specular plus
                    // the shader's wet-glaze and rim terms. Hovering
                    // bumps the pinpoint so the lifted tablet looks
                    // freshly polished rather than just brighter.
                    specular_strength: 0.95 + 0.25 * t.hover.clamp(0.0, 1.0),
                    specular_power: 72.0,
                };
                // Engraved-name decal: rasterise on label change, then bind
                // it as the per-instance albedo overlay. Cached by label hash
                // so the steady-state cost is one compare per slot per frame.
                let label_hash = tablet_label_hash(&t.name, 256, 96);
                let inst = &mut self.yaku_tablet_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                        &t.name,
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
                inst.write_uniform_with_decal(&self.queue, view_proj_arr, model, material, true);
                self.proj
                    .yaku_tablet_rects
                    .push(project_unit_cube_rect(model));
                self.last_yaku_tablet_models.push(model);
                self.last_debug_pickables.push((
                    "gameplay.hand.yaku_tablet".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }
    }
}
