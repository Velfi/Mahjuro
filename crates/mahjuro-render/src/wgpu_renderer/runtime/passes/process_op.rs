use super::*;

/// Per-frame borrowed state needed by `WgpuRenderer::process_op`.
/// Built fresh inside `render()` and passed to each pass that dispatches ops.
pub(super) struct ProcessOpCtx<'a> {
    pub frame: &'a UiFrame,
    /// Backing buffer for the per-frame bump pool (see
    /// `crate::wgpu_renderer::frame_pool`). The four slice
    /// arrays below index into this single buffer via `(offset, byte_len)`.
    pub frame_pool_buffer: &'a wgpu::Buffer,
    pub bg_inst_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub depth_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub overlay_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub overlay_squircle_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub gradient_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub arc_ring_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub squircle_quad_buffers: &'a [crate::wgpu_renderer::frame_pool::PoolSlice],
    pub flame_buffers: &'a [wgpu::Buffer],
    pub text_draws: &'a [TextDraw],
    pub tile_face_inst_buffers: &'a [wgpu::Buffer],
    pub tile_face_quads: &'a [TileFaceQuad],
    pub image_quad_inst_buffers: &'a [wgpu::Buffer],
    pub image_quads: &'a [crate::draw_cmd::ImageQuad],
    pub object3d_draw_list: &'a [(DrawKind, usize)],
    pub showcase_tile_batches: &'a [&'a [ShowcaseTilePlacement]],
    pub showcase_tile_batch_clips: &'a [Option<[f32; 4]>],
    pub tile_glows: &'a [GpuInstance],
    pub tile_glow_buffer: Option<&'a wgpu::Buffer>,
    pub relic_glows: &'a [GpuInstance],
    pub relic_glow_buffer: Option<&'a wgpu::Buffer>,
    pub glyph_popup_glows: &'a [GpuInstance],
    pub glyph_popup_glow_buffer: Option<&'a wgpu::Buffer>,
    pub relic_debuff_markers: &'a [GpuInstance],
    pub relic_debuff_buffer: Option<&'a wgpu::Buffer>,
    /// True when the active render pass color attachment is linear HDR
    /// (`Rgba16Float`): Pass A (`scene_color_view`), journal scene, or HDR
    /// swapchain text overlay. Uses `globals_scene_hdr_bind_group` so 2D shaders
    /// skip in-shader gamma; `tonemap_composite` applies the user slider once.
    pub scene_hdr_attachment: bool,
    /// Color attachment width for this pass (`render_size` in Pass A, window `size` in overlay).
    pub pass_target_w: u32,
    /// Color attachment height for this pass.
    pub pass_target_h: u32,
}

impl WgpuRenderer {
    #[inline]
    fn globals_bind_group_for(&self, scene_hdr_attachment: bool) -> &wgpu::BindGroup {
        if scene_hdr_attachment {
            &self.globals_scene_hdr_bind_group
        } else {
            &self.globals_bind_group
        }
    }

    #[inline]
    fn moonlit_water_bind_group_for(&self, scene_hdr_attachment: bool) -> &wgpu::BindGroup {
        if scene_hdr_attachment {
            &self.moonlit_water_scene_hdr_bind_group
        } else {
            &self.moonlit_water_bind_group
        }
    }

    /// Instanced draw for `tile_3d.wgsl` meshes (showcase tiles and glTF coins).
    fn draw_tile_gltf_instanced<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        primitives: &'a [TilePrimitiveGpu],
        instance_start: u32,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_vertex_buffer(1, self.tile_3d_instance_buffer.slice(..));
        for blend_phase in [false, true] {
            let mut textured_last_pi: Option<usize> = None;
            let mut textured_last_key: Option<TileGlbPipelineKey> = None;
            for (pi, prim) in primitives.iter().enumerate() {
                if prim.pipeline_key.is_blend() != blend_phase {
                    continue;
                }
                let Some(bg) = prim.material_bind_group.as_ref() else {
                    continue;
                };
                if textured_last_key != Some(prim.pipeline_key) {
                    pass.set_pipeline(self.tile_glb_pipeline(prim.pipeline_key));
                    textured_last_key = Some(prim.pipeline_key);
                }
                if textured_last_pi != Some(pi) {
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    textured_last_pi = Some(pi);
                }
                pass.set_bind_group(0, bg, &[]);
                pass.draw_indexed(
                    0..prim.index_count,
                    0,
                    instance_start..instance_start + instance_count,
                );
            }
        }
    }

    /// Translucent showcase tiles (per-instance opacity) via the alpha-blended pipeline.
    fn draw_tile_gltf_instanced_translucent<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        primitives: &'a [TilePrimitiveGpu],
        instance_start: u32,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.tile_pipeline_ghost_cull);
        pass.set_vertex_buffer(1, self.tile_3d_instance_buffer.slice(..));
        for (pi, prim) in primitives.iter().enumerate() {
            let Some(bg) = prim.material_bind_group.as_ref() else {
                continue;
            };
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_bind_group(0, bg, &[]);
            pass.draw_indexed(
                0..prim.index_count,
                0,
                instance_start..instance_start + instance_count,
            );
            let _ = pi;
        }
    }

    /// Dispatch a single render op into a render pass. Used by every
    /// scene-rendering pass (Pass A) so they all
    /// share the same op-dispatch table.
    pub(super) fn process_op<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        op: &RenderOp,
        ctx: &ProcessOpCtx<'a>,
    ) {
        let frame = ctx.frame;
        let bg_inst_buffers = ctx.bg_inst_buffers;
        let quad_buffers = ctx.quad_buffers;
        let depth_quad_buffers = ctx.depth_quad_buffers;
        let overlay_quad_buffers = ctx.overlay_quad_buffers;
        let gradient_quad_buffers = ctx.gradient_quad_buffers;
        let squircle_quad_buffers = ctx.squircle_quad_buffers;
        let flame_buffers = ctx.flame_buffers;
        let text_draws = ctx.text_draws;
        let tile_face_inst_buffers = ctx.tile_face_inst_buffers;
        let tile_face_quads = ctx.tile_face_quads;
        let image_quad_inst_buffers = ctx.image_quad_inst_buffers;
        let image_quads = ctx.image_quads;
        let object3d_draw_list = ctx.object3d_draw_list;
        let showcase_tile_batches = ctx.showcase_tile_batches;
        let showcase_tile_batch_clips = ctx.showcase_tile_batch_clips;
        let tile_glows = ctx.tile_glows;
        let tile_glow_buffer = ctx.tile_glow_buffer;
        let relic_glows = ctx.relic_glows;
        let glyph_popup_glows = ctx.glyph_popup_glows;
        let glyph_popup_glow_buffer = ctx.glyph_popup_glow_buffer;
        let relic_glow_buffer = ctx.relic_glow_buffer;
        let relic_debuff_markers = ctx.relic_debuff_markers;
        let relic_debuff_buffer = ctx.relic_debuff_buffer;
        let scene_hdr_attachment = ctx.scene_hdr_attachment;
        // UI layout uses window `size`; Pass A targets `render_size`, overlay pass the swapchain.
        let rs_w = ctx.pass_target_w.max(1);
        let rs_h = ctx.pass_target_h.max(1);
        let full_scissor = [0, 0, rs_w, rs_h];
        let to_scissor = |rect: [f32; 4]| -> Option<[u32; 4]> {
            let [x, y, w, h] = rect;
            if !(w > 0.0 && h > 0.0) {
                return None;
            }
            let sx = rs_w as f32 / self.size.width.max(1) as f32;
            let sy = rs_h as f32 / self.size.height.max(1) as f32;
            let x = x * sx;
            let y = y * sy;
            let w = w * sx;
            let h = h * sy;
            let max_w = rs_w as f32;
            let max_h = rs_h as f32;
            let x0 = x.max(0.0).min(max_w);
            let y0 = y.max(0.0).min(max_h);
            let x1 = (x + w).max(0.0).min(max_w);
            let y1 = (y + h).max(0.0).min(max_h);
            if !(x1 > x0 && y1 > y0) {
                return None;
            }
            Some([
                x0.floor() as u32,
                y0.floor() as u32,
                (x1.ceil() - x0.floor()).max(1.0) as u32,
                (y1.ceil() - y0.floor()).max(1.0) as u32,
            ])
        };
        match op {
            RenderOp::ClearSceneDepth => {
                // Marker only: Pass A is split into subpasses at this op; never drawn here.
            }
            RenderOp::Background { id, buf_idx } => {
                if let (Some(bg_tex), Some(slice)) = (
                    self.background_textures.get(id),
                    bg_inst_buffers.get(*buf_idx),
                ) {
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                    pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(
                        1,
                        ctx.frame_pool_buffer
                            .slice(slice.offset..slice.offset + slice.byte_len),
                    );
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
            RenderOp::Starfield => {
                pass.set_pipeline(&self.starfield_pipeline);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::GoldenDust => {
                pass.set_pipeline(&self.golden_dust_pipeline);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::MoonlitWater => {
                pass.set_pipeline(&self.moonlit_water_pipeline);
                pass.set_bind_group(
                    0,
                    self.moonlit_water_bind_group_for(scene_hdr_attachment),
                    &[],
                );
                pass.draw(0..3, 0..1);
            }
            RenderOp::SunlitWater => {
                pass.set_pipeline(&self.sunlit_water_pipeline);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::ShootingStarCascade => {
                // Cascade was rendered into the half-res offscreen target
                // in the pre-pass above; here we just sample+additively
                // composite it onto the main scene target.
                pass.set_pipeline(&self.cascade_composite_pipeline);
                pass.set_bind_group(0, &self.cascade_composite_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::Object3dBatch { start, end } => {
                pass.set_pipeline(&self.lit_mesh_pipeline);
                pass.set_bind_group(3, &self.lit_mesh_spot_frame_bind_group, &[]);
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
                let mut current_blended = false;
                for &(kind, slot_i) in &object3d_draw_list[*start..*end] {
                    let want_blended = matches!(
                        kind,
                        DrawKind::BugWingL
                            | DrawKind::BugWingR
                            | DrawKind::BugWingBlurL
                            | DrawKind::BugWingBlurR
                    );
                    if want_blended != current_blended {
                        if want_blended {
                            pass.set_pipeline(&self.lit_mesh_blended_pipeline);
                        } else {
                            pass.set_pipeline(&self.lit_mesh_pipeline);
                        }
                        current_blended = want_blended;
                    }
                    // Relic mesh is looked up per relic_id stored in relic_slot_texture.
                    if matches!(kind, DrawKind::Relic) {
                        let mesh = match self.relic_slot_texture.get(slot_i).copied().flatten() {
                            Some(rid) => self.relic_mesh_for(rid),
                            None => &self.relic_box_mesh,
                        };
                        if let Some(inst) = self.relic_instances.get(slot_i) {
                            pass.set_bind_group(0, &inst.bind_group, &[]);
                            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        }
                        continue;
                    }
                    if matches!(kind, DrawKind::BossIcon) {
                        let mesh =
                            match self.ordeal_icon_slot_texture.get(slot_i).copied().flatten() {
                                Some(bk) => self.ordeal_icon_mesh_for(bk),
                                None => &self.relic_box_mesh,
                            };
                        if let Some(inst) = self.ordeal_icon_instances.get(slot_i) {
                            pass.set_bind_group(0, &inst.bind_group, &[]);
                            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        }
                        continue;
                    }
                    // Extruded glyph mesh is per-label. Look it up from the scan
                    // of the cmd list; slot_i maps to the Nth ExtrudedGlyph in
                    // draw order.
                    if matches!(kind, DrawKind::ExtrudedGlyph) {
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
                        if let (Some(lbl), Some(inst)) =
                            (label, self.extruded_glyph_instances.get(slot_i))
                            && let Some(mesh) = self.extruded_glyph_meshes.get(lbl)
                        {
                            if let Some(gpb) = glyph_popup_glow_buffer {
                                if slot_i < glyph_popup_glows.len()
                                    && glyph_popup_glows[slot_i].color[3] > 0.001
                                {
                                    pass.set_pipeline(&self.tile_glow_pipeline);
                                    pass.set_bind_group(
                                        0,
                                        self.globals_bind_group_for(scene_hdr_attachment),
                                        &[],
                                    );
                                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                                    pass.set_vertex_buffer(1, gpb.slice(..));
                                    pass.set_index_buffer(
                                        self.index_buffer.slice(..),
                                        wgpu::IndexFormat::Uint16,
                                    );
                                    pass.draw_indexed(0..6, 0, slot_i as u32..slot_i as u32 + 1);
                                }
                            }
                            pass.set_pipeline(&self.lit_mesh_pipeline);
                            pass.set_bind_group(0, &inst.bind_group, &[]);
                            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        }
                        continue;
                    }
                    if matches!(kind, DrawKind::GltfCoin) {
                        let Some((coin_base, coin_count)) = self.coin_3d_batch_range else {
                            continue;
                        };
                        if slot_i as u32 >= coin_count {
                            continue;
                        }
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
                        pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                        let inst_start = coin_base + slot_i as u32;
                        self.draw_tile_gltf_instanced(
                            pass,
                            &self.coin_glb_primitives,
                            inst_start,
                            1,
                        );
                        pass.set_bind_group(3, &self.lit_mesh_spot_frame_bind_group, &[]);
                        continue;
                    }
                    if matches!(kind, DrawKind::TallyStickPlay | DrawKind::TallyStickDiscard) {
                        let Some((tally_base, tally_count)) = self.tally_stick_3d_batch_range
                        else {
                            continue;
                        };
                        if slot_i as u32 >= tally_count {
                            continue;
                        }
                        let prims = match kind {
                            DrawKind::TallyStickPlay => &self.tally_stick_play_primitives,
                            DrawKind::TallyStickDiscard => &self.tally_stick_discard_primitives,
                            _ => unreachable!(),
                        };
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
                        pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                        self.draw_tile_gltf_instanced(pass, prims, tally_base + slot_i as u32, 1);
                        pass.set_bind_group(3, &self.lit_mesh_spot_frame_bind_group, &[]);
                        continue;
                    }
                    if matches!(kind, DrawKind::Talisman) {
                        let Some(mesh) = self
                            .talisman_slot_kind
                            .get(slot_i)
                            .and_then(|k| k.and_then(|idx| self.talisman_mesh_for_kind_idx(idx)))
                        else {
                            continue;
                        };
                        let Some(inst) = self.talisman_instances.get(slot_i) else {
                            continue;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        continue;
                    }
                    let (mesh, inst_opt): (&LitMeshGpu, Option<&LitMeshInstance>) = match kind {
                        DrawKind::YakuTablet => (
                            &self.bone_tablet_mesh,
                            self.yaku_tablet_instances.get(slot_i),
                        ),
                        DrawKind::WoodTablet => (
                            &self.wood_tablet_mesh,
                            self.wood_tablet_instances.get(slot_i),
                        ),
                        DrawKind::Book => (&self.book_mesh, self.book_instances.get(slot_i)),
                        DrawKind::BookCover => {
                            (&self.book_cover_mesh, self.book_cover_instances.get(slot_i))
                        }
                        DrawKind::Pack => (&self.pack_mesh, self.pack_instances.get(slot_i)),
                        DrawKind::Ribbon => (&self.ribbon_mesh, self.ribbon_instances.get(slot_i)),
                        DrawKind::Talisman => unreachable!("handled above"),
                        DrawKind::BugBody => {
                            (&self.bug_body_mesh, self.bug_body_instances.get(slot_i))
                        }
                        DrawKind::BugWingL => {
                            (&self.bug_wing_mesh, self.bug_wing_instances.get(slot_i))
                        }
                        DrawKind::BugWingBlurL => (
                            &self.bug_wing_blur_mesh,
                            self.bug_wing_blur_instances.get(slot_i),
                        ),
                        DrawKind::BugWingR => {
                            (&self.bug_wing_mesh, self.bug_wing_r_instances.get(slot_i))
                        }
                        DrawKind::BugWingBlurR => (
                            &self.bug_wing_blur_mesh,
                            self.bug_wing_blur_r_instances.get(slot_i),
                        ),
                        DrawKind::Orb => (&self.orb_mesh, self.orb_instances.get(slot_i)),
                        DrawKind::Bowl => (&self.bowl_mesh, self.bowl_instances.get(slot_i)),
                        DrawKind::Mirror => (&self.mirror_mesh, self.mirror_instances.get(slot_i)),
                        DrawKind::Primitive(mid) => {
                            let mesh = self
                                .primitive_meshes
                                .get(&mid)
                                .map(|a| a.as_ref())
                                .expect("primitive mesh missing from registry");
                            let inst = self
                                .primitive_instances
                                .get(&mid)
                                .and_then(|pool| pool.get(slot_i));
                            (mesh, inst)
                        }
                        // Handled by the early-out blocks above.
                        DrawKind::Relic
                        | DrawKind::ExtrudedGlyph
                        | DrawKind::BossIcon
                        | DrawKind::GltfCoin
                        | DrawKind::TallyStickPlay
                        | DrawKind::TallyStickDiscard => {
                            unreachable!()
                        }
                    };
                    let Some(inst) = inst_opt else { continue };
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
                // Relic activation halos — additive bloom rects drawn
                // after the relic meshes so the falloff spills past the
                // silhouette. Fires whenever any relic in the scene this
                // frame had glow > 0 (Object3dKind::Relic accumulates).
                if let Some(rgb) = relic_glow_buffer {
                    pass.set_pipeline(&self.tile_glow_pipeline);
                    pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, rgb.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..relic_glows.len() as u32);
                }
                if let (Some(buf), Some(gpu)) =
                    (relic_debuff_buffer, self.debuff_marker_overlay.as_ref())
                    && !relic_debuff_markers.is_empty()
                {
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                    pass.set_bind_group(1, &gpu.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..relic_debuff_markers.len() as u32);
                }
            }
            RenderOp::ShopEnvironment => {
                if self.shop_environment.is_some() {
                    self.draw_shop_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::HallwayEnvironment => {
                if self.hallway_environment.is_some() {
                    self.draw_hallway_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::StaircaseEnvironment => {
                if self.staircase_environment.is_some() {
                    self.draw_staircase_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::ArchiveEnvironment => {
                if self.archive_environment.is_some() {
                    self.draw_archive_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::MainMenuEnvironment => {
                if self.main_menu_environment.is_some() {
                    self.draw_main_menu_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::GameplayEnvironment => {
                self.draw_gameplay_environment_for_op(pass, frame);
            }
            RenderOp::ShadowTestEnvironment => {
                if self.shadow_test_room_environment.is_some() {
                    self.draw_shadow_test_room_environment_meshes(pass, frame, false);
                }
            }
            RenderOp::ShowcaseTileBatch(batch_idx) => {
                if let Some(sc) = showcase_tile_batch_clips
                    .get(*batch_idx)
                    .copied()
                    .flatten()
                    .and_then(to_scissor)
                {
                    pass.set_scissor_rect(sc[0], sc[1], sc[2], sc[3]);
                }
                let batch = showcase_tile_batches[*batch_idx];
                if !batch.is_empty() {
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
                    pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                    let Some(&(batch_start, batch_count)) =
                        self.tile_3d_batch_ranges.get(*batch_idx)
                    else {
                        pass.set_scissor_rect(
                            full_scissor[0],
                            full_scissor[1],
                            full_scissor[2],
                            full_scissor[3],
                        );
                        return;
                    };

                    // Glow halos for selected hand tiles (additive, drawn before mesh).
                    let has_glow = batch.iter().any(|p| p.glow);
                    if has_glow && let Some(tgb) = tile_glow_buffer {
                        pass.set_pipeline(&self.tile_glow_pipeline);
                        pass.set_bind_group(
                            0,
                            self.globals_bind_group_for(scene_hdr_attachment),
                            &[],
                        );
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, tgb.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..tile_glows.len() as u32);
                    }

                    // Pass A: gold outline shells — one instanced draw per batch.
                    if let Some(&(base, cnt)) = self.tile_outline_batch_ranges.get(*batch_idx)
                        && cnt > 0
                        && self.active_tile_mesh().outline_index_count > 0
                    {
                        pass.set_pipeline(&self.tile_outline_pipeline);
                        pass.set_bind_group(0, &self.tile_outline_frame_bind_group, &[]);
                        pass.set_vertex_buffer(
                            0,
                            self.active_tile_mesh().outline_vertex_buffer.slice(..),
                        );
                        pass.set_vertex_buffer(1, self.tile_outline_instance_buffer.slice(..));
                        pass.set_index_buffer(
                            self.active_tile_mesh().outline_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(
                            0..self.active_tile_mesh().outline_index_count,
                            0,
                            base..base + cnt,
                        );
                    }

                    // Pass B: regular textured tile meshes (opaque instances only;
                    // translucent staging previews are drawn in ShowcaseTileTranslucent).
                    self.draw_tile_gltf_instanced(
                        pass,
                        &self.active_tile_mesh().primitives,
                        batch_start,
                        batch_count,
                    );
                }
                pass.set_scissor_rect(
                    full_scissor[0],
                    full_scissor[1],
                    full_scissor[2],
                    full_scissor[3],
                );
            }
            RenderOp::ShowcaseTileTranslucent => {
                let any_blend = self
                    .tile_3d_batch_blend_ranges
                    .iter()
                    .any(|(_, count)| *count > 0);
                if !any_blend {
                    return;
                }
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
                pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                for (batch_idx, &(blend_start, blend_count)) in
                    self.tile_3d_batch_blend_ranges.iter().enumerate()
                {
                    if blend_count == 0 {
                        continue;
                    }
                    if let Some(sc) = showcase_tile_batch_clips
                        .get(batch_idx)
                        .copied()
                        .flatten()
                        .and_then(to_scissor)
                    {
                        pass.set_scissor_rect(sc[0], sc[1], sc[2], sc[3]);
                    }
                    self.draw_tile_gltf_instanced_translucent(
                        pass,
                        &self.active_tile_mesh().primitives,
                        blend_start,
                        blend_count,
                    );
                }
                pass.set_scissor_rect(
                    full_scissor[0],
                    full_scissor[1],
                    full_scissor[2],
                    full_scissor[3],
                );
            }
            RenderOp::QuadBatch { buf_idx, count } => {
                let slice = quad_buffers[*buf_idx];
                let pipe = if ctx.scene_hdr_attachment {
                    &self.quad_pipeline
                } else {
                    &self.quad_pipeline_display
                };
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::DepthQuadBatch { buf_idx, count } => {
                let slice = depth_quad_buffers[*buf_idx];
                let show_rain_depth = ctx.frame.debug_rain_depth;
                let pipe = if ctx.scene_hdr_attachment {
                    if show_rain_depth {
                        &self.depth_quad_debug_pipeline
                    } else {
                        &self.depth_quad_pipeline
                    }
                } else {
                    if show_rain_depth {
                        &self.depth_quad_debug_pipeline_display
                    } else {
                        &self.depth_quad_pipeline_display
                    }
                };
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::OverlayQuadBatch { buf_idx, count } => {
                let slice = overlay_quad_buffers[*buf_idx];
                pass.set_pipeline(&self.quad_pipeline_display);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::OverlaySquircleQuadBatch { buf_idx, count } => {
                let slice = ctx.overlay_squircle_quad_buffers[*buf_idx];
                pass.set_pipeline(&self.squircle_quad_pipeline_display);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::GradientQuadBatch { buf_idx, count } => {
                let slice = gradient_quad_buffers[*buf_idx];
                pass.set_pipeline(&self.gradient_quad_pipeline);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::ArcRingQuadBatch { buf_idx, count } => {
                let slice = ctx.arc_ring_quad_buffers[*buf_idx];
                let pipe = if ctx.scene_hdr_attachment {
                    &self.arc_ring_quad_pipeline
                } else {
                    &self.arc_ring_quad_pipeline_display
                };
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::SquircleQuadBatch { buf_idx, count } => {
                let slice = squircle_quad_buffers[*buf_idx];
                let pipe = if ctx.scene_hdr_attachment {
                    &self.squircle_quad_pipeline
                } else {
                    &self.squircle_quad_pipeline_display
                };
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(
                    1,
                    ctx.frame_pool_buffer
                        .slice(slice.offset..slice.offset + slice.byte_len),
                );
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::FlameBatch { buf_idx, count } => {
                if *count > 0 && *buf_idx != usize::MAX {
                    let mesh = &self.flame_volume_mesh;
                    let layers: [&wgpu::RenderPipeline; 3] = [
                        &self.flame_glow_pipeline,
                        &self.flame_pipeline,
                        &self.flame_core_pipeline,
                    ];
                    pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                    pass.set_bind_group(1, &self.flame_view_bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, flame_buffers[*buf_idx].slice(..));
                    pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    for pipeline in layers {
                        pass.set_pipeline(pipeline);
                        pass.draw_indexed(0..mesh.index_count, 0, 0..*count);
                    }
                }
            }
            RenderOp::TextDraw(idx) => {
                let td = &text_draws[*idx];
                if let Some(sc) = td.scissor_rect.and_then(to_scissor) {
                    pass.set_scissor_rect(sc[0], sc[1], sc[2], sc[3]);
                } else {
                    pass.set_scissor_rect(
                        full_scissor[0],
                        full_scissor[1],
                        full_scissor[2],
                        full_scissor[3],
                    );
                }
                pass.set_pipeline(if scene_hdr_attachment {
                    &self.text_pipeline_scene_hdr
                } else {
                    &self.text_pipeline
                });
                pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
                pass.set_bind_group(1, &td.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..1);
                pass.set_scissor_rect(
                    full_scissor[0],
                    full_scissor[1],
                    full_scissor[2],
                    full_scissor[3],
                );
            }
            RenderOp::TileFaceQuad(idx) => {
                let face = &tile_face_quads[*idx];
                let key = (
                    face.tile.suit,
                    face.tile.rank,
                    face.tile.enhancement,
                    face.tile.debuffed_visual,
                );
                if let Some(gpu) = self.tile_face_overlays.get(&key) {
                    self.draw_image_textured_quad(
                        pass,
                        scene_hdr_attachment,
                        gpu,
                        &tile_face_inst_buffers[*idx],
                    );
                }
            }
            RenderOp::ImageQuad(idx) => {
                let quad = &image_quads[*idx];
                if let Some(sc) = quad.clip_rect.and_then(to_scissor) {
                    pass.set_scissor_rect(sc[0], sc[1], sc[2], sc[3]);
                } else {
                    pass.set_scissor_rect(
                        full_scissor[0],
                        full_scissor[1],
                        full_scissor[2],
                        full_scissor[3],
                    );
                }
                if let Some(gpu) = self.image_quad_overlays.get(&quad.source.cache_key()) {
                    self.draw_image_textured_quad(
                        pass,
                        scene_hdr_attachment,
                        gpu,
                        &image_quad_inst_buffers[*idx],
                    );
                }
                pass.set_scissor_rect(
                    full_scissor[0],
                    full_scissor[1],
                    full_scissor[2],
                    full_scissor[3],
                );
            }
        }
    }

    fn draw_image_textured_quad(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scene_hdr_attachment: bool,
        gpu: &TileFaceOverlayGpu,
        inst_buffer: &wgpu::Buffer,
    ) {
        pass.set_pipeline(if scene_hdr_attachment {
            &self.image_pipeline_scene_hdr
        } else {
            &self.image_pipeline
        });
        pass.set_bind_group(0, self.globals_bind_group_for(scene_hdr_attachment), &[]);
        pass.set_bind_group(1, &gpu.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, inst_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..1);
    }
}
