use super::*;

impl WgpuRenderer {
    /// Place a single `Object3dKind::Primitive` instance: write its uniform
    /// into the per-shape pool, rasterize its decal if needed, and project
    /// its screen-space rect for hit testing. Pushes a single
    /// `(DrawKind::Primitive(shape), slot_i)` entry onto `object3d_draw_list`
    /// (and a paired `CabinetRails` entry for `CabinetColumn`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_object3d_primitive(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        obj: &crate::draw_cmd::Object3d,
        shape: &crate::primitive::MeshId,
        material: &crate::primitive::MaterialSpec,
        pick_id: &Option<u32>,
        silhouette: &bool,
        obj3d_primitive_slot: &mut rustc_hash::FxHashMap<crate::primitive::MeshId, usize>,
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        object3d_shadow_draw_list: &mut Vec<(DrawKind, usize)>,
        shadow: &mut Option<&mut super::shadow_setup::Object3dShadowCtx<'_>>,
    ) {
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
        use crate::primitive::{MeshId, resolve_material, shape_orientation};
        if *shape == MeshId::Coin {
            self.place_object3d_gltf_coin(
                frame,
                camera,
                obj,
                pick_id,
                obj3d_primitive_slot,
                object3d_draw_list,
                object3d_shadow_draw_list,
                shadow,
            );
            return;
        }
        // Slot bookkeeping is per-shape so two
        // primitives of different shapes don't
        // fight for the same pool index.
        let cursor = obj3d_primitive_slot.entry(*shape).or_insert(0);
        let slot_i = *cursor;
        *cursor += 1;
        // Lazily grow the per-shape instance pool.
        // When a per-shape texture override is
        // registered, bind it to the instance's
        // albedo + relief slots so material
        // branches that sample heightmaps (e.g.
        // Metal coin) work.
        let (albedo_v, relief_v) = match self.primitive_textures.get(shape) {
            Some((a, r)) => (a, r),
            None => (
                &self.lit_mesh_white_view,
                &self.lit_mesh_relief_default_view,
            ),
        };
        let pool = self.primitive_instances.entry(*shape).or_default();
        while pool.len() < slot_i + 1 {
            pool.push(LitMeshInstance::new(
                &self.device,
                &self.lit_mesh_material_layout,
                &self.shadow_caster_layout,
                albedo_v,
                relief_v,
                &self.tile_sampler,
            ));
        }
        // Decal rasterization + cache, unified for
        // every shape via `rasterize_decal`.
        let has_decal = if *silhouette {
            false
        } else if let Some(decal_spec) = &material.decal {
            let (dw, dh) = crate::decal::decal_dimensions(&decal_spec.layout, obj.extents);
            let label_hash = tablet_label_hash(&decal_spec.text, dw, dh);
            let inst = &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
            if inst.decal_texture.is_none()
                || inst.decal_label_hash != label_hash
                || inst.decal_size != (dw, dh)
            {
                let rgba = crate::decal::rasterize_decal(
                    decal_spec,
                    dw,
                    dh,
                    self.ui_font.as_ref(),
                    self.emoji_font.as_ref(),
                );
                inst.set_decal(
                    crate::lit_mesh::DecalUploadCtx {
                        device: &self.device,
                        queue: &self.queue,
                        layout: &self.lit_mesh_material_layout,
                        sampler: &self.tile_sampler,
                        relief_view: &self.lit_mesh_relief_default_view,
                    },
                    &rgba,
                    dw,
                    dh,
                );
                inst.decal_label_hash = label_hash;
            }
            true
        } else {
            false
        };
        // Compose the per-shape mesh orientation
        // (identity for most; Y-up-to-Z-up for
        // Cylinder / DiscRound). Applied BEFORE
        // extents scaling — i.e. rotate the local
        // unit mesh into its canonical frame, then
        // scale, then translate+rotate into world.
        // Rebuild the model matrix here to preserve
        // legacy ordering `T * R * O * S`.
        let orient = shape_orientation(*shape);
        let model = translate_rot_scale(
            pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]),
            obj.rotation_matrix() * orient,
            glam::Vec3::from(obj.extents),
        );
        if let Some(pid) = pick_id {
            self.last_primitive_pick_models.insert(*pid, model);
        }
        let params = resolve_material(material, obj.color, *silhouette);
        let tint = if *silhouette {
            [0.04, 0.04, 0.05, obj.color[3]]
        } else {
            obj.color
        };
        let inst = &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
        if *silhouette {
            inst.write_uniform_tinted(&self.queue, view_proj_arr, model, params, tint);
        } else {
            inst.write_uniform_with_decal(&self.queue, view_proj_arr, model, params, has_decal);
        }
        self.write_lit_mesh_shadow(
            shadow,
            &self.primitive_instances.get(shape).unwrap()[slot_i],
            model,
            params.kind,
        );
        // Screen-space rect for focus/hover hit
        // testing. BeveledSlab projects only the
        // +Z face (back is never seen); other
        // shapes use the full AABB.
        let corners: &[glam::Vec3] = if *shape == MeshId::BeveledSlab {
            &[
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ]
        } else {
            &[
                glam::Vec3::new(-0.5, -0.5, -0.5),
                glam::Vec3::new(0.5, -0.5, -0.5),
                glam::Vec3::new(-0.5, 0.5, -0.5),
                glam::Vec3::new(0.5, 0.5, -0.5),
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ]
        };
        let mut mn_x = f32::INFINITY;
        let mut mn_y = f32::INFINITY;
        let mut mx_x = f32::NEG_INFINITY;
        let mut mx_y = f32::NEG_INFINITY;
        for c in corners {
            let w_pt = model.transform_point3(*c);
            let (sx, sy) = project_to_screen(w_pt);
            mn_x = mn_x.min(sx);
            mn_y = mn_y.min(sy);
            mx_x = mx_x.max(sx);
            mx_y = mx_y.max(sy);
        }
        if *shape == MeshId::BeveledSlab {
            self.proj
                .plaque_rects
                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
        }
        // Dish-shaped primitives feed the pick/focus
        // `aux_dish_rects` pipeline (shop trays,
        // pick-blind altars, gameplay talisman dish)
        // and the raycast AABB used by mouse picking.
        // ShopActionProp / Abacus / ShopBell reuse
        // `aux_dish_rects` as the shop's focus-nav
        // and click channel too — `ShopHit::Dish(pid)`
        // mapping is historical from when the props
        // piggy-backed on the dish rect list.
        if matches!(
            *shape,
            MeshId::DiscSquare
                | MeshId::DiscRound
                | MeshId::ShopActionProp
                | MeshId::Abacus
                | MeshId::ShopBell
        ) {
            self.proj
                .aux_dish_rects
                .push((*pick_id, [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
            let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
            let half = glam::Vec3::new(
                obj.extents[0] * 0.5,
                obj.extents[1] * 0.5,
                obj.extents[2] * 0.5,
            );
            self.last_aux_dish_aabbs.push((center, half));
        }
        self.push_object3d_draw(
            object3d_draw_list,
            object3d_shadow_draw_list,
            DrawKind::Primitive(*shape),
            slot_i,
        );
        // CabinetColumn emits a linked CabinetRails
        // instance sharing the same world-space
        // model matrix (post arrange override).
        if *shape == MeshId::CabinetColumn {
            let rails_cursor = obj3d_primitive_slot
                .entry(MeshId::CabinetRails)
                .or_insert(0);
            let rails_slot = *rails_cursor;
            *rails_cursor += 1;
            let rails_pool = self
                .primitive_instances
                .entry(MeshId::CabinetRails)
                .or_default();
            while rails_pool.len() < rails_slot + 1 {
                rails_pool.push(LitMeshInstance::new(
                    &self.device,
                    &self.lit_mesh_material_layout,
                    &self.shadow_caster_layout,
                    &self.lit_mesh_white_view,
                    &self.lit_mesh_relief_default_view,
                    &self.tile_sampler,
                ));
            }
            let rails_mesh = self
                .primitive_meshes
                .get(&MeshId::CabinetRails)
                .expect("CabinetRails mesh missing from registry");
            rails_pool[rails_slot].write_uniform_with_decal(
                &self.queue,
                view_proj_arr,
                model,
                rails_mesh.default_material,
                false,
            );
            let rails_kind = rails_mesh.default_material.kind;
            self.write_lit_mesh_shadow(
                shadow,
                &self.primitive_instances[&MeshId::CabinetRails][rails_slot],
                model,
                rails_kind,
            );
            self.push_object3d_draw(
                object3d_draw_list,
                object3d_shadow_draw_list,
                DrawKind::Primitive(MeshId::CabinetRails),
                rails_slot,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn place_object3d_gltf_coin(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        obj: &crate::draw_cmd::Object3d,
        pick_id: &Option<u32>,
        obj3d_primitive_slot: &mut rustc_hash::FxHashMap<crate::primitive::MeshId, usize>,
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
        object3d_shadow_draw_list: &mut Vec<(DrawKind, usize)>,
        shadow: &mut Option<&mut super::shadow_setup::Object3dShadowCtx<'_>>,
    ) {
        use crate::gltf_prop::{GLTF_PROP_BODY_KIND, make_gltf_prop_gpu};
        use crate::primitive::MeshId;

        let cursor = obj3d_primitive_slot.entry(MeshId::Coin).or_insert(0);
        let slot_i = *cursor;
        *cursor += 1;

        let orient = crate::primitive::shape_orientation(MeshId::Coin);
        let scale = crate::coin_glb::layout_scale_for_extents(obj.extents);
        let model = translate_rot_scale(
            pixel_to_world(camera.w, camera.h, obj.pos[0], obj.pos[1], obj.pos[2]),
            obj.rotation_matrix() * orient,
            scale,
        );
        if let Some(pid) = pick_id {
            self.last_primitive_pick_models.insert(*pid, model);
        }

        while self.coin_glb_instances.len() < slot_i + 1 {
            self.coin_glb_instances.push(make_gltf_prop_gpu(
                &self.device,
                &self.tile_material_layout,
                &self.shadow_caster_layout,
                &self.coin_glb_primitives,
                &self.lit_mesh_white_view,
                &self.tile_env_distortion_placeholder,
            ));
        }

        let mut hdr_tonemap = self.tile_hdr_tonemap(frame);
        hdr_tonemap[3] =
            self.room_punctual_inv_doc_scale(camera, frame.scene_lighting.embedded_gltf_punctual);
        let inst = &mut self.coin_glb_instances[slot_i];
        self.queue.write_buffer(
            &inst.uniform_buffer,
            0,
            bytemuck::bytes_of(&super::super::CameraUniform {
                view_proj: camera.view_proj_arr,
                model: model.to_cols_array(),
                base_color_factor: [
                    obj.color[0],
                    obj.color[1],
                    obj.color[2],
                    GLTF_PROP_BODY_KIND,
                ],
                cam_pos: camera.cam_pos.to_array(),
                tile_seed: 0.0,
                decal_atlas_uv: [0.0, 0.0, 1.0, 1.0],
                hdr_tonemap,
            }),
        );

        let light_view_proj = shadow
            .as_ref()
            .map(|s| s.light_view_proj)
            .unwrap_or(glam::Mat4::IDENTITY.to_cols_array());
        let su = crate::lit_mesh::ShadowCasterUniform {
            light_view_proj,
            model: model.to_cols_array(),
        };
        if inst.cached_shadow_caster != su {
            inst.cached_shadow_caster = su;
            self.queue
                .write_buffer(&inst.shadow_uniform_buffer, 0, bytemuck::bytes_of(&su));
            if let Some(shadow) = shadow.as_mut() {
                *shadow.changed = true;
            }
        }

        self.push_object3d_draw(
            object3d_draw_list,
            object3d_shadow_draw_list,
            DrawKind::GltfCoin,
            slot_i,
        );
    }
}
