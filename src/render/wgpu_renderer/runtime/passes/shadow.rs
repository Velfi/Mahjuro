use super::*;
use crate::render::wgpu_renderer::runtime::shadow_setup::ActiveRoomEnv;

impl WgpuRenderer {
    /// Shadow pre-pass — render room GLB + lit-mesh casters (table excluded) into the
    /// shadow map from the light's POV. Skipped entirely when shadows are
    /// disabled — the lit shaders short-circuit on `params.x = 0` and the
    /// stale map contents go unread.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_shadow_pre_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UiFrame,
        shadows_enabled: bool,
        shadow_uniforms_changed: bool,
        object3d_draw_list: &[(DrawKind, usize)],
        showcase_tile_batches: &[&[ShowcaseTilePlacement]],
        tile_3d_rects: &[(usize, [f32; 4])],
    ) {
        if !(shadows_enabled && shadow_uniforms_changed) {
            return;
        }
        let shadow_ts = self
            .gpu_profiler
            .pass_writes(crate::render::gpu_profiler::PassSlot::Shadow);
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow-pre-pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_map_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: shadow_ts,
            multiview_mask: None,
        });
        shadow_pass.set_pipeline(&self.shadow_pipeline);

        let shop_inspect_shadow_only = frame.shop_inspect_shadow_target.is_some();

        // Imported room GLB — opaque primitives only (blend/glass skipped).
        let active_room_env = super::shadow_setup::active_room_env(frame);
        match active_room_env {
            Some(ActiveRoomEnv::Shop) if !shop_inspect_shadow_only => {
                if let Some(ref gpu) = self.shop_environment {
                    self.draw_gltf_room_env_shadow(
                        &mut shadow_pass,
                        &self.shop_env_primitives,
                        gpu,
                        |_| false,
                    );
                }
            }
            Some(ActiveRoomEnv::Shop) => {}
            Some(ActiveRoomEnv::Hallway) => {
                if let Some(ref gpu) = self.hallway_environment {
                    self.draw_gltf_room_env_shadow(
                        &mut shadow_pass,
                        &self.hallway_env_primitives,
                        gpu,
                        |_| false,
                    );
                }
            }
            Some(ActiveRoomEnv::Archive) => {
                if let Some(ref gpu) = self.archive_environment {
                    self.draw_gltf_room_env_shadow(
                        &mut shadow_pass,
                        &self.archive_env_primitives,
                        gpu,
                        |pi| self.archive_env_skip_description_prim(pi, frame),
                    );
                }
            }
            Some(ActiveRoomEnv::MainMenu) => {
                if let Some(ref gpu) = self.main_menu_environment {
                    self.draw_gltf_room_env_shadow(
                        &mut shadow_pass,
                        &self.main_menu_env_primitives,
                        gpu,
                        |_| false,
                    );
                }
            }
            None => {}
        }

        // Archive catalog props are mounted inside authored cubbies and in front of
        // readable sign boards. Let the room cast shadows, but don't project the
        // whole relic grid back onto the description sign.
        if !matches!(active_room_env, Some(ActiveRoomEnv::Archive)) {
            shadow_pass.set_pipeline(&self.shadow_pipeline);
            for &(kind, slot_i) in object3d_draw_list {
                if shop_inspect_shadow_only
                    && self.shop_inspect_subject_shadow_slot != Some((kind, slot_i))
                {
                    continue;
                }
                self.draw_object3d_shadow_entry(&mut shadow_pass, frame, kind, slot_i);
            }
        }

        // Hand tiles — one draw per (tile, primitive). Same multi-prim
        // walk the main pass uses, but only the position attribute is
        // read by the shadow shader so the bind group is the per-tile
        // shadow uniform, not the multi-prim main bind group.
        if !self.tile_primitives.is_empty() && self.tile_outline_index_count > 0 {
            shadow_pass.set_vertex_buffer(0, self.tile_outline_vertex_buffer.slice(..));
            shadow_pass.set_index_buffer(
                self.tile_outline_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for (i, _) in tile_3d_rects.iter() {
                let Some(htg) = self.hand_tiles.get(*i) else {
                    continue;
                };
                shadow_pass.set_bind_group(0, &htg.shadow_bind_group, &[]);
                shadow_pass.set_bind_group(1, &self.shadow_warp_disabled_bind_group, &[]);
                shadow_pass.draw_indexed(0..self.tile_outline_index_count, 0, 0..1);
            }

            // Showcase tiles — same mesh, separate GPU resource pool.
            let total_showcase: usize = showcase_tile_batches
                .iter()
                .map(|b| b.len())
                .sum::<usize>()
                .min(MAX_SHOWCASE_TILE_SLOTS);
            for slot_i in 0..total_showcase {
                let Some(stg) = self.showcase_tiles.get(slot_i) else {
                    break;
                };
                shadow_pass.set_bind_group(0, &stg.shadow_bind_group, &[]);
                shadow_pass.set_bind_group(1, &self.shadow_warp_disabled_bind_group, &[]);
                shadow_pass.draw_indexed(0..self.tile_outline_index_count, 0, 0..1);
            }
        }
    }

    fn draw_object3d_shadow_entry(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &UiFrame,
        kind: DrawKind,
        slot_i: usize,
    ) {
        match kind {
            DrawKind::YakuTablet => {
                let Some(inst) = self.yaku_tablet_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bone_tablet_mesh, inst);
            }
            DrawKind::WoodTablet => {
                let Some(inst) = self.wood_tablet_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.wood_tablet_mesh, inst);
            }
            DrawKind::Book => {
                let Some(inst) = self.book_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.book_mesh, inst);
            }
            DrawKind::BookCover => {
                let Some(inst) = self.book_cover_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.book_cover_mesh, inst);
            }
            DrawKind::Relic => {
                let mesh = match self.relic_slot_texture.get(slot_i).copied().flatten() {
                    Some(rid) => self.relic_mesh_for(rid),
                    None => &self.relic_box_mesh,
                };
                let Some(inst) = self.relic_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, mesh, inst);
            }
            DrawKind::Pack => {
                let Some(inst) = self.pack_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.pack_mesh, inst);
            }
            DrawKind::Ribbon => {
                let Some(inst) = self.ribbon_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.ribbon_mesh, inst);
            }
            DrawKind::Talisman => {
                let Some(inst) = self.talisman_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.talisman_mesh, inst);
            }
            DrawKind::BugBody => {
                let Some(inst) = self.bug_body_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bug_body_mesh, inst);
            }
            DrawKind::BugWingL => {
                let Some(inst) = self.bug_wing_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bug_wing_mesh, inst);
            }
            DrawKind::BugWingR => {
                let Some(inst) = self.bug_wing_r_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bug_wing_mesh, inst);
            }
            DrawKind::BugWingBlurL => {
                let Some(inst) = self.bug_wing_blur_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bug_wing_blur_mesh, inst);
            }
            DrawKind::BugWingBlurR => {
                let Some(inst) = self.bug_wing_blur_r_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bug_wing_blur_mesh, inst);
            }
            DrawKind::Orb => {
                let Some(inst) = self.orb_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.orb_mesh, inst);
            }
            DrawKind::DoraPlinth => {
                let Some(inst) = self.dora_plinth_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.dora_plinth_mesh, inst);
            }
            DrawKind::Bowl => {
                let Some(inst) = self.bowl_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bowl_mesh, inst);
            }
            DrawKind::Mirror => {
                let Some(inst) = self.mirror_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.mirror_mesh, inst);
            }
            DrawKind::TallyStickBase => {
                let Some(inst) = self.tally_stick_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.tally_stick_base_mesh, inst);
            }
            DrawKind::TallyStickTip => {
                let Some(inst) = self.tally_stick_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.tally_stick_tip_mesh, inst);
            }
            DrawKind::CascadeToken => {
                let Some(inst) = self.cascade_token_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.bone_tablet_mesh, inst);
            }
            DrawKind::CandleWax => {
                let Some(pair) = self.candle_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.candle_wax_mesh, &pair[0]);
            }
            DrawKind::CandleWick => {
                let Some(pair) = self.candle_instances.get(slot_i) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, &self.candle_wick_mesh, &pair[1]);
            }
            DrawKind::ExtrudedGlyph => {
                let label: Option<&str> = frame
                    .cmds
                    .iter()
                    .flat_map(
                        |cmd| -> Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> {
                            match cmd {
                                DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                _ => Box::new(std::iter::empty()),
                            }
                        },
                    )
                    .filter_map(|o| match &o.kind {
                        crate::render::draw_cmd::Object3dKind::ExtrudedGlyph { label, .. } => {
                            Some(label.as_ref())
                        }
                        _ => None,
                    })
                    .nth(slot_i);
                let (Some(lbl), Some(inst)) = (label, self.extruded_glyph_instances.get(slot_i))
                else {
                    return;
                };
                let Some(mesh) = self.extruded_glyph_meshes.get(lbl) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, mesh, inst);
            }
            DrawKind::Primitive(shape) => {
                let (Some(mesh), Some(inst)) = (
                    self.primitive_meshes.get(&shape).map(|a| a.as_ref()),
                    self.primitive_instances
                        .get(&shape)
                        .and_then(|pool| pool.get(slot_i)),
                ) else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, mesh, inst);
            }
        }
    }

    #[inline]
    fn draw_lit_mesh_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        mesh: &LitMeshGpu,
        inst: &LitMeshInstance,
    ) {
        pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
        pass.set_bind_group(1, &self.shadow_warp_disabled_bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
    }
}
