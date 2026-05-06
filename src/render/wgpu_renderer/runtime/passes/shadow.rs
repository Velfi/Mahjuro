use super::*;

impl WgpuRenderer {
    /// Shadow pre-pass — render every caster (table excluded) into the
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
        showcase_tile_batches: &[&[ShowcaseTilePlacement]],
        shrine_batches: &[&[ShrinePlacement]],
        tile_3d_rects: &[(usize, [f32; 4])],
    ) {
        // ── Shadow pre-pass ─────────────────────────────────────────────
        // Render every caster (table excluded) into the shadow map from
        // the light's POV. Skipped entirely when shadows are disabled —
        // the lit shaders short-circuit on `params.x = 0` and the stale
        // map contents go unread.
        if shadows_enabled && shadow_uniforms_changed {
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

            // Candles (wax + wick) — pool is written above via Object3dKind::Candle.
            {
                let candle_count = frame
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
                    .filter(|o| {
                        matches!(o.kind, crate::render::draw_cmd::Object3dKind::Candle { .. })
                    })
                    .count();
                for slot_i in 0..candle_count {
                    let Some(instances) = self.candle_instances.get(slot_i) else {
                        break;
                    };
                    shadow_pass.set_bind_group(0, &instances[0].shadow_bind_group, &[]);
                    shadow_pass.set_vertex_buffer(0, self.candle_wax_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.candle_wax_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    shadow_pass.draw_indexed(0..self.candle_wax_mesh.index_count, 0, 0..1);

                    shadow_pass.set_bind_group(0, &instances[1].shadow_bind_group, &[]);
                    shadow_pass.set_vertex_buffer(0, self.candle_wick_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.candle_wick_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    shadow_pass.draw_indexed(0..self.candle_wick_mesh.index_count, 0, 0..1);
                }
            }

            // Shrines (pick-blind scene).
            {
                let total_shrines = shrine_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_SHRINE_SLOTS);
                if total_shrines > 0 {
                    shadow_pass.set_vertex_buffer(0, self.shrine_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.shrine_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_shrines {
                        let Some(inst) = self.shrine_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.shrine_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // (Dish shadow casting now flows through the generic
            // Primitive shadow block below.)

            // Ribbons (shop).
            {
                let total_ribbons = self.last_ribbon_slot_count;
                if total_ribbons > 0 {
                    shadow_pass.set_vertex_buffer(0, self.ribbon_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.ribbon_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_ribbons {
                        let Some(inst) = self.ribbon_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.ribbon_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Talismans — count Object3dKind::Talisman entries and draw their shadow instances.
            {
                let total_talismans = frame
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
                    .filter(|o| {
                        matches!(
                            o.kind,
                            crate::render::draw_cmd::Object3dKind::Talisman { .. }
                        )
                    })
                    .count()
                    .min(MAX_TALISMAN_SLOTS);
                if total_talismans > 0 {
                    shadow_pass.set_vertex_buffer(0, self.talisman_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.talisman_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_talismans {
                        let Some(inst) = self.talisman_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.talisman_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Primitive shadow casters — re-walk cmds to pair slot
            // indices with `shadow_caster: true` flags, then draw with
            // the registered mesh. Deterministic order (matches the
            // uniform-upload pass above).
            {
                use crate::render::primitive::MeshId;
                let mut cursors: std::collections::HashMap<MeshId, usize> =
                    std::collections::HashMap::new();
                for cmd in frame.cmds.iter() {
                    let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> =
                        match cmd {
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
                            let slot_i = *cursors.entry(*shape).or_insert(0);
                            *cursors.get_mut(shape).unwrap() += 1;
                            if *shadow_caster {
                                let (Some(mesh), Some(inst)) = (
                                    self.primitive_meshes.get(shape).map(|a| a.as_ref()),
                                    self.primitive_instances
                                        .get(shape)
                                        .and_then(|pool| pool.get(slot_i)),
                                ) else {
                                    continue;
                                };
                                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                                shadow_pass.set_index_buffer(
                                    mesh.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                            }
                            if *shape == MeshId::CabinetColumn {
                                *cursors.entry(MeshId::CabinetRails).or_insert(0) += 1;
                            }
                        }
                    }
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
                    shadow_pass.draw_indexed(0..self.tile_outline_index_count, 0, 0..1);
                }
            }
        }
    }
}
