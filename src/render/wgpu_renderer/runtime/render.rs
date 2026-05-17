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

#[derive(Clone, Copy, Default)]
enum PassAShopInspectHdrUpload {
    #[default]
    None,
}

/// Per-frame summary of which `RenderOp` variants appear in the current ops
/// list. Computed once with [`OpsFlags::scan`] and reused everywhere that
/// previously called `ops.iter().any(...)` — those scans showed up multiple
/// times in `render()` and each was a fresh O(n) walk over the op list.
#[derive(Default, Clone, Copy)]
struct OpsFlags {
    needs_table: bool,
    shop_env: bool,
    hallway_env: bool,
    archive_env: bool,
    cascade: bool,
}

impl OpsFlags {
    fn scan(ops: &[RenderOp]) -> Self {
        let mut f = OpsFlags::default();
        for op in ops {
            match op {
                RenderOp::Table => f.needs_table = true,
                RenderOp::ShopEnvironment => f.shop_env = true,
                RenderOp::HallwayEnvironment => f.hallway_env = true,
                RenderOp::ArchiveEnvironment => f.archive_env = true,
                RenderOp::ShootingStarCascade => f.cascade = true,
                _ => {}
            }
            // Early-out: if every flag we care about is set, stop scanning.
            if f.needs_table && f.shop_env && f.hallway_env && f.archive_env && f.cascade {
                break;
            }
        }
        f
    }
}

/// One contiguous slice of [`RenderOp`]s inside Pass A. `shop_inspect_hdr_upload` selects an
/// optional [`SsrGlobals.felt`] upload **before** this slice's render pass.
struct PassAChunk<'a> {
    ops: &'a [RenderOp],
    shop_inspect_hdr_upload: PassAShopInspectHdrUpload,
}

/// Split Pass A at [`RenderOp::ClearSceneDepth`] and [`RenderOp::ShopInspectLitMeshSubjectHdr`]
/// (markers omitted from chunks).
fn split_pass_a_chunks(ops: &[RenderOp]) -> Vec<PassAChunk<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut upload_before_next = PassAShopInspectHdrUpload::None;
    for (i, op) in ops.iter().enumerate() {
        let split = matches!(
            op,
            RenderOp::ClearSceneDepth
        );
        if !split {
            continue;
        }
        if start < i {
            out.push(PassAChunk {
                ops: &ops[start..i],
                shop_inspect_hdr_upload: upload_before_next,
            });
            upload_before_next = PassAShopInspectHdrUpload::None;
        }
        start = i + 1;
    }
    if start < ops.len() {
        out.push(PassAChunk {
            ops: &ops[start..],
            shop_inspect_hdr_upload: upload_before_next,
        });
    }
    out.into_iter().filter(|c| !c.ops.is_empty()).collect()
}

impl WgpuRenderer {
    /// Render one frame.
    ///
    /// `frame.cmds` is walked in order — earlier cmds render under later ones.
    /// Contiguous runs of `DrawCmd::Quad` are batched into a single instanced
    /// draw, which is invisible to scenes and preserves ordering.
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
            vhs_enabled,
        } = settings;
        let flatten_time_text_fx = matches!(
            effects_quality,
            crate::persistence::EffectsQuality::Off | crate::persistence::EffectsQuality::Low
        );
        // Master Options-toggle override: when the user has VHS off we kill
        // the per-scene branch even if the per-scene tuning has non-zero
        // amplitudes. Computed locally — `self.tonemap_vhs_enabled` keeps
        // the per-scene resolution so re-enabling restores the look.
        let effective_vhs_on = self.tonemap_vhs_enabled && vhs_enabled;
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
        self.gpu_profiler.begin_submit(!is_prepass);
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

        // Reset the per-frame bump pool. `frame_buffer_pool` backs the
        // hot quad-batch / text-instance / background-instance vertex
        // uploads below, replacing per-call `device.create_buffer_init`s
        // (~11/frame previously) with one growable persistent buffer
        // and `queue.write_buffer` per allocation.
        self.frame_buffer_pool.begin_frame();

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
                user: 0,
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
        let window_h = h;
        let make_text_draw = |device: &wgpu::Device,
                              queue: &wgpu::Queue,
                              text_bgl: &wgpu::BindGroupLayout,
                              sampler: &wgpu::Sampler,
                              cache: &mut rustc_hash::FxHashMap<
            TextLabelShapeKey,
            rustc_hash::FxHashMap<String, CachedTextLabel>,
        >,
                              frame_id: u64,
                              lbl: &TextLabel,
                              font: &fontdue::Font,
                              font_italic: Option<&fontdue::Font>,
                              emoji_fallback: Option<&fontdue::Font>|
         -> TextDraw {
            // Clamp before casting: `f32 as u32` saturates negatives/NaN to u32::MAX,
            // which blows past wgpu's 16384 texture limit and panics. Seen in arrange mode
            // when layout math produces a negative rect width.
            let tw_raw = (lbl.rect[2].clamp(1.0, 16384.0) as u32).max(1);
            let th_raw = (lbl.rect[3].clamp(1.0, 16384.0) as u32).max(1);
            let rotation_quarters = lbl.rotation_quarters.min(3);
            let (tw, th) = if rotation_quarters % 2 == 1 {
                (tw_raw.max(th_raw), tw_raw.min(th_raw))
            } else {
                (tw_raw, th_raw)
            };
            let align = match lbl.align {
                TextAlign::Left => LabelAlign::Left,
                TextAlign::Center => LabelAlign::Center,
                TextAlign::Right => LabelAlign::Right,
            };
            let scroll_offset_px = lbl.scroll_offset.round() as i32;
            let cacheable = scroll_offset_px == 0;
            let flavor = lbl
                .flavor_spans
                .and_then(|s| if s.is_empty() { None } else { Some(s) });
            let inline_face_bits =
                (lbl.bold as u8) | ((lbl.italic as u8) << 1) | ((lbl.underline as u8) << 2);
            let shape_key = TextLabelShapeKey {
                emoji_path: emoji_fallback.is_some(),
                flavor_spans: flavor.is_some(),
                inline_face_bits,
                font_px: lbl.font_px.map(|p| p.round() as u32),
                width_px: tw,
                height_px: th,
                align: lbl.align,
                scroll_offset_px,
                rotation_quarters,
                baseline_shift_q: (lbl.baseline_shift_px * 8.0).round() as i16,
            };
            let cache_inner_key: String = if let Some(spans) = flavor {
                crate::core::relic::flavor_spans_cache_key(spans)
            } else {
                lbl.text.clone()
            };

            let rasterize = || -> Vec<u8> {
                if let Some(spans) = flavor {
                    let italic = font_italic.unwrap_or(font);
                    let floor = crate::render::theme::typography::readable_floor_px(window_h);
                    let px = lbl.font_px.unwrap_or(floor).max(floor);
                    crate::render::decal::rasterize_label_flavor_spans(
                        font,
                        italic,
                        emoji_fallback,
                        spans,
                        tw,
                        th,
                        px,
                        align,
                    )
                } else if lbl.bold || lbl.italic || lbl.underline {
                    let italic = font_italic.unwrap_or(font);
                    let floor = crate::render::theme::typography::readable_floor_px(window_h);
                    let px = lbl.font_px.unwrap_or(floor).max(floor);
                    let syn = [crate::render::decal::RasterStyleSpan {
                        text: lbl.text.as_str(),
                        bold: lbl.bold,
                        italic: lbl.italic,
                        underline: lbl.underline,
                    }];
                    crate::render::decal::rasterize_label_raster_spans(
                        font,
                        italic,
                        emoji_fallback,
                        &syn,
                        tw,
                        th,
                        px,
                        align,
                    )
                } else {
                    rasterize_label_styled_with_fallback(
                        font,
                        emoji_fallback,
                        &lbl.text,
                        tw,
                        th,
                        crate::render::decal::LabelStyle {
                            font_px: lbl.font_px,
                            align,
                            scroll_offset: lbl.scroll_offset,
                            underline: lbl.underline,
                            baseline_shift_px: lbl.baseline_shift_px,
                        },
                    )
                }
            };

            let (bind_group, owned_tex) = if cacheable {
                let inner = cache.entry(shape_key).or_default();
                if let Some(entry) = inner.get_mut(&cache_inner_key) {
                    entry.last_used = frame_id;
                    (entry.bind_group.clone(), None)
                } else {
                    let rgba = rasterize();
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
                    let bg_clone = bg.clone();
                    inner.insert(
                        cache_inner_key,
                        CachedTextLabel {
                            tex,
                            bind_group: bg,
                            last_used: frame_id,
                        },
                    );
                    (bg_clone, None)
                }
            } else {
                let rgba = rasterize();
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

            let packed_effect = if flatten_time_text_fx && lbl.text_effect.uses_time_in_fragment() {
                crate::render::text_effect::TextEffectId::Flat
            } else {
                lbl.text_effect
            };
            let inst = GpuInstance {
                rect: lbl.rect,
                color: lbl.color,
                user: packed_effect.pack_with_rotation(rotation_quarters),
            };
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("text-inst"),
                contents: bytemuck::cast_slice(&[inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            TextDraw {
                inst_buf,
                bind_group,
                scissor_rect: lbl.clip_rect,
                _tex: owned_tex,
            }
        };

        // ── Hand tile face/emoji label GPU draws (consumed by HandTileFaces) ──
        // Bump the cache frame stamp once per render() so make_text_draw can
        // mark every entry it touches. Evict entries that haven't been hit
        // for TEXT_CACHE_TTL_FRAMES — labels whose text/size has changed
        // shouldn't keep their stale GPU texture pinned forever.
        //
        // The two-level retain walks the full cache, which can balloon to a
        // few hundred entries on long sessions. The TTL is measured in many
        // frames anyway, so sweeping every frame is wasteful. Run the eviction
        // every 32 frames; the absolute TTL still bounds how long stale
        // entries can stick around.
        self.text_cache_frame = self.text_cache_frame.wrapping_add(1);
        let cache_frame_id = self.text_cache_frame;
        const EVICTION_INTERVAL_FRAMES: u64 = 32;
        if cache_frame_id.is_multiple_of(EVICTION_INTERVAL_FRAMES) {
            let ttl_cutoff = cache_frame_id.saturating_sub(TEXT_CACHE_TTL_FRAMES);
            self.text_label_cache.retain(|_, inner| {
                inner.retain(|_, entry| entry.last_used >= ttl_cutoff);
                !inner.is_empty()
            });
        }
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
                    self.ui_font_italic.as_ref(),
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
                    None,
                ));
            }
        }

        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        // Pool slices for per-frame instance vertex buffers. The
        // backing storage lives in `self.frame_buffer_pool`; each
        // `PoolSlice` is `(offset, byte_len)` into that single
        // persistent buffer. See `frame_pool.rs`.
        let mut quad_buffers: Vec<crate::render::wgpu_renderer::frame_pool::PoolSlice> = Vec::new();
        let mut gradient_quad_buffers: Vec<crate::render::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        let mut squircle_quad_buffers: Vec<crate::render::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        // TODO(perf): route flame / tile-face / prompt-icon / tile-glow /
        // relic-glow / relic-debuff buffers through `frame_buffer_pool`
        // too. They're lower-frequency than the quad batches and text
        // instance vertices, but each is still a per-frame
        // `device.create_buffer_init` that could share the bump pool.
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut tile_face_quads: Vec<TileFaceQuad> = Vec::new();
        let mut tile_face_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut prompt_icon_quads: Vec<crate::render::draw_cmd::PromptIconQuad> = Vec::new();
        let mut prompt_icon_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        let yaku_tablet_batches: Vec<&[YakuTabletPlacement]> = Vec::new();
        let wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut object3d_cmds: Vec<&[crate::render::draw_cmd::Object3d]> = Vec::new();
        let mut object3d_draw_list: Vec<(DrawKind, usize)> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();
        let mut bg_inst_buffers: Vec<crate::render::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    let ww = self.size.width.max(1) as f32;
                    let wh = self.size.height.max(1) as f32;
                    let bg_inst = GpuInstance {
                        rect: [0.0, 0.0, ww, wh],
                        color: id.image_vertex_color(),
                        user: 0,
                    };
                    let slice = self.frame_buffer_pool.alloc(
                        &self.device,
                        &self.queue,
                        std::slice::from_ref(&bg_inst),
                    );
                    let buf_idx = bg_inst_buffers.len();
                    bg_inst_buffers.push(slice);
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
                DrawCmd::HallwayEnvironment => {
                    ops.push(RenderOp::HallwayEnvironment);
                    i += 1;
                }
                DrawCmd::ArchiveEnvironment => {
                    ops.push(RenderOp::ArchiveEnvironment);
                    i += 1;
                }
                DrawCmd::ClearSceneDepth => {
                    ops.push(RenderOp::ClearSceneDepth);
                    i += 1;
                }
                DrawCmd::Quad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Quad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = quad_buffers.len();
                    quad_buffers.push(slice);
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
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = gradient_quad_buffers.len();
                    gradient_quad_buffers.push(slice);
                    ops.push(RenderOp::GradientQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::SquircleQuad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::SquircleQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = squircle_quad_buffers.len();
                    squircle_quad_buffers.push(slice);
                    ops.push(RenderOp::SquircleQuadBatch {
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
                            self.ui_font_italic.as_ref(),
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
                DrawCmd::PromptIconQuad(icon) => {
                    let key = icon.source.cache_key();
                    if self.prompt_icon_missing.contains(&key) {
                        i += 1;
                        continue;
                    }
                    if !self.prompt_icon_overlays.contains_key(&key) {
                        match make_prompt_icon_overlay_gpu(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            &icon.source,
                        ) {
                            Some(gpu) => {
                                self.prompt_icon_overlays.insert(key.clone(), gpu);
                            }
                            None => {
                                log::warn!("prompt icon missing or invalid: {key}");
                                self.prompt_icon_missing.insert(key);
                                i += 1;
                                continue;
                            }
                        }
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("prompt-icon-quad"),
                            contents: bytemuck::cast_slice(&[icon.inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let idx = prompt_icon_quads.len();
                    prompt_icon_quads.push(icon.clone());
                    prompt_icon_inst_buffers.push(buf);
                    ops.push(RenderOp::PromptIconQuad(idx));
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
            let label_size = crate::render::theme::typography::size(
                crate::render::theme::typography::H24,
                h,
            );
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
                    self.ui_font_italic.as_ref(),
                    None,
                );
                let idx = text_draws.len();
                text_draws.push(td);
                ops.push(RenderOp::TextDraw(idx));
            }
        }

        // ── Single-pass scan over `ops` to compute the flags every later
        // section needs. Replaces a handful of repeated `ops.iter().any(...)`
        // walks (table / cascade / shop / hallway / archive / SSR-relevant)
        // with one O(n) loop.
        let ops_flags = OpsFlags::scan(&ops);

        // ── Update procedural lit-mesh uniforms (table + candles) ───────
        // Written before the render pass begins, since the pass borrows
        // `self` immutably.
        if ops_flags.needs_table {
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
        }
        // Reset the debug pickable catch-all for this frame; each draw
        // loop below appends entries it wants to expose to
        // `pick_debug_object`.
        self.last_debug_pickables.clear();

        // Candles migrated to Object3dKind::Candle.

        // ── Relic placeholders (migrated to Object3dKind::Relic) ──────
        self.last_relic_models.clear();
        self.last_pickable_relic_models.clear();
        let mut relic_slot_cursor: usize = 0;
        let _ = &mut relic_slot_cursor;

        // ── Pack placeholders (same mesh + pipeline as relics) ──────────
        self.proj.pack_rects.clear();
        // Pack placements migrated to Object3dKind::Pack.

        // Auxiliary dishes migrated to Object3dKind::Dish.

        // ── Ribbon batches (shop scene) ────────────────────────────────
        // Each ribbon uses one draw slot — the whole hanging banner is a
        // single textured mesh sampling one tall portrait per zodiac.
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
            frame,
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
        // Borrow-split: `push_arrange_bbox_overlay` reads only
        // `device` / `debug_arrange_override` / `last_debug_pickables`
        // off `self`, so we hand it its dependencies directly to keep
        // `frame_buffer_pool` mutably borrowable alongside.
        Self::push_arrange_bbox_overlay(
            &self.device,
            self.debug_arrange_override.as_ref(),
            &self.last_debug_pickables,
            frame,
            &camera,
            &mut self.frame_buffer_pool,
            &self.queue,
            &mut quad_buffers,
            &mut ops,
        );

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
        let shadow_frame = self.setup_shadow_frame(&camera, shadows_enabled, frame);
        let light_view_proj_arr = shadow_frame.light_view_proj_arr;

        let shadow_just_enabled = shadows_enabled && !self.prev_frame_shadows_enabled;
        self.prev_frame_shadows_enabled = shadows_enabled;
        let mut shadow_uniforms_changed = shadow_just_enabled;

        self.write_per_instance_shadow_casters(
            frame,
            &camera,
            light_view_proj_arr,
            &tile_pick_models,
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

        if ops_flags.shop_env {
            self.write_shop_environment_uniforms(frame, &camera, false);
        }
        if ops_flags.hallway_env {
            self.write_hallway_environment_uniforms(frame, &camera, false);
        }
        if self.archive_environment.is_some() {
            self.sync_archive_description_decal_texture(frame);
        }
        if ops_flags.archive_env {
            self.write_archive_environment_uniforms(frame, &camera, false);
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
            &tile_3d_rects,
        );

        // Pass A renders into `scene_color_view`
        // (`Rgba16Float`). Do not key this on `is_prepass`: the journal
        // pre-pass still draws the 3D scene into that HDR buffer before
        // tonemapping to the journal target.
        let process_ctx_scene = ProcessOpCtx {
            frame,
            frame_pool_buffer: self.frame_buffer_pool.buffer(),
            bg_inst_buffers: &bg_inst_buffers,
            quad_buffers: &quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            squircle_quad_buffers: &squircle_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            prompt_icon_inst_buffers: &prompt_icon_inst_buffers,
            prompt_icon_quads: &prompt_icon_quads,
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
            frame_pool_buffer: self.frame_buffer_pool.buffer(),
            bg_inst_buffers: &bg_inst_buffers,
            quad_buffers: &quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            squircle_quad_buffers: &squircle_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            prompt_icon_inst_buffers: &prompt_icon_inst_buffers,
            prompt_icon_quads: &prompt_icon_quads,
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
        let cascade_active = ops_flags.cascade;
        if cascade_active {
            let cascade_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Cascade);
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
                timestamp_writes: cascade_ts,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shooting_star_cascade_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Pass A: clear + draw main scene ───────────────────────────────
        {
            #[cfg(debug_assertions)]
            let split_main_for_profile = self.gpu_profiler.is_sampling() && ops_flags.needs_table;
            #[cfg(not(debug_assertions))]
            let split_main_for_profile = self.gpu_profiler.is_sampling() && ops_flags.needs_table;

            let mut pass_a_chunks = split_pass_a_chunks(&ops);
            if pass_a_chunks.is_empty() && !ops.is_empty() {
                pass_a_chunks.push(PassAChunk {
                    ops: ops.as_slice(),
                    shop_inspect_hdr_upload: PassAShopInspectHdrUpload::None,
                });
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
                        if matches!(op, RenderOp::TextDraw(_) | RenderOp::PromptIconQuad(_)) {
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
                    for op in $chunk.ops.iter() {
                        if matches!(op, RenderOp::TextDraw(_) | RenderOp::PromptIconQuad(_)) {
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
                        match chunk.shop_inspect_hdr_upload {
                            PassAShopInspectHdrUpload::None => {}
                        }
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
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &self.depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: depth_load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
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
                        match chunk.shop_inspect_hdr_upload {
                            PassAShopInspectHdrUpload::None => {}
                        }
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
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &self.depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: depth_load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
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

        // Linear HDR bloom source for `room_glb` — emissive + bright BRDF survive
        // thresholding (bloom extract still keys off tonemapped `scene_color` elsewhere).
        let glb_room_bloom_linear = bloom_active
            && frame.uses_room_glb_shader()
            && (ops_flags.shop_env || ops_flags.hallway_env || ops_flags.archive_env);
        if glb_room_bloom_linear {
            if ops_flags.shop_env
                && self.shop_environment.is_some()
                && !self.shop_env_primitives.is_empty()
            {
                self.write_shop_environment_uniforms(frame, &camera, true);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::render::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shop-linear-bloom-pass"),
                        color_attachments: &[
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.shop_linear_bloom_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.room_emissive_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                        ],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        occlusion_query_set: None,
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_shop_environment_meshes(&mut pass, frame, true);
                }
                self.write_shop_environment_uniforms(frame, &camera, false);
            }
            if ops_flags.hallway_env
                && self.hallway_environment.is_some()
                && !self.hallway_env_primitives.is_empty()
            {
                self.write_hallway_environment_uniforms(frame, &camera, true);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::render::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("hallway-linear-bloom-pass"),
                        color_attachments: &[
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.shop_linear_bloom_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.room_emissive_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                        ],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        occlusion_query_set: None,
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_hallway_environment_meshes(&mut pass, frame, true);
                }
                self.write_hallway_environment_uniforms(frame, &camera, false);
            }
            if ops_flags.archive_env
                && self.archive_environment.is_some()
                && !self.archive_env_primitives.is_empty()
            {
                self.write_archive_environment_uniforms(frame, &camera, true);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::render::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("archive-linear-bloom-pass"),
                        color_attachments: &[
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.shop_linear_bloom_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                            Some(wgpu::RenderPassColorAttachment {
                                view: &self.room_emissive_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                        ],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        occlusion_query_set: None,
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_archive_environment_meshes(&mut pass, frame, true);
                }
                self.write_archive_environment_uniforms(frame, &camera, false);
            }
        }

        // GI compute / apply / composite are only meaningful when we have
        // a real room AABB to march through. Resolve once here so the
        // composite block below shares the same gate, dropping the
        // clear-only pass + redundant composite when there's no room
        // (loading frames, scenes that pull in `glb_room_bloom_linear`
        // before the GLB has parsed, etc.).
        let room_gi_aabb: Option<(glam::Vec3, glam::Vec3)> = if glb_room_bloom_linear
            && !is_prepass
            && effects_quality >= crate::persistence::EffectsQuality::Medium
        {
            if ops_flags.shop_env {
                crate::render::room_glb::with_shop_glb_cpu(|cpu| {
                    cpu.and_then(|c| {
                        let corners = crate::render::room_glb::room_world_bounds_corners_centered(
                            camera.h,
                            self.room_gltf_height_scale,
                            c,
                        );
                        crate::render::room_glb::room_probe_world_aabb(&corners, 0.035)
                    })
                })
            } else if ops_flags.hallway_env {
                crate::render::hallway_glb::with_hallway_glb_cpu(|cpu| {
                    cpu.and_then(|c| {
                        let corners = crate::render::room_glb::room_world_bounds_corners_centered(
                            camera.h,
                            self.room_gltf_height_scale,
                            c,
                        );
                        crate::render::room_glb::room_probe_world_aabb(&corners, 0.035)
                    })
                })
            } else if ops_flags.archive_env {
                crate::render::archive_glb::with_archive_glb_cpu(|cpu| {
                    cpu.and_then(|c| {
                        let corners = crate::render::room_glb::room_world_bounds_corners_centered(
                            camera.h,
                            self.room_gltf_height_scale,
                            c,
                        );
                        crate::render::room_glb::room_probe_world_aabb(&corners, 0.035)
                    })
                })
            } else {
                None
            }
        } else {
            None
        };
        let gi_runs_this_frame = room_gi_aabb.is_some();

        let gi_room = if gi_runs_this_frame {
            crate::render::room_gi_bake::RoomGiRoom::from_ops(
                ops_flags.shop_env,
                ops_flags.hallway_env,
                ops_flags.archive_env,
            )
        } else {
            None
        };
        let mut gi_clear_gpu_probes = !gi_runs_this_frame && self.probe_gi_had_room;
        let mut gi_baked_upload: Option<(
            crate::render::room_gi_bake::RoomGiRoom,
            std::sync::Arc<[u8]>,
        )> = None;

        // Quality-dependent GI tuning: Medium cuts dir samples and march steps to ~1/3 of
        // High and doubles the amortization interval, for roughly 6× cheaper compute.
        let gi_is_high = effects_quality >= crate::persistence::EffectsQuality::High;
        let gi_dir_samples = if gi_is_high {
            crate::render::room_glb::ROOM_EMISSIVE_PROBE_DIR_SAMPLES
        } else {
            8
        };
        let gi_march_steps = if gi_is_high {
            crate::render::room_glb::ROOM_EMISSIVE_PROBE_MARCH_STEPS
        } else {
            6
        };
        let gi_update_interval = if gi_is_high {
            crate::render::room_glb::ROOM_EMISSIVE_PROBE_UPDATE_INTERVAL
        } else {
            4
        };

        let gw = self.size.width.max(1) as f32;
        let gh = self.size.height.max(1) as f32;
        let use_baked_probes = if gi_runs_this_frame && !frame.room_gi_dynamic {
            if let (Some(room), Some((mn, mx))) = (gi_room, room_gi_aabb) {
                if let Some(bake) = crate::render::room_gi_bake::cached_room_gi_bake(room) {
                    if bake.aabb_matches(mn, mx) {
                        if self.probe_gi_gpu_room != Some(room) {
                            gi_baked_upload =
                                Some((room, std::sync::Arc::clone(&bake.probe_sh_bytes)));
                        }
                        true
                    } else {
                        gi_clear_gpu_probes = true;
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            if gi_runs_this_frame
                && frame.room_gi_dynamic
                && self.probe_gi_gpu_room.is_some()
            {
                gi_clear_gpu_probes = true;
            }
            false
        };
        if gi_clear_gpu_probes {
            self.probe_gi_gpu_room = None;
        }
        if let Some((room, bytes)) = gi_baked_upload {
            self.queue
                .write_buffer(&self.probe_sh_buffer, 0, &bytes);
            self.probe_gi_gpu_room = Some(room);
        }
        let mut gi_update_probes = if use_baked_probes {
            self.probe_gi_had_room = true;
            false
        } else {
            crate::render::room_glb::probe_gi_should_update_probes(
                &mut self.probe_gi_tick,
                &mut self.probe_gi_last_view_proj,
                &mut self.probe_gi_last_size,
                &mut self.probe_gi_had_room,
                &camera.view_proj_arr,
                (gw as u32, gh as u32),
                gi_runs_this_frame,
                gi_update_interval,
            )
        };
        if self.room_gi_capture_pending.is_some() && gi_runs_this_frame {
            gi_update_probes = true;
        }

        if gi_runs_this_frame {
            let [nx, ny, nz] = crate::render::room_glb::ROOM_EMISSIVE_PROBE_GRID;
            let probe_count = nx * ny * nz;
            debug_assert!(probe_count <= crate::render::room_glb::ROOM_EMISSIVE_PROBE_MAX);

            let inv_vp = glam::Mat4::from_cols_array(&camera.view_proj_arr).inverse();

            let (mn, mx) = room_gi_aabb.expect("gi_runs_this_frame implies Some");
            let gi = crate::render::wgpu_renderer::ProbeGiFrameUniform {
                inv_view_proj: inv_vp.to_cols_array(),
                view_proj: camera.view_proj_arr,
                world_min: [mn.x, mn.y, mn.z, 0.0],
                world_max: [mx.x, mx.y, mx.z, 0.0],
                grid_dims: [nx, ny, nz, probe_count],
                screen_march: [
                    gw,
                    gh,
                    crate::render::room_glb::ROOM_EMISSIVE_PROBE_MARCH_WORLD,
                    crate::render::room_glb::SHOP_ROOM_EMISSIVE_GI_STRENGTH,
                ],
                cam_pos: [camera.cam_pos.x, camera.cam_pos.y, camera.cam_pos.z, 1.0],
                sample_params: [gi_dir_samples, gi_march_steps, 0, 0],
            };
            self.queue.write_buffer(
                &self.probe_gi_frame_uniform_buffer,
                0,
                bytemuck::bytes_of(&gi),
            );

            if probe_count > 0 {
                if gi_update_probes {
                    let gi_compute_ts = self.gpu_profiler.compute_pass_writes(
                        crate::render::gpu_profiler::PassSlot::GiCompute,
                    );
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("emissive-probe-update-pass"),
                        timestamp_writes: gi_compute_ts,
                    });
                    cpass.set_pipeline(&self.emissive_probe_update_pipeline);
                    cpass.set_bind_group(0, &self.emissive_probe_update_bind_group, &[]);
                    let wg = probe_count.div_ceil(64);
                    cpass.dispatch_workgroups(wg, 1, 1);
                    if let Some(room) = self.room_gi_capture_pending
                        && gi_room == Some(room) {
                            let (mn, mx) = room_gi_aabb.expect("gi frame");
                            self.room_gi_capture_meta = Some(
                                crate::render::room_gi_bake::probe_sh_meta(
                                    room,
                                    mn,
                                    mx,
                                    camera.view_proj_arr,
                                    gw as u32,
                                    gh as u32,
                                ),
                            );
                        }
                }
                {
                    let gi_apply_ts = self
                        .gpu_profiler
                        .pass_writes(crate::render::gpu_profiler::PassSlot::GiApply);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("emissive-probe-apply-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.emissive_gi_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        occlusion_query_set: None,
                        timestamp_writes: gi_apply_ts,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.emissive_probe_apply_pipeline);
                    pass.set_bind_group(0, &self.emissive_probe_apply_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }

        // ── SSR snapshot ────────────────────────────────────────────────
        // After full Pass A, copy linear HDR colour + depth into
        // `scene_prev_texture` / `ssr_prev_depth_texture` for next frame's
        // lacquered-table SSR. Only the primary visible pass updates history —
        // not `output_override` prepasses (e.g. shop journal → book texture).
        //
        // At 1080p the color copy alone is ~16 MB of `Rgba16Float` per frame,
        // plus ~8 MB of depth. Skip it on scenes that won't sample SSR:
        // - prepass (`is_prepass`): journal/etc. don't feed the visible history.
        // - SSR disabled by user setting (`ssr_enabled = false`).
        // - Table material is not the lacquered wood that samples SSR; the
        //   green-felt surface doesn't read `scene_prev_texture`.
        // - No table on screen this frame at all (`!ops_flags.needs_table`).
        //
        // When SSR could be active next frame after a transition, the stale
        // copy lags one frame — the existing fallback when the camera moves.
        let ssr_writes_history = !is_prepass
            && ssr_enabled
            && surface_kind == crate::persistence::SurfaceKind::Walnut
            && ops_flags.needs_table;
        if ssr_writes_history {
            // Half-res blit replaces the old full-res
            // `copy_texture_to_texture` of `scene_color → scene_prev`.
            // `scene_prev_texture` is allocated at half size (see
            // `scene_prev_size` / `create_scene_prev`), so a fullscreen
            // triangle that samples `scene_color_view` at the half-res
            // viewport gives a 4-tap-equivalent bilinear box filter — a
            // little softer than the previous exact copy, but SSR
            // already integrates over reflection rays so the loss is
            // imperceptible. Bandwidth: ~16 MB → ~4 MB at 1080p.
            let ssr_history_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::SsrHistory);
            let mut ds_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-color-downsample-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene_prev_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: ssr_history_ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            ds_pass.set_pipeline(&self.scene_color_downsample_pipeline);
            ds_pass.set_bind_group(0, &self.scene_color_downsample_bind_group, &[]);
            ds_pass.draw(0..3, 0..1);
            drop(ds_pass);
            // Depth is still copied at full resolution (`ssr_prev_depth_texture`
            // matches `depth_texture`). The lit_mesh SSR sampler reads both
            // via normalised UVs so the size mismatch with `scene_prev` is
            // fine. A future change can downsample depth to halve this too.
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
            data1: [
                0.0,
                0.0,
                if bloom_active { 0.02 } else { 9999.0 },
                if bloom_active { 1.15 } else { 0.0 },
            ],
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
                let bloom_extract_ts = self.gpu_profiler.pass_writes(
                    crate::render::gpu_profiler::PassSlot::BloomExtract,
                );
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
                    timestamp_writes: bloom_extract_ts,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_extract_pipeline);
                pass.set_bind_group(0, &self.bloom_scene_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let bloom_blur_h_ts = self.gpu_profiler.pass_writes(
                    crate::render::gpu_profiler::PassSlot::BloomBlurH,
                );
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
                    timestamp_writes: bloom_blur_h_ts,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &self.bloom_ping_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let bloom_blur_v_ts = self.gpu_profiler.pass_writes(
                    crate::render::gpu_profiler::PassSlot::BloomBlurV,
                );
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
                    timestamp_writes: bloom_blur_v_ts,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &self.bloom_pong_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        // The scene-composite pass applies bloom + fisheye + vignette and
        // produces `post_bloom_view` for tonemap (and as an additive
        // target for GI). When all three are inactive, the pass collapses
        // to a fullscreen copy `scene_color → post_bloom`. On the Steam
        // Deck baseline that's a ~16 MB read+write per frame at 1080p
        // for nothing — skip it and have tonemap sample `scene_color`
        // directly via `tonemap_bind_group_scene`. GI's additive
        // composite still wants `post_bloom_view` as a stable target
        // (it `LoadOp::Load`s and adds), so when GI runs we keep the
        // copy.
        let skip_scene_composite = !bloom_active && fisheye_strength == 0.0 && !gi_runs_this_frame;
        if !skip_scene_composite {
            let scene_composite_ts = self.gpu_profiler.pass_writes(
                crate::render::gpu_profiler::PassSlot::SceneComposite,
            );
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
                timestamp_writes: scene_composite_ts,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_composite_pipeline);
            pass.set_bind_group(0, &self.bloom_composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if gi_runs_this_frame {
            let gi_composite_ts = self.gpu_profiler.pass_writes(
                crate::render::gpu_profiler::PassSlot::GiComposite,
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("emissive-gi-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.post_bloom_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: gi_composite_ts,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.emissive_gi_composite_pipeline);
            pass.set_bind_group(0, &self.emissive_gi_composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Journal prepass writes bloom composite into `journal_scene_texture` for the
        // book mesh to sample — keep that linear. Swapchain HDR (`Rgba16Float`) must
        // use the same ACES fitted curve as SDR or ingame colors read oversaturated.
        let tonemap_mode = if is_prepass { 1.0f32 } else { 0.0f32 };
        // Journal prepass also forces VHS off so the book-page mesh never
        // resamples a buffer with overlay artifacts baked in.
        let vhs_on_now = if is_prepass { false } else { effective_vhs_on };
        let tonemap_time = self.creation_time.elapsed().as_secs_f32();
        let grain_frame = self.film_grain_frame as f32;
        self.film_grain_frame = self.film_grain_frame.wrapping_add(1);
        self.queue.write_buffer(
            &self.tonemap_params_buffer,
            0,
            bytemuck::bytes_of(&TonemapParams {
                exposure: self.tonemap_exposure,
                mode: tonemap_mode,
                vhs_enabled: if vhs_on_now { 1.0 } else { 0.0 },
                time: tonemap_time,
                vhs_chromatic: self.tonemap_vhs_chromatic,
                vhs_scanline: self.tonemap_vhs_scanline,
                vhs_grain: self.tonemap_vhs_grain,
                vhs_vignette: self.tonemap_vhs_vignette,
                film_grain: self.tonemap_film_grain,
                grain_frame,
            }),
        );
        let tonemap_pipe = if is_prepass {
            &self.tonemap_rgba16f_pipeline
        } else {
            &self.tonemap_pipeline
        };
        {
            let tonemap_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Tonemap);
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
                timestamp_writes: tonemap_ts,
                multiview_mask: None,
            });
            pass.set_pipeline(tonemap_pipe);
            // When the scene-composite pass was skipped, `post_bloom_view`
            // holds whatever the previous frame left in it — sample
            // `scene_color_view` directly instead.
            let tonemap_bg = if skip_scene_composite {
                &self.tonemap_bind_group_scene
            } else {
                &self.tonemap_bind_group
            };
            pass.set_bind_group(0, tonemap_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Overlay pass: 2D HUD text labels ────────────────────────────
        // After tonemap, Load the final target so labels are not in the linear
        // HDR `scene_prev_texture` used for lacquered-table SSR.
        if ops
            .iter()
            .any(|o| matches!(o, RenderOp::TextDraw(_) | RenderOp::PromptIconQuad(_)))
        {
            let overlay_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Overlay);
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
                timestamp_writes: overlay_ts,
                multiview_mask: None,
            });
            for op in &ops {
                if matches!(op, RenderOp::TextDraw(_) | RenderOp::PromptIconQuad(_)) {
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
        let mut screenshot_path = if is_prepass {
            None
        } else {
            self.pending_screenshot.take()
        };
        if let Some(ref path) = screenshot_path
            && matches!(self.target, RenderTarget::Surface(_))
                && !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
            {
                log::warn!(
                    "screenshot skipped ({}): swapchain has no COPY_SRC (see MAHJURO_VULKAN_WIN_SURFACE_COPY on Windows Vulkan)",
                    path.display()
                );
                screenshot_path = None;
            }
        let screenshot_staging = match (&screenshot_path, &frame_texture_opt) {
            (Some(path), Some(ft)) => {
                log::debug!("screenshot: encoding capture for {}", path.display());
                Some(self.encode_screenshot_copy(&mut encoder, ft, path))
            }
            _ => None,
        };
        let room_gi_capture_staging = self
            .room_gi_capture_meta
            .take()
            .map(|meta| self.encode_room_gi_capture_copy(&mut encoder, meta));

        self.queue.submit(std::iter::once(encoder.finish()));

        if let (Some(path), Some(staging)) = (screenshot_path, screenshot_staging) {
            match self.finalize_screenshot(staging, &path) {
                Ok(()) => log::info!("screenshot saved → {}", path.display()),
                Err(e) => log::error!("screenshot finalize failed: {e:?}"),
            }
        }
        if let Some(staging) = room_gi_capture_staging {
            match self.finalize_room_gi_capture(staging) {
                Ok(bake) => {
                    self.room_gi_captured = Some(bake);
                    self.room_gi_capture_pending = None;
                }
                Err(e) => log::error!("room GI capture readback failed: {e:?}"),
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
        // Reuse a per-renderer scratch set to avoid a fresh `HashSet` allocation
        // every frame. Cleared at the start of each call; the underlying capacity
        // sticks around between frames.
        self.tile_uid_scratch.clear();
        self.tile_uid_scratch.extend(self.tile_uids.iter().copied());
        let live = &self.tile_uid_scratch;
        self.prev_tile_world.retain(|k, _| live.contains(k));
    }
}
