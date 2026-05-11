use super::*;

/// Per-frame borrowed state needed by `WgpuRenderer::process_op`.
/// Built fresh inside `render()` and passed to each pass that dispatches ops.
pub(super) struct ProcessOpCtx<'a> {
    pub frame: &'a UiFrame,
    pub bg_inst_buffers: &'a [wgpu::Buffer],
    pub quad_buffers: &'a [wgpu::Buffer],
    pub gradient_quad_buffers: &'a [wgpu::Buffer],
    pub squircle_quad_buffers: &'a [wgpu::Buffer],
    pub flame_buffers: &'a [wgpu::Buffer],
    pub text_draws: &'a [TextDraw],
    pub tile_face_inst_buffers: &'a [wgpu::Buffer],
    pub tile_face_quads: &'a [TileFaceQuad],
    pub prompt_icon_inst_buffers: &'a [wgpu::Buffer],
    pub prompt_icon_quads: &'a [crate::render::draw_cmd::PromptIconQuad],
    pub object3d_draw_list: &'a [(DrawKind, usize)],
    pub showcase_tile_batches: &'a [&'a [ShowcaseTilePlacement]],
    pub tile_glows: &'a [GpuInstance],
    pub tile_glow_buffer: Option<&'a wgpu::Buffer>,
    pub relic_glows: &'a [GpuInstance],
    pub relic_glow_buffer: Option<&'a wgpu::Buffer>,
    pub relic_debuff_markers: &'a [GpuInstance],
    pub relic_debuff_buffer: Option<&'a wgpu::Buffer>,
    /// True when the active render pass color attachment is linear HDR
    /// (`Rgba16Float`): Pass A (`scene_color_view`), journal scene, or HDR
    /// swapchain text overlay.
    pub scene_hdr_attachment: bool,
}

impl WgpuRenderer {
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
        let gradient_quad_buffers = ctx.gradient_quad_buffers;
        let squircle_quad_buffers = ctx.squircle_quad_buffers;
        let flame_buffers = ctx.flame_buffers;
        let text_draws = ctx.text_draws;
        let tile_face_inst_buffers = ctx.tile_face_inst_buffers;
        let tile_face_quads = ctx.tile_face_quads;
        let prompt_icon_inst_buffers = ctx.prompt_icon_inst_buffers;
        let prompt_icon_quads = ctx.prompt_icon_quads;
        let object3d_draw_list = ctx.object3d_draw_list;
        let showcase_tile_batches = ctx.showcase_tile_batches;
        let tile_glows = ctx.tile_glows;
        let tile_glow_buffer = ctx.tile_glow_buffer;
        let relic_glows = ctx.relic_glows;
        let relic_glow_buffer = ctx.relic_glow_buffer;
        let relic_debuff_markers = ctx.relic_debuff_markers;
        let relic_debuff_buffer = ctx.relic_debuff_buffer;
        let scene_hdr_attachment = ctx.scene_hdr_attachment;
        match op {
            RenderOp::ClearSceneDepth => {
                // Marker only: Pass A is split into subpasses at this op; never drawn here.
            }
            RenderOp::ShopInspectLitMeshSubjectHdr => {
                // Marker only: split for `SsrGlobals` upload between subpasses.
            }
            RenderOp::Background { id, buf_idx } => {
                if let (Some(bg_tex), Some(inst_buf)) = (
                    self.background_textures.get(id),
                    bg_inst_buffers.get(*buf_idx),
                ) {
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, inst_buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
            RenderOp::Starfield => {
                pass.set_pipeline(&self.starfield_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::EmberDrift => {
                pass.set_pipeline(&self.ember_drift_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::GoldenDust => {
                pass.set_pipeline(&self.golden_dust_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::MoonlitWater => {
                pass.set_pipeline(&self.moonlit_water_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &self.moon_albedo_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::MountainHaze => {
                pass.set_pipeline(&self.mountain_haze_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &self.haze_uniform_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            RenderOp::SunlitWater => {
                pass.set_pipeline(&self.sunlit_water_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
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
            RenderOp::Table => {
                pass.set_pipeline(&self.lit_mesh_pipeline);
                pass.set_bind_group(3, &self.lit_mesh_spot_ssr_bind_group, &[]);
                pass.set_bind_group(0, &self.table_instance.bind_group, &[]);
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                pass.set_vertex_buffer(0, self.table_mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.table_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..self.table_mesh.index_count, 0, 0..1);
            }
            RenderOp::Object3dBatch { start, end } => {
                pass.set_pipeline(&self.lit_mesh_pipeline);
                pass.set_bind_group(3, &self.lit_mesh_spot_ssr_bind_group, &[]);
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
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
                    // Extruded glyph mesh is per-label. Look it up from the scan
                    // of the cmd list; slot_i maps to the Nth ExtrudedGlyph in
                    // draw order.
                    if matches!(kind, DrawKind::ExtrudedGlyph) {
                        let label: Option<&str> = frame
                            .cmds
                            .iter()
                            .flat_map(
                                |cmd| -> Box<
                                    dyn Iterator<Item = &crate::render::draw_cmd::Object3d>,
                                > {
                                    match cmd {
                                        DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                        DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                        _ => Box::new(std::iter::empty()),
                                    }
                                },
                            )
                            .filter_map(|o| match &o.kind {
                                crate::render::draw_cmd::Object3dKind::ExtrudedGlyph {
                                    label,
                                    ..
                                } => Some(label.as_ref()),
                                _ => None,
                            })
                            .nth(slot_i);
                        if let (Some(lbl), Some(inst)) =
                            (label, self.extruded_glyph_instances.get(slot_i))
                            && let Some(mesh) = self.extruded_glyph_meshes.get(lbl)
                        {
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
                    // Candle uses a [LitMeshInstance; 2] pair indexed by slot_i;
                    // wax = pair[0], wick = pair[1].
                    if matches!(kind, DrawKind::CandleWax | DrawKind::CandleWick) {
                        let (mesh, idx) = match kind {
                            DrawKind::CandleWax => (&self.candle_wax_mesh, 0),
                            _ => (&self.candle_wick_mesh, 1),
                        };
                        if let Some(pair) = self.candle_instances.get(slot_i) {
                            pass.set_bind_group(0, &pair[idx].bind_group, &[]);
                            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        }
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
                        DrawKind::Talisman => {
                            (&self.talisman_mesh, self.talisman_instances.get(slot_i))
                        }
                        DrawKind::Shrine => (&self.shrine_mesh, self.shrine_instances.get(slot_i)),
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
                        DrawKind::DoraPlinth => (
                            &self.dora_plinth_mesh,
                            self.dora_plinth_instances.get(slot_i),
                        ),
                        DrawKind::Bowl => (&self.bowl_mesh, self.bowl_instances.get(slot_i)),
                        DrawKind::Mirror => (&self.mirror_mesh, self.mirror_instances.get(slot_i)),
                        DrawKind::TallyStickBase => (
                            &self.tally_stick_base_mesh,
                            self.tally_stick_instances.get(slot_i),
                        ),
                        DrawKind::TallyStickTip => (
                            &self.tally_stick_tip_mesh,
                            self.tally_stick_instances.get(slot_i),
                        ),
                        DrawKind::CascadeToken => (
                            &self.bone_tablet_mesh,
                            self.cascade_token_instances.get(slot_i),
                        ),
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
                        | DrawKind::CandleWax
                        | DrawKind::CandleWick => unreachable!(),
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
                if let Some(ref rgb) = relic_glow_buffer {
                    pass.set_pipeline(&self.tile_glow_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, rgb.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..relic_glows.len() as u32);
                }
                if let (Some(ref buf), Some(ref gpu)) =
                    (relic_debuff_buffer, self.debuff_marker_overlay.as_ref())
                    && !relic_debuff_markers.is_empty()
                {
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
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
            RenderOp::ShowcaseTileBatch(batch_idx) => {
                if !self.tile_primitives.is_empty() {
                    let batch = showcase_tile_batches[*batch_idx];
                    if !batch.is_empty() {
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                        pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                        let start_slot: usize = showcase_tile_batches
                            .iter()
                            .take(*batch_idx)
                            .map(|b| b.len())
                            .sum();

                        // Glow halos for selected hand tiles (additive, drawn before mesh).
                        let has_glow = batch.iter().any(|p| p.glow);
                        if has_glow && let Some(ref tgb) = tile_glow_buffer {
                            pass.set_pipeline(&self.tile_glow_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
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
                            && self.tile_outline_index_count > 0
                        {
                            pass.set_pipeline(&self.tile_outline_pipeline);
                            pass.set_bind_group(0, &self.tile_outline_frame_bind_group, &[]);
                            pass.set_vertex_buffer(0, self.tile_outline_vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, self.tile_outline_instance_buffer.slice(..));
                            pass.set_index_buffer(
                                self.tile_outline_index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(
                                0..self.tile_outline_index_count,
                                0,
                                base..base + cnt,
                            );
                        }

                        // Pass B: regular textured tile meshes (opaque before blend).
                        for blend_phase in [false, true] {
                            let mut textured_last_pi: Option<usize> = None;
                            let mut textured_last_key: Option<TileGlbPipelineKey> = None;
                            for (i, _) in batch.iter().enumerate() {
                                let slot_i = start_slot + i;
                                if slot_i >= MAX_SHOWCASE_TILE_SLOTS {
                                    break;
                                }
                                let Some(stg) = self.showcase_tiles.get(slot_i) else {
                                    break;
                                };
                                for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                    if prim.pipeline_key.is_blend() != blend_phase {
                                        continue;
                                    }
                                    if textured_last_key != Some(prim.pipeline_key) {
                                        pass.set_pipeline(
                                            self.tile_glb_pipeline(prim.pipeline_key),
                                        );
                                        textured_last_key = Some(prim.pipeline_key);
                                    }
                                    if textured_last_pi != Some(pi) {
                                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                        pass.set_index_buffer(
                                            prim.index_buffer.slice(..),
                                            wgpu::IndexFormat::Uint32,
                                        );
                                        textured_last_pi = Some(pi);
                                    }
                                    let Some(bg) = stg.bind_groups.get(pi) else {
                                        continue;
                                    };
                                    pass.set_bind_group(0, bg, &[]);
                                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                }
                            }
                        }
                    }
                }
            }
            RenderOp::QuadBatch { buf_idx, count } => {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, quad_buffers[*buf_idx].slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::GradientQuadBatch { buf_idx, count } => {
                pass.set_pipeline(&self.gradient_quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, gradient_quad_buffers[*buf_idx].slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::SquircleQuadBatch { buf_idx, count } => {
                pass.set_pipeline(&self.squircle_quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, squircle_quad_buffers[*buf_idx].slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..*count);
            }
            RenderOp::FlameBatch { buf_idx, count } => {
                if *count > 0 && *buf_idx != usize::MAX {
                    pass.set_pipeline(&self.flame_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &self.flame_view_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, flame_buffers[*buf_idx].slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..*count);
                }
            }
            RenderOp::TextDraw(idx) => {
                let td = &text_draws[*idx];
                pass.set_pipeline(if scene_hdr_attachment {
                    &self.text_pipeline_scene_hdr
                } else {
                    &self.text_pipeline
                });
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &td.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..1);
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
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &gpu.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, tile_face_inst_buffers[*idx].slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
            RenderOp::PromptIconQuad(idx) => {
                let icon = &prompt_icon_quads[*idx];
                if let Some(gpu) = self.prompt_icon_overlays.get(&icon.source.cache_key()) {
                    pass.set_pipeline(if scene_hdr_attachment {
                        &self.image_pipeline_scene_hdr
                    } else {
                        &self.image_pipeline
                    });
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &gpu.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, prompt_icon_inst_buffers[*idx].slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
        }
    }
}
