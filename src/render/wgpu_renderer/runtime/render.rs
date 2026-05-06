use super::*;

/// HDR scene buffer clear — uniform void behind the table / props.
#[inline]
fn scene_viewport_clear() -> wgpu::Color {
    wgpu::Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}

/// Split [`RenderOp`]s at [`RenderOp::ClearSceneDepth`] markers (markers omitted).
fn split_render_ops_by_clear_depth(ops: &[RenderOp]) -> Vec<&[RenderOp]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, op) in ops.iter().enumerate() {
        if matches!(op, RenderOp::ClearSceneDepth) {
            if start < i {
                out.push(&ops[start..i]);
            }
            start = i + 1;
        }
    }
    if start < ops.len() {
        out.push(&ops[start..]);
    }
    out.into_iter().filter(|c| !c.is_empty()).collect()
}

impl WgpuRenderer {
    pub fn render(&mut self, frame: &UiFrame, settings: RenderSettings) -> anyhow::Result<()> {
        self.render_to(frame, settings, None)
    }

    /// Render `frame` into the offscreen `journal_scene_texture`. Used
    /// by the shop scene as a pre-pass while the journal book is open
    /// so the book mesh can sample a live render of the embedded yaku-
    /// journal scene as its page-content surface.
    pub fn render_journal_prepass(
        &mut self,
        frame: &UiFrame,
        settings: RenderSettings,
    ) -> anyhow::Result<()> {
        // Move the view out of `self` for the duration of the call so
        // the borrow checker is happy with `&mut self` re-entering
        // `render_to`. The view is cheap to recreate from the texture
        // (it's just a TextureView descriptor).
        let view = self
            .journal_scene_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to(frame, settings, Some(&view))
    }

    /// Render `frame` either to the swapchain (when `output_override` is
    /// `None`) or to a caller-provided color view (when `Some`). The
    /// override path is used for the journal pre-pass: the shop calls
    /// `render_to` once with `output_override = Some(&journal_scene_view)`
    /// and a journal `UiFrame`, then again with `output_override = None`
    /// for the live shop frame whose book mesh samples the journal target.
    ///
    /// Internal scratch buffers (`scene_color_texture`, depth, bloom
    /// ping/pong) are reused across both calls — only one render is in
    /// flight at a time and the override path's encoder is submitted
    /// before the swapchain path begins. Temporal SSR inputs
    /// (`scene_prev_texture`, `ssr_prev_depth_texture`) are updated only
    /// when `output_override` is `None` (the visible frame).
    pub fn render_to(
        &mut self,
        frame: &UiFrame,
        settings: RenderSettings,
        output_override: Option<&wgpu::TextureView>,
    ) -> anyhow::Result<()> {
        let RenderSettings {
            effects_quality,
            tile_preset,
            tile_material,
            surface_kind,
            tileset_name,
            draw_settle_speed,
            sort_settle_speed,
            gamma,
            shadows_enabled,
            ssr_enabled,
        } = settings;
        self.apply_render_settings(tile_material, surface_kind, effects_quality, &tileset_name);

        // Upload any relic/background textures that finished decoding.
        self.poll_relic_textures();
        self.poll_background_textures();

        // Acquire the per-frame texture to draw into. In the interactive
        // path this is a swapchain image; in headless screenshot mode it's
        // a plain render-attachment texture owned by `self.target`. Either
        // way we end up with a `&wgpu::Texture` (for the screenshot copy)
        // and a `TextureView` (for the render passes).
        //
        // Journal pre-pass override: when `output_override` is `Some`, skip
        // surface acquisition entirely and use the caller's view. Screenshot
        // capture is also skipped on this path — only the final swapchain
        // pass produces a presentable image.
        let is_prepass = output_override.is_some();
        let (surface_frame, frame_texture_opt): (
            Option<wgpu::SurfaceTexture>,
            Option<wgpu::Texture>,
        ) = if is_prepass {
            (None, None)
        } else {
            let sf = match self.acquire_render_frame()? {
                RenderFrame::Draw(frame) => frame,
                RenderFrame::Skip => return Ok(()),
            };
            let tex: wgpu::Texture = match (&sf, &self.target) {
                (Some(sf), _) => sf.texture.clone(),
                (None, RenderTarget::Offscreen { texture, .. }) => texture.clone(),
                (None, RenderTarget::Surface(_)) => {
                    unreachable!("Surface target always produces a surface_frame or early-returns")
                }
            };
            (sf, Some(tex))
        };
        let owned_view: Option<wgpu::TextureView> = frame_texture_opt
            .as_ref()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let view: &wgpu::TextureView = match output_override {
            Some(v) => v,
            None => owned_view
                .as_ref()
                .expect("non-prepass render must own a swapchain view"),
        };
        let bloom_active = Self::bloom_is_active(frame);

        // Lerp per-tile slide animations toward 0 (ease-out) and advance
        // short-lived departing-tile clocks.
        let dt = self.advance_frame_timers(draw_settle_speed, sort_settle_speed);

        self.upload_frame_uniforms(frame, effects_quality, gamma);

        // Build 2D backdrop quads (selection borders, hint pulses) and text
        // labels (just the focused arrow — the symbol+emoji live in the 3D
        // tile decal now).  Per-tile model matrices for the 3D mesh draw are
        // also written here.
        let mut tile_quads: Vec<GpuInstance> = Vec::new();
        let mut tile_labels: Vec<TextLabel> = Vec::new();
        let mut emoji_labels: Vec<TextLabel> = Vec::new();
        let mut tile_3d_rects: Vec<(usize, [f32; 4])> = Vec::new();
        // Per-tile world-space model matrices, snapshotted for next frame's
        // cursor pick (`pick_hand_tile`).
        let mut tile_pick_models: Vec<(usize, Mat4)> = Vec::new();
        // Additive glow halos for selected tiles, drawn behind the 3D
        // tile mesh as part of the Tiles3d render op.
        let mut tile_glows: Vec<GpuInstance> = Vec::new();
        // Additive glow halos for relics activated by the scoring cascade.
        // Populated below from the relic projection loop (we need each
        // relic's projected screen rect to size the halo). Drawn through
        // `tile_glow_pipeline` immediately after the 3D relic boxes so the
        // warm light blooms out around the box silhouette.
        let mut relic_glows: Vec<GpuInstance> = Vec::new();
        let mut relic_debuff_markers: Vec<GpuInstance> = Vec::new();

        // ── Person-at-the-table camera ──────────────────────────────────
        // Z-up world, standard right-hand conventions: +X right, +Y into the
        // table (away from player), +Z up from the felt. Table is z = 0 (XY).
        // Camera sits at large -Y (behind the player), elevated in +Z, looking
        // toward +Y. See [`crate::render::world_space::pixel_to_world`] for the
        // pixel → world mapping:
        //
        //   world_x =  pixel_x - w * 0.5       (screen-right → +X)
        //   world_y =  h * 0.5 - pixel_y       (screen-bottom → -Y, toward player)
        //   world_z =  lift above the felt
        //
        // The 2D UI overlays (score panel, buttons, text) keep using the
        // pixel-orthographic quad pipeline and float over the 3D scene as
        // a HUD.
        let camera = CameraFrame::build(frame, self.size);
        self.upload_camera_uniforms(&camera, ssr_enabled, frame);
        let look_target = camera.look_target;
        let view_proj_arr = camera.view_proj_arr;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };

        // ── Debug axes overlay ──────────────────────────────────────────
        self.write_debug_axes_uniforms(frame, &camera);

        // ── Flame emitters (world-space) ─────────────────────────────
        let flame_emitters = build_flame_emitters(frame, w, h);

        let tile_basis = tile_mesh_local_to_world();
        // Hand tiles render via ShowcaseTileBatch (further below); tile hints
        // come through as real green PointLights from gameplay scene.

        // Render departing tiles (two-phase: arc into river, then drift).
        for dep in &self.departing_tiles {
            let t = dep.elapsed.max(0.0);
            let w = dep.start_rect[2];
            let h = dep.start_rect[3];
            let start_cx = dep.start_rect[0] + w * 0.5;
            let start_cy = dep.start_rect[1] + h * 0.5;

            // Phase split: Arcing → Drifting at t = arc_dur.
            let (cx, cy, alpha, scale) = if t < dep.arc_dur {
                // Phase 1 — quadratic Bezier from the hand slot, over an
                // apex above the midpoint, into the river center. The
                // apex sits 110px above the higher of the two endpoints
                // so the tile reads as being *thrown* upward before
                // arcing down into the water rather than sliding in a
                // straight line.
                let u = (t / dep.arc_dur).clamp(0.0, 1.0);
                let mid_x = (start_cx + dep.river_target.0) * 0.5;
                let mid_y = start_cy.min(dep.river_target.1) - 110.0;
                let one_u = 1.0 - u;
                let bx =
                    one_u * one_u * start_cx + 2.0 * one_u * u * mid_x + u * u * dep.river_target.0;
                let by =
                    one_u * one_u * start_cy + 2.0 * one_u * u * mid_y + u * u * dep.river_target.1;
                // Slight shrink during the arc — the tile reads as
                // moving away from the camera as it falls into the
                // recessed water surface.
                let s = 1.0 - 0.18 * u;
                (bx, by, 1.0, s)
            } else {
                // Phase 2 — drift downstream and fade. Position
                // continues from the splash point along `drift_dir` at
                // `drift_speed`. Alpha eases from 1 → 0 over the drift
                // duration; scale shrinks further so the tile reads as
                // sinking into the water.
                let dt2 = t - dep.arc_dur;
                let u2 = (dt2 / dep.drift_dur).clamp(0.0, 1.0);
                let dx = dep.drift_dir.0 * dep.drift_speed * dt2;
                let dy = dep.drift_dir.1 * dep.drift_speed * dt2;
                let bx = dep.river_target.0 + dx;
                let by = dep.river_target.1 + dy;
                let a = 1.0 - u2;
                let s = 0.82 - 0.40 * u2;
                (bx, by, a, s)
            };

            let sw = w * scale;
            let sh = h * scale;
            let sx = cx - sw * 0.5;
            let sy = cy - sh * 0.5;

            // Tile background.
            tile_quads.push(GpuInstance {
                rect: [sx, sy, sw, sh],
                color: [0.0, 0.0, 0.0, alpha],
            });

            // Main label.
            let inset_x = sw * 0.10;
            let top_h = sh * 0.50;
            tile_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.05, sw - inset_x * 2.0, top_h],
                text: dep.symbol.clone(),
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
                ..Default::default()
            });

            // Suit emoji.
            let bot_h = sh * 0.40;
            emoji_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.55, sw - inset_x * 2.0, bot_h],
                text: dep.suit_emoji.clone(),
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
                ..Default::default()
            });
        }

        // Tile glow instance buffer (additive halo behind selected tiles).
        let tile_glow_buffer = if tile_glows.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-glow-instances"),
                        contents: bytemuck::cast_slice(&tile_glows),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // ── Pre-rasterize text labels → GPU textures + bind groups ──────
        // Caches the rasterized texture + bind group in `text_label_cache`
        // keyed on the inputs that affect produced pixels. The per-frame
        // instance buffer (rect + color) is rebuilt every call since it's
        // cheap and animates freely. Marquee labels (non-zero scroll_offset)
        // bypass the cache because the offset is baked into the raster and
        // would otherwise fill the cache with single-use entries.
        let make_text_draw =
            |device: &wgpu::Device,
             queue: &wgpu::Queue,
             text_bgl: &wgpu::BindGroupLayout,
             sampler: &wgpu::Sampler,
             cache: &mut HashMap<TextLabelShapeKey, HashMap<String, CachedTextLabel>>,
             frame_id: u64,
             lbl: &TextLabel,
             font: &fontdue::Font,
             emoji_fallback: Option<&fontdue::Font>|
             -> TextDraw {
                // Clamp before casting: `f32 as u32` saturates negatives/NaN to u32::MAX,
                // which blows past wgpu's 16384 texture limit and panics. Seen in arrange mode
                // when layout math produces a negative rect width.
                let tw = (lbl.rect[2].clamp(1.0, 16384.0) as u32).max(1);
                let th = (lbl.rect[3].clamp(1.0, 16384.0) as u32).max(1);
                let align = match lbl.align {
                    TextAlign::Left => LabelAlign::Left,
                    TextAlign::Center => LabelAlign::Center,
                    TextAlign::Right => LabelAlign::Right,
                };
                let scroll_offset_px = lbl.scroll_offset.round() as i32;
                let cacheable = scroll_offset_px == 0;
                let shape_key = TextLabelShapeKey {
                    emoji_path: emoji_fallback.is_some(),
                    font_px: lbl.font_px.map(|p| p.round() as u32),
                    width_px: tw,
                    height_px: th,
                    align: lbl.align,
                    scroll_offset_px,
                };

                let (bind_group, owned_tex) = if cacheable {
                    // Two-level lookup: hit path borrows &str (no String alloc).
                    let inner = cache.entry(shape_key).or_default();
                    if let Some(entry) = inner.get_mut(lbl.text.as_str()) {
                        entry.last_used = frame_id;
                        (entry.bind_group.clone(), None)
                    } else {
                        let rgba = rasterize_label_styled_with_fallback(
                            font,
                            emoji_fallback,
                            &lbl.text,
                            tw,
                            th,
                            crate::render::decal::LabelStyle {
                                font_px: lbl.font_px,
                                align,
                                scroll_offset: lbl.scroll_offset,
                            },
                        );
                        let (tex, view) =
                            upload_rgba_texture(device, queue, "text-lbl", &rgba, tw, th);
                        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("text-lbl-bg"),
                            layout: text_bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(sampler),
                                },
                            ],
                        });
                        let bg_clone = bg.clone();
                        inner.insert(
                            lbl.text.clone(),
                            CachedTextLabel {
                                tex,
                                bind_group: bg,
                                last_used: frame_id,
                            },
                        );
                        (bg_clone, None)
                    }
                } else {
                    // Marquee path: rasterize fresh, do not insert into cache.
                    let rgba = rasterize_label_styled_with_fallback(
                        font,
                        emoji_fallback,
                        &lbl.text,
                        tw,
                        th,
                        crate::render::decal::LabelStyle {
                            font_px: lbl.font_px,
                            align,
                            scroll_offset: lbl.scroll_offset,
                        },
                    );
                    let (tex, view) = upload_rgba_texture(device, queue, "text-lbl", &rgba, tw, th);
                    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("text-lbl-bg"),
                        layout: text_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    });
                    (bg, Some(tex))
                };

                let inst = GpuInstance {
                    rect: lbl.rect,
                    color: lbl.color,
                };
                let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("text-inst"),
                    contents: bytemuck::cast_slice(&[inst]),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                TextDraw {
                    inst_buf,
                    bind_group,
                    _tex: owned_tex,
                }
            };

        // ── Hand tile face/emoji label GPU draws (consumed by HandTileFaces) ──
        // Bump the cache frame stamp once per render() so make_text_draw can
        // mark every entry it touches. Evict entries that haven't been hit
        // for TEXT_CACHE_TTL_FRAMES — labels whose text/size has changed
        // shouldn't keep their stale GPU texture pinned forever.
        self.text_cache_frame = self.text_cache_frame.wrapping_add(1);
        let cache_frame_id = self.text_cache_frame;
        let ttl_cutoff = cache_frame_id.saturating_sub(TEXT_CACHE_TTL_FRAMES);
        // Walk the two-level map: drop stale entries from each inner bucket,
        // then drop any inner bucket that became empty.
        self.text_label_cache.retain(|_, inner| {
            inner.retain(|_, entry| entry.last_used >= ttl_cutoff);
            !inner.is_empty()
        });
        let mut hand_face_draws: Vec<TextDraw> = Vec::new();
        if let Some(ref font) = self.ui_font {
            for lbl in &tile_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    &mut self.text_label_cache,
                    cache_frame_id,
                    lbl,
                    font,
                    None,
                ));
            }
        }
        if let Some(ref font) = self.emoji_font {
            for lbl in &emoji_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    &mut self.text_label_cache,
                    cache_frame_id,
                    lbl,
                    font,
                    None,
                ));
            }
        }

        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        let mut quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut gradient_quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut tile_face_quads: Vec<TileFaceQuad> = Vec::new();
        let mut tile_face_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        // Skeuomorphic gameplay HUD cmd buffers (phase 1).
        // Dead empty vecs — kept so existing shadow/draw loops that still iterate
        // these compile; scenes no longer push to these variants.
        let shrine_batches: Vec<&[ShrinePlacement]> = Vec::new();
        let yaku_tablet_batches: Vec<&[YakuTabletPlacement]> = Vec::new();
        let wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut object3d_cmds: Vec<&[crate::render::draw_cmd::Object3d]> = Vec::new();
        // Flat draw list built during the Object3d pre-pass: (DrawKind, slot_i).
        let mut object3d_draw_list: Vec<(DrawKind, usize)> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();
        let mut bg_inst_buffers: Vec<wgpu::Buffer> = Vec::new();

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    let ww = self.size.width.max(1) as f32;
                    let wh = self.size.height.max(1) as f32;
                    let bg_inst = GpuInstance {
                        rect: [0.0, 0.0, ww, wh],
                        color: id.image_vertex_color(),
                    };
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("bg-inst"),
                            contents: bytemuck::cast_slice(&[bg_inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = bg_inst_buffers.len();
                    bg_inst_buffers.push(buf);
                    ops.push(RenderOp::Background { id: *id, buf_idx });
                    i += 1;
                }
                DrawCmd::Starfield => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::Starfield);
                    }
                    i += 1;
                }
                DrawCmd::EmberDrift => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::EmberDrift);
                    }
                    i += 1;
                }
                DrawCmd::GoldenDust => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::GoldenDust);
                    }
                    i += 1;
                }
                DrawCmd::MoonlitWater => {
                    // Not gated on `effects_quality`: `EffectLayers::BASELINE` forces the
                    // renderer tier to `Off` for procedural surfaces, but game-over still
                    // pushes this when `fullscreen_water_backdrop` is on (see
                    // `effect_layers.rs`).
                    ops.push(RenderOp::MoonlitWater);
                    i += 1;
                }
                DrawCmd::SunlitWater => {
                    ops.push(RenderOp::SunlitWater);
                    i += 1;
                }
                DrawCmd::MountainHaze => {
                    ops.push(RenderOp::MountainHaze);
                    i += 1;
                }
                DrawCmd::ShootingStarCascade => {
                    if effects_quality >= crate::persistence::EffectsQuality::Low {
                        ops.push(RenderOp::ShootingStarCascade);
                    }
                    i += 1;
                }
                DrawCmd::Table => {
                    ops.push(RenderOp::Table);
                    i += 1;
                }
                DrawCmd::ShopEnvironment => {
                    ops.push(RenderOp::ShopEnvironment);
                    i += 1;
                }
                DrawCmd::ClearSceneDepth => {
                    ops.push(RenderOp::ClearSceneDepth);
                    i += 1;
                }
                DrawCmd::Quad(_) => {
                    // Collect contiguous run of Quad cmds into a single batch.
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Quad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("quad-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = quad_buffers.len();
                    quad_buffers.push(buf);
                    ops.push(RenderOp::QuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::GradientQuad(_) => {
                    let mut batch: Vec<GradientQuadInstance> = Vec::new();
                    while let Some(DrawCmd::GradientQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("gradient-quad-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = gradient_quad_buffers.len();
                    gradient_quad_buffers.push(buf);
                    ops.push(RenderOp::GradientQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::Flame(_) => {
                    // Drain the contiguous run of Flame cmds. Per-flame
                    // brightness + phase were already harvested into
                    // `flame_emitters` above, so here we just advance `i`
                    // and emit a single `FlameBatch` op that the dispatch
                    // side will expand into particle-system state.
                    while let Some(DrawCmd::Flame(_)) = frame.cmds.get(i) {
                        i += 1;
                    }
                    // Step the particle system once per frame and upload
                    // the live particles into a fresh instance buffer.
                    let flame_time_s = self.creation_time.elapsed().as_secs_f32();
                    self.flame_particles
                        .step(&flame_emitters, self.frame_dt, flame_time_s);
                    let count = self.flame_particles.fill_gpu_instances(
                        &flame_emitters,
                        flame_time_s,
                        &mut self.flame_particle_staging,
                    );
                    if count == 0 {
                        // Nothing to draw yet (first frame, or all
                        // particles expired during a pause). Still push
                        // the op so the downstream code has a consistent
                        // shape; the dispatch side handles count=0.
                        ops.push(RenderOp::FlameBatch {
                            buf_idx: usize::MAX,
                            count: 0,
                        });
                    } else {
                        let buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("flame-particles"),
                                    contents: bytemuck::cast_slice(
                                        &self.flame_particle_staging[..count],
                                    ),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        let buf_idx = flame_buffers.len();
                        flame_buffers.push(buf);
                        ops.push(RenderOp::FlameBatch {
                            buf_idx,
                            count: count as u32,
                        });
                    }
                }
                DrawCmd::Text(lbl) => {
                    if let Some(ref font) = self.ui_font {
                        let td = make_text_draw(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            &mut self.text_label_cache,
                            cache_frame_id,
                            lbl,
                            font,
                            self.emoji_font.as_ref(),
                        );
                        let idx = text_draws.len();
                        text_draws.push(td);
                        ops.push(RenderOp::TextDraw(idx));
                    }
                    i += 1;
                }
                DrawCmd::TileFaceQuad(face) => {
                    let key = (
                        face.tile.suit,
                        face.tile.rank,
                        face.tile.enhancement,
                        face.tile.debuffed_visual,
                    );
                    if !self.tile_face_overlays.contains_key(&key) {
                        let overlay = make_tile_face_overlay_gpu(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            self.ui_font.as_ref(),
                            self.emoji_font.as_ref(),
                            &face.tile,
                            self.tile_set.as_deref(),
                        );
                        self.tile_face_overlays.insert(key, overlay);
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("tile-face-quad"),
                            contents: bytemuck::cast_slice(&[face.inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let idx = tile_face_quads.len();
                    tile_face_quads.push(*face);
                    tile_face_inst_buffers.push(buf);
                    ops.push(RenderOp::TileFaceQuad(idx));
                    i += 1;
                }
                DrawCmd::ShowcaseTileBatch(placements) => {
                    let idx = showcase_tile_batches.len();
                    showcase_tile_batches.push(placements.as_slice());
                    ops.push(RenderOp::ShowcaseTileBatch(idx));
                    i += 1;
                }
                DrawCmd::Object3d(obj) => {
                    object3d_cmds.push(std::slice::from_ref(obj));
                    // start/end will be filled in during the pre-pass; push placeholder
                    ops.push(RenderOp::Object3dBatch { start: 0, end: 0 });
                    i += 1;
                }
                DrawCmd::Object3dBatch(objs) => {
                    object3d_cmds.push(objs.as_slice());
                    ops.push(RenderOp::Object3dBatch { start: 0, end: 0 });
                    i += 1;
                }
            }
        }

        // ── Debug axes overlay labels ───────────────────────────────────
        // After walking the scene's cmds, append three text labels (one per
        // axis) projected from the world-space tip of each debug-axes bar.
        // These get rasterized into ordinary text draws so they ride along
        // in the same render pass as the bars themselves.
        if frame.debug_axes
            && let Some(ref font) = self.ui_font
        {
            let length = h * 0.35;
            let label_size = (h * 0.04).max(18.0);
            let label_w = label_size * 3.5;
            let label_h = label_size * 1.5;
            let labels: [(glam::Vec3, &str, [f32; 4]); 3] = [
                (
                    look_target + glam::Vec3::X * length,
                    "+X",
                    [1.0, 0.25, 0.25, 1.0],
                ),
                (
                    look_target + glam::Vec3::Y * length,
                    "+Y",
                    [0.25, 1.0, 0.25, 1.0],
                ),
                (
                    look_target + glam::Vec3::Z * length,
                    "+Z",
                    [0.45, 0.65, 1.0, 1.0],
                ),
            ];
            for (tip_world, text, color) in labels.iter() {
                let (sx, sy) = project_to_screen(*tip_world);
                let lbl = TextLabel {
                    rect: [sx - label_w * 0.5, sy - label_h * 0.5, label_w, label_h],
                    text: (*text).to_string(),
                    color: *color,
                    font_px: Some(label_size),
                    align: TextAlign::Center,
                    no_glossary: true,
                    ..Default::default()
                };
                let td = make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    &mut self.text_label_cache,
                    cache_frame_id,
                    &lbl,
                    font,
                    None,
                );
                let idx = text_draws.len();
                text_draws.push(td);
                ops.push(RenderOp::TextDraw(idx));
            }
        }

        // ── Update procedural lit-mesh uniforms (table + candles) ───────
        // Written before the render pass begins, since the pass borrows
        // `self` immutably.
        let needs_table = ops.iter().any(|o| matches!(o, RenderOp::Table));
        if needs_table {
            // Horizontal table: mesh is local XY with +Z normal; Y-up mesh
            // chain uses Rx(-90°) so the felt normal is +Y in that basis, then
            // [`translate_rot_scale`] maps to world +Z. Wood grain is
            // evaluated in world XY in the shader.
            let table_extent = h * 30.0;
            let table_w = table_extent;
            let table_d = table_extent;
            let model = translate_rot_scale(
                glam::Vec3::ZERO,
                table_mesh_lay_flat(),
                glam::Vec3::new(table_w, table_d, 1.0),
            );
            self.table_instance.write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.table_material,
            );

            // Felt shells share `table_instance`'s uniform; shell layer comes
            // from `@builtin(instance_index)` in `vs_felt_shell_instanced`.
        }
        // Reset the debug pickable catch-all for this frame; each draw
        // loop below appends entries it wants to expose to
        // `pick_debug_object`.
        self.last_debug_pickables.clear();
        self.last_gameplay_fog_wall_horizon_y = frame.gameplay_fog_wall_horizon_y;
        self.last_gameplay_fog_wall_center_x = frame.gameplay_fog_wall_center_x;
        if let Some(hy) = frame.gameplay_fog_wall_horizon_y {
            let py = hy.clamp(0.0, 1.0) * h;
            let cx_px = frame
                .gameplay_fog_wall_center_x
                .unwrap_or(0.5)
                .clamp(0.0, 1.0)
                * w;
            let center = pixel_to_world(w, h, cx_px, py, 720.0);
            let model = translate_rot_scale(
                center,
                Mat4::IDENTITY,
                glam::Vec3::new(w * 12.0, 160.0, 140.0),
            );
            self.last_debug_pickables.push((
                "gameplay.fog_wall".to_string(),
                model,
                glam::Vec3::splat(0.5),
                0.0,
            ));
        }

        // Candles migrated to Object3dKind::Candle.

        // ── Relic placeholders (migrated to Object3dKind::Relic) ──────
        self.last_relic_models.clear();
        self.last_pickable_relic_models.clear();
        let mut relic_slot_cursor: usize = 0;
        let _ = &mut relic_slot_cursor;

        // ── Pack placeholders (same mesh + pipeline as relics) ──────────
        self.proj.pack_rects.clear();
        // Pack placements migrated to Object3dKind::Pack.
        // ── Shrines (pick-blind scene). Each placement gets its own slot. ─
        // The shrine mesh is built in normalized -0.5..+0.5 local space, so
        // a per-instance scale by `extents` sizes Small/Big/Boss
        // independently. `world_pos` is the *base center*, so we lift the
        // model up by half the height to put the plinth on the ground.
        self.proj.shrine_rects.clear();
        {
            let mut shrine_cursor: usize = 0;
            for batch in &shrine_batches {
                for s in batch.iter() {
                    if shrine_cursor >= MAX_SHRINE_SLOTS {
                        break;
                    }
                    let slot_i = shrine_cursor;
                    shrine_cursor += 1;
                    let center = pixel_to_world(
                        w,
                        h,
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = translate_rot_scale(
                        center,
                        Mat4::IDENTITY,
                        glam::Vec3::new(s.extents[0], s.extents[1], s.extents[2]),
                    );
                    // Project the shrine's 8 AABB corners to screen and
                    // take the bounding rect — gives the scene a 2D rect
                    // it can anchor labels to without re-projecting the
                    // perspective transform itself.
                    let hx = s.extents[0] * 0.5;
                    let hy = s.extents[1] * 0.5;
                    let hz = s.extents[2] * 0.5;
                    let mut mn_x = f32::INFINITY;
                    let mut mn_y = f32::INFINITY;
                    let mut mx_x = f32::NEG_INFINITY;
                    let mut mx_y = f32::NEG_INFINITY;
                    for cx in [-hx, hx] {
                        for cy in [-hy, hy] {
                            for cz in [-hz, hz] {
                                let world = center + glam::Vec3::new(cx, cy, cz);
                                let (px, py) = project_to_screen(world);
                                mn_x = mn_x.min(px);
                                mn_y = mn_y.min(py);
                                mx_x = mx_x.max(px);
                                mx_y = mx_y.max(py);
                            }
                        }
                    }
                    self.proj
                        .shrine_rects
                        .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                    // Glow gently brightens the shrine's tint so the
                    // upcoming shrine reads as the active choice at
                    // rest, but most of the warmth still comes from
                    // the warm spotlight tinting the stone — the
                    // shrine itself shouldn't self-illuminate.
                    let g = s.glow.clamp(0.0, 1.0);
                    let base_color = if g > 0.0 {
                        let target = [1.10, 1.05, 0.95, s.color[3]];
                        [
                            s.color[0] + (target[0] - s.color[0]) * g,
                            s.color[1] + (target[1] - s.color[1]) * g,
                            s.color[2] + (target[2] - s.color[2]) * g,
                            s.color[3],
                        ]
                    } else {
                        s.color
                    };
                    // Rough stone material: very low specular strength
                    // and a low specular power so any highlight that
                    // does catch is wide and soft (like weathered
                    // rock, not polished marble). Identical params for
                    // every shrine so cleared/future ones can't catch
                    // sharp glints from ambient fills.
                    let material = MaterialParams {
                        kind: MaterialKind::Plain,
                        base_color,
                        specular_strength: 0.06,
                        specular_power: 8.0,
                    };
                    self.shrine_instances[slot_i].write_uniform(
                        &self.queue,
                        view_proj_arr,
                        model,
                        material,
                    );
                }
            }
        }

        // Auxiliary dishes migrated to Object3dKind::Dish.

        // ── Ribbon batches (shop scene) ────────────────────────────────
        // Each textured ribbon uses up to 3 draw slots (top cap, tileable
        // middle, bottom cap) so its length is independent of texture aspect.
        // Untextured (plain) ribbons still use a single slot.
        self.last_ribbon_models.clear();
        self.last_ribbon_batch_slot_counts.clear();
        // Zodiac ribbons migrated to Object3dKind::ZodiacRibbon.

        // ── Talisman batches (shop scene) ──────────────────────────────
        self.last_talisman_models.clear();
        // Talismans migrated to Object3dKind::Talisman.

        // Coins migrated to Object3dKind::Coin.

        // ── Reset per-frame singletons owned by Object3d handlers ──────
        self.write_gameplay_hud_uniforms(&camera, &yaku_tablet_batches);

        // Wood tablets migrated to Object3dKind::WoodTablet.

        self.run_object3d_placement(
            &camera,
            &object3d_cmds,
            &wall_stack_cmds,
            &mut object3d_draw_list,
            &mut ops,
            &mut relic_glows,
            &mut relic_debuff_markers,
        );

        if !relic_debuff_markers.is_empty() && self.debuff_marker_overlay.is_none() {
            self.debuff_marker_overlay = Some(super::super::make_debuff_marker_overlay_gpu(
                &self.device,
                &self.queue,
                &self.text_bind_group_layout,
                &self.tile_sampler,
            ));
        }

        // ── Arrange-mode bounding box overlay ──────────────────────────────
        self.push_arrange_bbox_overlay(frame, &camera, &mut quad_buffers, &mut ops);

        // Relic activation halo buffer — built from `relic_glows` populated
        // during the relic projection loop above. Drawn through the same
        // additive `tile_glow_pipeline` as the selected-tile halos, right
        // after the 3D relic boxes so the bloom blossoms around them.
        let relic_glow_buffer = if relic_glows.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("relic-glow-instances"),
                        contents: bytemuck::cast_slice(&relic_glows),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        let relic_debuff_buffer = if relic_debuff_markers.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("relic-debuff-marker-instances"),
                        contents: bytemuck::cast_slice(&relic_debuff_markers),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        self.garbage_collect_prev_tile_world();

        // ── Shadow map setup ────────────────────────────────────────────
        let shadow_frame = self.setup_shadow_frame(&camera, shadows_enabled);
        let light_view_proj_arr = shadow_frame.light_view_proj_arr;

        let shadow_just_enabled = shadows_enabled && !self.prev_frame_shadows_enabled;
        self.prev_frame_shadows_enabled = shadows_enabled;
        let mut shadow_uniforms_changed = shadow_just_enabled;

        self.write_per_instance_shadow_casters(
            frame,
            &camera,
            light_view_proj_arr,
            &tile_pick_models,
            &shrine_batches,
            &mut shadow_uniforms_changed,
        );

        self.run_showcase_tiles_placement(
            frame,
            &camera,
            tile_basis,
            tile_preset,
            dt,
            light_view_proj_arr,
            &showcase_tile_batches,
            &mut tile_3d_rects,
            &mut tile_pick_models,
            &mut tile_glows,
            &mut shadow_uniforms_changed,
        );

        if ops.iter().any(|o| matches!(o, RenderOp::ShopEnvironment)) {
            self.write_shop_environment_uniforms(frame, &camera, frame.shop_env_gltf_punctual);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        self.render_shadow_pre_pass(
            &mut encoder,
            frame,
            shadows_enabled,
            shadow_uniforms_changed,
            &showcase_tile_batches,
            &shrine_batches,
            &tile_3d_rects,
        );

        // Pass A renders into `scene_color_view`
        // (`Rgba16Float`). Do not key this on `is_prepass`: the journal
        // pre-pass still draws the 3D scene into that HDR buffer before
        // tonemapping to the journal target.
        let process_ctx_scene = ProcessOpCtx {
            frame,
            bg_inst_buffers: &bg_inst_buffers,
            quad_buffers: &quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            object3d_draw_list: &object3d_draw_list,
            showcase_tile_batches: &showcase_tile_batches,
            tile_glows: &tile_glows,
            tile_glow_buffer: tile_glow_buffer.as_ref(),
            relic_glows: &relic_glows,
            relic_glow_buffer: relic_glow_buffer.as_ref(),
            relic_debuff_markers: &relic_debuff_markers,
            relic_debuff_buffer: relic_debuff_buffer.as_ref(),
            scene_hdr_attachment: true,
        };
        // Text overlay loads the final `view` (swapchain or journal), not
        // `scene_color_view`; match that attachment's format for 2D pipelines.
        let overlay_hdr =
            is_prepass || matches!(self.config.format, wgpu::TextureFormat::Rgba16Float);
        let process_ctx_overlay = ProcessOpCtx {
            frame,
            bg_inst_buffers: &bg_inst_buffers,
            quad_buffers: &quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            object3d_draw_list: &object3d_draw_list,
            showcase_tile_batches: &showcase_tile_batches,
            tile_glows: &tile_glows,
            tile_glow_buffer: tile_glow_buffer.as_ref(),
            relic_glows: &relic_glows,
            relic_glow_buffer: relic_glow_buffer.as_ref(),
            relic_debuff_markers: &relic_debuff_markers,
            relic_debuff_buffer: relic_debuff_buffer.as_ref(),
            scene_hdr_attachment: overlay_hdr,
        };

        // ── Pre-pass: shooting-star cascade into half-res offscreen ─────
        // The cascade shader is extremely heavy per-pixel, so it renders at
        // quarter-area (half dims) and is additively composited up to the
        // main scene target inside `Pass A`. Skip the pass entirely when no
        // cascade op is queued so the clear isn't paid for on every frame.
        let cascade_active = ops
            .iter()
            .any(|o| matches!(o, RenderOp::ShootingStarCascade));
        if cascade_active {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cascade-offscreen-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.cascade_offscreen_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shooting_star_cascade_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Pass A: clear + draw main scene ───────────────────────────────
        {
            #[cfg(debug_assertions)]
            let split_main_for_profile = self.gpu_profiler.is_sampling()
                && ops.iter().any(|o| matches!(o, RenderOp::Table));
            #[cfg(not(debug_assertions))]
            let split_main_for_profile = false;

            let mut pass_a_chunks = split_render_ops_by_clear_depth(&ops);
            if pass_a_chunks.is_empty() && !ops.is_empty() {
                pass_a_chunks.push(ops.as_slice());
            }

            macro_rules! pass_a_draw_loop {
                ($pass:expr, $skip_table:expr, $only_table:expr) => {{
                    for op in &ops {
                        // 2D HUD text labels are drawn in a later overlay pass
                        // (Load on the tonemapped target) so they are not stored
                        // in `scene_prev_texture` and do not appear in the
                        // lacquered-table SSR sample. Gameplay plaques are
                        // `Object3d` meshes (engraved decal on the mesh); they
                        // render here in Pass A like other lit meshes.
                        if matches!(op, RenderOp::TextDraw(_)) {
                            continue;
                        }
                        let is_table = matches!(op, RenderOp::Table);
                        if $only_table && !is_table {
                            continue;
                        }
                        if $skip_table && is_table {
                            continue;
                        }
                        self.process_op(&mut $pass, op, &process_ctx_scene);
                    }
                }};
            }

            macro_rules! pass_a_draw_chunk {
                ($pass:expr, $chunk:expr, $skip_table:expr, $only_table:expr) => {{
                    for op in $chunk.iter() {
                        if matches!(op, RenderOp::TextDraw(_)) {
                            continue;
                        }
                        let is_table = matches!(op, RenderOp::Table);
                        if $only_table && !is_table {
                            continue;
                        }
                        if $skip_table && is_table {
                            continue;
                        }
                        self.process_op(&mut $pass, op, &process_ctx_scene);
                    }
                }};
            }

            macro_rules! pass_a_debug_axes {
                ($pass:expr) => {{
                    if frame.debug_axes {
                        $pass.set_pipeline(&self.lit_mesh_pipeline);
                        $pass.set_bind_group(3, &self.lit_mesh_spot_ssr_bind_group, &[]);
                        $pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        $pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                        $pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                        $pass.set_index_buffer(
                            self.relic_box_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        for inst in self.debug_axes_instances.iter() {
                            $pass.set_bind_group(0, &inst.bind_group, &[]);
                            $pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                        }
                    }
                }};
            }

            if split_main_for_profile {
                let ts_table = self
                    .gpu_profiler
                    .pass_writes(crate::render::gpu_profiler::PassSlot::MainTable);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main-pass-table"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.scene_color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(scene_viewport_clear()),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: ts_table,
                    multiview_mask: None,
                });
                pass_a_draw_loop!(pass, false, true);
                drop(pass);

                if !pass_a_chunks.is_empty() {
                    let n_scene_chunks = pass_a_chunks.len();
                    for (ci, chunk) in pass_a_chunks.iter().enumerate() {
                        let depth_load = if ci == 0 {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        };
                        let is_last_scene_chunk = ci + 1 == n_scene_chunks;
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main-pass-scene"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &self.scene_color_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                                view: &self.depth_view,
                                depth_ops: Some(wgpu::Operations {
                                    load: depth_load,
                                    store: wgpu::StoreOp::Store,
                                }),
                                stencil_ops: None,
                            }),
                            occlusion_query_set: None,
                            timestamp_writes: if ci == 0 {
                                self.gpu_profiler
                                    .pass_writes(crate::render::gpu_profiler::PassSlot::MainScene)
                            } else {
                                None
                            },
                            multiview_mask: None,
                        });
                        pass_a_draw_chunk!(pass, chunk, true, false);
                        if is_last_scene_chunk {
                            pass_a_debug_axes!(pass);
                        }
                    }
                }
            } else {
                if !pass_a_chunks.is_empty() {
                    let n_chunks = pass_a_chunks.len();
                    for (ci, chunk) in pass_a_chunks.iter().enumerate() {
                        let color_load = if ci == 0 {
                            wgpu::LoadOp::Clear(scene_viewport_clear())
                        } else {
                            wgpu::LoadOp::Load
                        };
                        let depth_load = wgpu::LoadOp::Clear(1.0);
                        let is_last_chunk = ci + 1 == n_chunks;
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main-pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &self.scene_color_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: color_load,
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                                view: &self.depth_view,
                                depth_ops: Some(wgpu::Operations {
                                    load: depth_load,
                                    store: wgpu::StoreOp::Store,
                                }),
                                stencil_ops: None,
                            }),
                            occlusion_query_set: None,
                            timestamp_writes: if ci == 0 {
                                self.gpu_profiler
                                    .pass_writes(crate::render::gpu_profiler::PassSlot::Main)
                            } else {
                                None
                            },
                            multiview_mask: None,
                        });
                        pass_a_draw_chunk!(pass, chunk, false, false);
                        if is_last_chunk {
                            pass_a_debug_axes!(pass);
                        }
                    }
                }
            }
        }

        // ── SSR snapshot ────────────────────────────────────────────────
        // After full Pass A, copy linear HDR colour + depth into
        // `scene_prev_texture` / `ssr_prev_depth_texture` for next frame's
        // lacquered-table SSR. Only the primary visible pass updates history —
        // not `output_override` prepasses (e.g. shop journal → book texture).
        //
        // Skipped when `is_prepass`.
        if !is_prepass {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.scene_color_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.scene_prev_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.size.width.max(1),
                    height: self.size.height.max(1),
                    depth_or_array_layers: 1,
                },
            );
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ssr_prev_depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::Extent3d {
                    width: self.size.width.max(1),
                    height: self.size.height.max(1),
                    depth_or_array_layers: 1,
                },
            );
        }

        let bloom_w = (self.size.width.max(1) / 2).max(1);
        let bloom_h = (self.size.height.max(1) / 2).max(1);
        let bloom_threshold = if bloom_active { 1.05 } else { 9999.0 };
        let bloom_strength = if bloom_active { 0.92 } else { 0.0 };
        let extract_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0; 4],
        };
        let blur_h_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [1.0, 0.0, 0.0, 0.0],
        };
        let blur_v_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0, 1.0, 0.0, 0.0],
        };
        let fisheye_strength = frame.fisheye_strength.max(0.0);
        // Vignette tracks fisheye so the warp's corner squish fades into
        // darkness (hiding the clamp seam) and reinforces the "looking
        // down a long cabinet" feel without swamping the image when
        // fisheye is off.
        let vignette_strength = (fisheye_strength * 1.4).min(0.85);
        let composite_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [fisheye_strength, vignette_strength, 0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.bloom_extract_params_buffer,
            0,
            bytemuck::bytes_of(&extract_params),
        );
        self.queue.write_buffer(
            &self.bloom_blur_h_params_buffer,
            0,
            bytemuck::bytes_of(&blur_h_params),
        );
        self.queue.write_buffer(
            &self.bloom_blur_v_params_buffer,
            0,
            bytemuck::bytes_of(&blur_v_params),
        );
        self.queue.write_buffer(
            &self.bloom_composite_params_buffer,
            0,
            bytemuck::bytes_of(&composite_params),
        );

        if bloom_active {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-extract-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_ping_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_extract_pipeline);
                pass.set_bind_group(0, &self.bloom_scene_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-blur-h-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_pong_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &self.bloom_ping_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-blur-v-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_ping_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &self.bloom_pong_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.post_bloom_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_composite_pipeline);
            pass.set_bind_group(0, &self.bloom_composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Journal prepass writes bloom composite into `journal_scene_texture` for the
        // book mesh to sample — keep that linear. Swapchain HDR (`Rgba16Float`) must
        // use the same ACES fitted curve as SDR or ingame colors read oversaturated.
        let tonemap_mode = if is_prepass { 1.0f32 } else { 0.0f32 };
        self.queue.write_buffer(
            &self.tonemap_params_buffer,
            0,
            bytemuck::bytes_of(&TonemapParams {
                exposure: self.tonemap_exposure,
                mode: tonemap_mode,
                _pad: [0.0; 2],
            }),
        );
        let tonemap_pipe = if is_prepass {
            &self.tonemap_rgba16f_pipeline
        } else {
            &self.tonemap_pipeline
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(tonemap_pipe);
            pass.set_bind_group(0, &self.tonemap_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Overlay pass: 2D HUD text labels ────────────────────────────
        // After tonemap, Load the final target so labels are not in the linear
        // HDR `scene_prev_texture` used for lacquered-table SSR.
        if ops.iter().any(|o| matches!(o, RenderOp::TextDraw(_))) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-overlay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            for op in &ops {
                if matches!(op, RenderOp::TextDraw(_)) {
                    self.process_op(&mut pass, op, &process_ctx_overlay);
                }
            }
        }

        // GPU profiler: resolve query set + stage readback before submit,
        // then block on map after submit so the readback is frame-accurate.
        // Both calls are no-ops when no profiling session is active.
        self.gpu_profiler.before_submit(&mut encoder);

        // Headless screenshot capture: if a path is queued, copy the
        // surface texture into a staging buffer in the same submission.
        // After submit + poll(Wait), map and PNG-encode synchronously.
        // The surface texture is still owned by us until present(), so
        // this is safe. Tied into the same encoder so no extra submit.
        // Skipped on the journal pre-pass — only the final swapchain
        // pass produces a presentable image worth screenshotting.
        let screenshot_path = if is_prepass {
            None
        } else {
            self.pending_screenshot.take()
        };
        let screenshot_staging = match (&screenshot_path, &frame_texture_opt) {
            (Some(path), Some(ft)) => {
                log::info!("screenshot: encoding capture for {}", path.display());
                Some(self.encode_screenshot_copy(&mut encoder, ft, path))
            }
            _ => None,
        };

        self.queue.submit(std::iter::once(encoder.finish()));

        if let (Some(path), Some(staging)) = (screenshot_path, screenshot_staging) {
            match self.finalize_screenshot(staging, &path) {
                Ok(()) => log::info!("screenshot: wrote {}", path.display()),
                Err(e) => log::error!("screenshot finalize failed: {e:?}"),
            }
        }

        if let Some(sf) = surface_frame {
            sf.present();
        }
        self.gpu_profiler.after_submit(&self.device);
        Ok(())
    }

    /// Drop `prev_tile_world` entries for tile uids not present this frame.
    fn garbage_collect_prev_tile_world(&mut self) {
        if self.prev_tile_world.is_empty() {
            return;
        }
        let live: std::collections::HashSet<u32> = self.tile_uids.iter().copied().collect();
        self.prev_tile_world.retain(|k, _| live.contains(k));
    }
}
