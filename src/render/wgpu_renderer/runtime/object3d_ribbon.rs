use super::*;

impl WgpuRenderer {
    /// Place a single `Object3dKind::ZodiacRibbon` instance: write per-segment
    /// uniforms into the ribbon pool and append `(DrawKind::Ribbon, slot_i)`
    /// entries to `object3d_draw_list`. Skips the placement when the pool is
    /// full.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_object3d_ribbon(
        &mut self,
        camera: &CameraFrame,
        obj: &crate::render::draw_cmd::Object3d,
        center: glam::Vec3,
        kind: &Option<crate::core::zodiac::ZodiacKind>,
        ribbon_slot: &mut usize,
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
    ) {
        let view_proj_arr = camera.view_proj_arr;
        let project_unit_cube_rect =
            |model: Mat4| -> [f32; 4] { camera.project_unit_cube_rect(model) };
        let mut obj3d_ribbon_slot = *ribbon_slot;
        // extents: [width, length, depth].
        let eff_w = obj.extents[0];
        let eff_l = obj.extents[1];
        let depth = obj.extents[2];
        // Push the overall ribbon AABB for arrange-mode picking.
        // (Individual segments aren't separately selectable.)
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
            .push(project_unit_cube_rect(full_ribbon_model));
        self.last_debug_pickables.push((
            ribbon_arr_name.to_string(),
            full_ribbon_model,
            glam::Vec3::new(0.5, 0.5, 0.5),
            0.0,
        ));
        // Three texture tiles (top / mid / bot) are authored as equal squares;
        // split world length in thirds so each segment maps 1:1 without stretching.
        let seg_h = eff_l / 3.0;
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
        // Emit segments: top (0), mid (1), bottom (2).
        let segments: &[(f32, f32, u8)] = &[
            (0.0, seg_h, 0),
            (-seg_h, seg_h, 1),
            (-(2.0 * seg_h), seg_h, 2),
        ];
        for &(offset, seg_h, seg_idx) in segments {
            if obj3d_ribbon_slot >= MAX_RIBBON_SLOTS {
                break;
            }
            let slot_i = obj3d_ribbon_slot;
            obj3d_ribbon_slot += 1;
            let seg_model =
                ribbon_submesh(base_transform, offset, glam::Vec3::new(eff_w, seg_h, depth));
            let rzod = zodiac_id.map(|ti| (ti, seg_idx));
            if self.ribbon_slot_zodiac[slot_i] != rzod {
                let view: &wgpu::TextureView = match rzod {
                    Some((idx, 0)) => &self.ribbon_zodiac_tex.top_views[idx as usize],
                    Some((idx, 1)) => &self.ribbon_zodiac_tex.mid_views[idx as usize],
                    Some((idx, _)) => &self.ribbon_zodiac_tex.bot_views[idx as usize],
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
                self.ribbon_slot_zodiac[slot_i] = rzod;
            }
            self.ribbon_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                seg_model,
                silk_mat,
            );
            object3d_draw_list.push((DrawKind::Ribbon, slot_i));
        }
        *ribbon_slot = obj3d_ribbon_slot;
    }
}
