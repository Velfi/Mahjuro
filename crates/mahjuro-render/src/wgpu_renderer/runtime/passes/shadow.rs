use super::*;
use crate::projected_light_shadow::ProjectedShadowLightSetup;
use crate::wgpu_renderer::runtime::shadow_setup::ActiveRoomEnv;

impl WgpuRenderer {
    /// Shadow pre-pass — one projected depth layer per point/spot light.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_shadow_pre_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UiFrame,
        camera_h: f32,
        shadow_quality: mahjuro_gfx_types::ShadowQuality,
        _shadow_uniforms_changed: bool,
        projected_lights: &[ProjectedShadowLightSetup],
        object3d_draw_list: &[(DrawKind, usize)],
        showcase_tile_batches: &[&[ShowcaseTilePlacement]],
        tile_3d_rects: &[(usize, [f32; 4])],
        tile_pick_models: &[(usize, glam::Mat4)],
    ) {
        if !shadow_quality.active() || projected_lights.is_empty() {
            return;
        }

        for (light_i, light) in projected_lights.iter().enumerate() {
            let depth_view = self
                .point_shadow_array
                .layer_views
                .get(light.layer_index as usize);
            let Some(depth_view) = depth_view else {
                continue;
            };
            let lvp = light.light_view_proj.to_cols_array();
            self.rewrite_shadow_casters_for_light(
                lvp,
                object3d_draw_list,
                tile_pick_models,
                showcase_tile_batches,
            );
            self.prepare_room_env_shadow_casters_for_light(frame, camera_h, lvp);

            let shadow_ts = if light_i == 0 {
                self.gpu_profiler
                    .pass_writes(crate::gpu_profiler::PassSlot::Shadow)
            } else {
                None
            };
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pre-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
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
            let _ = self.draw_shadow_casters(
                &mut shadow_pass,
                frame,
                object3d_draw_list,
                showcase_tile_batches,
                tile_3d_rects,
            );
        }
    }

    /// Upload room-GLB shadow caster uniforms before the depth pass (queue writes
    /// must not run inside an active render pass).
    fn prepare_room_env_shadow_casters_for_light(
        &self,
        frame: &UiFrame,
        camera_h: f32,
        lvp: [f32; 16],
    ) {
        let Some(active_room_env) = super::shadow_setup::active_room_env(frame) else {
            return;
        };
        let model = self.room_env_shadow_base_model(active_room_env, camera_h);
        let prim_deltas = match active_room_env {
            ActiveRoomEnv::Shop => self.shop_gltf_anim_prim_deltas(frame),
            ActiveRoomEnv::Gameplay => self.gameplay_env_prim_deltas(frame),
            _ => rustc_hash::FxHashMap::default(),
        };
        let mut _changed = false;
        let gpu = match active_room_env {
            ActiveRoomEnv::Shop => self.shop_environment.as_ref(),
            ActiveRoomEnv::Hallway => self.hallway_environment.as_ref(),
            ActiveRoomEnv::Stairway => self.staircase_environment.as_ref(),
            ActiveRoomEnv::Archive => self.archive_environment.as_ref(),
            ActiveRoomEnv::MainMenu => self.main_menu_environment.as_ref(),
            ActiveRoomEnv::Gameplay => self.gameplay_environment.as_ref(),
        };
        if let Some(gpu) = gpu {
            self.write_room_env_shadow_caster(gpu, lvp, model, &prim_deltas, &mut _changed);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_shadow_casters(
        &self,
        shadow_pass: &mut wgpu::RenderPass<'_>,
        frame: &UiFrame,
        object3d_draw_list: &[(DrawKind, usize)],
        _showcase_tile_batches: &[&[ShowcaseTilePlacement]],
        _tile_3d_rects: &[(usize, [f32; 4])],
    ) -> u32 {
        let mut room_draws = 0u32;
        if let Some(active_room_env) = super::shadow_setup::active_room_env(frame) {
            match active_room_env {
                ActiveRoomEnv::Shop => {
                    room_draws += self.draw_shop_environment_shadow(shadow_pass, frame);
                }
                ActiveRoomEnv::Hallway => {
                    if let Some(ref gpu) = self.hallway_environment {
                        room_draws += self.draw_gltf_room_env_shadow(
                            shadow_pass,
                            &self.hallway_env_primitives,
                            gpu,
                            |_| false,
                        );
                    }
                }
                ActiveRoomEnv::Stairway => {
                    if let Some(ref gpu) = self.staircase_environment {
                        room_draws += self.draw_gltf_room_env_shadow(
                            shadow_pass,
                            &self.staircase_env_primitives,
                            gpu,
                            |_| false,
                        );
                    }
                }
                ActiveRoomEnv::Archive => {
                    room_draws += self.draw_archive_environment_shadow(shadow_pass, frame);
                }
                ActiveRoomEnv::MainMenu => {
                    if !frame.main_menu_env_moon_only {
                        if let Some(ref gpu) = self.main_menu_environment {
                            room_draws += self.draw_gltf_room_env_shadow(
                                shadow_pass,
                                &self.main_menu_env_primitives,
                                gpu,
                                |pi| self.main_menu_env_skip_prim(pi, frame),
                            );
                        }
                    }
                }
                ActiveRoomEnv::Gameplay => {
                    if let Some(ref gpu) = self.gameplay_environment {
                        room_draws += self.draw_gltf_room_env_shadow(
                            shadow_pass,
                            &self.gameplay_env_primitives,
                            gpu,
                            |pi| {
                                if frame.gameplay_env_cash_in_only {
                                    !self.gameplay_cash_in_prim_indices.contains(&pi)
                                } else {
                                    !frame.gameplay_cash_in_button_visible
                                        && self.gameplay_cash_in_prim_indices.contains(&pi)
                                }
                            },
                        );
                    }
                }
            }
        }

        shadow_pass.set_pipeline(&self.shadow_pipeline);
        for &(kind, slot_i) in object3d_draw_list {
            self.draw_object3d_shadow_entry(shadow_pass, frame, kind, slot_i);
        }

        if !self.active_tile_mesh().primitives.is_empty()
            && self.active_tile_mesh().outline_index_count > 0
            && !self.tile_shadow_batch_ranges.is_empty()
        {
            shadow_pass.set_pipeline(&self.shadow_pipeline_instanced);
            shadow_pass.set_bind_group(0, &self.tile_shadow_frame_bind_group, &[]);
            shadow_pass.set_bind_group(1, &self.shadow_warp_disabled_bind_group, &[]);
            shadow_pass.set_vertex_buffer(
                0,
                self.active_tile_mesh().outline_vertex_buffer.slice(..),
            );
            shadow_pass.set_vertex_buffer(1, self.tile_shadow_instance_buffer.slice(..));
            shadow_pass.set_index_buffer(
                self.active_tile_mesh().outline_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );

            for &(batch_start, batch_count) in &self.tile_shadow_batch_ranges {
                if batch_count == 0 {
                    continue;
                }
                shadow_pass.draw_indexed(
                    0..self.active_tile_mesh().outline_index_count,
                    0,
                    batch_start..batch_start + batch_count,
                );
            }
        }

        if let Some((coin_shadow_start, coin_shadow_count)) = self.coin_shadow_batch_range
            && coin_shadow_count > 0
        {
            shadow_pass.set_pipeline(&self.shadow_pipeline_instanced);
            shadow_pass.set_bind_group(0, &self.tile_shadow_frame_bind_group, &[]);
            shadow_pass.set_bind_group(1, &self.shadow_warp_disabled_bind_group, &[]);
            shadow_pass.set_vertex_buffer(1, self.tile_shadow_instance_buffer.slice(..));
            for prim in &self.coin_glb_primitives {
                shadow_pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(
                    prim.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                shadow_pass.draw_indexed(
                    0..prim.index_count,
                    0,
                    coin_shadow_start..coin_shadow_start + coin_shadow_count,
                );
            }
        }
        room_draws
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
            DrawKind::BossIcon => {
                let mesh = match self.ordeal_icon_slot_texture.get(slot_i).copied().flatten() {
                    Some(bk) => self.ordeal_icon_mesh_for(bk),
                    None => &self.relic_box_mesh,
                };
                let Some(inst) = self.ordeal_icon_instances.get(slot_i) else {
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
                let Some(mesh) = self
                    .talisman_slot_kind
                    .get(slot_i)
                    .and_then(|k| k.and_then(|idx| self.talisman_mesh_for_kind_idx(idx)))
                else {
                    return;
                };
                self.draw_lit_mesh_shadow(pass, mesh, inst);
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
            DrawKind::ExtrudedGlyph => {
                let label: Option<&str> = frame
                    .cmds
                    .iter()
                    .flat_map(
                        |cmd| -> Box<dyn Iterator<Item = &crate::draw_cmd::Object3d>> {
                            match cmd {
                                DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                _ => Box::new(std::iter::empty()),
                            }
                        },
                    )
                    .filter_map(|o| match &o.kind {
                        crate::draw_cmd::Object3dKind::ExtrudedGlyph { label, .. } => {
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
            DrawKind::GltfCoin => {
                // Coin shadows are drawn in one instanced batch after showcase tiles.
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
