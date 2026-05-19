use super::*;

impl WgpuRenderer {
    /// Place a single `Object3dKind::ZodiacRibbon` instance: write the
    /// uniform into one ribbon pool slot and append `(DrawKind::Ribbon,
    /// slot_i)` to `object3d_draw_list`. Skips the placement when the
    /// pool is full.
    ///
    /// The ribbon is rendered as a single mesh with one tall portrait
    /// texture per zodiac (`zodiac_<slug>.png`); the texture is mapped
    /// full-bleed across the ribbon's length, so the finial / silk body
    /// / tassel proportions are baked into the source image.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_object3d_ribbon(
        &mut self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        obj: &crate::render::draw_cmd::Object3d,
        center: glam::Vec3,
        kind: &Option<crate::core::zodiac::ZodiacKind>,
        ribbon_slot: &mut usize,
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        shadow: &mut Option<&mut super::shadow_setup::Object3dShadowCtx<'_>>,
    ) {
        let view_proj_arr = camera.view_proj_arr;
        // Ribbon mesh local y ∈ [-0.5, 0.5] with origin at centroid; z half
        // is HALF_THICKNESS (0.05), not 0.5 — project that AABB, not the unit cube.
        const RIBBON_HALF: glam::Vec3 = glam::Vec3::new(0.5, 0.5, 0.5);
        const RIBBON_CENTER_Y: f32 = 0.0;
        let project_ribbon_rect = |model: Mat4| -> [f32; 4] {
            camera.project_aabb_rect(
                model,
                [RIBBON_HALF.x, RIBBON_HALF.y, RIBBON_HALF.z],
                RIBBON_CENTER_Y,
            )
        };
        // extents: [width, length, depth].
        let eff_w = obj.extents[0];
        let eff_l = obj.extents[1];
        let depth = obj.extents[2];
        let ribbon_arr_name = obj.arrange_name.unwrap_or("shop.for_sale.ribbons");
        let base_transform = self.apply_arrange_override(
            ribbon_arr_name,
            translate_rot_scale(center, obj.rotation_matrix(), glam::Vec3::splat(1.0)),
        );
        let full_ribbon_model =
            ribbon_submesh(base_transform, 0.0, glam::Vec3::new(eff_w, eff_l, depth));
        self.last_ribbon_models.push(full_ribbon_model);
        self.proj
            .ribbon_rects
            .push(project_ribbon_rect(full_ribbon_model));
        self.last_debug_pickables.push((
            ribbon_arr_name.to_string(),
            full_ribbon_model,
            RIBBON_HALF,
            RIBBON_CENTER_Y,
        ));
        if *ribbon_slot >= MAX_RIBBON_SLOTS {
            return;
        }
        let silk_mat = MaterialParams {
            kind: MaterialKind::Plain,
            base_color: obj.color,
            specular_strength: 0.25,
            specular_power: 16.0,
        };
        let zodiac_id: Option<u8> = kind.as_ref().and_then(|z| {
            let tex_idx = crate::core::zodiac::ZodiacKind::all()
                .iter()
                .position(|&k| k == *z)? as u8;
            Some(tex_idx)
        });
        let slot_i = *ribbon_slot;
        *ribbon_slot += 1;
        self.shadow_placement_anim_id = obj.anim_id;
        if self.ribbon_slot_zodiac[slot_i] != zodiac_id {
            let view: &wgpu::TextureView = match zodiac_id {
                Some(idx) => &self.ribbon_zodiac_tex.views[idx as usize],
                None => &self.lit_mesh_white_view,
            };
            let inst = &mut self.ribbon_instances[slot_i];
            inst.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ribbon-bg-obj3d"),
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
                        resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            &self.lit_mesh_relief_default_view,
                        ),
                    },
                ],
            });
            self.ribbon_slot_zodiac[slot_i] = zodiac_id;
        }
        self.ribbon_instances[slot_i].write_uniform(
            &self.queue,
            view_proj_arr,
            full_ribbon_model,
            silk_mat,
        );
        self.register_placement_shadow_slot(DrawKind::Ribbon, slot_i);
        if self.placement_shadow_writes(frame) {
            self.write_lit_mesh_shadow(
                shadow,
                &self.ribbon_instances[slot_i],
                full_ribbon_model,
                silk_mat.kind,
            );
        }
        WgpuRenderer::push_object3d_draw(object3d_draw_list, DrawKind::Ribbon, slot_i);
    }
}
