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
        camera: &CameraFrame,
        obj: &crate::render::draw_cmd::Object3d,
        shape: &crate::render::primitive::MeshId,
        material: &crate::render::primitive::MaterialSpec,
        pick_id: &Option<u32>,
        silhouette: &bool,
        obj3d_primitive_slot: &mut std::collections::HashMap<crate::render::primitive::MeshId, usize>,
        object3d_draw_list: &mut Vec<(DrawKind, usize)>,
    ) {
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen = |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };
                            use crate::render::primitive::{
                                MeshId, resolve_material, shape_orientation,
                            };
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
                                let (dw, dh) = crate::render::decal::decal_dimensions(
                                    &decal_spec.layout,
                                    obj.extents,
                                );
                                let label_hash = tablet_label_hash(&decal_spec.text, dw, dh);
                                let inst =
                                    &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
                                if inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash
                                    || inst.decal_size != (dw, dh)
                                {
                                    let rgba = crate::render::decal::rasterize_decal(
                                        decal_spec,
                                        dw,
                                        dh,
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
                                obj.rotation * orient,
                                glam::Vec3::from(obj.extents),
                            );
                            // Arrange-name compat shim: for BeveledSlab
                            // without an explicit arrange_name,
                            // synthesise the legacy plaque name so
                            // saved arrange_overrides.json still works.
                            let arrange_name: String = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else if *shape == MeshId::BeveledSlab {
                                match (self.active_scene_key, slot_i) {
                                    (Some("gameplay"), 0) => {
                                        "gameplay.score_panel.plaque".to_string()
                                    }
                                    (Some("gameplay"), 1) => {
                                        "gameplay.score_panel.scoring_placard".to_string()
                                    }
                                    (Some("shop"), i) => format!("shop.plaque[{i}]"),
                                    (_, i) => format!("plaque[{i}]"),
                                }
                            } else {
                                format!("primitive.{:?}[{}]", shape, slot_i)
                            };
                            let model = self.apply_arrange_override(&arrange_name, model);
                            if let Some(pid) = pick_id {
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            let params = resolve_material(material, obj.color, *silhouette);
                            let tint = if *silhouette {
                                [0.04, 0.04, 0.05, obj.color[3]]
                            } else {
                                obj.color
                            };
                            let inst =
                                &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
                            if *silhouette {
                                inst.write_uniform_tinted(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    params,
                                    tint,
                                );
                            } else {
                                inst.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    params,
                                    has_decal,
                                );
                            }
                            self.last_debug_pickables.push((
                                arrange_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
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
                            // ShopActionProp reuses `aux_dish_rects` as
                            // the shop's focus-nav/click channel too —
                            // its `ShopHit::Dish(pid)` mapping is
                            // historical from when the props piggy-backed
                            // on the dish rect list.
                            if matches!(
                                *shape,
                                MeshId::DiscSquare | MeshId::DiscRound | MeshId::ShopActionProp
                            ) {
                                self.proj
                                    .aux_dish_rects
                                    .push((*pick_id, [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
                                let center =
                                    pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                                let half = glam::Vec3::new(
                                    obj.extents[0] * 0.5,
                                    obj.extents[1] * 0.5,
                                    obj.extents[2] * 0.5,
                                );
                                self.last_aux_dish_aabbs.push((center, half));
                            }
                            object3d_draw_list.push((DrawKind::Primitive(*shape), slot_i));
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
                                object3d_draw_list
                                    .push((DrawKind::Primitive(MeshId::CabinetRails), rails_slot));
                            }
    }
}
