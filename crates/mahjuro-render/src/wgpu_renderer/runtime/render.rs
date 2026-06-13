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
    shop_env: bool,
    hallway_env: bool,
    staircase_env: bool,
    archive_env: bool,
    main_menu_env: bool,
    gameplay_env: bool,
    shadow_test_env: bool,
    cascade: bool,
}

impl OpsFlags {
    fn scan(ops: &[RenderOp]) -> Self {
        let mut f = OpsFlags::default();
        for op in ops {
            match op {
                RenderOp::ShopEnvironment => f.shop_env = true,
                RenderOp::HallwayEnvironment => f.hallway_env = true,
                RenderOp::StaircaseEnvironment => f.staircase_env = true,
                RenderOp::ArchiveEnvironment => f.archive_env = true,
                RenderOp::MainMenuEnvironment => f.main_menu_env = true,
                RenderOp::GameplayEnvironment => f.gameplay_env = true,
                RenderOp::ShadowTestEnvironment => f.shadow_test_env = true,
                RenderOp::ShootingStarCascade => f.cascade = true,
                _ => {}
            }
            // Early-out: if every flag we care about is set, stop scanning.
            if f.shop_env
                && f.hallway_env
                && f.staircase_env
                && f.archive_env
                && f.main_menu_env
                && f.gameplay_env
                && f.shadow_test_env
                && f.cascade
            {
                break;
            }
        }
        f
    }
}

/// One contiguous slice of [`RenderOp`]s inside Pass A. `shop_inspect_hdr_upload` selects an
/// optional [`LitMeshFrameGlobals.hdr_tonemap`] upload **before** this slice's render pass.
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
        let split = matches!(op, RenderOp::ClearSceneDepth);
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
    /// before the swapchain path begins.
    pub fn render_to(
        &mut self,
        frame: &UiFrame,
        settings: RenderSettings,
        output_override: Option<&wgpu::TextureView>,
    ) -> anyhow::Result<()> {
        let RenderSettings {
            effects_quality,
            cascade_effects_quality,
            tile_preset,
            tile_material,
            tileset_name,
            draw_settle_speed,
            sort_settle_speed,
            gamma,
            shadow_quality,
            vhs_enabled,
        } = settings;
        self.shadow_quality = shadow_quality;
        let flatten_time_text_fx = matches!(
            effects_quality,
            mahjuro_gfx_types::EffectsQuality::Off | mahjuro_gfx_types::EffectsQuality::Low
        );
        // Master Options-toggle override: when the user has VHS off we kill
        // the per-scene branch even if the per-scene tuning has non-zero
        // amplitudes. Computed locally — `self.tonemap_vhs_enabled` keeps
        // the per-scene resolution so re-enabling restores the look.
        let effective_vhs_on = self.tonemap_vhs_enabled && vhs_enabled;
        self.apply_render_settings(tile_material, effects_quality, &tileset_name);

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

        // Lerp per-tile slide animations toward 0 (ease-out).
        let dt = self.advance_frame_timers(draw_settle_speed, sort_settle_speed);

        self.upload_frame_uniforms(frame, effects_quality, cascade_effects_quality, gamma);

        // Build 2D backdrop quads (selection borders, hint pulses) and text
        // labels (just the focused arrow — the symbol+emoji live in the 3D
        // tile decal now).  Per-tile model matrices for the 3D mesh draw are
        // also written here.
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
        let mut glyph_popup_glows: Vec<GpuInstance> = Vec::new();
        let mut relic_debuff_markers: Vec<GpuInstance> = Vec::new();

        // ── Person-at-the-table camera ──────────────────────────────────
        // Z-up world, standard right-hand conventions: +X right, +Y into the
        // table (away from player), +Z up from the felt. Table is z = 0 (XY).
        // Camera sits at large -Y (behind the player), elevated in +Z, looking
        // toward +Y. See [`crate::world_space::pixel_to_world`] for the
        // pixel → world mapping:
        //
        //   world_x =  pixel_x - w * 0.5       (screen-right → +X)
        //   world_y =  h * 0.5 - pixel_y       (screen-bottom → -Y, toward player)
        //   world_z =  lift above the felt
        //
        // The 2D UI overlays (score panel, buttons, text) keep using the
        // pixel-orthographic quad pipeline and float over the 3D scene as
        // a HUD.
        // Camera / room GLB use window-space layout (`layout.window_*`); only the
        // offscreen targets are allocated at `render_size`.
        let camera = CameraFrame::build(frame, self.size);
        let showcase_camera = if frame.camera_override_after_depth_clear.is_some() {
            CameraFrame::build_from(frame.foreground_camera(), frame, self.size)
        } else {
            camera
        };
        self.upload_camera_uniforms(&camera, frame);
        self.pass_a_draw_camera = Some(camera);
        self.pass_a_frame_gamma = gamma;
        let look_target = camera.look_target;
        let w = camera.w;
        let h = camera.h;
        let project_to_screen =
            |world: glam::Vec3| -> (f32, f32) { camera.project_to_screen(world) };

        // ── Debug axes overlay ──────────────────────────────────────────
        self.write_debug_axes_uniforms(frame, &camera);
        self.write_debug_rain_hit_uniforms(frame, &camera);

        // ── Flame emitters (world-space) ─────────────────────────────
        let flame_emitters = build_flame_emitters(frame, w, h);

        let tile_basis = tile_mesh_local_to_world();
        // Hand tiles render via ShowcaseTileBatch (further below); tile hints
        // come through as real green PointLights from gameplay scene.

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
                              font_mono: Option<&fontdue::Font>,
                              emoji_fallback: Option<&fontdue::Font>|
         -> TextDraw {
            // Clamp before casting: `f32 as u32` saturates negatives/NaN to u32::MAX,
            // which blows past wgpu's 16384 texture limit and panics when too many unique textures load at once
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
            let face_font = if lbl.mono {
                font_mono.unwrap_or(font)
            } else {
                font
            };
            let face_italic = if lbl.mono {
                font_mono.unwrap_or(font_italic.unwrap_or(font))
            } else {
                font_italic.unwrap_or(font)
            };
            let vertical_align = match lbl.block_vertical_align {
                super::internal_slots::TextBlockVerticalAlign::Top => {
                    crate::decal::LabelVerticalAlign::Top
                }
                super::internal_slots::TextBlockVerticalAlign::Bottom => {
                    crate::decal::LabelVerticalAlign::Bottom
                }
            };
            let shape_key = TextLabelShapeKey {
                mono: lbl.mono,
                emoji_path: emoji_fallback.is_some(),
                flavor_spans: flavor.is_some(),
                inline_face_bits,
                font_px: lbl.font_px.map(|p| p.round() as u32),
                width_px: tw,
                height_px: th,
                align: lbl.align,
                block_vertical_align: lbl.block_vertical_align,
                scroll_offset_px,
                rotation_quarters,
                baseline_shift_q: (lbl.baseline_shift_px * 8.0).round() as i16,
            };
            let cache_inner_key: String = if let Some(spans) = flavor {
                mahjuro_core::core::relic::flavor_spans_cache_key(spans)
            } else {
                lbl.text.clone()
            };

            let rasterize = || -> Vec<u8> {
                if let Some(spans) = flavor {
                    let floor = crate::theme::typography::readable_floor_px(window_h);
                    let target = lbl.font_px.unwrap_or(floor).max(floor);
                    let px = crate::decal::resolve_flavor_spans_font_px(
                        &crate::decal::DecalFonts {
                            regular: face_font,
                            italic: Some(face_italic),
                            emoji: emoji_fallback,
                        },
                        spans,
                        tw,
                        th,
                        target,
                        floor,
                    );
                    crate::decal::rasterize_label_flavor_spans(
                        &crate::decal::DecalFonts {
                            regular: face_font,
                            italic: Some(face_italic),
                            emoji: emoji_fallback,
                        },
                        spans,
                        &crate::decal::LabelRasterParams {
                            width: tw,
                            height: th,
                            font_px: px,
                            align,
                            vertical_align,
                        },
                    )
                } else if lbl.bold || lbl.italic || lbl.underline {
                    let floor = crate::theme::typography::readable_floor_px(window_h);
                    let syn = [crate::decal::RasterStyleSpan {
                        text: lbl.text.as_str(),
                        bold: lbl.bold,
                        italic: lbl.italic,
                        underline: lbl.underline,
                    }];
                    let px = crate::decal::resolve_raster_spans_font_px(
                        &crate::decal::DecalFonts {
                            regular: face_font,
                            italic: Some(face_italic),
                            emoji: emoji_fallback,
                        },
                        &syn,
                        tw,
                        th,
                        lbl.font_px,
                        floor,
                    );
                    crate::decal::rasterize_label_raster_spans(
                        &crate::decal::DecalFonts {
                            regular: face_font,
                            italic: Some(face_italic),
                            emoji: emoji_fallback,
                        },
                        &syn,
                        &crate::decal::LabelRasterParams {
                            width: tw,
                            height: th,
                            font_px: px,
                            align,
                            vertical_align,
                        },
                    )
                } else {
                    rasterize_label_styled_with_fallback(
                        face_font,
                        emoji_fallback,
                        &lbl.text,
                        tw,
                        th,
                        crate::decal::LabelStyle {
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
                            _tex: tex,
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

            let packed_effect =
                if flatten_time_text_fx && lbl.text_effect.flattened_when_effects_low() {
                    crate::text_effect::TextEffectId::Flat
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
        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        // Pool slices for per-frame instance vertex buffers. The
        // backing storage lives in `self.frame_buffer_pool`; each
        // `PoolSlice` is `(offset, byte_len)` into that single
        // persistent buffer. See `frame_pool.rs`.
        let mut quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> = Vec::new();
        let mut depth_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> = Vec::new();
        let mut overlay_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> = Vec::new();
        let mut debug_overlay_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        let mut overlay_squircle_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        let mut gradient_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        let mut arc_ring_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        let mut squircle_quad_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> =
            Vec::new();
        // TODO(perf): route flame / tile-face / prompt-icon / tile-glow /
        // relic-glow / relic-debuff buffers through `frame_buffer_pool`
        // too. They're lower-frequency than the quad batches and text
        // instance vertices, but each is still a per-frame
        // `device.create_buffer_init` that could share the bump pool.
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut debug_text_draws: Vec<TextDraw> = Vec::new();
        let mut debug_ops: Vec<RenderOp> = Vec::new();
        let mut tile_face_quads: Vec<TileFaceQuad> = Vec::new();
        let mut tile_face_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut image_quads: Vec<crate::draw_cmd::ImageQuad> = Vec::new();
        let mut image_quad_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        let yaku_tablet_batches: Vec<&[YakuTabletPlacement]> = Vec::new();
        let wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut showcase_tile_batch_clips: Vec<Option<[f32; 4]>> = Vec::new();
        let mut object3d_cmds: Vec<&[crate::draw_cmd::Object3d]> = Vec::new();
        let mut object3d_draw_list: Vec<(DrawKind, usize)> = Vec::new();
        let mut object3d_shadow_draw_list: Vec<(DrawKind, usize)> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();
        let mut bg_inst_buffers: Vec<crate::wgpu_renderer::frame_pool::PoolSlice> = Vec::new();

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
                    if effects_quality >= mahjuro_gfx_types::EffectsQuality::Medium {
                        ops.push(RenderOp::Starfield);
                    }
                    i += 1;
                }
                DrawCmd::GoldenDust => {
                    if effects_quality >= mahjuro_gfx_types::EffectsQuality::Medium {
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
                    if effects_quality >= mahjuro_gfx_types::EffectsQuality::Low {
                        ops.push(RenderOp::ShootingStarCascade);
                    }
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
                DrawCmd::StaircaseEnvironment => {
                    ops.push(RenderOp::StaircaseEnvironment);
                    i += 1;
                }
                DrawCmd::ArchiveEnvironment => {
                    ops.push(RenderOp::ArchiveEnvironment);
                    i += 1;
                }
                DrawCmd::MainMenuEnvironment => {
                    ops.push(RenderOp::MainMenuEnvironment);
                    i += 1;
                }
                DrawCmd::GameplayEnvironment => {
                    ops.push(RenderOp::GameplayEnvironment);
                    i += 1;
                }
                DrawCmd::ShadowTestEnvironment => {
                    ops.push(RenderOp::ShadowTestEnvironment);
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
                DrawCmd::DepthQuad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::DepthQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = depth_quad_buffers.len();
                    depth_quad_buffers.push(slice);
                    ops.push(RenderOp::DepthQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::OverlayQuad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::OverlayQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = overlay_quad_buffers.len();
                    overlay_quad_buffers.push(slice);
                    ops.push(RenderOp::OverlayQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::OverlaySquircleQuad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::OverlaySquircleQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = overlay_squircle_quad_buffers.len();
                    overlay_squircle_quad_buffers.push(slice);
                    ops.push(RenderOp::OverlaySquircleQuadBatch {
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
                DrawCmd::ArcRingQuad(_) => {
                    let mut batch: Vec<ArcRingQuadInstance> = Vec::new();
                    while let Some(DrawCmd::ArcRingQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = arc_ring_quad_buffers.len();
                    arc_ring_quad_buffers.push(slice);
                    ops.push(RenderOp::ArcRingQuadBatch {
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
                DrawCmd::Flame => {
                    while let Some(DrawCmd::Flame) = frame.cmds.get(i) {
                        i += 1;
                    }
                    // One Godot-style volume mesh per candle emitter.
                    let count = crate::flame_volume::fill_gpu_instances(
                        &flame_emitters,
                        &mut self.flame_instance_staging,
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
                                        &self.flame_instance_staging[..count],
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
                            self.mono_font.as_ref(),
                            self.emoji_font.as_ref(),
                        );
                        let idx = text_draws.len();
                        text_draws.push(td);
                        ops.push(RenderOp::TextDraw(idx));
                    }
                    i += 1;
                }
                DrawCmd::DebugOverlayQuad(_) => {
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::DebugOverlayQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let slice = self
                        .frame_buffer_pool
                        .alloc(&self.device, &self.queue, &batch);
                    let buf_idx = debug_overlay_quad_buffers.len();
                    debug_overlay_quad_buffers.push(slice);
                    debug_ops.push(RenderOp::OverlayQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::DebugOverlayText(lbl) => {
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
                            self.mono_font.as_ref(),
                            self.emoji_font.as_ref(),
                        );
                        let idx = debug_text_draws.len();
                        debug_text_draws.push(td);
                        debug_ops.push(RenderOp::TextDraw(idx));
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
                        let overlay = make_tile_face_overlay_gpu(&TileFaceOverlayGpuParams {
                            device: &self.device,
                            queue: &self.queue,
                            layout: &self.text_bind_group_layout,
                            sampler: &self.tile_sampler,
                            ui_font: self.ui_font.as_ref(),
                            emoji_font: self.emoji_font.as_ref(),
                            tile: &face.tile,
                            tile_set: self.tile_set.as_deref(),
                        });
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
                DrawCmd::ImageQuad(quad) => {
                    let key = quad.source.cache_key();
                    if self.image_quad_missing.contains(&key) {
                        i += 1;
                        continue;
                    }
                    if !self.image_quad_overlays.contains_key(&key) {
                        match make_image_quad_overlay_gpu(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            &quad.source,
                        ) {
                            Some(gpu) => {
                                self.image_quad_overlays.insert(key.clone(), gpu);
                            }
                            None => {
                                log::warn!("image quad missing or invalid: {key}");
                                self.image_quad_missing.insert(key);
                                i += 1;
                                continue;
                            }
                        }
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("image-quad-inst"),
                            contents: bytemuck::cast_slice(&[quad.inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let idx = image_quads.len();
                    image_quads.push(quad.clone());
                    image_quad_inst_buffers.push(buf);
                    ops.push(RenderOp::ImageQuad(idx));
                    i += 1;
                }
                DrawCmd::ShowcaseTileBatch(batch) => {
                    let idx = showcase_tile_batches.len();
                    showcase_tile_batches.push(batch.placements.as_slice());
                    showcase_tile_batch_clips.push(batch.clip_rect);
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
            let label_size = crate::theme::typography::size(crate::theme::typography::H24, h);
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
                    self.mono_font.as_ref(),
                    None,
                );
                let idx = text_draws.len();
                text_draws.push(td);
                ops.push(RenderOp::TextDraw(idx));
            }
        }

        // ── Single-pass scan over `ops` to compute the flags every later
        // section needs. Replaces a handful of repeated `ops.iter().any(...)`
        // walks (table / cascade / shop / hallway / archive)
        // with one O(n) loop.
        let ops_flags = OpsFlags::scan(&ops);
        self.ensure_room_gpu_for_draw_cmds(&frame.cmds);

        // Reset the debug pickable catch-all for this frame; each draw
        // loop below appends entries it wants to expose to

        // Candles migrated to Object3dKind::Candle.

        // ── Relic placeholders (migrated to Object3dKind::Relic) ──────
        self.last_relic_models.clear();
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

        self.upload_active_room_baked_shadow_globals(frame);
        // Offline `.msh` capture needs a real punctual shadow frustum and a fixed
        // map size even when live settings have shadows Off (Performance / LowMemory).
        let shadow_quality_for_pass = if self.room_shadow_capture_pending.is_some() {
            crate::room_shadow_bake::ROOM_SHADOW_BAKE_CAPTURE_QUALITY
        } else {
            shadow_quality
        };
        if self.recreate_shadow_depth_arrays_if_needed(shadow_quality_for_pass) {
            self.cached_shadow_light_view_proj = [0.0; 16];
        }
        let projected_frame =
            self.prepare_projected_shadow_frame(frame, &camera, shadow_quality_for_pass);
        let active_room_env = super::shadow_setup::ActiveRoomEnv::from_frame(frame).or_else(|| {
            self.active_scene_key
                .and_then(super::shadow_setup::ActiveRoomEnv::from_scene_key)
        });
        let light_view_proj_arr = projected_frame.first_light_view_proj;
        let contact_ao_active = shadow_quality.contact_ao()
            && (self.active_lab_baked_shadow || self.active_room_baked_shadow.is_some());
        let contact_ao_view_proj = if self.active_lab_baked_shadow {
            self.lab_baked_shadow
                .as_ref()
                .map(|(_, gpu)| gpu.baked_light_view_proj)
                .unwrap_or([0.0; 16])
        } else if let Some(room) = self.active_room_baked_shadow {
            self.room_baked_shadow_gpu[crate::room_gi_bake::room_gi_room_index(room)]
                .as_ref()
                .map(|gpu| gpu.baked_light_view_proj)
                .unwrap_or([0.0; 16])
        } else {
            [0.0; 16]
        };
        let shadow_just_enabled = shadow_quality.active() && !self.prev_shadow_quality.active();
        let shadow_quality_changed = shadow_quality != self.prev_shadow_quality;
        self.prev_shadow_quality = shadow_quality;
        let shadow_light_changed = self.cached_shadow_light_view_proj != light_view_proj_arr;
        self.cached_shadow_light_view_proj = light_view_proj_arr;
        let mut shadow_uniforms_changed = shadow_just_enabled
            || shadow_quality_changed
            || shadow_light_changed
            || projected_frame.changed;
        let mut object3d_shadow =
            shadow_quality
                .active()
                .then_some(super::shadow_setup::Object3dShadowCtx {
                    light_view_proj: light_view_proj_arr,
                    changed: &mut shadow_uniforms_changed,
                });

        self.last_gameplay_cash_in_button_visible = frame.gameplay_cash_in_button_visible;

        if let Some(ref picks) = frame.gameplay_action_picks {
            self.seed_gameplay_action_pick_proxies(&camera, picks);
        }

        self.run_object3d_placement(
            frame,
            &camera,
            &object3d_cmds,
            &wall_stack_cmds,
            &mut object3d_draw_list,
            &mut object3d_shadow_draw_list,
            &mut ops,
            &mut relic_glows,
            &mut glyph_popup_glows,
            &mut relic_debuff_markers,
            object3d_shadow.as_mut(),
        );

        if !relic_debuff_markers.is_empty() && self.debuff_marker_overlay.is_none() {
            self.debuff_marker_overlay = Some(super::super::make_debuff_marker_overlay_gpu(
                &self.device,
                &self.queue,
                &self.text_bind_group_layout,
                &self.tile_sampler,
            ));
        }

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
        let glyph_popup_glow_buffer = if glyph_popup_glows.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("glyph-popup-glow-instances"),
                        contents: bytemuck::cast_slice(&glyph_popup_glows),
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

        self.run_showcase_tiles_placement(
            frame,
            &showcase_camera,
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
        if self
            .tile_3d_batch_blend_ranges
            .iter()
            .any(|(_, count)| *count > 0)
            && let Some(pos) = ops
                .iter()
                .rposition(|op| matches!(op, RenderOp::ShowcaseTileBatch(_)))
        {
            ops.insert(pos + 1, RenderOp::ShowcaseTileTranslucent);
        }

        let dynamic_receiver_shadow_strength = if shadow_quality.active()
            && active_room_env == Some(super::shadow_setup::ActiveRoomEnv::Shop)
            && !object3d_shadow_draw_list.is_empty()
            && active_room_env.is_some_and(|env| {
                super::shadow_setup::skip_room_env_live_shadow_pass(
                    env,
                    self.active_room_baked_shadow,
                )
            }) {
            0.55
        } else {
            0.0
        };
        self.write_active_room_baked_shadow_globals(
            shadow_quality,
            &projected_frame.build,
            camera.h,
            contact_ao_active,
            dynamic_receiver_shadow_strength,
        );
        self.upload_projected_shadow_globals(
            shadow_quality,
            &projected_frame.build,
            contact_ao_active,
            contact_ao_view_proj,
            camera.h,
            dynamic_receiver_shadow_strength,
        );
        let room_shadow_active = shadow_quality.active();
        let offline_room_baked_loaded = self.active_room_baked_shadow.is_some();
        // #region agent log
        {
            use super::{
                agent_shadow_log, probe_baked_ao_at_world, probe_baked_ao_at_world_scaled,
                shadow_caster_z_up_probe,
            };
            use crate::hallway_glb::with_hallway_glb_cpu;
            use crate::projected_light_shadow::punctual_light_world;
            use crate::room_gi_bake::room_gi_room_index;
            use crate::room_glb::room_env_world_scale;
            let skip_live = active_room_env.is_some_and(|env| {
                super::shadow_setup::skip_room_env_live_shadow_pass(
                    env,
                    self.active_room_baked_shadow,
                )
            });
            let ao_scale = if self.active_lab_baked_shadow {
                crate::shadow_ao_lab::CONTACT_AO_WORLD_SCALE
            } else if contact_ao_active {
                crate::room_shadow_bake::contact_ao_world_scale_ratio(camera.h)
            } else {
                1.0
            };
            let mut ao_probe = serde_json::Value::Null;
            let mut hallway_far_wall_probe = serde_json::Value::Null;
            if let (Some(room), Some(gpu)) = (
                self.active_room_baked_shadow,
                self.active_room_baked_shadow
                    .and_then(|room| self.room_baked_shadow_gpu[room_gi_room_index(room)].as_ref()),
            ) {
                if let Ok(bake) = crate::room_shadow_bake::require_room_shadow_bake(room) {
                    if let Some(ao) = bake.ao_bytes.as_ref() {
                        ao_probe = serde_json::json!({
                            "room": format!("{room:?}"),
                            "origin_ndc_uv_ao": probe_baked_ao_at_world(
                                gpu.baked_light_view_proj,
                                ao,
                                bake.width,
                                bake.height,
                                glam::Vec3::ZERO,
                            )
                            .map(|(ndc, uv, a)| serde_json::json!({
                                "ndc": ndc.to_array(),
                                "uv": uv,
                                "ao": a,
                            })),
                        });
                        if active_room_env == Some(super::shadow_setup::ActiveRoomEnv::Hallway) {
                            let height = self.env_tune_for(crate::scene_keys::HALLWAY).height_scale;
                            let far_wall_world = with_hallway_glb_cpu(|cpu| {
                                let cpu = cpu?;
                                let bounds = cpu.environment_bounds_doc?;
                                let center = bounds.center();
                                let s = room_env_world_scale(camera.h, height);
                                let doc = glam::Vec3::new(
                                    center.x,
                                    bounds.max.y - 0.05,
                                    bounds.min.z + (bounds.max.z - bounds.min.z) * 0.45,
                                );
                                Some((doc - center) * s)
                            });
                            if let Some(world) = far_wall_world {
                                hallway_far_wall_probe = probe_baked_ao_at_world_scaled(
                                    gpu.baked_light_view_proj,
                                    Some(&bake.depth_bytes),
                                    ao,
                                    bake.width,
                                    bake.height,
                                    world,
                                    ao_scale,
                                )
                                .map(|p| {
                                    serde_json::json!({
                                        "world": world.to_array(),
                                        "ndc": p.ndc.to_array(),
                                        "uv": p.uv,
                                        "ao": p.ao,
                                        "baked_depth": p.baked_depth,
                                        "depth_delta": p.depth_delta,
                                        "ao_would_apply_pre_fix": p.ao < 128,
                                        "ao_would_apply_post_fix": p.ao_would_apply,
                                    })
                                })
                                .unwrap_or(serde_json::Value::Null);
                            }
                        }
                    }
                }
            }
            let hallway_z_up = if active_room_env
                == Some(super::shadow_setup::ActiveRoomEnv::Hallway)
                && let Some(caster) = projected_frame.casters().first()
            {
                let i = caster.source_light_index as usize;
                let light_world = frame
                    .scene_lighting
                    .punctual
                    .get(i)
                    .map(|entry| {
                        punctual_light_world(
                            camera.w,
                            camera.h,
                            entry,
                            frame.camera_override.as_ref(),
                            frame
                                .showcase_render_hints
                                .layout_uses_ray_plane(self.active_scene_key),
                        )
                    })
                    .unwrap_or(glam::Vec3::ZERO);
                Some(shadow_caster_z_up_probe(light_world, glam::Vec3::ZERO))
            } else {
                None
            };
            agent_shadow_log(
                "H1-H5",
                "render.rs:shadow_frame",
                "shadow frame state",
                serde_json::json!({
                    "shadow_quality": shadow_quality.label(),
                    "shadow_active": shadow_quality.active(),
                    "contact_ao_active": contact_ao_active,
                    "offline_baked_loaded": offline_room_baked_loaded,
                    "skip_room_env_live": skip_live,
                    "room_glb_brdf": frame.uses_room_glb_shader(),
                    "punctual_count": frame.scene_lighting.punctual.len(),
                    "caster_count": projected_frame.casters().len(),
                    "dynamic_receiver_shadow_strength": dynamic_receiver_shadow_strength,
                    "active_env": active_room_env.map(|e| format!("{e:?}")),
                    "scene_key": self.active_scene_key,
                    "camera_h": camera.h,
                    "ao_world_scale": ao_scale,
                    "ao_probe": ao_probe,
                    "hallway_far_wall_probe": hallway_far_wall_probe,
                    "hallway_z_up_probe": hallway_z_up,
                }),
            );
            if let Some(layout) = frame.shadow_ao_lab_layout {
                let hdr = self.tile_hdr_tonemap(frame);
                let bake = crate::shadow_ao_lab::synthetic_bake(layout);
                let mid = glam::Vec3::new(0.0, 5950.0, 1200.0);
                let light_w = crate::shadow_ao_lab::light_world();
                let ao_mid = crate::room_shadow_bake::probe_contact_ao_at_world(
                    &bake,
                    mid,
                    crate::shadow_ao_lab::CONTACT_AO_WORLD_SCALE,
                );
                let punctual_kind = frame.scene_lighting.punctual.first().map(|e| match e {
                    crate::draw_cmd::ScenePunctualLight::Smooth(_) => "smooth",
                    crate::draw_cmd::ScenePunctualLight::SmoothNoShadow(_) => "smooth_no_shadow",
                    crate::draw_cmd::ScenePunctualLight::InverseSquare(_) => "inverse_square",
                });
                let synth_lvp = crate::shadow_ao_lab::punctual_light_view_proj(layout);
                let live_lvp = projected_frame
                    .build
                    .casters
                    .first()
                    .map(|c| c.light_view_proj);
                let lvp_max_delta = live_lvp.map(|m| {
                    let a = m.to_cols_array();
                    let b = synth_lvp.to_cols_array();
                    a.iter()
                        .zip(b.iter())
                        .map(|(x, y)| (x - y).abs())
                        .fold(0.0f32, f32::max)
                });
                let back_mid_ndc = live_lvp.map(|m| {
                    let clip = m * mid.extend(1.0);
                    (clip.truncate() / clip.w).to_array()
                });
                agent_shadow_log(
                    "LAB-S1-S3",
                    "render.rs:shadow_ao_lab",
                    "lab shadow diagnostics",
                    serde_json::json!({
                        "layout": format!("{layout:?}"),
                        "hdr_tonemap": hdr,
                        "punctual_kind": punctual_kind,
                        "light_world": light_w.to_array(),
                        "shadow_look_at": crate::shadow_ao_lab::punctual_shadow_look_at().to_array(),
                        "lvp_max_delta_vs_synthetic": lvp_max_delta,
                        "back_mid_ndc_live_lvp": back_mid_ndc,
                        "shadow_caster_draws": object3d_shadow_draw_list.len(),
                        "contact_ao_active": contact_ao_active,
                        "active_lab_baked": self.active_lab_baked_shadow,
                        "object3d_cmd_count": object3d_cmds.len(),
                        "back_mid_ao_probe": ao_mid.map(|p| serde_json::json!({
                            "ndc": p.ndc,
                            "uv": p.uv,
                            "ao": p.ao,
                            "applies": p.applies,
                        })),
                    }),
                );
            }
        }
        // #endregion
        macro_rules! room_env_shadow_upload {
            ($env:expr) => {
                super::shadow_setup::room_env_shadow_upload_active(
                    room_shadow_active,
                    super::shadow_setup::skip_room_env_live_shadow_pass(
                        $env,
                        self.active_room_baked_shadow,
                    ),
                )
                .then_some((light_view_proj_arr, &mut shadow_uniforms_changed))
            };
        }
        if ops_flags.shop_env {
            self.write_shop_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::Shop),
            );
            if frame.scene_lighting.embedded_gltf_punctual {
                self.write_shop_room_punctual_occluders(&camera);
            }
        }
        if ops_flags.hallway_env {
            self.write_hallway_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::Hallway),
            );
        }
        if ops_flags.staircase_env {
            self.write_staircase_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::Stairway),
            );
        }
        if ops_flags.shadow_test_env {
            self.write_shadow_test_room_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::ShadowTest),
            );
        }
        if self.archive_environment.is_some() {
            self.sync_archive_description_decal_texture(frame);
        }
        if ops_flags.archive_env {
            self.write_archive_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::Archive),
            );
        }
        if ops_flags.main_menu_env {
            self.write_main_menu_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::MainMenu),
            );
            if frame.scene_lighting.embedded_gltf_punctual {
                self.write_main_menu_room_punctual_occluders(&camera);
            }
        }
        if ops_flags.gameplay_env && frame.gameplay_cash_in_overlay_camera.is_none() {
            self.write_gameplay_environment_uniforms(
                frame,
                &camera,
                false,
                room_env_shadow_upload!(super::shadow_setup::ActiveRoomEnv::Gameplay),
            );
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        self.render_shadow_pre_pass(
            &mut encoder,
            frame,
            camera.h,
            shadow_quality,
            shadow_uniforms_changed,
            &projected_frame.casters(),
            &object3d_shadow_draw_list,
            &showcase_tile_batches,
            &tile_3d_rects,
            &tile_pick_models,
        );
        let shadow_probe_staging = self.schedule_shadow_probe_copy(
            &mut encoder,
            &projected_frame.build,
            frame.scene_lighting.punctual.len(),
            active_room_env,
        );
        let room_shadow_capture_staging = self.room_shadow_capture_pending.map(|room| {
            const BIAS: f32 = 0.005;
            let size = crate::room_shadow_bake::ROOM_SHADOW_BAKE_MAP_SIZE;
            self.encode_room_shadow_capture_copy(
                &mut encoder,
                frame,
                room,
                size,
                size,
                light_view_proj_arr,
                BIAS,
                camera.h,
            )
        });

        // Pass A renders into `scene_color_view`
        // (`Rgba16Float`). Do not key this on `is_prepass`: the journal
        // pre-pass still draws the 3D scene into that HDR buffer before
        // tonemapping to the journal target.
        let process_ctx_scene = ProcessOpCtx {
            frame,
            frame_pool_buffer: self.frame_buffer_pool.buffer(),
            bg_inst_buffers: &bg_inst_buffers,
            quad_buffers: &quad_buffers,
            depth_quad_buffers: &depth_quad_buffers,
            overlay_quad_buffers: &overlay_quad_buffers,
            overlay_squircle_quad_buffers: &overlay_squircle_quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            arc_ring_quad_buffers: &arc_ring_quad_buffers,
            squircle_quad_buffers: &squircle_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            image_quad_inst_buffers: &image_quad_inst_buffers,
            image_quads: &image_quads,
            object3d_draw_list: &object3d_draw_list,
            showcase_tile_batches: &showcase_tile_batches,
            showcase_tile_batch_clips: &showcase_tile_batch_clips,
            tile_glows: &tile_glows,
            tile_glow_buffer: tile_glow_buffer.as_ref(),
            relic_glows: &relic_glows,
            relic_glow_buffer: relic_glow_buffer.as_ref(),
            glyph_popup_glows: &glyph_popup_glows,
            glyph_popup_glow_buffer: glyph_popup_glow_buffer.as_ref(),
            relic_debuff_markers: &relic_debuff_markers,
            relic_debuff_buffer: relic_debuff_buffer.as_ref(),
            scene_hdr_attachment: true,
            pass_target_w: self.render_size.width,
            pass_target_h: self.render_size.height,
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
            depth_quad_buffers: &depth_quad_buffers,
            overlay_quad_buffers: &overlay_quad_buffers,
            overlay_squircle_quad_buffers: &overlay_squircle_quad_buffers,
            gradient_quad_buffers: &gradient_quad_buffers,
            arc_ring_quad_buffers: &arc_ring_quad_buffers,
            squircle_quad_buffers: &squircle_quad_buffers,
            flame_buffers: &flame_buffers,
            text_draws: &text_draws,
            tile_face_inst_buffers: &tile_face_inst_buffers,
            tile_face_quads: &tile_face_quads,
            image_quad_inst_buffers: &image_quad_inst_buffers,
            image_quads: &image_quads,
            object3d_draw_list: &object3d_draw_list,
            showcase_tile_batches: &showcase_tile_batches,
            showcase_tile_batch_clips: &showcase_tile_batch_clips,
            tile_glows: &tile_glows,
            tile_glow_buffer: tile_glow_buffer.as_ref(),
            relic_glows: &relic_glows,
            relic_glow_buffer: relic_glow_buffer.as_ref(),
            glyph_popup_glows: &glyph_popup_glows,
            glyph_popup_glow_buffer: glyph_popup_glow_buffer.as_ref(),
            relic_debuff_markers: &relic_debuff_markers,
            relic_debuff_buffer: relic_debuff_buffer.as_ref(),
            scene_hdr_attachment: overlay_hdr,
            pass_target_w: self.size.width,
            pass_target_h: self.size.height,
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
                .pass_writes(crate::gpu_profiler::PassSlot::Cascade);
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
            let split_main_for_profile = false;

            let mut pass_a_chunks = split_pass_a_chunks(&ops);
            if pass_a_chunks.is_empty() && !ops.is_empty() {
                pass_a_chunks.push(PassAChunk {
                    ops: ops.as_slice(),
                    shop_inspect_hdr_upload: PassAShopInspectHdrUpload::None,
                });
            }

            macro_rules! pass_a_draw_loop {
                ($pass:expr) => {{
                    for op in &ops {
                        // 2D HUD text labels are drawn in a later overlay pass
                        // (Load on the tonemapped target). Gameplay plaques are
                        // `Object3d` meshes (engraved decal on the mesh); they
                        // render here in Pass A like other lit meshes.
                        if matches!(
                            op,
                            RenderOp::TextDraw(_)
                                | RenderOp::ImageQuad(_)
                                | RenderOp::ArcRingQuadBatch { .. }
                                | RenderOp::OverlayQuadBatch { .. }
                                | RenderOp::OverlaySquircleQuadBatch { .. }
                        ) {
                            continue;
                        }
                        self.process_op(&mut $pass, op, &process_ctx_scene);
                    }
                }};
            }

            macro_rules! pass_a_draw_chunk {
                ($pass:expr, $chunk:expr) => {{
                    for op in $chunk.ops.iter() {
                        if matches!(
                            op,
                            RenderOp::TextDraw(_)
                                | RenderOp::ImageQuad(_)
                                | RenderOp::ArcRingQuadBatch { .. }
                                | RenderOp::OverlayQuadBatch { .. }
                                | RenderOp::OverlaySquircleQuadBatch { .. }
                        ) {
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
                        $pass.set_bind_group(3, &self.lit_mesh_spot_frame_bind_group, &[]);
                        $pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        $pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
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
                    if frame.debug_rain_hit_colliders {
                        self.draw_debug_rain_hit_colliders(&mut $pass);
                    }
                }};
            }

            if split_main_for_profile {
                let ts_table = self
                    .gpu_profiler
                    .pass_writes(crate::gpu_profiler::PassSlot::MainTable);
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
                pass_a_draw_loop!(pass);
                drop(pass);

                if !pass_a_chunks.is_empty() {
                    let n_scene_chunks = pass_a_chunks.len();
                    for (ci, chunk) in pass_a_chunks.iter().enumerate() {
                        if ci > 0 && frame.camera_override_after_depth_clear.is_some() {
                            let chunk_cam = CameraFrame::build_from(
                                frame.pass_a_chunk_camera(ci),
                                frame,
                                self.size,
                            );
                            self.upload_camera_uniforms(&chunk_cam, frame);
                            self.upload_punctual_light_buffers(
                                frame,
                                frame.foreground_scene_lighting(),
                                frame.foreground_camera(),
                                gamma,
                            );
                            self.pass_a_draw_camera = Some(chunk_cam);
                        }
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
                                    .pass_writes(crate::gpu_profiler::PassSlot::MainScene)
                            } else {
                                None
                            },
                            multiview_mask: None,
                        });
                        pass_a_draw_chunk!(pass, chunk);
                        if is_last_scene_chunk {
                            pass_a_debug_axes!(pass);
                        }
                    }
                }
            } else {
                if !pass_a_chunks.is_empty() {
                    let n_chunks = pass_a_chunks.len();
                    for (ci, chunk) in pass_a_chunks.iter().enumerate() {
                        if ci > 0 && frame.camera_override_after_depth_clear.is_some() {
                            let chunk_cam = CameraFrame::build_from(
                                frame.pass_a_chunk_camera(ci),
                                frame,
                                self.size,
                            );
                            self.upload_camera_uniforms(&chunk_cam, frame);
                            self.upload_punctual_light_buffers(
                                frame,
                                frame.foreground_scene_lighting(),
                                frame.foreground_camera(),
                                gamma,
                            );
                            self.pass_a_draw_camera = Some(chunk_cam);
                        }
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
                                    .pass_writes(crate::gpu_profiler::PassSlot::Main)
                            } else {
                                None
                            },
                            multiview_mask: None,
                        });
                        pass_a_draw_chunk!(pass, chunk);
                        if is_last_chunk {
                            pass_a_debug_axes!(pass);
                        }
                    }
                }
            }
        }

        // Emissive-only pre-pass for room bloom. GI no longer reads this
        // screen-space target; room lightmaps are sampled by material shaders.
        // Every scene shader writes linear HDR to `scene_color` directly —
        // bloom_extract reads from there, and the dedicated linear-HDR target is gone.
        let room_glb_emissive_env = frame.uses_room_glb_shader()
            && (ops_flags.shop_env
                || ops_flags.hallway_env
                || ops_flags.staircase_env
                || ops_flags.archive_env
                || ops_flags.main_menu_env
                || ops_flags.gameplay_env);
        let glb_room_emissive_prefetch = room_glb_emissive_env && bloom_active;
        if glb_room_emissive_prefetch {
            if ops_flags.shop_env
                && self.shop_environment.is_some()
                && !self.shop_env_primitives.is_empty()
            {
                self.write_shop_environment_uniforms(frame, &camera, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shop-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_shop_environment_meshes(&mut pass, frame, true);
                }
                self.write_shop_environment_uniforms(frame, &camera, false, None);
            }
            if ops_flags.hallway_env
                && self.hallway_environment.is_some()
                && !self.hallway_env_primitives.is_empty()
            {
                self.write_hallway_environment_uniforms(frame, &camera, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("hallway-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_hallway_environment_meshes(&mut pass, frame, true);
                }
                self.write_hallway_environment_uniforms(frame, &camera, false, None);
            }
            if ops_flags.staircase_env
                && self.staircase_environment.is_some()
                && !self.staircase_env_primitives.is_empty()
            {
                self.write_staircase_environment_uniforms(frame, &camera, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("staircase-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_staircase_environment_meshes(&mut pass, frame, true);
                }
                self.write_staircase_environment_uniforms(frame, &camera, false, None);
            }
            if ops_flags.archive_env
                && self.archive_environment.is_some()
                && !self.archive_env_primitives.is_empty()
            {
                self.write_archive_environment_uniforms(frame, &camera, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("archive-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_archive_environment_meshes(&mut pass, frame, true);
                }
                self.write_archive_environment_uniforms(frame, &camera, false, None);
            }
            if ops_flags.main_menu_env
                && self.main_menu_environment.is_some()
                && !self.main_menu_env_primitives.is_empty()
            {
                self.write_main_menu_environment_uniforms(frame, &camera, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("main-menu-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_main_menu_environment_meshes(&mut pass, frame, true);
                }
                self.write_main_menu_environment_uniforms(frame, &camera, false, None);
            }
            if ops_flags.gameplay_env
                && self.gameplay_environment.is_some()
                && !self.gameplay_env_primitives.is_empty()
            {
                let gameplay_cam = frame
                    .gameplay_cash_in_overlay_camera
                    .as_ref()
                    .map(|c| CameraFrame::build_from(Some(c), frame, self.size))
                    .unwrap_or(camera);
                let merged_overlay_lit = frame.gameplay_cash_in_overlay_lighting_merged();
                let overlay_lit = merged_overlay_lit
                    .as_ref()
                    .or(frame.gameplay_cash_in_overlay_lighting.as_ref())
                    .unwrap_or(&frame.scene_lighting);
                if frame.gameplay_cash_in_overlay_camera.is_some() {
                    self.upload_punctual_light_buffers(
                        frame,
                        overlay_lit,
                        frame.gameplay_cash_in_overlay_camera.as_ref(),
                        gamma,
                    );
                }
                self.write_gameplay_environment_uniforms(frame, &gameplay_cam, true, None);
                {
                    let room_bloom_ts = self
                        .gpu_profiler
                        .pass_writes(crate::gpu_profiler::PassSlot::RoomBloom);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("gameplay-emissive-prefetch-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.room_emissive_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                        timestamp_writes: room_bloom_ts,
                        multiview_mask: None,
                    });
                    self.draw_gameplay_environment_meshes(&mut pass, frame, true);
                }
                self.write_gameplay_environment_uniforms(frame, &camera, false, None);
            }
        }

        let bloom_w = (self.render_size.width.max(1) / 2).max(1);
        let bloom_h = (self.render_size.height.max(1) / 2).max(1);
        // `scene_color` is linear HDR — see `tonemap_composite.wgsl`.
        //
        // Bloom extract threshold must stay **high (~1.0+ scene-linear luminance)**:
        // the old two-texture path used this level on `scene_color` while routing
        // room emissive through a separate low-threshold buffer. A low threshold
        // here (e.g. 0.04) pulls in the procedural starfield and every mid-bright
        // pixel, which half-res blur turns into large glowing discs. Strength scales
        // with room linear exposure — emissive is absolute HDR while lit crush is not.
        let (bloom_threshold, bloom_strength, bloom_extract_scale) =
            Self::bloom_render_tuning(frame, &self.active_frame_env());
        let extract_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0, 0.0, 0.0, bloom_extract_scale],
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
                let bloom_extract_ts = self
                    .gpu_profiler
                    .pass_writes(crate::gpu_profiler::PassSlot::BloomExtract);
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
                let bloom_blur_h_ts = self
                    .gpu_profiler
                    .pass_writes(crate::gpu_profiler::PassSlot::BloomBlurH);
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
                let bloom_blur_v_ts = self
                    .gpu_profiler
                    .pass_writes(crate::gpu_profiler::PassSlot::BloomBlurV);
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
        // produces `post_bloom_view` for tonemap. When both are inactive, the pass collapses
        // to a fullscreen copy `scene_color → post_bloom`. On the Steam
        // Deck baseline that's a ~16 MB read+write per frame at 1080p
        // for nothing — skip it and have tonemap sample `scene_color`
        // directly via `tonemap_bind_group_scene`.
        let skip_scene_composite = !bloom_active && fisheye_strength == 0.0;
        if !skip_scene_composite {
            let scene_composite_ts = self
                .gpu_profiler
                .pass_writes(crate::gpu_profiler::PassSlot::SceneComposite);
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

        // Journal prepass and HDR swapchain both want linear scene+bloom out of this pass.
        // SDR applies ACES here; HDR leaves values > 1.0 for the OS / display tonemapper.
        let swapchain_hdr =
            !is_prepass && matches!(self.config.format, wgpu::TextureFormat::Rgba16Float);
        let tonemap_mode = if is_prepass {
            2.0f32
        } else if swapchain_hdr {
            1.0f32
        } else {
            0.0f32
        };
        // Journal prepass also forces VHS off so the book-page mesh never
        // resamples a buffer with overlay artifacts baked in.
        let vhs_on_now = if is_prepass { false } else { effective_vhs_on };
        let tonemap_time = self.creation_time.elapsed().as_secs_f32();
        let grain_frame = self.vhs_grain_frame as f32;
        self.vhs_grain_frame = self.vhs_grain_frame.wrapping_add(1);
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
                grain_frame,
                // Prepass float target must stay un-gamma'd (the book mesh
                // resamples it); only the visible swapchain honors the slider.
                gamma: if is_prepass { 1.0 } else { gamma.max(0.01) },
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
                .pass_writes(crate::gpu_profiler::PassSlot::Tonemap);
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
        // After tonemap, Load the final target so labels stay in display space
        // instead of the linear HDR scene buffer.
        if ops.iter().any(|o| {
            matches!(
                o,
                RenderOp::TextDraw(_)
                    | RenderOp::ImageQuad(_)
                    | RenderOp::ArcRingQuadBatch { .. }
                    | RenderOp::OverlayQuadBatch { .. }
                    | RenderOp::OverlaySquircleQuadBatch { .. }
            )
        }) {
            let overlay_ts = self
                .gpu_profiler
                .pass_writes(crate::gpu_profiler::PassSlot::Overlay);
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
                    view: &self.overlay_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: overlay_ts,
                multiview_mask: None,
            });
            for op in &ops {
                if matches!(
                    op,
                    RenderOp::TextDraw(_)
                        | RenderOp::ImageQuad(_)
                        | RenderOp::ArcRingQuadBatch { .. }
                        | RenderOp::OverlayQuadBatch { .. }
                        | RenderOp::OverlaySquircleQuadBatch { .. }
                ) {
                    self.process_op(&mut pass, op, &process_ctx_overlay);
                }
            }
        }

        // ── Debug overlay pass: tuning panels, focus-nav overlay, FPS ───
        // After normal UI so scene text/quads never paint over debug panels.
        if !debug_ops.is_empty() {
            let process_ctx_debug = ProcessOpCtx {
                frame,
                frame_pool_buffer: self.frame_buffer_pool.buffer(),
                bg_inst_buffers: &bg_inst_buffers,
                quad_buffers: &quad_buffers,
                depth_quad_buffers: &depth_quad_buffers,
                overlay_quad_buffers: &debug_overlay_quad_buffers,
                overlay_squircle_quad_buffers: &overlay_squircle_quad_buffers,
                gradient_quad_buffers: &gradient_quad_buffers,
                arc_ring_quad_buffers: &arc_ring_quad_buffers,
                squircle_quad_buffers: &squircle_quad_buffers,
                flame_buffers: &flame_buffers,
                text_draws: &debug_text_draws,
                tile_face_inst_buffers: &tile_face_inst_buffers,
                tile_face_quads: &tile_face_quads,
                image_quad_inst_buffers: &image_quad_inst_buffers,
                image_quads: &image_quads,
                object3d_draw_list: &object3d_draw_list,
                showcase_tile_batches: &showcase_tile_batches,
                showcase_tile_batch_clips: &showcase_tile_batch_clips,
                tile_glows: &tile_glows,
                tile_glow_buffer: tile_glow_buffer.as_ref(),
                relic_glows: &relic_glows,
                relic_glow_buffer: relic_glow_buffer.as_ref(),
                glyph_popup_glows: &glyph_popup_glows,
                glyph_popup_glow_buffer: glyph_popup_glow_buffer.as_ref(),
                relic_debuff_markers: &relic_debuff_markers,
                relic_debuff_buffer: relic_debuff_buffer.as_ref(),
                scene_hdr_attachment: overlay_hdr,
                pass_target_w: self.size.width,
                pass_target_h: self.size.height,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("debug-overlay-pass"),
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
                    view: &self.overlay_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            for op in &debug_ops {
                if matches!(
                    op,
                    RenderOp::TextDraw(_)
                        | RenderOp::ImageQuad(_)
                        | RenderOp::ArcRingQuadBatch { .. }
                        | RenderOp::OverlayQuadBatch { .. }
                        | RenderOp::OverlaySquircleQuadBatch { .. }
                ) {
                    self.process_op(&mut pass, op, &process_ctx_debug);
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
        self.queue.submit(std::iter::once(encoder.finish()));

        if let (Some(path), Some(staging)) = (screenshot_path, screenshot_staging) {
            match self.finalize_screenshot(staging, &path) {
                Ok(()) => log::info!("screenshot saved → {}", path.display()),
                Err(e) => log::error!("screenshot finalize failed: {e:?}"),
            }
        }
        if let Some(staging) = room_shadow_capture_staging {
            match self.finalize_room_shadow_capture(staging) {
                Ok(bake) => {
                    self.room_shadow_captured = Some(bake);
                    self.room_shadow_capture_pending = None;
                }
                Err(e) => log::error!("room shadow capture readback failed: {e:?}"),
            }
        }
        if let Some(staging) = shadow_probe_staging {
            self.finalize_shadow_probe(staging);
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
