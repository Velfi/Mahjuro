use super::*;

impl WgpuRenderer {
    pub fn screenshot_pending(&self) -> bool {
        let p = self.pending_screenshot.take();
        let pending = p.is_some();
        // Restore (Cell::take() removes the value; put it back so the
        // next draw can fulfil it).
        self.pending_screenshot.set(p);
        pending
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        match &self.target {
            RenderTarget::Surface(surface) => surface.configure(&self.device, &self.config),
            RenderTarget::Offscreen { .. } => {
                // Headless screenshot mode renders at a fixed size chosen
                // at startup; window resize events never reach this path.
                // Leaving the offscreen texture untouched here keeps the
                // per-frame render target stable across ticks.
            }
        }

        self.depth_texture.destroy();
        let (dt, dv) = create_depth(&self.device, new_size.width, new_size.height);
        self.depth_texture = dt;
        self.depth_view = dv;
        self.depth_copy_texture.destroy();
        let (dct, dcv) = create_depth_copy(&self.device, new_size.width, new_size.height);
        self.depth_copy_texture = dct;
        self.depth_copy_view = dcv;
        self.ssr_prev_depth_texture.destroy();
        let (sdt, sdv) = create_depth_copy(&self.device, new_size.width, new_size.height);
        self.ssr_prev_depth_texture = sdt;
        self.ssr_prev_depth_view = sdv;

        // SSR scene history texture follows the swapchain size; rebuild
        // the bind group so it points at the freshly allocated views.
        self.scene_prev_texture.destroy();
        let (spt, spv) = create_scene_prev(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.scene_prev_texture = spt;
        self.scene_prev_view = spv;
        self.scene_color_texture.destroy();
        let (sct, scv) = create_scene_color(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.scene_color_texture = sct;
        self.scene_color_view = scv;
        self.cascade_offscreen_texture.destroy();
        let (cot, cov) = create_cascade_offscreen(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.cascade_offscreen_texture = cot;
        self.cascade_offscreen_view = cov;
        self.cascade_composite_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cascade-composite-bg"),
                layout: &self.cascade_composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.cascade_offscreen_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.cascade_composite_sampler),
                    },
                ],
            });
        self.bloom_ping_texture.destroy();
        self.bloom_pong_texture.destroy();
        let bloom_w = (new_size.width.max(1) / 2).max(1);
        let bloom_h = (new_size.height.max(1) / 2).max(1);
        let (bpt, bpv) = create_post_texture(
            &self.device,
            self.config.format,
            bloom_w,
            bloom_h,
            "bloom-ping",
        );
        self.bloom_ping_texture = bpt;
        self.bloom_ping_view = bpv;
        let (bot, bov) = create_post_texture(
            &self.device,
            self.config.format,
            bloom_w,
            bloom_h,
            "bloom-pong",
        );
        self.bloom_pong_texture = bot;
        self.bloom_pong_view = bov;
        self.lit_mesh_ssr_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-ssr-bg"),
            layout: &self.lit_mesh_ssr_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.lit_mesh_ssr_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.ssr_prev_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.lit_mesh_ssr_sampler),
                },
            ],
        });
        self.bloom_scene_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-scene-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_ping_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-ping-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_pong_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-pong-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_pong_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_composite_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-composite-bg"),
                layout: &self.bloom_composite_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.bloom_params_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                    },
                ],
            });

        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [new_size.width as f32, new_size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                // Gamma will be re-uploaded on the next render() call.
                gamma: 1.0,
                cursor_pos: [new_size.width as f32 * 0.5, new_size.height as f32 * 0.5],
                transition_progress: 0.0,
                quality_level: 2.0,
                moon_phase: current_moon_phase(),
                _globals_pad: [0.0; 3],
            }),
        );
        self.queue.write_buffer(
            &self.bloom_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0: [1.1, 0.0, 1.0 / bloom_w as f32, 1.0 / bloom_h as f32],
                data1: [1.0, 0.0, 0.0, 0.0],
            }),
        );

        if let Some(ref mut fluid) = self.fluid {
            fluid.update_screen_size(new_size.width as f32, new_size.height as f32);
        }
        // Depth view was just recreated — the volumetric smoke pass needs a
        // fresh bind group that points at the new view.
        self.fluid_render_bg_dirty = true;
    }

    pub fn render(&mut self, frame: &UiFrame, settings: RenderSettings) -> anyhow::Result<()> {
        let RenderSettings {
            smoke_quality,
            smoke_amount,
            effects_quality,
            tile_preset,
            tile_material,
            tileset_name,
            draw_settle_speed,
            sort_settle_speed,
            gamma,
            shadows_enabled,
            ssr_enabled,
        } = settings;
        // Encode the tile material choice into base_color_factor.w so the
        // tile_3d shader can branch on it (0 = bamboo, 1 = plastic, …).
        self.tile_base_color_factor[3] = tile_material.shader_id();

        // Swap tilesets: if the user picked a different set in Options, update
        // the active name and blow the per-tile decal caches so the next frame
        // re-rasterizes against the new set's PNGs.
        if self.tile_set.as_deref() != Some(tileset_name.as_str()) {
            self.tile_set = Some(tileset_name.clone());
            self.tile_face_overlays.clear();
            self.hand_tiles.clear();
            self.showcase_tiles.clear();
        }

        // Hand tile fields removed — hand tiles now rendered via ShowcaseTileBatch.
        let hand_slots: &[(f32, f32, f32, f32)] = &[];
        let focus: usize = usize::MAX;
        let selected: &[bool] = &[];
        let hint_indices: &[usize] = &[];
        // Upload any relic/background textures that finished decoding.
        self.poll_relic_textures();
        self.poll_background_textures();

        // Acquire the per-frame texture to draw into. In the interactive
        // path this is a swapchain image; in headless screenshot mode it's
        // a plain render-attachment texture owned by `self.target`. Either
        // way we end up with a `&wgpu::Texture` (for the screenshot copy)
        // and a `TextureView` (for the render passes).
        let surface_frame: Option<wgpu::SurfaceTexture> = match &self.target {
            RenderTarget::Surface(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(&self.device, &self.config);
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                    return Ok(());
                }
            },
            RenderTarget::Offscreen { .. } => None,
        };
        let frame_texture: &wgpu::Texture = match (&surface_frame, &self.target) {
            (Some(sf), _) => &sf.texture,
            (None, RenderTarget::Offscreen { texture, .. }) => texture,
            (None, RenderTarget::Surface(_)) => {
                unreachable!("Surface target always produces a surface_frame or early-returns")
            }
        };
        let view = frame_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = &self.scene_color_view;
        let bloom_active = {
            use crate::render::draw_cmd::Object3dKind;
            frame.cmds.iter().any(|cmd| match cmd {
                DrawCmd::MoonlitWater => true,
                DrawCmd::Object3d(obj) => matches!(obj.kind, Object3dKind::ShopLamp { .. }),
                DrawCmd::Object3dBatch(objs) => objs
                    .iter()
                    .any(|o| matches!(o.kind, Object3dKind::ShopLamp { .. })),
                _ => false,
            })
        };

        let _w = self.size.width.max(1) as f32;
        let _h = self.size.height.max(1) as f32;

        // Detect focus changes and start a 360° CW spin for the newly focused tile.
        if focus != self.last_focus {
            self.focus_spin = Some((focus, Instant::now()));
            self.last_focus = focus;
        }

        // Lerp per-tile slide animations toward 0 (ease-out).
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.05);
        self.last_frame = now;
        // Cache for downstream prep loops (bowl/mirror hover envelopes,
        // etc.) so they don't have to recompute or re-clamp the timestamp.
        self.frame_dt = dt;
        let slide_speed = draw_settle_speed; // higher = faster settle
        for y in self.tile_anim_y.iter_mut() {
            *y *= (-slide_speed * dt).exp(); // exponential ease-out
            if y.abs() < 0.5 {
                *y = 0.0;
            }
        }
        let slide_speed_x = sort_settle_speed; // horizontal settle for sort/drag
        for x in self.tile_anim_x.iter_mut() {
            *x *= (-slide_speed_x * dt).exp();
            if x.abs() < 0.01 {
                *x = 0.0;
            }
        }

        // Update globals with current time for animated shaders.
        let w_f = self.size.width as f32;
        let h_f = self.size.height as f32;
        let (cx, cy) = frame.cursor_pos.unwrap_or((w_f * 0.5, h_f * 0.5));
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [w_f, h_f],
                time: self.creation_time.elapsed().as_secs_f32(),
                gamma: gamma.max(0.01),
                cursor_pos: [cx, cy],
                transition_progress: frame.transition_progress,
                quality_level: effects_quality.quality_level_f32(),
                moon_phase: current_moon_phase(),
                _globals_pad: [0.0; 3],
            }),
        );

        // Upload point lights for the tile shader (group 1). Scenes push
        // candle/spot lights into `frame.point_lights` in pixel-layout
        // coordinates; we map them onto the table-plane world for upload.
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &frame.point_lights,
                frame.candle_light_count,
                frame.flame_height_world,
                pl_w,
                pl_h,
                gamma,
                self.creation_time.elapsed().as_secs_f32(),
            )),
        );

        // Upload spotlights for the tile shader (group 3). Scenes push
        // directional cone lights into `frame.spot_lights`; only the tile
        // pipeline samples them (lit_mesh and the smoke lightbake don't).
        self.queue.write_buffer(
            &self.spot_lights_buffer,
            0,
            bytemuck::bytes_of(&SpotLightsBuf::from_lights(&frame.spot_lights, pl_w, pl_h)),
        );

        // Advance departing-tile timers. The two-phase trajectory is
        // recomputed analytically from `elapsed` in the render block
        // below, so the only state to update here is the clock + the
        // cull on tiles past their combined lifetime.
        for tile in self.departing_tiles.iter_mut() {
            tile.elapsed += dt;
        }
        self.departing_tiles.retain(|t| t.elapsed < t.lifetime);

        // Save hand slots for next frame's departure animations.
        self.prev_hand_slots = hand_slots.to_vec();

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
        let w = self.size.width.max(1) as f32;
        let h = self.size.height.max(1) as f32;
        let aspect = w / h;
        let (cam_pos, look_target, fov_y) = if let Some(ref c) = frame.camera_override {
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        } else {
            let c = crate::render::draw_cmd::CameraParams::default_table_camera(h);
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        };
        let up_v = frame
            .camera_override
            .as_ref()
            .map(|c| glam::Vec3::from_array(c.up))
            .unwrap_or(glam::Vec3::Z);
        let view_mat = Mat4::look_at_rh(cam_pos, look_target, up_v);
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, h * 12.0);
        let view_proj = proj * view_mat;
        let view_proj_arr = view_proj.to_cols_array();

        // Upload the SSR globals so the lacquered-floor branch in
        // lit_mesh.wgsl can unproject screen-space depth taps and march
        // reflection rays in world space. Tunables match the plan:
        // ~24 linear steps with binary refinement, max distance scaled
        // to the screen height. Disabled when the user toggles SSR off.
        let ssr_max_distance = h * 2.0;
        let ssr_stride = h * 0.04;
        let ssr_max_steps = 24.0;
        self.queue.write_buffer(
            &self.lit_mesh_ssr_buffer,
            0,
            bytemuck::bytes_of(&SsrGlobals {
                inv_view_proj: view_proj.inverse().to_cols_array(),
                view_proj: view_proj_arr,
                view_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
                params: [
                    if ssr_enabled { 1.0 } else { 0.0 },
                    ssr_max_distance,
                    ssr_stride,
                    ssr_max_steps,
                ],
            }),
        );

        // Matching upload for the flame pipeline's view uniform. Same
        // camera, smaller struct — see `FlameViewUniform`.
        self.queue.write_buffer(
            &self.flame_view_buffer,
            0,
            bytemuck::bytes_of(&FlameViewUniform {
                view_proj: view_proj_arr,
                view_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            }),
        );

        // Table plane mapping: see [`crate::render::world_space::pixel_to_world`].
        // Helper: project a world position to integer screen pixels for use
        // in 2D overlay quads (selection halos, hint pulses, hover arrows).
        let project_to_screen = |world: glam::Vec3| -> (f32, f32) {
            let clip = view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
            let inv_w = 1.0 / clip.w.max(1e-6);
            let nx = clip.x * inv_w;
            let ny = clip.y * inv_w;
            let sx = (nx * 0.5 + 0.5) * w;
            let sy = (1.0 - (ny * 0.5 + 0.5)) * h;
            (sx, sy)
        };

        // ── Debug axes overlay ──────────────────────────────────────────
        // When `frame.debug_axes` is set, write three thin colored boxes
        // (red = +X, green = +Y, blue = +Z) anchored at the current camera
        // look target. Each axis box extends from the origin in the
        // *positive* direction so the user can read sign as well as axis.
        if frame.debug_axes {
            // Length: a chunky fraction of screen height so the bars are
            // visible against the table from the default camera.
            let length = h * 0.35;
            let thickness = (h * 0.012).max(4.0);
            let origin = look_target;
            let axes: [(glam::Vec3, glam::Vec3, [f32; 4]); 3] = [
                // +X — red
                (
                    glam::Vec3::X,
                    glam::Vec3::new(length, thickness, thickness),
                    [1.6, 0.10, 0.10, 1.0],
                ),
                // +Y — green
                (
                    glam::Vec3::Y,
                    glam::Vec3::new(thickness, length, thickness),
                    [0.10, 1.6, 0.10, 1.0],
                ),
                // +Z — blue
                (
                    glam::Vec3::Z,
                    glam::Vec3::new(thickness, thickness, length),
                    [0.20, 0.40, 1.8, 1.0],
                ),
            ];
            for (i, (axis_dir, scale, color)) in axes.iter().enumerate() {
                // Center the box halfway down the positive axis so its
                // -end sits at `origin` and its +end sticks out by `length`.
                let center = origin + *axis_dir * (length * 0.5);
                let model = translate_rot_scale(center, Mat4::IDENTITY, *scale);
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: *color,
                    specular_strength: 0.0,
                    specular_power: 8.0,
                };
                if let Some(inst) = self.debug_axes_instances.get(i) {
                    inst.write_uniform(&self.queue, view_proj_arr, model, material);
                }
            }
        }

        // ── Flame emitters (world-space) ─────────────────────────────
        // Each candle in the scene becomes one emitter for the 3D flame
        // particle system. We walk the cmd list, find every Candle, and
        // project the wick tip into world space using the same
        // `pixel_to_world` mapping the rest of the scene uses. The
        // `DrawCmd::Flame` batch loop below consumes the candle-ordered
        // list of emitters in submission order.
        //
        // Brightness + phase are pulled from the scene's per-candle
        // `GpuInstance`: `color.b` = brightness, `color.a` = phase. The
        // anchor loop also computes a per-candle wind vector by sampling
        // `frame.wind_gusts` at the wick's world position so the particle
        // system can lean the plume in real wind.
        let flame_emitters: Vec<crate::render::flame_particles::FlameEmitter> = {
            let mut out: Vec<crate::render::flame_particles::FlameEmitter> = Vec::new();
            // Walk all Object3ds in the frame, picking out Candles in
            // submission order (matches scene `frame.flames(...)` order).
            let candles: Vec<(&crate::render::draw_cmd::Object3d, f32, f32)> = frame
                .cmds
                .iter()
                .flat_map(|cmd| {
                    let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> =
                        match cmd {
                            DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                            DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                            _ => Box::new(std::iter::empty()),
                        };
                    objs.filter_map(|o| {
                        if let crate::render::draw_cmd::Object3dKind::Candle {
                            scale,
                            height_scale,
                        } = o.kind
                        {
                            Some((o, scale, height_scale))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                })
                .collect();
            // Scene-supplied per-flame data (brightness + phase), pulled
            // out of the cmd stream in the same order candles appear.
            let mut flame_cmd_iter = frame.cmds.iter().filter_map(|cmd| match cmd {
                DrawCmd::Flame(inst) => Some(*inst),
                _ => None,
            });
            for (o, p_scale, p_height) in candles.into_iter() {
                let p_pos = o.pos;
                let tip_world = pixel_to_world(
                    w,
                    h,
                    p_pos[0],
                    p_pos[1],
                    crate::render::candle_mesh::WICK_TIP_Y * p_scale * p_height,
                );
                let scene_inst = flame_cmd_iter.next();
                let (brightness, phase) = scene_inst
                    .map(|inst| (inst.color[2], inst.color[3]))
                    .unwrap_or((1.0, 0.0));

                // Sample scene wind gusts at the wick, weighted by a
                // soft falloff around each gust's world-space radius.
                // Convert the resulting world-space velocity into the
                // flame-relative units the particle system uses.
                let mut wind_world = glam::Vec3::ZERO;
                for g in frame.wind_gusts.iter() {
                    let g_world = pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
                    let dist = (g_world - tip_world).length();
                    let r = (g.radius * 3.0).max(1.0);
                    let falloff = (1.0 - (dist / r).clamp(0.0, 1.0)).powf(1.5);
                    if falloff <= 0.0 {
                        continue;
                    }
                    wind_world +=
                        glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]) * falloff;
                }
                // Flame-relative wind: normalise against a reference
                // per-candle velocity so neighbouring candles react to
                // the same gust by the same visible amount. 300 units/s
                // → 1.0 in flame-relative space is the heuristic that
                // matched the previous 2D behaviour.
                let wind_scale = 1.0 / 300.0;
                let wind = glam::Vec2::new(
                    (wind_world.x * wind_scale).clamp(-1.5, 1.5),
                    (wind_world.z * wind_scale).clamp(-1.5, 1.5),
                );

                out.push(crate::render::flame_particles::FlameEmitter {
                    wick_world: tip_world,
                    // Scale the particle size to the candle's physical
                    // scale. A candle drawn at scale `p_scale * p_height`
                    // (world units) should produce a plume whose width
                    // is a fraction of that scale; 0.22 lines up with
                    // the previous 2D flame's visual width.
                    scale: p_scale * p_height * 0.22,
                    wind,
                    brightness,
                    phase,
                });
            }
            out
        };

        let tile_basis = tile_mesh_local_to_world();
        // After tile_basis, the tile lies flat with face normal pointing +Z.
        // Rx(+π/2) rotates +Z → -Y so the face points toward the camera (at large -Y).
        let hand_tile_face_to_camera = rot_x_rad(std::f32::consts::PI / 2.0);

        {
            for (i, _htg_ref) in self.hand_tiles.iter().enumerate() {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(i) else {
                    continue;
                };
                let is_focused = i == focus;
                let is_selected = selected.get(i).copied().unwrap_or(false);
                let slide_y = self.tile_anim_y.get(i).copied().unwrap_or(0.0);
                let slide_x_slots = self.tile_anim_x.get(i).copied().unwrap_or(0.0);

                // Tile face dimensions in pixel units (pre-projection). The
                // long axis runs front-back on the table. The face aspect
                // and thickness come from the user-selected regional preset
                // (Wikipedia gives Chinese 30×20×15, Japanese 26×19×16,
                // American 32×25×19) so swapping presets actually changes
                // the tile shape, not just a uniform scale.
                let tile_short_px = sw * 0.85; // left-right footprint on the table
                let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();

                // Tile center in pixel-layout coords (toward bottom of slot = toward player).
                let cx_px = sx + sw * 0.5 + slide_x_slots * sw;
                // The slide_y residual still pushes the tile briefly; larger
                // `py` → more −world Y (nearer player; see [`pixel_to_world`]).
                let cy_px = sy + sh * crate::ui::layout::HAND_TILE_MESH_Y_FRAC + slide_y;

                // World position: laid flat just above the table.
                let world_y_lift = tile_thickness_px * 0.5 + 4.0;
                let world = pixel_to_world(w, h, cx_px, cy_px, world_y_lift);

                // Smoke impulse from per-tile motion: compare to last
                // frame's world position for this uid and inject the
                // delta as velocity. Skip the first frame (no history).
                if let Some(uid) = self.tile_uids.get(i).copied() {
                    if let Some(prev) = self.prev_tile_world.get(&uid).copied() {
                        let delta = world - prev;
                        let speed = delta.length();
                        if speed > 0.5
                            && let Some(ref mut fluid) = self.fluid
                        {
                            let inv_dt = 1.0 / dt.max(1.0 / 120.0);
                            fluid.inject_impulse(
                                world,
                                delta * inv_dt * 0.45,
                                tile_short_px * 0.55,
                                speed * 0.04,
                                0.0,
                                0.0,
                            );
                        }
                    }
                    self.prev_tile_world.insert(uid, world);
                }

                let r_static = hand_tile_face_to_camera * tile_basis;

                // Tilt rotation, computed once and reused for both the
                // model matrix below and the overlay-anchor projection.
                // Pivot: bottom‑front corner in **world** Z-up axes (after
                // `hand_tile_face_to_camera` * `tile_mesh_local_to_world` * scale):
                // +Y = along table toward larger `py`, +Z = up from felt.
                let tilt_angle = 22.0_f32.to_radians();
                let tilt_pivot = hand_tile_face_to_camera.transform_point3(glam::Vec3::new(
                    0.0,
                    tile_long_px * 0.5,
                    -tile_thickness_px * 0.5,
                ));
                let tilt = rotation_around_point_x_rad(tilt_pivot, tilt_angle);

                // Helper: offset from tile center in **world** axes after
                // `r_static` (mesh → world, no tilt), then tilt and project.
                let tilted_to_screen = |pre_tilt: glam::Vec3| -> (f32, f32) {
                    let tilted = tilt.transform_point3(pre_tilt);
                    project_to_screen(world + tilted)
                };

                // Project the tile center to screen space so 2D overlay
                // anchors (selection halo, hint pulse, hover arrow) follow
                // the tile's actual on-screen position under the tilted
                // camera.
                let (proj_cx, proj_cy) = tilted_to_screen(glam::Vec3::ZERO);
                // Project all 8 corners of the tilted slab and take the
                // actual screen-space AABB. Earlier this used a single
                // back-top corner mirrored around the projected center,
                // which underestimates the rect for tiles off the optical
                // axis: under perspective the silhouette is asymmetric
                // around the projected center, so click/hover hit-testing
                // felt off near the camera edges.
                let lx = tile_long_px * 0.5; // mesh local ±X (long)
                let ly = tile_thickness_px * 0.5; // mesh local ±Y (thick)
                let lz = tile_short_px * 0.5; // mesh local ±Z (short)
                let corners = [
                    glam::Vec3::new(-lx, -ly, -lz),
                    glam::Vec3::new(lx, -ly, -lz),
                    glam::Vec3::new(-lx, ly, -lz),
                    glam::Vec3::new(lx, ly, -lz),
                    glam::Vec3::new(-lx, -ly, lz),
                    glam::Vec3::new(lx, -ly, lz),
                    glam::Vec3::new(-lx, ly, lz),
                    glam::Vec3::new(lx, ly, lz),
                ];
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for c in corners {
                    let pre_tilt = r_static.transform_point3(c);
                    let (px, py) = tilted_to_screen(pre_tilt);
                    if px < min_x {
                        min_x = px;
                    }
                    if py < min_y {
                        min_y = py;
                    }
                    if px > max_x {
                        max_x = px;
                    }
                    if py > max_y {
                        max_y = py;
                    }
                }
                let overlay_w = (max_x - min_x).max(16.0);
                let overlay_h = (max_y - min_y).max(16.0);
                let overlay_x = min_x;
                let overlay_y = min_y;
                // Keep the projected center available for downstream
                // anchors that want a tile-centered point rather than the
                // AABB.
                let _ = (proj_cx, proj_cy);

                // Selected tiles get a 3D gold-metal outline shell drawn
                // by the outline pipeline below, plus an additive radial
                // glow halo (built here, drawn at the start of the
                // Tiles3d render op so it sits behind the tile mesh).
                if is_selected {
                    // Glow rect ~2× the tile in both axes so the falloff
                    // has room to spill out around the silhouette.
                    let gw = overlay_w * 2.10;
                    let gh = overlay_h * 2.20;
                    let gx = overlay_x + (overlay_w - gw) * 0.5;
                    let gy = overlay_y + (overlay_h - gh) * 0.5;
                    tile_glows.push(GpuInstance {
                        rect: [gx, gy, gw, gh],
                        // Warm champagne gold. The alpha channel scales
                        // overall intensity inside the glow shader.
                        color: [1.00, 0.78, 0.32, 1.10],
                    });
                }

                // Hint tiles get a vertical light beam (built below) but no
                // border-style halo — the rectangular halo reads as a
                // selection indicator and confused which tiles are actually
                // selected.

                tile_3d_rects.push((i, [overlay_x, overlay_y, overlay_w, overlay_h]));

                // Hover arrow above the focused tile (in screen space).
                if is_focused {
                    let bob_period = 1.5_f32;
                    let bob_amp = overlay_h * 0.08;
                    let bob_y = (self.creation_time.elapsed().as_secs_f32() / bob_period
                        * std::f32::consts::TAU)
                        .sin()
                        * bob_amp;
                    let arrow_h = overlay_h * 0.32;
                    let arrow_w = overlay_w * 0.65;
                    let arrow_x = overlay_x + (overlay_w - arrow_w) * 0.5;
                    let arrow_y = overlay_y - arrow_h - overlay_h * 0.05 + bob_y;
                    tile_labels.push(TextLabel {
                        rect: [arrow_x, arrow_y, arrow_w, arrow_h],
                        text: "▼".to_string(),
                        color: [0.85, 0.1, 0.1, 1.0],
                        ..Default::default()
                    });
                }

                // Build the per-tile model matrix and write its uniform.
                if let Some(htg) = self.hand_tiles.get(i) {
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ); // local X,Y,Z — oriented by [`tile_mesh_local_to_world`]
                    let oriented = tilt * hand_tile_face_to_camera * tile_basis;
                    // Pack enhancement kind into .z so the tile shader can
                    // apply fresnel-masked sheen effects per-enhancement.
                    let mut bcf = self.tile_base_color_factor;
                    // Channels .x and .y carry showcase-tile flags
                    // (brightness, selection). Hand tiles use the outline
                    // shell + glow halo instead, so force neutral values.
                    bcf[0] = 1.0;
                    bcf[1] = 0.0;
                    bcf[2] = htg.tile_id.2.map_or(0.0, |e| e.shader_id());
                    // When this tile is selected, also write an inflated
                    // model matrix into the outline shell uniform so the
                    // outline pipeline draws a slightly larger version of
                    // the same mesh around the tile silhouette.
                    // Slot index drives per-tile procedural variation (e.g.
                    // tortoise shell mottling) in tile_3d.wgsl.
                    let tile_seed = i as f32;
                    if is_selected {
                        // ~5–6% larger; tuned so the rim is visible without
                        // overlapping neighbouring tiles.
                        const OUTLINE_GROW: f32 = 1.055;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(world, oriented, outline_scale);
                        self.queue.write_buffer(
                            &htg.outline_uniform_buffer,
                            0,
                            bytemuck::bytes_of(&CameraUniform {
                                view_proj: view_proj_arr,
                                model: outline_model.to_cols_array(),
                                base_color_factor: bcf,
                                cam_pos: cam_pos.to_array(),
                                tile_seed,
                            }),
                        );
                    }
                    // `tilt` was computed above the projection block so
                    // both the model matrix and the overlay anchors share
                    // the same rotation.
                    let model = translate_rot_scale(world, oriented, scale);
                    // Snapshot for next frame's cursor pick.
                    tile_pick_models.push((i, model));
                    self.queue.write_buffer(
                        &htg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: bcf,
                            cam_pos: cam_pos.to_array(),
                            tile_seed,
                        }),
                    );
                }
            }
        }

        // Snapshot deferred to after the showcase pre-pass so that
        // ShowcaseTileBatch tiles with pick_id also land in hand_rects and
        // last_pick_models (the showcase pre-pass runs further below).

        // Tile hints are now real green PointLights pushed by the gameplay
        // scene into `frame.point_lights` (see the hint-lights block in
        // `scenes/gameplay.rs`). The 2D fake "light beam" overlay that
        // used to live here was removed in favour of letting the real
        // lighting model do the work — the hinted tile picks up a green
        // top-down pool through the same shader path as the candles.
        let _ = hint_indices;

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
        struct TextDraw {
            inst_buf: wgpu::Buffer,
            bind_group: wgpu::BindGroup,

            _tex: wgpu::Texture,
        }
        let make_text_draw = |device: &wgpu::Device,
                              queue: &wgpu::Queue,
                              text_bgl: &wgpu::BindGroupLayout,
                              sampler: &wgpu::Sampler,
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
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                _tex: tex,
            }
        };

        // ── Hand tile face/emoji label GPU draws (consumed by HandTileFaces) ──
        let mut hand_face_draws: Vec<TextDraw> = Vec::new();
        if let Some(ref font) = self.ui_font {
            for lbl in &tile_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
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
                    lbl,
                    font,
                    None,
                ));
            }
        }

        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        enum RenderOp {
            Background(BackgroundId),
            Starfield,
            EmberDrift,
            GoldenDust,
            MoonlitWater,
            SunlitWater,
            MountainHaze,
            ShootingStarCascade,
            Table,
            QuadBatch { buf_idx: usize, count: u32 },
            GradientQuadBatch { buf_idx: usize, count: u32 },
            FlameBatch { buf_idx: usize, count: u32 },
            TextDraw(usize),
            TileFaceQuad(usize),
            FluidSmoke,
            // Skeuomorphic gameplay HUD (phase 1).
            ShowcaseTileBatch(usize), // index into `showcase_tile_batches`
            Object3dBatch { start: usize, end: usize }, // range into `object3d_draw_list`
        }

        // Each Object3dKind that gets drawn through the lit-mesh pipeline
        // gets one variant here. The pre-pass pushes `(DrawKind, slot_i)`
        // into `object3d_draw_list`; the dispatch loop matches on the
        // variant to pick the right mesh + instance pool. Keeping this as
        // an enum (rather than raw u8 ids) means the compiler catches any
        // collision or missing dispatch arm.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum DrawKind {
            YakuTablet,
            WoodTablet,
            Relic,
            Pack,
            Ribbon,
            Talisman,
            Shrine,
            SellTray,
            LampBody,
            LampBulb,
            BugBody,
            BugWingL,
            BugWingBlurL,
            Orb,
            DoraPlinth,
            Bowl,
            Mirror,
            TallyStickBase,
            TallyStickTip,
            CandleWax,
            CandleWick,
            CascadeToken,
            ExtrudedGlyph,
            BugWingR,
            BugWingBlurR,
            Primitive(crate::render::primitive::MeshId),
        }

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

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    ops.push(RenderOp::Background(*id));
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
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::MoonlitWater);
                    }
                    i += 1;
                }
                DrawCmd::SunlitWater => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::SunlitWater);
                    }
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
                DrawCmd::FluidSmoke => {
                    ops.push(RenderOp::FluidSmoke);
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
                    // Smoke-on paths skip the actual draw (see the
                    // `FlameBatch` branch below), but we still step so
                    // smoke → no-smoke toggles mid-game don't suddenly
                    // drop an empty pool into view.
                    self.flame_particles.step(&flame_emitters, self.frame_dt);
                    let count = self
                        .flame_particles
                        .fill_gpu_instances(&flame_emitters, &mut self.flame_particle_staging);
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
                DrawCmd::GlossaryAnchor { .. } => {
                    // Pure metadata for the tooltip overlay; no draw work.
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
                self.table_mesh.default_material,
            );
        }
        // Reset the debug pickable catch-all for this frame; each draw
        // loop below appends entries it wants to expose to
        // `pick_debug_object`.
        self.last_debug_pickables.clear();
        self.last_debug_trimesh_pickables.clear();

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
        self.last_sell_tray_model = None;
        self.last_sell_card_model = None;

        // ── Skeuomorphic gameplay HUD uniform writes (phase 1) ─────────
        //
        // The new HUD meshes (plaque, ofuda, tablets, bowl, peg block, wall
        // stack) all share the lit-mesh pipeline. Each gets its
        // own slot pool above; per-frame we walk the cmds, write the
        // per-instance uniform, and (where the scene needs it for hit
        // testing in later phases) project the AABB to a screen-space rect.
        self.proj.yaku_tablet_rects.clear();
        self.proj.wood_tablet_rects.clear();
        self.proj.plaque_rects.clear();
        self.proj.bowl_rect = None;
        self.proj.mirror_rect = None;
        self.proj.dora_plinth_rect = None;
        self.proj.peg_rects = [None, None];
        self.proj.aux_dish_rects.clear();
        self.last_aux_dish_aabbs.clear();
        self.last_yaku_tablet_models.clear();
        self.last_wood_tablet_models.clear();
        self.last_bowl_model = None;
        self.last_mirror_model = None;

        // Helper closure: project the unit-cube AABB transformed by `model`
        // into a screen-space rect. Used by tablets/bowl for hit testing.
        let project_unit_cube_rect = |model: Mat4| -> [f32; 4] {
            let corners = [
                glam::Vec3::new(-0.5, -0.5, -0.5),
                glam::Vec3::new(0.5, -0.5, -0.5),
                glam::Vec3::new(-0.5, 0.5, -0.5),
                glam::Vec3::new(0.5, 0.5, -0.5),
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in corners {
                let w = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
        };

        // Like `project_unit_cube_rect` but projects the actual mesh AABB
        // (given by half-extents and a Y-axis center offset) instead of
        // the full `[-0.5, 0.5]³` unit cube. This produces a much tighter
        // screen rect for flat objects like the river trough and mirror.
        let project_aabb_rect = |model: Mat4, half: [f32; 3], center_y: f32| -> [f32; 4] {
            let corners = [
                glam::Vec3::new(-half[0], center_y - half[1], -half[2]),
                glam::Vec3::new(half[0], center_y - half[1], -half[2]),
                glam::Vec3::new(-half[0], center_y + half[1], -half[2]),
                glam::Vec3::new(half[0], center_y + half[1], -half[2]),
                glam::Vec3::new(-half[0], center_y - half[1], half[2]),
                glam::Vec3::new(half[0], center_y - half[1], half[2]),
                glam::Vec3::new(-half[0], center_y + half[1], half[2]),
                glam::Vec3::new(half[0], center_y + half[1], half[2]),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in corners {
                let w = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
        };

        // Plaques (single instance per cmd).
        // Yaku tablet batches.
        let mut yaku_tablet_slot_cursor: usize = 0;
        for batch in &yaku_tablet_batches {
            for t in batch.iter() {
                if yaku_tablet_slot_cursor >= MAX_YAKU_TABLET_SLOTS {
                    break;
                }
                let slot_i = yaku_tablet_slot_cursor;
                yaku_tablet_slot_cursor += 1;
                // Hover lift along world Y so the tablet rises off the tray.
                let lift = t.hover.clamp(0.0, 1.0) * t.extents[1] * 0.4;
                let center = pixel_to_world(
                    w,
                    h,
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift,
                );
                // Tilt top face toward the camera (same Rx sign as wood tablets).
                let tilt_deg = -25.0_f32;
                let model = translate_rot_scale(
                    center,
                    rot_rz_rx_deg(tilt_deg, t.rotation_z_deg),
                    glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]),
                );
                let model = self.apply_arrange_override("gameplay.hand.yaku_tablet", model);
                // Active tablets warm up to a champagne tint; dim ones stay
                // bone. The decal pass (phase 2) will paint the engraved name
                // on top via a per-instance albedo texture.
                // Porcelain: bright cool-white when idle; a warmer cream
                // cast when this yaku is the selected target so it still
                // reads as "active" against the row.
                let base = if t.active {
                    [1.00, 0.97, 0.90, 1.0]
                } else {
                    [0.97, 0.96, 0.94, 1.0]
                };
                let material = MaterialParams {
                    kind: MaterialKind::Porcelain,
                    base_color: base,
                    // Shiny-chiclet glaze: strong pinpoint specular plus
                    // the shader's wet-glaze and rim terms. Hovering
                    // bumps the pinpoint so the lifted tablet looks
                    // freshly polished rather than just brighter.
                    specular_strength: 0.95 + 0.25 * t.hover.clamp(0.0, 1.0),
                    specular_power: 72.0,
                };
                // Engraved-name decal: rasterise on label change, then bind
                // it as the per-instance albedo overlay. Cached by label hash
                // so the steady-state cost is one compare per slot per frame.
                let label_hash = tablet_label_hash(&t.name, 256, 96);
                let inst = &mut self.yaku_tablet_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                        &t.name,
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
                        256,
                        96,
                    );
                    inst.decal_label_hash = label_hash;
                }
                inst.write_uniform_with_decal(&self.queue, view_proj_arr, model, material, true);
                self.proj
                    .yaku_tablet_rects
                    .push(project_unit_cube_rect(model));
                self.last_yaku_tablet_models.push(model);
                self.last_debug_pickables.push((
                    "gameplay.hand.yaku_tablet".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }

        // Wood tablets migrated to Object3dKind::WoodTablet.

        // ── Object3d general-purpose placement pre-pass ──────────────────
        // Walk all Object3d batches and write uniforms into the appropriate
        // per-kind instance pools. Each kind uses its own slot cursor so
        // Object3d instances don't collide with legacy placement instances.
        // Also fills in the start/end range in the corresponding RenderOp.
        {
            let _camera_pitch_deg = {
                let look = look_target - cam_pos;
                look.z.atan2(look.y.abs()).to_degrees() + 180.0
            };

            let mut obj3d_primitive_slot: HashMap<crate::render::primitive::MeshId, usize> =
                HashMap::new();
            let mut obj3d_yaku_slot: usize = 0;
            let mut obj3d_wood_slot: usize = 0;
            let mut obj3d_relic_slot: usize = 0;
            let mut obj3d_pack_slot: usize = 0;
            let mut obj3d_talisman_slot: usize = 0;
            let mut obj3d_ribbon_slot: usize = 0;
            let mut obj3d_shrine_slot: usize = 0;
            let mut obj3d_dora_plinth_slot: usize = 0;
            let mut obj3d_orb_slot: usize = 0;
            let mut obj3d_bowl_slot: usize = 0;
            let mut obj3d_mirror_slot: usize = 0;
            let mut obj3d_tally_fan_idx: usize = 0;
            let mut obj3d_tally_stick_cursor: usize = 0;
            let mut obj3d_candle_slot: usize = 0;
            let mut obj3d_cascade_token_slot: usize = 0;
            let mut obj3d_glyph_slot: usize = 0;

            // Find the RenderOp::Object3dBatch ops to patch their start/end.
            let mut op_batch_idx: usize = 0;
            let mut obj3d_cmd_idx: usize = 0;

            for batch in &object3d_cmds {
                let batch_start = object3d_draw_list.len();

                for obj in batch.iter() {
                    use crate::render::draw_cmd::Object3dKind;
                    let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                    let model = translate_rot_scale(
                        center,
                        obj.rotation, // Mat4 set directly by the scene
                        glam::Vec3::from(obj.extents),
                    );

                    match &obj.kind {
                        Object3dKind::Primitive {
                            shape,
                            material,
                            pick_id,
                            shadow_caster: _,
                            silhouette,
                        } => {
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
                        Object3dKind::YakuTablet {
                            label,
                            active,
                            hover,
                        } => {
                            let slot_i = obj3d_yaku_slot;
                            obj3d_yaku_slot += 1;
                            if slot_i >= MAX_YAKU_TABLET_SLOTS {
                                continue;
                            }
                            let base = if *active {
                                [1.00_f32, 0.92, 0.72, 1.0]
                            } else {
                                [0.93_f32, 0.89, 0.78, 1.0]
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.30 + 0.20 * hover.clamp(0.0, 1.0),
                                specular_power: 32.0,
                            };
                            // All slots share one placement (gameplay.hand.yaku_tablet).
                            let _ = slot_i;
                            let yaku_name = "gameplay.hand.yaku_tablet";
                            let model = self.apply_arrange_override(yaku_name, model);
                            let label_hash = tablet_label_hash(label, 256, 96);
                            let inst = &mut self.yaku_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                                    label,
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
                                    256,
                                    96,
                                );
                                inst.decal_label_hash = label_hash;
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                true,
                            );
                            self.last_debug_pickables.push((
                                yaku_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::YakuTablet, slot_i));
                        }
                        Object3dKind::WoodTablet { label, pick_id } => {
                            let slot_i = obj3d_wood_slot;
                            obj3d_wood_slot += 1;
                            if slot_i >= MAX_WOOD_TABLET_SLOTS {
                                continue;
                            }
                            // Explicit `arrange_name` wins; otherwise
                            // fall back to the legacy gameplay-slot
                            // convention so saved arrange overrides for
                            // the action bar keep loading.
                            let wood_name = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else {
                                match slot_i {
                                    0 => "gameplay.action_bar.tablet_sort_suit".to_string(),
                                    1 => "gameplay.action_bar.tablet_sort_rank".to_string(),
                                    2 => "gameplay.action_bar.tablet_cash_in".to_string(),
                                    3 => "gameplay.action_bar.tablet_journal".to_string(),
                                    _ => "gameplay.action_bar.tablet".to_string(),
                                }
                            };
                            let model = self.apply_arrange_override(&wood_name, model);
                            let label_hash = tablet_label_hash(label, 512, 192);
                            let inst = &mut self.wood_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
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
                                    512,
                                    192,
                                );
                                inst.decal_label_hash = label_hash;
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                self.wood_tablet_mesh.default_material,
                                true,
                            );
                            self.proj
                                .wood_tablet_rects
                                .push(project_unit_cube_rect(model));
                            self.last_wood_tablet_models.push(model);
                            self.last_debug_pickables.push((
                                wood_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            // When a scene routes this tablet's click
                            // via `ShopHit::Dish(pid)` (shop journal
                            // button), publish the rect + model into
                            // the primitive-pick channels.
                            if let Some(pid) = pick_id {
                                self.proj
                                    .aux_dish_rects
                                    .push((Some(*pid), project_unit_cube_rect(model)));
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            object3d_draw_list.push((DrawKind::WoodTablet, slot_i));
                        }
                        Object3dKind::Relic {
                            relic_id,
                            glow,
                            silhouette,
                            pick_id,
                        } => {
                            if obj3d_relic_slot >= MAX_RELIC_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_relic_slot;
                            obj3d_relic_slot += 1;
                            // Object3dKind::Relic fires for shop for-sale relics
                            // (single column Placement) and gameplay relics
                            // (single sidebar Placement).
                            let relic_arr_name = match self.active_scene_key {
                                Some("shop") => "shop.for_sale.relics".to_string(),
                                Some("gameplay") => "gameplay.relic_col".to_string(),
                                _ => format!("relic[{slot_i}]"),
                            };
                            let model = self.apply_arrange_override(&relic_arr_name, model);
                            // obj.rotation already encodes pitch/roll; extents are full.
                            let g = if *silhouette {
                                0.0
                            } else {
                                glow.clamp(0.0, 1.0)
                            };
                            let base_color = if *silhouette {
                                // Silhouette tint: the scene controls
                                // this via `obj.color`. Collection scene
                                // passes a muted rarity accent so locked
                                // relics still read as "earned-worth-
                                // chasing" rather than pure-black dots.
                                // Any caller that wants the old solid
                                // matte can pass `[0.04, 0.04, 0.05, 1]`
                                // explicitly — which is now the accent
                                // math's identity for near-zero tint.
                                obj.color
                            } else if g > 0.0 {
                                let target = [1.55, 1.32, 0.78, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = if *silhouette {
                                crate::render::lit_mesh::MaterialParams {
                                    kind: crate::render::lit_mesh::MaterialKind::Plain,
                                    base_color,
                                    specular_strength: 0.0,
                                    specular_power: 1.0,
                                }
                            } else {
                                relic_material_params(*relic_id, base_color, g)
                            };
                            if *silhouette {
                                self.relic_instances[slot_i].write_uniform_tinted(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                    base_color,
                                );
                            } else {
                                self.relic_instances[slot_i].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                );
                            }
                            // Silhouette pass skips the relic albedo/relief
                            // texture — we want the shape only, not the
                            // engraved artwork.
                            let want_tex = if *silhouette {
                                None
                            } else if self.relic_textures.contains_key(relic_id) {
                                Some(*relic_id)
                            } else {
                                None
                            };
                            if self.relic_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.relic_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("relic-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.relic_slot_texture[slot_i] = want_tex;
                            }
                            self.last_relic_models.push((model, *relic_id));
                            if let Some(pid) = pick_id {
                                self.last_pickable_relic_models
                                    .push((*pid, model, *relic_id));
                            }
                            let projected_rect = project_unit_cube_rect(model);
                            self.proj.relic_rects.push(projected_rect);
                            self.last_debug_pickables.push((
                                relic_arr_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            if g > 0.0 {
                                // Activation halo: champagne bloom inflated past
                                // the projected rect so the additive falloff
                                // spills out past the relic silhouette.
                                let [rx, ry, rw, rh] = projected_rect;
                                let pad_x = rw * 0.85;
                                let pad_y = rh * 0.95;
                                relic_glows.push(GpuInstance {
                                    rect: [
                                        rx - pad_x,
                                        ry - pad_y,
                                        rw + pad_x * 2.0,
                                        rh + pad_y * 2.0,
                                    ],
                                    color: [1.00, 0.82, 0.36, 1.20 * g],
                                });
                            }
                            object3d_draw_list.push((DrawKind::Relic, slot_i));
                        }
                        Object3dKind::Pack { kind, pick_id } => {
                            if obj3d_pack_slot >= self.pack_instances.len() {
                                continue;
                            }
                            let slot_i = obj3d_pack_slot;
                            obj3d_pack_slot += 1;
                            let _ = slot_i;
                            let pack_arr_name = obj.arrange_name.unwrap_or("shop.for_sale.packs");
                            let model = self.apply_arrange_override(pack_arr_name, model);
                            let material = MaterialParams {
                                kind: MaterialKind::Foil,
                                base_color: obj.color,
                                specular_strength: 0.70,
                                specular_power: 48.0,
                            };
                            self.pack_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            let want_tex = if self.pack_textures.contains_key(kind) {
                                Some(*kind)
                            } else {
                                None
                            };
                            if self.pack_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.pack_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("pack-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.pack_slot_texture[slot_i] = want_tex;
                            }
                            // Project the 8 unit-cube corners via the model matrix to get
                            // the screen-space bounding rect. This feeds focus-nav and
                            // controller selection via aux_dish_rects (appended below).
                            self.proj
                                .pack_rects
                                .push((project_unit_cube_rect(model), *pick_id));
                            self.last_debug_pickables.push((
                                pack_arr_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Pack, slot_i));
                        }
                        Object3dKind::Talisman { kind } => {
                            if obj3d_talisman_slot >= MAX_TALISMAN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_talisman_slot;
                            obj3d_talisman_slot += 1;
                            // extents already encode full size; scale to mesh local half-extents.
                            let sx = obj.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                            let sy = obj.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                            let sz = obj.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                            let _ = slot_i;
                            // Default to the for-sale stall arrange group, but
                            // let the caller opt into a different group (e.g.
                            // owned-inventory talismans, which shouldn't share
                            // the shop's Rx/Ry/Rz arrange rotation).
                            let talisman_name =
                                obj.arrange_name.unwrap_or("shop.for_sale.talismans");
                            let talisman_center_arr = self.apply_arrange_override(
                                talisman_name,
                                translate_rot_scale(
                                    center,
                                    obj.rotation,
                                    glam::Vec3::new(sx, sy, sz),
                                ),
                            );
                            // Re-decompose center after possible override; simpler: re-derive center from matrix.
                            let talisman_model = talisman_center_arr;
                            let material = talisman_material(*kind, obj.color);
                            let kind_idx = crate::core::talisman::TalismanKind::all()
                                .iter()
                                .position(|&k| k == *kind)
                                .unwrap_or(0) as u8;
                            if self.talisman_slot_kind[slot_i] != Some(kind_idx) {
                                self.talisman_instances[slot_i].rebind_texture(
                                    &self.device,
                                    &self.lit_mesh_material_layout,
                                    &self.talisman_height_views[kind_idx as usize],
                                    &self.lit_mesh_relief_default_view,
                                    &self.tile_sampler,
                                );
                                self.talisman_slot_kind[slot_i] = Some(kind_idx);
                            }
                            self.talisman_instances[slot_i].write_uniform_raw_w(
                                &self.queue,
                                view_proj_arr,
                                talisman_model,
                                material,
                                kind_idx as f32,
                            );
                            self.last_talisman_models.push(talisman_model);
                            self.proj
                                .talisman_rects
                                .push(project_unit_cube_rect(talisman_model));
                            self.last_debug_pickables.push((
                                talisman_name.to_string(),
                                talisman_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Talisman, slot_i));
                        }
                        Object3dKind::ZodiacRibbon { kind } => {
                            // extents: [width, length, depth].
                            let eff_w = obj.extents[0];
                            let eff_l = obj.extents[1];
                            let depth = obj.extents[2];
                            // Push the overall ribbon AABB for arrange-mode picking.
                            // (Individual segments aren't separately selectable.)
                            let ribbon_arr_name =
                                obj.arrange_name.unwrap_or("shop.for_sale.ribbons");
                            let base_transform = self.apply_arrange_override(
                                ribbon_arr_name,
                                translate_rot_scale(center, obj.rotation, glam::Vec3::splat(1.0)),
                            );
                            let full_ribbon_model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, eff_l, depth),
                            );
                            self.last_ribbon_models.push(full_ribbon_model);
                            self.proj
                                .ribbon_rects
                                .push(project_unit_cube_rect(full_ribbon_model));
                            self.last_debug_pickables.push((
                                ribbon_arr_name.to_string(),
                                full_ribbon_model,
                                glam::Vec3::new(0.5, 0.5, 0.5),
                                0.0,
                            ));
                            let cap_h = eff_w * 0.6;
                            let mid_h = (eff_l - cap_h * 2.0).max(0.0);
                            let silk_mat = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.25,
                                specular_power: 16.0,
                            };
                            let zodiac_id: Option<u8> = kind.as_ref().and_then(|z| {
                                let tex_idx = crate::core::zodiac::ZodiacKind::all()
                                    .iter()
                                    .position(|&k| k == *z)?
                                    as u8;
                                Some(tex_idx)
                            });
                            // Emit segments: top cap (seg 0), optional mid (seg 1), bottom cap (seg 2).
                            let segments: &[(f32, f32, u8)] = if mid_h > 0.0 {
                                &[
                                    (0.0, cap_h, 0),
                                    (-cap_h, mid_h, 1),
                                    (-(cap_h + mid_h), cap_h, 2),
                                ]
                            } else {
                                &[(0.0, cap_h, 0), (-(cap_h), cap_h, 2)]
                            };
                            for &(offset, seg_h, seg_idx) in segments {
                                if obj3d_ribbon_slot >= MAX_RIBBON_SLOTS {
                                    break;
                                }
                                let slot_i = obj3d_ribbon_slot;
                                obj3d_ribbon_slot += 1;
                                let seg_model = ribbon_submesh(
                                    base_transform,
                                    offset,
                                    glam::Vec3::new(eff_w, seg_h, depth),
                                );
                                let rzod = zodiac_id.map(|ti| (ti, seg_idx));
                                if self.ribbon_slot_zodiac[slot_i] != rzod {
                                    let view: &wgpu::TextureView = match rzod {
                                        Some((idx, 0)) => {
                                            &self.ribbon_zodiac_tex.top_views[idx as usize]
                                        }
                                        Some((idx, 1)) => {
                                            &self.ribbon_zodiac_tex.mid_views[idx as usize]
                                        }
                                        Some((idx, _)) => {
                                            &self.ribbon_zodiac_tex.bot_views[idx as usize]
                                        }
                                        None => &self.lit_mesh_white_view,
                                    };
                                    let inst = &mut self.ribbon_instances[slot_i];
                                    inst.bind_group =
                                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some("ribbon-bg-obj3d"),
                                            layout: &self.lit_mesh_material_layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: inst
                                                        .uniform_buffer
                                                        .as_entire_binding(),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 1,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        view,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.tile_sampler,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 3,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &self.lit_mesh_relief_default_view,
                                                    ),
                                                },
                                            ],
                                        });
                                    self.ribbon_slot_zodiac[slot_i] = rzod;
                                }
                                self.ribbon_instances[slot_i].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    seg_model,
                                    silk_mat,
                                );
                                object3d_draw_list.push((DrawKind::Ribbon, slot_i));
                            }
                        }
                        Object3dKind::Shrine { glow } => {
                            if obj3d_shrine_slot >= MAX_SHRINE_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_shrine_slot;
                            obj3d_shrine_slot += 1;
                            // Shrines are pick-blind only; one placement per slot.
                            let shrine_name = match slot_i {
                                0 => "pick_blind.shrine[0]",
                                1 => "pick_blind.shrine[1]",
                                2 => "pick_blind.shrine[2]",
                                _ => "pick_blind.shrine",
                            };
                            // Shrine center is lifted by half-height; scene passes base pos.
                            let shrine_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            // The shrine mesh is built Y-up; rotate into Z-up world so it
                            // stands upright rather than lying flat. Compose with any
                            // scene-level obj.rotation (e.g. arrange-mode overrides).
                            let shrine_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let shrine_model = self.apply_arrange_override(
                                shrine_name,
                                translate_rot_scale(
                                    shrine_center,
                                    shrine_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 1.05, 0.95, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color,
                                specular_strength: 0.06,
                                specular_power: 8.0,
                            };
                            self.shrine_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                shrine_model,
                                material,
                            );
                            // Project AABB for shrine_rects (label anchoring).
                            let shrine_world_center = shrine_model.w_axis.truncate();
                            let [hx, hy, hz] = [
                                obj.extents[0] * 0.5,
                                obj.extents[1] * 0.5,
                                obj.extents[2] * 0.5,
                            ];
                            let (mut mn_x, mut mn_y, mut mx_x, mut mx_y) = (
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            );
                            for cx in [-hx, hx] {
                                for cy in [-hy, hy] {
                                    for cz in [-hz, hz] {
                                        let world =
                                            shrine_world_center + glam::Vec3::new(cx, cy, cz);
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
                            self.last_debug_pickables.push((
                                shrine_name.to_string(),
                                shrine_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Shrine, slot_i));
                        }
                        Object3dKind::DoraPlinth { glow } => {
                            if obj3d_dora_plinth_slot >= MAX_DORA_PLINTH_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_dora_plinth_slot;
                            obj3d_dora_plinth_slot += 1;
                            let plinth_name = "gameplay.dora_plinth";
                            // Mesh is built Y-up centered; lift the world position
                            // by half-height so `obj.pos` describes the plinth's
                            // base sitting on the table felt.
                            let plinth_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            let plinth_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let plinth_model = self.apply_arrange_override(
                                plinth_name,
                                translate_rot_scale(
                                    plinth_center,
                                    plinth_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 0.95, 0.55, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Metal,
                                base_color,
                                specular_strength: 0.85,
                                specular_power: 64.0,
                            };
                            self.dora_plinth_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                plinth_model,
                                material,
                            );
                            // Project AABB → screen rect for hover/focus.
                            let plinth_world_center = plinth_model.w_axis.truncate();
                            let [hx, hy, hz] = [
                                obj.extents[0] * 0.5,
                                obj.extents[1] * 0.5,
                                obj.extents[2] * 0.5,
                            ];
                            let (mut mn_x, mut mn_y, mut mx_x, mut mx_y) = (
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            );
                            for cx in [-hx, hx] {
                                for cy in [-hy, hy] {
                                    for cz in [-hz, hz] {
                                        let world =
                                            plinth_world_center + glam::Vec3::new(cx, cy, cz);
                                        let (px, py) = project_to_screen(world);
                                        mn_x = mn_x.min(px);
                                        mn_y = mn_y.min(py);
                                        mx_x = mx_x.max(px);
                                        mx_y = mx_y.max(py);
                                    }
                                }
                            }
                            self.proj.dora_plinth_rect =
                                Some([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            self.last_debug_pickables.push((
                                plinth_name.to_string(),
                                plinth_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::DoraPlinth, slot_i));
                        }
                        Object3dKind::SellTray { pick_id } => {
                            // Round dish mesh is built Y-up; rotate local Y
                            // into world Z so the rim sits flat on the table
                            // and `extents[1]` (rim) becomes vertical
                            // thickness. Compose with any scene rotation.
                            let oriented = mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let model = translate_rot_scale(
                                center,
                                oriented,
                                glam::Vec3::from(obj.extents),
                            );
                            let model = self.apply_arrange_override("shop.shelf.sell_tray", model);
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.3,
                                specular_power: 16.0,
                            };
                            self.sell_tray_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            if let Some(pid) = pick_id {
                                self.last_sell_tray_model = Some((model, *pid));
                            }
                            // Folded "SELL" tent card sits in the recess when
                            // the tray is focused (any control method). The
                            // shop scene encodes focus state via hover_target
                            // (≥0.5 = focused/hovered).
                            if obj.hover_target >= 0.5 {
                                if !self.sell_card_decal_ready {
                                    let rgba = crate::render::decal::rasterize_tablet_label_decal(
                                        "SELL",
                                        self.ui_font.as_ref(),
                                        self.emoji_font.as_ref(),
                                        256,
                                        128,
                                        [0.62, 0.18, 0.14, 1.0],
                                    );
                                    self.sell_card_instance.set_decal(
                                        crate::render::lit_mesh::DecalUploadCtx {
                                            device: &self.device,
                                            queue: &self.queue,
                                            layout: &self.lit_mesh_material_layout,
                                            sampler: &self.tile_sampler,
                                            relief_view: &self.lit_mesh_relief_default_view,
                                        },
                                        &rgba,
                                        256,
                                        128,
                                    );
                                    self.sell_card_decal_ready = true;
                                }
                                // Build the card model matrix anchored to the
                                // tray. Local card extents: x=-0.5..0.5,
                                // y=0..0.5, z=-0.5..0.5. The tray is a unit
                                // box with rim top at +0.5 and recess at +0.2;
                                // we shrink the card to fit inside the rim and
                                // sit on the recessed floor.
                                let (scale, rot, trans) = model.to_scale_rotation_translation();
                                // Card footprint: 60% of rim diameter, height
                                // ~70% of rim depth.
                                // Card height is decoupled from the (very
                                // shallow) rim thickness so it stays readable
                                // on the flat plate; sized off the plate
                                // footprint instead.
                                let footprint = scale.x.min(scale.z);
                                let card_scale = glam::Vec3::new(
                                    scale.x * 0.55,
                                    footprint * 0.55,
                                    scale.z * 0.55,
                                );
                                // Sit the card just above the rim top
                                // (local y=+0.5) so it doesn't poke through
                                // the shallow plate. Nudged back along local
                                // -z (world +y, deeper into scene) so the
                                // card stands toward the rear of the dish
                                // instead of centered in the recess.
                                let local_floor = glam::Vec3::new(0.0, 0.55, -0.15);
                                let world_floor = trans + rot * (local_floor * scale);
                                // Yaw the card 100° around world +Z so the
                                // crease faces the camera at a slight angle.
                                let yaw = glam::Quat::from_rotation_z(100.0_f32.to_radians());
                                let card_rot = yaw * rot;
                                let card_model = Mat4::from_scale_rotation_translation(
                                    card_scale,
                                    card_rot,
                                    world_floor,
                                );
                                let card_material = MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: [0.96, 0.93, 0.84, 1.0],
                                    specular_strength: 0.10,
                                    specular_power: 8.0,
                                };
                                self.sell_card_instance.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    card_model,
                                    card_material,
                                    true,
                                );
                                self.last_sell_card_model = Some(card_model);
                            }
                            self.last_debug_pickables.push((
                                "shop.shelf.sell_tray".to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::SellTray, 0));
                        }
                        Object3dKind::ShopLamp { glow } => {
                            // Lamp mesh is in world-space Z-up convention: no corrective
                            // rotation needed. pos is the apex/cord-attachment point (high Z).
                            // The shade rim (wide, open end) hangs below at lower Z. ✓
                            let lamp_center =
                                pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let lamp_model = self.apply_arrange_override(
                                "shop.props.lamp",
                                translate_rot_scale(
                                    lamp_center,
                                    obj.rotation,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            // Body — brass Metal material.
                            self.lamp_body_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                self.lamp_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::LampBody, 0));
                            // Bulb — Glass material. Push brightness well above
                            // 1.0 when glow is active so the HDR bulb color
                            // crosses the bloom extract threshold and glares.
                            let g = glow.clamp(0.0, 1.0);
                            let dm = &self.lamp_bulb_mesh.default_material;
                            let bulb_mat = MaterialParams {
                                kind: crate::render::lit_mesh::MaterialKind::Glass,
                                base_color: [
                                    dm.base_color[0] * (1.0 + g * 1.4),
                                    dm.base_color[1] * (1.0 + g * 1.0),
                                    dm.base_color[2] * (1.0 + g * 0.5),
                                    1.0,
                                ],
                                specular_strength: dm.specular_strength,
                                specular_power: dm.specular_power,
                            };
                            self.lamp_bulb_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                bulb_mat,
                            );
                            object3d_draw_list.push((DrawKind::LampBulb, 0));
                            // Trimesh pick: AABB of extents [w,h,w] is a bad
                            // silhouette for a lamp (thin cord on top of a wide
                            // shade) and invites accidental grabs on empty air
                            // above the shade. Ray-cast against the actual cord
                            // + cone triangles so the pick region matches what
                            // the player sees.
                            self.last_debug_trimesh_pickables.push((
                                "shop.props.lamp".to_string(),
                                lamp_model,
                                TrimeshRef::LampBody,
                            ));
                        }
                        Object3dKind::Bug {
                            slot,
                            flap_rad,
                            live_wing_alpha,
                            blur_alpha,
                        } => {
                            let slot = *slot;
                            if slot >= MAX_BUG_SLOTS {
                                continue;
                            }
                            let bug_model = translate_rot_scale(
                                center,
                                obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            self.bug_body_instances[slot].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                self.bug_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::BugBody, slot));
                            // Live wing model matrices: the mesh lives in +Y,
                            // so the left wing is the identity and the
                            // right wing flips Y (mirror across body).
                            // Flap rotates about mesh +X, which is the
                            // body axis — the right wing uses -flap so
                            // the two counter-sweep like a moth's.
                            let flap_l = glam::Mat4::from_rotation_x(*flap_rad);
                            let flap_r = glam::Mat4::from_rotation_x(-*flap_rad)
                                * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0));
                            let live_a = live_wing_alpha.clamp(0.0, 1.0);
                            let wing_mat = self.bug_wing_mesh.default_material;
                            let live_tint = [
                                wing_mat.base_color[0],
                                wing_mat.base_color[1],
                                wing_mat.base_color[2],
                                wing_mat.base_color[3] * live_a,
                            ];
                            self.bug_wing_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_l,
                                wing_mat,
                                live_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingL, slot));
                            self.bug_wing_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_r,
                                wing_mat,
                                live_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingR, slot));
                            // Blur fans — the swept-volume mesh is drawn once per
                            // side with no flap rotation (the mesh itself is the
                            // full sweep). The right side reuses the same mesh
                            // with a Y-mirror transform, matching how the live
                            // wing pair is built.
                            let blur_a = blur_alpha.clamp(0.0, 1.0);
                            let blur_mat = self.bug_wing_blur_mesh.default_material;
                            let blur_tint = [
                                blur_mat.base_color[0],
                                blur_mat.base_color[1],
                                blur_mat.base_color[2],
                                blur_mat.base_color[3] * blur_a,
                            ];
                            self.bug_wing_blur_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                blur_mat,
                                blur_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingBlurL, slot));
                            self.bug_wing_blur_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0)),
                                blur_mat,
                                blur_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingBlurR, slot));
                        }
                        Object3dKind::MaterialOrb { material } => {
                            if obj3d_orb_slot >= MAX_ORB_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_orb_slot;
                            obj3d_orb_slot += 1;
                            self.orb_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                *material,
                            );
                            object3d_draw_list.push((DrawKind::Orb, slot_i));
                        }
                        Object3dKind::Mirror {
                            rotation_x_deg,
                            rotation_z_deg,
                        } => {
                            if obj3d_mirror_slot >= MAX_MIRROR_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_mirror_slot;
                            obj3d_mirror_slot += 1;
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 2 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            let tilt_deg = *rotation_x_deg + anim * 22.0;
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                rot_rz_rx_deg(tilt_deg, *rotation_z_deg),
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.mirror", hover_model);
                            self.mirror_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.mirror_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.mirror_rect = Some(project_aabb_rect(
                                    hover_model,
                                    MIRROR_LOCAL_HALF,
                                    MIRROR_LOCAL_CENTER_Y,
                                ));
                                self.last_mirror_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.mirror".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    MIRROR_LOCAL_HALF[0],
                                    MIRROR_LOCAL_HALF[1],
                                    MIRROR_LOCAL_HALF[2],
                                ),
                                MIRROR_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Mirror, slot_i));
                        }
                        Object3dKind::ExtrudedGlyph {
                            scale: g_scale,
                            rotation_x: g_rx,
                            rotation_y: g_ry,
                            label,
                            emissive,
                            material: g_mat,
                        } => {
                            if obj3d_glyph_slot >= MAX_EXTRUDED_GLYPH_SLOTS {
                                continue;
                            }
                            if !self.extruded_glyph_meshes.contains_key(label) {
                                if let Some(cpu) = self.glyph_cpu_cache.mesh_for(label) {
                                    let gpu = LitMeshGpu::new(
                                        &self.device,
                                        cpu,
                                        &format!("glyph-{}", label),
                                    );
                                    self.extruded_glyph_meshes.insert(label.clone(), gpu);
                                } else {
                                    continue;
                                }
                            }
                            let slot_i = obj3d_glyph_slot;
                            obj3d_glyph_slot += 1;
                            let g_center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let glyph_model = translate_rot_scale(
                                g_center,
                                score_popup_glyph_rot_rad(
                                    *g_ry,
                                    -std::f32::consts::FRAC_PI_2 + *g_rx,
                                ),
                                glam::Vec3::splat(*g_scale),
                            );
                            let glyph_model =
                                self.apply_arrange_override("gameplay.score_popup", glyph_model);
                            let material = match g_mat {
                                crate::render::draw_cmd::GlyphMaterial::Metal => MaterialParams {
                                    kind: MaterialKind::Metal,
                                    base_color: obj.color,
                                    specular_strength: 1.0,
                                    specular_power: 128.0,
                                },
                                crate::render::draw_cmd::GlyphMaterial::Polychrome => {
                                    MaterialParams {
                                        kind: MaterialKind::Polychrome,
                                        base_color: obj.color,
                                        specular_strength: 0.85,
                                        specular_power: 48.0,
                                    }
                                }
                                crate::render::draw_cmd::GlyphMaterial::Plain => MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: obj.color,
                                    specular_strength: 0.35 + 0.20 * emissive.clamp(0.0, 1.0),
                                    specular_power: 96.0,
                                },
                            };
                            self.extruded_glyph_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                glyph_model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                "gameplay.score_popup".to_string(),
                                glyph_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::ExtrudedGlyph, slot_i));
                        }
                        Object3dKind::CascadeToken { kind: ck, pulse } => {
                            if obj3d_cascade_token_slot >= MAX_CASCADE_TOKEN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_cascade_token_slot;
                            obj3d_cascade_token_slot += 1;
                            let pulse_f = pulse.clamp(0.0, 1.0);
                            let pulse_scale = 1.0 + 0.18 * pulse_f;
                            let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let cascade_token_name = match ck {
                                CascadeTokenKind::Chips => "gameplay.cascade_token.chips",
                                CascadeTokenKind::Mult => "gameplay.cascade_token.mult",
                            };
                            let model = translate_rot_scale(
                                center,
                                Mat4::IDENTITY,
                                glam::Vec3::new(
                                    obj.extents[0] * pulse_scale,
                                    obj.extents[1] * pulse_scale,
                                    obj.extents[2] * pulse_scale,
                                ),
                            );
                            let model = self.apply_arrange_override(cascade_token_name, model);
                            let base = match ck {
                                CascadeTokenKind::Chips => [0.55, 0.78, 1.00, 1.0],
                                CascadeTokenKind::Mult => [0.85, 0.32, 0.42, 1.0],
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.40 + 0.30 * pulse_f,
                                specular_power: 48.0,
                            };
                            self.cascade_token_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                cascade_token_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::CascadeToken, slot_i));
                        }
                        Object3dKind::Candle {
                            scale,
                            height_scale,
                        } => {
                            let slot_i = obj3d_candle_slot;
                            obj3d_candle_slot += 1;
                            if self.candle_instances.get(slot_i).is_none() {
                                continue;
                            }
                            let base = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let s = *scale;
                            let candle_model = translate_rot_scale(
                                base,
                                mesh_y_thickness_along_local_y_to_z_up(),
                                glam::Vec3::new(s, s * *height_scale, s),
                            );
                            let candle_name = self.scene_path("candle");
                            let candle_model =
                                self.apply_arrange_override(&candle_name, candle_model);
                            self.candle_instances[slot_i][0].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wax_mesh.default_material,
                            );
                            self.candle_instances[slot_i][1].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wick_mesh.default_material,
                            );
                            self.last_debug_pickables.push((
                                candle_name,
                                candle_model,
                                glam::Vec3::new(0.36, 0.305, 0.36),
                                0.305,
                            ));
                            object3d_draw_list.push((DrawKind::CandleWax, slot_i));
                            object3d_draw_list.push((DrawKind::CandleWick, slot_i));
                        }
                        Object3dKind::TallyFan {
                            stick_len,
                            stick_wide,
                            stick_thickness,
                            count,
                            max_count,
                            spread_deg,
                            tip_color,
                            rotation_y_deg,
                            kind: fan_kind,
                        } => {
                            let fan_i = obj3d_tally_fan_idx;
                            obj3d_tally_fan_idx += 1;
                            if fan_i >= MAX_TALLY_FAN_SLOTS {
                                continue;
                            }
                            let max_c: u32 = (*max_count).max(1);
                            let count_usize = (*count).min(max_c) as usize;
                            let spread_rad = spread_deg.to_radians();
                            let slot_angle = |k: u32| -> f32 {
                                if max_c <= 1 {
                                    0.0
                                } else {
                                    -spread_rad * 0.5
                                        + (k as f32) * (spread_rad / (max_c as f32 - 1.0))
                                }
                            };
                            let pivot = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let base_orient = mesh_y_thickness_along_local_y_to_z_up();
                            let fan_yaw = Mat4::from_rotation_z(rotation_y_deg.to_radians());
                            let base_scale =
                                glam::Vec3::new(*stick_wide, *stick_len, *stick_thickness);
                            let base_material = self.tally_stick_base_mesh.default_material;
                            let tip_material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: *tip_color,
                                specular_strength: 0.40,
                                specular_power: 42.0,
                            };
                            let arrange_name = match fan_kind {
                                TallyFanKind::Draws => "gameplay.counter.draws_fan",
                                TallyFanKind::Discards => "gameplay.counter.discards_fan",
                            };
                            let missing = (max_c as usize).saturating_sub(count_usize);
                            let mut visible_slots: Vec<u32> = (0..max_c).collect();
                            for trim in 0..missing {
                                if trim % 2 == 0 {
                                    visible_slots.pop();
                                } else {
                                    visible_slots.remove(0);
                                }
                            }
                            for (stick_i, &k) in visible_slots.iter().enumerate() {
                                if obj3d_tally_stick_cursor + 1 >= MAX_TALLY_STICK_SLOTS * 2 {
                                    break;
                                }
                                let angle = slot_angle(k);
                                let rot = fan_yaw * Mat4::from_rotation_y(angle) * base_orient;
                                let model = translate_rot_scale(pivot, rot, base_scale);
                                let model = self.apply_arrange_override(arrange_name, model);
                                if stick_i == 0 {
                                    self.last_debug_pickables.push((
                                        arrange_name.to_string(),
                                        model,
                                        glam::Vec3::new(0.5, 0.5, 0.5),
                                        0.0,
                                    ));
                                }
                                self.tally_stick_instances[obj3d_tally_stick_cursor].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    base_material,
                                );
                                self.tally_stick_instances[obj3d_tally_stick_cursor + 1]
                                    .write_uniform(&self.queue, view_proj_arr, model, tip_material);
                                object3d_draw_list
                                    .push((DrawKind::TallyStickBase, obj3d_tally_stick_cursor));
                                object3d_draw_list
                                    .push((DrawKind::TallyStickTip, obj3d_tally_stick_cursor + 1));
                                obj3d_tally_stick_cursor += 2;
                            }
                            let fan_width =
                                *stick_len * (spread_rad * 0.5).sin() * 2.0 + *stick_wide;
                            let fan_height = *stick_len + *stick_wide * 0.5;
                            let fan_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + *stick_len * 0.5,
                            );
                            let fan_model = translate_rot_scale(
                                fan_center,
                                fan_yaw,
                                glam::Vec3::new(fan_width, *stick_thickness * 2.0, fan_height),
                            );
                            let slot_idx = match fan_kind {
                                TallyFanKind::Draws => 0,
                                TallyFanKind::Discards => 1,
                            };
                            self.proj.peg_rects[slot_idx] = Some(project_unit_cube_rect(fan_model));
                        }
                        Object3dKind::Bowl => {
                            if obj3d_bowl_slot >= MAX_BOWL_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_bowl_slot;
                            obj3d_bowl_slot += 1;
                            // Ease hover in-place (can't call self.ease_hover mid-borrow).
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 1 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            // Recompute model with hover lift + tilt baked in.
                            // Scene passes rotation_x_deg via obj.rotation (Mat4::from_rotation_x).
                            let tilt = anim * 18.0_f32.to_radians();
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                glam::Mat4::from_rotation_x(tilt) * obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.bowl", hover_model);
                            self.bowl_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.bowl_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.bowl_rect = Some(project_aabb_rect(
                                    hover_model,
                                    BOWL_LOCAL_HALF,
                                    BOWL_LOCAL_CENTER_Y,
                                ));
                                self.last_bowl_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.bowl".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    BOWL_LOCAL_HALF[0],
                                    BOWL_LOCAL_HALF[1],
                                    BOWL_LOCAL_HALF[2],
                                ),
                                BOWL_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Bowl, slot_i));
                        }
                    }
                }

                let batch_end = object3d_draw_list.len();
                // Patch the placeholder RenderOp that was pushed during the cmd walk.
                // Find the correct Object3dBatch op by scanning from op_batch_idx.
                while op_batch_idx < ops.len() {
                    if let RenderOp::Object3dBatch { start, end } = &mut ops[op_batch_idx]
                        && *start == 0
                        && *end == 0
                    {
                        *start = batch_start;
                        *end = batch_end;
                        op_batch_idx += 1;
                        break;
                    }
                    op_batch_idx += 1;
                }
                obj3d_cmd_idx += 1;
            }
            let _ = obj3d_cmd_idx;
        }

        // Wall stack: facedown tiles laid out in a row at the back of the
        // table, height growing slightly as the stack thickens. Phase 1 uses
        // the bone tablet mesh (a plain box) — phase 7 may swap to the real
        // tile mesh.
        let mut wall_tile_slot_cursor: usize = 0;
        for w_cmd in wall_stack_cmds.iter() {
            let row_len = w_cmd.row_len.max(1);
            let total = w_cmd.remaining.min(MAX_WALL_TILE_SLOTS as u32);
            let tile_w = w_cmd.tile_extents[0];
            let tile_h = w_cmd.tile_extents[1];
            let tile_d = w_cmd.tile_extents[2];
            for k in 0..total {
                if wall_tile_slot_cursor >= MAX_WALL_TILE_SLOTS {
                    break;
                }
                let col = k % row_len;
                let layer = k / row_len;
                let px = w_cmd.world_pos[0] + col as f32 * tile_w;
                let py = w_cmd.world_pos[1] + layer as f32 * tile_d;
                let pz = w_cmd.world_pos[2] + tile_h * 0.5;
                let center = pixel_to_world(w, h, px, py, pz);
                let model = translate_rot_scale(
                    center,
                    Mat4::IDENTITY,
                    glam::Vec3::new(tile_w, tile_h, tile_d),
                );
                let model = self.apply_arrange_override("WallTile", model);
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: [0.86, 0.81, 0.69, 1.0],
                    specular_strength: 0.20,
                    specular_power: 24.0,
                };
                self.wall_tile_instances[wall_tile_slot_cursor].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                // Wall tiles aren't arrangeable — keep the legacy name so
                // the hit-test debug overlay still identifies them.
                self.last_debug_pickables.push((
                    "gameplay.wall_tile".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
                wall_tile_slot_cursor += 1;
            }
        }

        // Cascade scoring tokens migrated to Object3dKind::CascadeToken.

        // Extruded-glyph score popups migrated to Object3dKind::ExtrudedGlyph.

        // ── Arrange-mode bounding box overlay ──────────────────────────────
        // When an object is selected in arrange mode, draw a 2D screen-space
        // rectangle outline around its projected AABB so the user can see
        // exactly what they're moving.
        if let Some(ref ov) = self.debug_arrange_override {
            let aabb = self
                .last_debug_pickables
                .iter()
                .find(|(n, _, _, _)| n == &ov.name)
                .map(|(_, m, h, o)| (*m, *h, *o))
                .or_else(|| {
                    self.last_debug_trimesh_pickables
                        .iter()
                        .find(|(n, _, _)| n == &ov.name)
                        .map(|(_, m, mesh)| match mesh {
                            TrimeshRef::LampBody => {
                                (*m, self.lamp_body_local_half, self.lamp_body_local_center_y)
                            }
                        })
                });
            if let Some((model, half, center_y)) = aabb {
                let [rx, ry, rw, rh] = project_aabb_rect(model, [half.x, half.y, half.z], center_y);
                let t = (h * 0.003).max(2.0); // border thickness in pixels
                let color = [1.0_f32, 0.85, 0.25, 0.9]; // gold
                let border_quads: [GpuInstance; 4] = [
                    // top
                    GpuInstance {
                        rect: [rx, ry, rw, t],
                        color,
                    },
                    // bottom
                    GpuInstance {
                        rect: [rx, ry + rh - t, rw, t],
                        color,
                    },
                    // left
                    GpuInstance {
                        rect: [rx, ry, t, rh],
                        color,
                    },
                    // right
                    GpuInstance {
                        rect: [rx + rw - t, ry, t, rh],
                        color,
                    },
                ];
                let buf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("arrange-bbox"),
                        contents: bytemuck::cast_slice(&border_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let buf_idx = quad_buffers.len();
                quad_buffers.push(buf);
                ops.push(RenderOp::QuadBatch { buf_idx, count: 4 });
            }

            // Clamp-band hint for the selected pickable. Two thin lines at
            // the clamp walls — dim gold when the current `center_frac` is
            // inside the band, red-thick on whichever wall is currently
            // pinning it. Tells the user at a glance why a nudge isn't
            // moving the object any further.
            if let Some(clamp) = frame.arrange_clamps.iter().find(|c| c.name == ov.name) {
                use crate::render::draw_cmd::ClampAxis;
                let dim = [1.0_f32, 0.85, 0.25, 0.35];
                let pin = [1.0_f32, 0.30, 0.25, 0.95];
                let line_t = (h * 0.0018).max(1.5);
                let pin_t = line_t * 3.0;
                let below = clamp.center_frac < clamp.lo_frac;
                let above = clamp.center_frac > clamp.hi_frac;
                let (lo_color, lo_thick) = if below { (pin, pin_t) } else { (dim, line_t) };
                let (hi_color, hi_thick) = if above { (pin, pin_t) } else { (dim, line_t) };
                let clamp_quads: [GpuInstance; 2] = match clamp.axis {
                    ClampAxis::Horizontal => {
                        let lo_px = clamp.lo_frac * w;
                        let hi_px = clamp.hi_frac * w;
                        [
                            GpuInstance {
                                rect: [lo_px - lo_thick * 0.5, 0.0, lo_thick, h],
                                color: lo_color,
                            },
                            GpuInstance {
                                rect: [hi_px - hi_thick * 0.5, 0.0, hi_thick, h],
                                color: hi_color,
                            },
                        ]
                    }
                };
                let buf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("arrange-clamp"),
                        contents: bytemuck::cast_slice(&clamp_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let buf_idx = quad_buffers.len();
                quad_buffers.push(buf);
                ops.push(RenderOp::QuadBatch { buf_idx, count: 2 });
            }
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

        // ── Volumetric smoke setup (camera, bounds, cursor) ─────────────
        // Done before the encoder is created so the per-tile impulses
        // queued during the tile-loop above are still pending. The grid
        // is sized to comfortably bracket the table area in world units.
        if let Some(ref mut fluid) = self.fluid {
            // Grid bounds: a box roughly enclosing the table with vertical
            // headroom for smoke to rise. World space is Z-up (see
            // `crate::render::world_space`): X is screen-horizontal, Y is
            // screen far/near, Z is up out of the felt. The buoyancy and
            // floor passes use world Z for height_frac, so the grid must be
            // *tall in Z* — not Y, which was the old Y-up convention.
            let half_w = w * 0.75;
            let half_y = h * 0.75;
            let smoke_box_h = h * 0.75 + 12.0;
            let grid_min = glam::Vec3::new(-half_w, -half_y, -12.0);
            let grid_max = glam::Vec3::new(half_w, half_y, grid_min.z + 2.0 * smoke_box_h);
            fluid.set_grid_bounds(grid_min, grid_max);

            // Scene-driven wind gusts. Scenes (currently gameplay) push these
            // when they want a deliberate, time-shaped breath of wind on the
            // smoke — e.g. blowing the post-deal smoke off the hand strip a
            // few seconds after dealing. Coordinates are layout pixels; we
            // run them through `pixel_to_world` so the gust lands on the
            // table plane.
            for g in frame.wind_gusts.iter() {
                let pos = pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
                fluid.inject_impulse(
                    pos,
                    glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]),
                    g.radius,
                    g.density * 0.35,
                    0.0,
                    0.0,
                );
            }

            // Opaque shadow casters (shop bugs, etc.). Project pixel-space
            // xy onto the table plane using the same mapping as wind
            // gusts so the shadows land where the meshes visibly are.
            let occluders: Vec<crate::render::fluid::BugOccluder> = frame
                .bug_occluders
                .iter()
                .map(|b| crate::render::fluid::BugOccluder {
                    world_pos: pixel_to_world(w, h, b.center_px.0, b.center_px.1, b.lift),
                    radius: b.radius,
                    strength: b.strength,
                })
                .collect();
            fluid.set_occluders(&occluders);

            // Cursor → table-plane impulse trail. Unproject the screen
            // cursor, intersect z=5, then interpolate between the previous
            // and current world positions to inject a *chain* of small
            // puffs so the trail has no gaps even at low frame rates or
            // fast flicks.
            if let Some((cx, cy)) = frame.cursor_pos {
                // Gate on actual screen-space pointer motion. Without this,
                // a stationary cursor over an orbiting/swaying camera would
                // emit continuous puffs as the unprojected table-plane hit
                // drifts with the camera.
                let screen_moved = match self.prev_cursor_screen {
                    Some((pcx, pcy)) => (cx - pcx).abs() > 0.01 || (cy - pcy).abs() > 0.01,
                    None => false,
                };
                self.prev_cursor_screen = Some((cx, cy));
                let inv_vp = view_proj.inverse();
                let nx = (cx / w) * 2.0 - 1.0;
                let ny = 1.0 - (cy / h) * 2.0;
                let near = inv_vp * glam::Vec4::new(nx, ny, 0.0, 1.0);
                let far = inv_vp * glam::Vec4::new(nx, ny, 1.0, 1.0);
                let near3 = glam::Vec3::new(near.x / near.w, near.y / near.w, near.z / near.w);
                let far3 = glam::Vec3::new(far.x / far.w, far.y / far.w, far.z / far.w);
                let dir = (far3 - near3).normalize_or_zero();
                if dir.z.abs() > 1e-4 {
                    let plane_z = 5.0;
                    let t = (plane_z - near3.z) / dir.z;
                    if t > 0.0 {
                        let hit = near3 + dir * t;
                        if let Some(prev) = self.prev_cursor_world {
                            let raw_delta = hit - prev;
                            let jump = raw_delta.length();
                            let win_scale = (h / 1080.0).max(0.5);
                            let max_jump = 42.0 * win_scale;
                            if screen_moved && jump.is_finite() && jump <= max_jump {
                                let speed_threshold = 0.4 * win_scale;
                                if jump > speed_threshold {
                                    // Drop a line of overlapping gaussian
                                    // puffs between the previous and
                                    // current cursor world positions. The
                                    // new density-only sim transports them
                                    // upward via its drift + curl field,
                                    // so we just need to seed enough mass
                                    // for the plume to read as solid smoke.
                                    let puff_radius = 18.0 * win_scale;
                                    // Spacing below the radius so adjacent
                                    // Gaussians overlap heavily (~e^-0.5 ≈ 60%
                                    // at the midpoint), leaving no visible
                                    // gaps along a fast flick. Cap raised
                                    // from 8 so long drags still fill.
                                    let step_size = puff_radius * 0.8;
                                    let n_puffs = ((jump / step_size).ceil() as u32).clamp(1, 24);

                                    // Perpendicular basis for in-plane jitter:
                                    // table-plane is z=5, so XY are the free axes.
                                    let tangent = raw_delta.normalize_or_zero();
                                    let perp = glam::Vec3::new(-tangent.y, tangent.x, 0.0);

                                    // Wake-vortex strength scales with cursor
                                    // speed — stronger flicks shed stronger
                                    // eddies. Divide by dt so `speed` is in
                                    // world units per second.
                                    let speed = jump / dt.max(1.0 / 120.0);
                                    // Rotational velocity applied to each
                                    // vortex puff along ±perp, producing a
                                    // counter-rotating pair behind the cursor.
                                    let swirl_vel = (speed * 1.1).min(640.0 * win_scale);
                                    // Small retrograde push so vortices sit
                                    // behind the leading edge rather than
                                    // racing ahead with the trail.
                                    let retrograde = speed * 0.12;

                                    use rand::RngExt;
                                    let mut rng = rand::rng();
                                    for i in 0..n_puffs {
                                        let frac = if n_puffs == 1 {
                                            1.0
                                        } else {
                                            (i as f32 + 1.0) / n_puffs as f32
                                        };
                                        let jitter_perp: f32 = rng.random_range(-1.0..1.0);
                                        let jitter_along: f32 = rng.random_range(-0.35..0.35);
                                        let jitter_z: f32 = rng.random_range(-0.4..0.4);
                                        let radius_mul: f32 = rng.random_range(0.75..1.25);
                                        let density_mul: f32 = rng.random_range(0.7..1.15);

                                        let center = prev
                                            + raw_delta * frac
                                            + perp * (jitter_perp * puff_radius * 0.35)
                                            + tangent * (jitter_along * step_size);
                                        let z_lift = glam::Vec3::new(
                                            0.0,
                                            0.0,
                                            (4.0 + jitter_z * 3.0) * win_scale,
                                        );
                                        fluid.inject_impulse(
                                            center + z_lift,
                                            glam::Vec3::ZERO,
                                            puff_radius * radius_mul,
                                            0.13 * density_mul * smoke_amount.density_mul(),
                                            0.0,
                                            0.0,
                                        );

                                        // Shed a counter-rotating vortex pair
                                        // per step, offset ±perp from the
                                        // trail. Alternate which side leads
                                        // per step so the wake staggers like
                                        // a Kármán vortex street instead of
                                        // reading as two parallel rails.
                                        let lead_sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                                        let offset = puff_radius * 0.9;
                                        for side in [-1.0_f32, 1.0_f32] {
                                            let s = side * lead_sign;
                                            let pos = center + perp * (s * offset)
                                                - tangent * (offset * 0.4)
                                                + z_lift;
                                            let vel = perp * (s * swirl_vel) - tangent * retrograde;
                                            fluid.inject_impulse(
                                                pos,
                                                vel,
                                                puff_radius * 0.75 * radius_mul,
                                                0.07 * density_mul * smoke_amount.density_mul(),
                                                0.0,
                                                0.0,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        self.prev_cursor_world = Some(hit);
                    }
                }
            }

            // (Re)allocate the offscreen smoke target whenever the user
            // changes the detail dropdown OR the window resizes. Cheap
            // no-op when nothing changed. Reallocating invalidates the
            // render bgs (they bind the offscreen views as TAA history
            // inputs), so `render_bgs_need_rebuild()` picks that up in
            // the block below.
            fluid.set_detail(&self.device, smoke_quality, &self.depth_copy_view);

            // Build/rebuild the volume render bind groups on first use,
            // after every depth-texture recreation (resize), and after
            // any offscreen reallocation. The smoke pass samples a
            // SNAPSHOT of the depth (`depth_copy_view`) copied between
            // the pre-smoke and post-smoke passes — the live
            // `depth_view` would alias the active depth attachment.
            if self.fluid_render_bg_dirty || fluid.render_bgs_need_rebuild() {
                fluid.rebuild_render_bind_group(
                    &self.device,
                    &self.depth_copy_view,
                    &self.point_lights_buffer,
                );
                self.fluid_render_bg_dirty = false;
            }

            // Upload the per-frame camera uniform consumed by the volume
            // raymarch shader.
            fluid.upload_camera_uniform(
                &self.queue,
                view_proj,
                cam_pos,
                smoke_quality,
                smoke_amount,
            );
        }

        // Garbage-collect stale per-tile world cache entries — drop any uid
        // that wasn't seen in `tile_uids` this frame so the HashMap doesn't
        // grow unbounded across runs.
        if !self.prev_tile_world.is_empty() {
            let live: std::collections::HashSet<u32> = self.tile_uids.iter().copied().collect();
            self.prev_tile_world.retain(|k, _| live.contains(k));
        }

        // ── Shadow map setup ────────────────────────────────────────────
        // Build a single directional shadow camera anchored to the same
        // key direction the lit shaders use (`(0.25, 1.0, 0.35)` normalized).
        // The orthographic frustum is sized to cover the play area where
        // casters live, not the full table — most of the table is empty
        // wood and would burn shadow texels for nothing.
        const SHADOW_MAP_SIZE: f32 = 2048.0;
        let key_dir = glam::Vec3::new(0.25, 1.0, 0.35).normalize();
        // Half-extents in world units. Generous so candles + relics on the
        // sides of the play area stay inside the frustum at any window
        // aspect. World X spans ±w/2, world Z spans ±h/2 — make the
        // ortho box cover both with margin.
        let shadow_half_x = (w * 0.6).max(h * 0.6);
        let shadow_half_z = (w * 0.6).max(h * 0.6);
        // Orthographic basis: light eye sits along +key_dir from the
        // play-area center. The eye distance + far plane are kept TIGHT
        // around the scene (~80 world units of headroom along the light
        // axis) so the [0,1] light-space depth resolves the few units of
        // height between casters and the table well — a generous depth
        // range here would burn precision on empty space and force a
        // huge bias to fight acne.
        let shadow_center = glam::Vec3::new(0.0, 0.0, 0.0);
        let scene_height = 80.0_f32;
        let eye_dist = scene_height * 0.5;
        let shadow_eye = shadow_center + key_dir * eye_dist;
        let shadow_view = Mat4::look_at_rh(shadow_eye, shadow_center, glam::Vec3::Y);
        let shadow_proj = Mat4::orthographic_rh(
            -shadow_half_x,
            shadow_half_x,
            -shadow_half_z,
            shadow_half_z,
            0.1,
            scene_height,
        );
        let light_view_proj = shadow_proj * shadow_view;
        let light_view_proj_arr = light_view_proj.to_cols_array();
        let shadow_enabled_flag = if shadows_enabled { 1.0_f32 } else { 0.0 };
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: light_view_proj_arr,
                params: [
                    shadow_enabled_flag,
                    // Depth bias in light-space [0,1] depth. With
                    // scene_height = 80, 0.005 ≈ 0.4 world units —
                    // big enough to hide self-shadow acne, small enough
                    // that 1-unit-tall tiles still cast onto the table.
                    0.005,
                    1.0 / SHADOW_MAP_SIZE,
                    0.0,
                ],
            }),
        );

        // ── Per-instance shadow caster uniforms ─────────────────────────
        // Mirror the model matrices written into the main lit-mesh +
        // hand-tile uniforms above so the shadow pre-pass can re-render
        // the same geometry from the light's POV.
        // Table is excluded — it's a flat receiver, not a caster.
        // Candle shadows: walk Object3dKind::Candle in the cmd list.
        {
            let mut slot_i = 0usize;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::Candle {
                        scale,
                        height_scale,
                    } = o.kind
                    {
                        let Some(instances) = self.candle_instances.get(slot_i) else {
                            break;
                        };
                        let base = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let model = translate_rot_scale(
                            base,
                            mesh_y_thickness_along_local_y_to_z_up(),
                            glam::Vec3::new(scale, scale * height_scale, scale),
                        );
                        let candle_name = self.scene_path("candle");
                        let model = self.apply_arrange_override(&candle_name, model);
                        instances[0].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        instances[1].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        slot_i += 1;
                    }
                }
            }
        }
        // Shrine shadow casters (pick-blind scene). Same model as the main
        // pass: scale by extents, lift base by half-height.
        {
            let mut shrine_shadow_cursor: usize = 0;
            for batch in &shrine_batches {
                for s in batch.iter() {
                    if shrine_shadow_cursor >= MAX_SHRINE_SLOTS {
                        break;
                    }
                    let slot_i = shrine_shadow_cursor;
                    shrine_shadow_cursor += 1;
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
                    self.shrine_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Ribbon shadow casters — walk Object3dKind::ZodiacRibbon.
        {
            let mut ribbon_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::ZodiacRibbon { kind } = &o.kind {
                        if ribbon_shadow_cursor >= MAX_RIBBON_SLOTS {
                            break;
                        }
                        let anchor = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let eff_w = o.extents[0];
                        let eff_l = o.extents[1];
                        let depth = o.extents[2];
                        let base_transform =
                            translate_rot_scale(anchor, o.rotation, glam::Vec3::splat(1.0));
                        if kind.is_some() {
                            let nominal_cap = eff_w * 0.33;
                            let cap_h = if eff_l < 2.0 * nominal_cap {
                                eff_l / 2.0
                            } else {
                                nominal_cap
                            };
                            let mid_h = (eff_l - 2.0 * cap_h).max(0.0);
                            let slots_needed = if mid_h > 0.0 { 3 } else { 2 };
                            if ribbon_shadow_cursor + slots_needed > MAX_RIBBON_SLOTS {
                                break;
                            }
                            let top_model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                top_model,
                            );
                            ribbon_shadow_cursor += 1;
                            if mid_h > 0.0 {
                                let mid_model = ribbon_submesh(
                                    base_transform,
                                    -cap_h,
                                    glam::Vec3::new(eff_w, mid_h, depth),
                                );
                                self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                    &self.queue,
                                    light_view_proj_arr,
                                    mid_model,
                                );
                                ribbon_shadow_cursor += 1;
                            }
                            let bot_model = ribbon_submesh(
                                base_transform,
                                -(cap_h + mid_h),
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                bot_model,
                            );
                            ribbon_shadow_cursor += 1;
                        } else {
                            let model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, eff_l, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                model,
                            );
                            ribbon_shadow_cursor += 1;
                        }
                    }
                }
            }
        }
        // Talisman shadows: walk Object3dKind::Talisman in the cmd list.
        {
            let mut talisman_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if matches!(
                        o.kind,
                        crate::render::draw_cmd::Object3dKind::Talisman { .. }
                    ) {
                        if talisman_shadow_cursor >= MAX_TALISMAN_SLOTS {
                            break;
                        }
                        let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let sx = o.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                        let sy = o.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                        let sz = o.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                        let model =
                            translate_rot_scale(center, o.rotation, glam::Vec3::new(sx, sy, sz));
                        self.talisman_instances[talisman_shadow_cursor].write_shadow_uniform(
                            &self.queue,
                            light_view_proj_arr,
                            model,
                        );
                        talisman_shadow_cursor += 1;
                    }
                }
            }
        }
        // Primitive shadow casters: walk every `Object3dKind::Primitive`
        // in frame cmds whose `shadow_caster` flag is true, and upload
        // the shadow uniform into the matching per-shape instance slot.
        // Slot cursors must track the main dispatch's
        // `obj3d_primitive_slot` exactly so each caster maps to the
        // instance the draw-pass will select.
        {
            use crate::render::primitive::{MeshId, shape_orientation};
            let mut cursors: HashMap<MeshId, usize> = HashMap::new();
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
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
                        // Step the cursor for every Primitive (matches
                        // the main dispatch). Only *write* a shadow
                        // uniform when `shadow_caster: true`.
                        let slot_i = *cursors.entry(*shape).or_insert(0);
                        *cursors.get_mut(shape).unwrap() += 1;
                        if *shadow_caster {
                            let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                            let orient = shape_orientation(*shape);
                            let model = translate_rot_scale(
                                center,
                                o.rotation * orient,
                                glam::Vec3::from(o.extents),
                            );
                            if let Some(pool) = self.primitive_instances.get_mut(shape)
                                && let Some(inst) = pool.get_mut(slot_i)
                            {
                                inst.write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                            }
                        }
                        // CabinetColumn pairs with a CabinetRails
                        // instance in the main dispatch — bump the
                        // rails cursor so shadow slots stay in sync,
                        // but don't cast rails shadows (too thin).
                        if *shape == MeshId::CabinetColumn {
                            *cursors.entry(MeshId::CabinetRails).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        // Hand tile shadow uniforms — pull each tile's model matrix from
        // `tile_pick_models` (snapshot of the per-tile model written above).
        for (i, model) in &tile_pick_models {
            if let Some(htg) = self.hand_tiles.get(*i) {
                self.queue.write_buffer(
                    &htg.shadow_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    }),
                );
            }
        }

        // ── Showcase tile GPU resources + uniforms ────────────────────────
        // Grow or update the pool so each tile in every ShowcaseTileBatch has
        // a ready-to-draw ShowcaseTileGpu slot with the correct decal and
        // up-to-date model matrix.
        {
            let total_showcase: usize = showcase_tile_batches
                .iter()
                .map(|b| b.len())
                .sum::<usize>()
                .min(MAX_SHOWCASE_TILE_SLOTS);

            // Ensure we have enough slots.
            while self.showcase_tiles.len() < total_showcase {
                // Placeholder — will be rebuilt immediately below if tile_id
                // doesn't match, but we need *something* to hold the GPU
                // resources. Use the first tile from the first batch.
                let placeholder_tile = showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .next()
                    .map(|p| &p.tile);
                if let Some(tile) = placeholder_tile {
                    let ctx = ShowcaseTileCtx {
                        device: &self.device,
                        queue: &self.queue,
                        layout: &self.tile_material_layout,
                        shadow_caster_layout: &self.shadow_caster_layout,
                        primitives: &self.tile_primitives,
                        sampler: &self.tile_sampler,
                        ui_font: self.ui_font.as_ref(),
                        emoji_font: self.emoji_font.as_ref(),
                    };
                    let stg = make_showcase_tile_gpu(
                        &ctx,
                        self.tile_base_color_factor,
                        tile,
                        self.tile_set.as_deref(),
                    );
                    self.showcase_tiles.push(stg);
                } else {
                    break;
                }
            }

            // ── HandStrip arrange-mode pre-pass ────────────────────────────
            // When a "HandStrip" arrange override is active, compute the
            // strip's world-space pivot (centroid of all hand tiles — those
            // with a pick_id) and build a delta-rotation matrix so each
            // tile's center is rotated around that pivot before the
            // translation offset is added.
            let hand_strip_arrange: Option<(glam::Vec3, Mat4, glam::Vec3)> = {
                if let Some(ref ov) = self.debug_arrange_override {
                    if ov.name == "HandStrip" {
                        // Collect world centers of hand tiles (pick_id = Some).
                        let hand_centers: Vec<glam::Vec3> = showcase_tile_batches
                            .iter()
                            .flat_map(|b| b.iter())
                            .filter(|p| p.pick_id.is_some())
                            .map(|p| {
                                pixel_to_world(
                                    w,
                                    h,
                                    p.center_pos[0],
                                    p.center_pos[1],
                                    p.center_pos[2],
                                )
                            })
                            .collect();
                        if !hand_centers.is_empty() {
                            let count = hand_centers.len() as f32;
                            let pivot =
                                hand_centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                            // Delta rotation applied around the pivot in world space.
                            let r_delta = Mat4::from_rotation_z(ov.delta_rz_deg.to_radians())
                                * Mat4::from_rotation_y(ov.delta_ry_deg.to_radians())
                                * Mat4::from_rotation_x(ov.delta_rx_deg.to_radians());
                            // Translation offset: pixel_x → +world_x, pixel_y → -world_y.
                            let translation =
                                glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
                            Some((pivot, r_delta, translation))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            // Track hand-tile world centers for the HandStrip debug pickable
            // (registered after the loop).
            let mut hand_strip_centers: Vec<glam::Vec3> = Vec::new();

            let mut slot_cursor = 0usize;
            for batch in &showcase_tile_batches {
                for p in batch.iter() {
                    if slot_cursor >= MAX_SHOWCASE_TILE_SLOTS {
                        break;
                    }
                    let wanted_id = (
                        p.tile.suit,
                        p.tile.rank,
                        p.tile.enhancement,
                        p.tile.debuffed_visual,
                    );
                    // Re-rasterise decal if the tile identity changed.
                    if self.showcase_tiles[slot_cursor].tile_id != wanted_id {
                        let ctx = ShowcaseTileCtx {
                            device: &self.device,
                            queue: &self.queue,
                            layout: &self.tile_material_layout,
                            shadow_caster_layout: &self.shadow_caster_layout,
                            primitives: &self.tile_primitives,
                            sampler: &self.tile_sampler,
                            ui_font: self.ui_font.as_ref(),
                            emoji_font: self.emoji_font.as_ref(),
                        };
                        self.showcase_tiles[slot_cursor] = make_showcase_tile_gpu(
                            &ctx,
                            self.tile_base_color_factor,
                            &p.tile,
                            self.tile_set.as_deref(),
                        );
                    }

                    // Build model matrix from the placement's explicit 3D transform.
                    let mut center =
                        pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2]);
                    let tile_short_px = p.size_px * 0.85;
                    let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                    let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ) * p.scale;

                    let mut base_rotation =
                        rot_euler_xyz_rad(p.rotation[0], p.rotation[1], p.rotation[2]);

                    // Apply HandStrip arrange override: rotate each hand tile's
                    // center around the strip pivot, then add the translation.
                    if let (true, Some((pivot, r_delta, translation))) =
                        (p.pick_id.is_some(), &hand_strip_arrange)
                    {
                        let offset = center - *pivot;
                        let rotated_offset = r_delta.transform_vector3(offset);
                        center = *pivot + rotated_offset + *translation;
                        // Also rotate the tile's own orientation so the face
                        // tracks the strip rotation (e.g. ry spins tiles in
                        // place as well as revolving their centers).
                        base_rotation = *r_delta * base_rotation;
                        hand_strip_centers.push(center);
                    }

                    let oriented = base_rotation * tile_basis;
                    let model = translate_rot_scale(center, oriented, scale);

                    // Smoke impulse: compare world position to previous frame's
                    // position for this tile uid and inject velocity into fluid sim.
                    if let Some(pick_id) = p.pick_id {
                        let uid = p.tile.id;
                        if let Some(prev) = self.prev_tile_world.get(&uid).copied() {
                            let delta = center - prev;
                            let speed = delta.length();
                            if speed > 0.5
                                && let Some(ref mut fluid) = self.fluid
                            {
                                let inv_dt = 1.0 / dt.max(1.0 / 120.0);
                                fluid.inject_impulse(
                                    center,
                                    delta * inv_dt * 0.45,
                                    tile_short_px * 0.55,
                                    speed * 0.04,
                                    0.0,
                                    0.0,
                                );
                            }
                        }
                        self.prev_tile_world.insert(uid, center);

                        // Project the tile's 8 corners for the screen AABB,
                        // used for pick tracking and glow rect sizing.
                        let lx = tile_long_px * 0.5;
                        let ly = tile_thickness_px * 0.5;
                        let lz = tile_short_px * 0.5;
                        let sc_corners = [
                            glam::Vec3::new(-lx, -ly, -lz),
                            glam::Vec3::new(lx, -ly, -lz),
                            glam::Vec3::new(-lx, ly, -lz),
                            glam::Vec3::new(lx, ly, -lz),
                            glam::Vec3::new(-lx, -ly, lz),
                            glam::Vec3::new(lx, -ly, lz),
                            glam::Vec3::new(-lx, ly, lz),
                            glam::Vec3::new(lx, ly, lz),
                        ];
                        let mut sc_min_x = f32::INFINITY;
                        let mut sc_min_y = f32::INFINITY;
                        let mut sc_max_x = f32::NEG_INFINITY;
                        let mut sc_max_y = f32::NEG_INFINITY;
                        for c in sc_corners {
                            let world_c = center + oriented.transform_point3(c);
                            let (px, py) = project_to_screen(world_c);
                            sc_min_x = sc_min_x.min(px);
                            sc_min_y = sc_min_y.min(py);
                            sc_max_x = sc_max_x.max(px);
                            sc_max_y = sc_max_y.max(py);
                        }
                        let overlay_w = (sc_max_x - sc_min_x).max(16.0);
                        let overlay_h = (sc_max_y - sc_min_y).max(16.0);
                        let overlay_x = sc_min_x;
                        let overlay_y = sc_min_y;

                        tile_3d_rects.push((pick_id, [overlay_x, overlay_y, overlay_w, overlay_h]));
                        tile_pick_models.push((pick_id, model));

                        if p.glow {
                            let gw = overlay_w * 1.50;
                            let gh = overlay_h * 1.55;
                            let gx = overlay_x + (overlay_w - gw) * 0.5;
                            let gy = overlay_y + (overlay_h - gh) * 0.5;
                            tile_glows.push(GpuInstance {
                                rect: [gx, gy, gw, gh],
                                color: p.glow_color.unwrap_or([1.00, 0.78, 0.32, 0.55]),
                            });
                        }
                    }

                    let stg = &self.showcase_tiles[slot_cursor];
                    let mut sc_bcf = self.tile_base_color_factor;
                    sc_bcf[0] = p.brightness;
                    // 1.0 = selected (gold rim), 0.5 = hovered (cool rim),
                    // 0.0 = none. Hovered supersedes selected.
                    sc_bcf[1] = if p.hovered {
                        0.5
                    } else if p.selected {
                        1.0
                    } else {
                        0.0
                    };
                    sc_bcf[2] = p.tile.enhancement.map_or(0.0, |e| e.shader_id());
                    // Per-tile procedural variation in tile_3d.wgsl (e.g.
                    // tortoise shell mottling) is seeded from the tile's
                    // unique run-scoped id so a given tile keeps the same
                    // pattern across draws, shuffles, and reorders.
                    let tile_seed = p.tile.id as f32;
                    self.queue.write_buffer(
                        &stg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: sc_bcf,
                            cam_pos: cam_pos.to_array(),
                            tile_seed,
                        }),
                    );
                    // Outline shell: write inflated model matrix when requested.
                    if p.outline {
                        const OUTLINE_GROW: f32 = 1.055;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(center, oriented, outline_scale);
                        self.queue.write_buffer(
                            &stg.outline_uniform_buffer,
                            0,
                            bytemuck::bytes_of(&CameraUniform {
                                view_proj: view_proj_arr,
                                model: outline_model.to_cols_array(),
                                base_color_factor: sc_bcf,
                                cam_pos: cam_pos.to_array(),
                                tile_seed,
                            }),
                        );
                    }
                    self.queue.write_buffer(
                        &stg.shadow_uniform_buffer,
                        0,
                        bytemuck::bytes_of(&ShadowCasterUniform {
                            light_view_proj: light_view_proj_arr,
                            model: model.to_cols_array(),
                        }),
                    );

                    slot_cursor += 1;
                }
            }

            // Register the hand strip as a single debug-pickable so arrange
            // mode can select it by clicking any tile. The pickable is an AABB
            // that encloses all hand-tile centers (or their arrange-moved
            // positions when an override is already active).
            if !hand_strip_centers.is_empty() || {
                // Fallback: compute from batch placements when the override is
                // not yet active (first click selection).

                showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .any(|p| p.pick_id.is_some())
            } {
                // Use the centers we collected (post-override) if available,
                // otherwise derive directly from placements.
                let centers: Vec<glam::Vec3> = if !hand_strip_centers.is_empty() {
                    hand_strip_centers.clone()
                } else {
                    showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .filter(|p| p.pick_id.is_some())
                        .map(|p| {
                            pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2])
                        })
                        .collect()
                };
                if !centers.is_empty() {
                    let count = centers.len() as f32;
                    let centroid = centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                    // Build half-extents that encompass all tile centers plus
                    // one tile-width of padding so clicking the end tiles works.
                    let tile_half = showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .find(|p| p.pick_id.is_some())
                        .map(|p| p.size_px * 0.5)
                        .unwrap_or(40.0);
                    let mut hx = tile_half;
                    let mut hy = tile_half;
                    let mut hz = tile_half;
                    for c in &centers {
                        let d = (*c - centroid).abs();
                        hx = hx.max(d.x + tile_half);
                        hy = hy.max(d.y + tile_half);
                        hz = hz.max(d.z + tile_half);
                    }
                    let strip_model =
                        translate_rot_scale(centroid, Mat4::IDENTITY, glam::Vec3::new(hx, hy, hz));
                    self.last_debug_pickables.push((
                        "gameplay.hand.strip".to_string(),
                        strip_model,
                        glam::Vec3::splat(0.5),
                        0.0,
                    ));
                }
            }
        }

        // Snapshot projected tile rects and pick models now that both the hand
        // pre-pass and showcase pre-pass have had a chance to push entries.
        self.proj.hand_rects = tile_3d_rects.clone();
        self.last_pick_models = tile_pick_models.clone();
        self.last_pick_camera = Some(PickCamera {
            inv_view_proj: view_proj.inverse(),
            viewport_w: w,
            viewport_h: h,
        });

        // Rebuild projected screen rects for relics/ribbons/talismans from
        // the authoritative `last_*_models` lists. Keeping this as a single
        // bulk step — instead of per-site pushes paired with each model
        // push — means mouse pick (model list) and focus nav (rect list)
        // always see the same set of items; a new draw path can't add a
        // model without a matching rect.
        self.proj.relic_rects.clear();
        for (model, _rid) in &self.last_relic_models {
            self.proj.relic_rects.push(project_unit_cube_rect(*model));
        }
        // Ribbons: mesh local AABB is x ∈ [-0.5, 0.5], y ∈ [-1, 0],
        // z ∈ [-0.05, 0.05] — not the unit cube. Project those bounds so the
        // screen rect lines up with the actual ribbon (otherwise it ends up
        // half-height and shifted up by half the ribbon length).
        self.proj.ribbon_rects.clear();
        for model in &self.last_ribbon_models {
            self.proj
                .ribbon_rects
                .push(project_aabb_rect(*model, [0.5, 0.5, 0.05], -0.5));
        }
        // Talismans: local mesh AABB is `TALISMAN_LOCAL_HALF` (y=0.7, z=0.09),
        // not ±0.5. The model already bakes the world scale (see sx/sy/sz
        // derivations against `TALISMAN_LOCAL_HALF * 2`), so we must project
        // the real local bounds — unit-cube projection clips ~30% off height
        // and 5.5× overstates depth.
        self.proj.talisman_rects.clear();
        for model in &self.last_talisman_models {
            self.proj
                .talisman_rects
                .push(project_aabb_rect(*model, TALISMAN_LOCAL_HALF, 0.0));
        }

        // Sync singleton shop-prop models (journal book, reroll prop, leave
        // prop, sell tray) into `aux_dish_rects` so focus nav can reach
        // them. Dishes authored via `DishExplicit` were already pushed
        // during their pass; these props come through Object3d kinds that
        // only update model snapshots, so we project them here. Packs live
        // in `pack_rects` (both the PackBatch and Object3d paths populate
        // it) and get appended last.
        if let Some((model, pid)) = self.last_sell_tray_model {
            self.proj
                .aux_dish_rects
                .push((Some(pid), project_unit_cube_rect(model)));
        }
        for (rect, pick_id) in &self.proj.pack_rects {
            self.proj.aux_dish_rects.push((*pick_id, *rect));
        }

        // Tile occluder buffer — analytic AABBs for the per-fragment ray
        // occlusion test that gives the candle pools their tile shadows.
        // Each tile contributes a single conservative world-space AABB
        // built from the 8 transformed local corners of its mesh extent.
        // Limited to MAX_TILE_OCCLUDERS so the uniform stays bounded.
        //
        // After collecting per-tile boxes we inflate adjacent tiles toward
        // each other so their AABBs touch along the row axis. Without this,
        // the back candles sit high above the table and their light threads
        // through the visible gaps between hand tiles, painting sharp
        // specular streaks on the table in front of the row (the row is
        // visually contiguous but physically gappy). The inflation per side
        // is half the gap to the nearest neighbour, so distant tiles never
        // smear into each other.
        {
            let hx = LOCAL_X_EXTENT * 0.5;
            let hy = LOCAL_Y_EXTENT * 0.5;
            let hz = LOCAL_Z_EXTENT * 0.5;
            let local_corners = [
                glam::Vec3::new(-hx, -hy, -hz),
                glam::Vec3::new(hx, -hy, -hz),
                glam::Vec3::new(-hx, hy, -hz),
                glam::Vec3::new(hx, hy, -hz),
                glam::Vec3::new(-hx, -hy, hz),
                glam::Vec3::new(hx, -hy, hz),
                glam::Vec3::new(-hx, hy, hz),
                glam::Vec3::new(hx, hy, hz),
            ];
            let mut tiles: Vec<(glam::Vec3, glam::Vec3)> =
                Vec::with_capacity(tile_pick_models.len().min(MAX_TILE_OCCLUDERS));
            for (_, model) in &tile_pick_models {
                if tiles.len() >= MAX_TILE_OCCLUDERS {
                    break;
                }
                let mut lo = glam::Vec3::splat(f32::INFINITY);
                let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                for c in local_corners.iter() {
                    let w = model.transform_point3(*c);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
                tiles.push(((lo + hi) * 0.5, (hi - lo) * 0.5));
            }

            // Pick the dominant horizontal axis (X or Y on the felt; Z is up) by
            // comparing the spread of tile centers. The hand is laid out
            // along screen X — that's world X after `pixel_to_world` — but
            // detecting it from the data keeps this robust if the layout
            // ever rotates.
            if tiles.len() >= 2 {
                let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
                let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);
                for (c, _) in &tiles {
                    min_x = min_x.min(c.x);
                    max_x = max_x.max(c.x);
                    min_z = min_z.min(c.z);
                    max_z = max_z.max(c.z);
                }
                let row_axis_x = (max_x - min_x) >= (max_z - min_z);

                let mut order: Vec<usize> = (0..tiles.len()).collect();
                order.sort_by(|&a, &b| {
                    let ka = if row_axis_x {
                        tiles[a].0.x
                    } else {
                        tiles[a].0.z
                    };
                    let kb = if row_axis_x {
                        tiles[b].0.x
                    } else {
                        tiles[b].0.z
                    };
                    ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                });
                for win in order.windows(2) {
                    let (a, b) = (win[0], win[1]);
                    let (ca, cb) = (tiles[a].0, tiles[b].0);
                    let (ha, hb) = (tiles[a].1, tiles[b].1);
                    let gap = if row_axis_x {
                        (cb.x - ca.x) - (ha.x + hb.x)
                    } else {
                        (cb.z - ca.z) - (ha.z + hb.z)
                    };
                    if gap > 0.0 {
                        let pad = gap * 0.5;
                        if row_axis_x {
                            tiles[a].1.x += pad;
                            tiles[b].1.x += pad;
                        } else {
                            tiles[a].1.z += pad;
                            tiles[b].1.z += pad;
                        }
                    }
                }
            }

            let mut occ = TileOccludersBuf::empty();
            for (i, (center, half)) in tiles.iter().enumerate() {
                occ.boxes[i] = TileOccluderGpu {
                    center: [center.x, center.y, center.z, 0.0],
                    half_extents: [half.x, half.y, half.z, 0.0],
                };
            }
            occ.count[0] = tiles.len() as u32;
            self.queue
                .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // Run fluid simulation compute passes (before render pass).
        //
        // Use the real inter-frame `dt` captured at the top of `render()`.
        // The previous `self.last_frame.elapsed()` here was a bug: by this
        // point we've already reassigned `self.last_frame = now`, so the
        // elapsed value is just the time spent on render work earlier in
        // this same function — typically 5–15 ms regardless of FPS. That
        // made the sim advance only ~0.5–0.9 seconds of simulated time per
        // wall second, so the post-deal wind sweep (1.4s wall) only got
        // ~0.7s of advection and intermittently failed to push the opening
        // smoke curtain off-grid before the overlay finished fading.
        // `dt` is already capped at 50 ms above, which is plenty of
        // headroom for the semi-Lagrangian step to stay stable.
        if let Some(ref mut fluid) = self.fluid {
            let step_dt = dt.max(1.0 / 120.0);
            fluid.step(&mut encoder, &self.queue, step_dt, smoke_quality);
        }

        // ── Shadow pre-pass ─────────────────────────────────────────────
        // Render every caster (table excluded) into the shadow map from
        // the light's POV. Skipped entirely when shadows are disabled —
        // the lit shaders short-circuit on `params.x = 0` and the stale
        // map contents go unread.
        if shadows_enabled {
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
            if !self.tile_primitives.is_empty() {
                for (i, _) in &tile_3d_rects {
                    let Some(htg) = self.hand_tiles.get(*i) else {
                        continue;
                    };
                    shadow_pass.set_bind_group(0, &htg.shadow_bind_group, &[]);
                    for prim in &self.tile_primitives {
                        shadow_pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            prim.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.draw_indexed(0..prim.index_count, 0, 0..1);
                    }
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
                    for prim in &self.tile_primitives {
                        shadow_pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            prim.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.draw_indexed(0..prim.index_count, 0, 0..1);
                    }
                }
            }
        }

        // Pre-create background image instance buffer (must outlive render pass).
        let bg_inst = GpuInstance {
            rect: [
                0.0,
                0.0,
                self.size.width.max(1) as f32,
                self.size.height.max(1) as f32,
            ],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let bg_inst_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bg-inst"),
                contents: bytemuck::cast_slice(&[bg_inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // Find the FluidSmoke marker so we can split the render pass: the
        // smoke fragment shader samples the depth buffer, which can't alias
        // the live depth attachment, so we end pass A right before the
        // smoke draw, copy depth → depth_copy, then start pass B (loading
        // color & depth) with the smoke as its first draw.
        let split_idx = ops.iter().position(|o| matches!(o, RenderOp::FluidSmoke));
        let split_end = split_idx.unwrap_or(ops.len());
        // Closure that processes one render op against the supplied pass.
        // Captures self + per-frame locals immutably (Rust 2021 disjoint
        // capture). Used twice — once for ops before smoke, once for ops
        // from smoke onwards.
        let process_op = |pass: &mut wgpu::RenderPass<'_>, op: &RenderOp| {
            match op {
                RenderOp::Background(id) => {
                    if let Some(bg_tex) = self.background_textures.get(id) {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, bg_inst_buf.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
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
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
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
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    let mut current_blended = false;
                    for &(kind, slot_i) in &object3d_draw_list[*start..*end] {
                        // Live wings carry per-frame `live_wing_alpha` tinting
                        // (crisp at turnarounds, faded at mid-stroke) and the
                        // blur fans carry `blur_alpha` (inverse). Both need the
                        // alpha-blended pipeline now that they're tinted.
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
                            let mesh = match self.relic_slot_texture.get(slot_i).copied().flatten()
                            {
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
                                    } => Some(label.as_str()),
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
                            DrawKind::Pack => (&self.pack_mesh, self.pack_instances.get(slot_i)),
                            DrawKind::Ribbon => {
                                (&self.ribbon_mesh, self.ribbon_instances.get(slot_i))
                            }
                            DrawKind::Talisman => {
                                (&self.talisman_mesh, self.talisman_instances.get(slot_i))
                            }
                            DrawKind::Shrine => {
                                (&self.shrine_mesh, self.shrine_instances.get(slot_i))
                            }
                            DrawKind::SellTray => {
                                (&self.round_dish_mesh, Some(&self.sell_tray_instance))
                            }
                            DrawKind::LampBody => {
                                (&self.lamp_body_mesh, Some(&self.lamp_body_instance))
                            }
                            DrawKind::LampBulb => {
                                (&self.lamp_bulb_mesh, Some(&self.lamp_bulb_instance))
                            }
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
                            DrawKind::Mirror => {
                                (&self.mirror_mesh, self.mirror_instances.get(slot_i))
                            }
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
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                    // Sell-tray "SELL" tent card — drawn last in the same
                    // pipeline state when the tray was focused this frame.
                    if self.last_sell_card_model.is_some() && self.sell_card_decal_ready {
                        if current_blended {
                            pass.set_pipeline(&self.lit_mesh_pipeline);
                        }
                        pass.set_bind_group(0, &self.sell_card_instance.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.sell_card_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.sell_card_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.sell_card_mesh.index_count, 0, 0..1);
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
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..relic_glows.len() as u32);
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

                            // Pass A: gold outline shells for tiles with outline=true.
                            let has_outline = batch.iter().any(|p| p.outline);
                            if has_outline {
                                pass.set_pipeline(&self.tile_outline_pipeline);
                                for (i, p) in batch.iter().enumerate() {
                                    if !p.outline {
                                        continue;
                                    }
                                    let slot_i = start_slot + i;
                                    if slot_i >= MAX_SHOWCASE_TILE_SLOTS {
                                        break;
                                    }
                                    let Some(stg) = self.showcase_tiles.get(slot_i) else {
                                        break;
                                    };
                                    for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                        let Some(bg) = stg.outline_bind_groups.get(pi) else {
                                            continue;
                                        };
                                        pass.set_bind_group(0, bg, &[]);
                                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                        pass.set_index_buffer(
                                            prim.index_buffer.slice(..),
                                            wgpu::IndexFormat::Uint32,
                                        );
                                        pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                    }
                                }
                            }

                            // Pass B: regular textured tile meshes.
                            pass.set_pipeline(&self.tile_pipeline);
                            for (i, _) in batch.iter().enumerate() {
                                let slot_i = start_slot + i;
                                if slot_i >= MAX_SHOWCASE_TILE_SLOTS {
                                    break;
                                }
                                let Some(stg) = self.showcase_tiles.get(slot_i) else {
                                    break;
                                };
                                for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                    let Some(bg) = stg.bind_groups.get(pi) else {
                                        continue;
                                    };
                                    pass.set_bind_group(0, bg, &[]);
                                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                    pass.set_index_buffer(
                                        prim.index_buffer.slice(..),
                                        wgpu::IndexFormat::Uint32,
                                    );
                                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                }
                            }
                        }
                    }
                }
                RenderOp::FluidSmoke => {
                    if smoke_quality != crate::persistence::SmokeQuality::Off
                        && let Some(ref fluid) = self.fluid
                    {
                        // Composite the offscreen smoke target onto the
                        // swap chain. The actual raymarch ran earlier in
                        // its own offscreen pass; this is just a
                        // bilinear sample + premultiplied blend.
                        fluid.draw_composite(pass);
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
                RenderOp::FlameBatch { buf_idx, count } => {
                    // When the volumetric smoke sim is active, candle flames
                    // are rendered as 3D emission inside the volume lightbake
                    // pass — skip the particle billboards so we don't
                    // double-draw. With smoke Off, the fluid sim doesn't
                    // step and volumetric flames wouldn't appear, so we
                    // drive the 3D particle system instead.
                    if smoke_quality == crate::persistence::SmokeQuality::Off
                        && *count > 0
                        && *buf_idx != usize::MAX
                    {
                        pass.set_pipeline(&self.flame_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &self.flame_view_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, flame_buffers[*buf_idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..*count);
                    }
                }
                RenderOp::TextDraw(idx) => {
                    let td = &text_draws[*idx];
                    pass.set_pipeline(&self.text_pipeline);
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
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &gpu.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, tile_face_inst_buffers[*idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                }
            }
        }; // end process_op closure

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

        // ── Pass A: clear + draw everything that lives behind the smoke ──
        {
            let main_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Main);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
                timestamp_writes: main_ts,
                multiview_mask: None,
            });
            for op in &ops[..split_end] {
                // 2D HUD text labels are drawn into the swapchain in a
                // separate overlay pass after the SSR snapshot, so they
                // don't end up in `scene_prev_texture` and get reflected by
                // the lacquered table. See the overlay pass below the
                // end-of-frame copies.
                //
                // `Plaque` ops are also held back: the score plaque now
                // carries an engraved decal texture (the score header
                // text) baked onto its +Z face, and if it were drawn here
                // the lacquered-table SSR would reflect that engraved
                // text into the table — recreating the exact ghost-text
                // artefact the overlay pass was originally introduced to
                // avoid. We snapshot `scene_prev` + `ssr_prev_depth`
                // immediately after this loop and *then* draw the plaques
                // in a sibling pass that loads the swapchain.
                if matches!(op, RenderOp::TextDraw(_)) {
                    continue;
                }
                process_op(&mut pass, op);
            }

            // Debug world-axes overlay: draw three colored bars after the
            // normal pass-A 3D ops so they sit on top of the table. Uses
            // the shared `relic_box_mesh` unit cube; per-instance uniforms
            // were written above.
            if frame.debug_axes {
                pass.set_pipeline(&self.lit_mesh_pipeline);
                pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.relic_box_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                for inst in self.debug_axes_instances.iter() {
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                }
            }
        }

        // ── SSR snapshot ────────────────────────────────────────────────
        // Capture the swapchain colour and depth buffers BEFORE the
        // hanging plaques are drawn. The lacquered-table SSR samples
        // these textures next frame, so plaques (and the engraved score
        // text decal on their +Z face) never end up in the table's
        // reflection. The smoke pass below still gets a fresh, full
        // (with-plaques) depth via its own `depth_copy_texture` copy.
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

        // ── Pass B: only created when there's a FluidSmoke marker. The
        // ── live depth buffer is copied into a sibling texture so the
        // ── smoke fragment shader can sample it without aliasing the
        // ── still-bound depth attachment.
        if let Some(split) = split_idx {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_copy_texture,
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

            // ── Smoke-only timing pass (debug profiling only) ────────
            // When a GPU profile session is active, render smoke with
            // flames disabled into the offscreen target first. The real
            // smoke-offscreen pass below overwrites the target with the
            // correct smoke+flames result, so this has no visual effect.
            // Placed here (before smoke-offscreen) so multiple subsequent
            // render passes flush the end-of-pass timestamp on Metal.
            #[cfg(debug_assertions)]
            if self.gpu_profiler.is_sampling()
                && smoke_quality != crate::persistence::SmokeQuality::Off
                && let Some(ref fluid) = self.fluid
            {
                fluid.set_render_mode_encoder(&mut encoder, true);
                // Smoke-only timing: no flame AABB needed
                // because the shader skips flames in this mode.
                let scissor = fluid.screen_aabb_rect(view_proj, None);
                let smoke_only_ts = self
                    .gpu_profiler
                    .pass_writes(crate::render::gpu_profiler::PassSlot::SmokeOnly);
                fluid.render_offscreen(
                    &mut encoder,
                    &self.globals_bind_group,
                    scissor,
                    smoke_only_ts,
                );
                fluid.set_render_mode_encoder(&mut encoder, false);
            }

            // ── Offscreen smoke raymarch pass ──────────────────────────
            // Run the volumetric ray-march into the (reduced-resolution)
            // smoke target BEFORE the swap-chain pass-B begins. The depth
            // copy above means the shader can sample scene depth without
            // aliasing the live depth attachment, and rendering offscreen
            // means the next pass-B can simply sample + bilinear-upsample
            // the result instead of paying for full-screen ray-marching.
            //
            // Skipped entirely when smoke is disabled — the post-smoke
            // pass below still runs so any UI/text ops queued after the
            // FluidSmoke marker draw correctly.
            if smoke_quality != crate::persistence::SmokeQuality::Off
                && let Some(ref fluid) = self.fluid
            {
                // Flame AABB: the raymarch runs its per-candle SDF
                // sub-march inside the same pass, so we have to
                // include the flame bounding spheres in the scissor
                // or flames disappear when the smoke field is empty.
                let flame_aabb = compute_flame_world_aabb(
                    &frame.point_lights[..frame
                        .candle_light_count
                        .min(frame.point_lights.len() as u32)
                        as usize],
                    frame.flame_height_world,
                    self.size.width.max(1) as f32,
                    self.size.height.max(1) as f32,
                );
                let scissor = fluid.screen_aabb_rect(view_proj, flame_aabb);
                let smoke_ts = self
                    .gpu_profiler
                    .pass_writes(crate::render::gpu_profiler::PassSlot::SmokeOffscreen);
                // `None` means both smoke and flames contribute
                // nothing — clear the offscreen target and skip the
                // raymarch. The composite still runs (sampling a
                // transparent texture) so queued ops after the
                // FluidSmoke marker draw correctly.
                if scissor.is_some() {
                    fluid.render_offscreen(
                        &mut encoder,
                        &self.globals_bind_group,
                        scissor,
                        smoke_ts,
                    );
                } else {
                    fluid.clear_offscreen(&mut encoder, smoke_ts);
                }
            }

            let post_smoke_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::PostSmoke);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post-smoke-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
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
                timestamp_writes: post_smoke_ts,
                multiview_mask: None,
            });
            for op in &ops[split..] {
                if matches!(op, RenderOp::TextDraw(_)) {
                    continue;
                }
                process_op(&mut pass, op);
            }
        }

        let bloom_w = (self.size.width.max(1) / 2).max(1);
        let bloom_h = (self.size.height.max(1) / 2).max(1);
        let bloom_threshold = if bloom_active { 1.05 } else { 9999.0 };
        let bloom_strength = if bloom_active { 0.92 } else { 0.0 };
        let make_bloom_bg =
            |label: &'static str, params: BloomParams, texture_view: &wgpu::TextureView| {
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytemuck::bytes_of(&params),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.bloom_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                        },
                    ],
                });
                (buffer, bind_group)
            };
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
        let (_extract_params_buf, bloom_scene_bg) = make_bloom_bg(
            "bloom-scene-pass-bg",
            extract_params,
            &self.scene_color_view,
        );
        let (_blur_h_params_buf, bloom_ping_bg) =
            make_bloom_bg("bloom-ping-pass-bg", blur_h_params, &self.bloom_ping_view);
        let (_blur_v_params_buf, bloom_pong_bg) =
            make_bloom_bg("bloom-pong-pass-bg", blur_v_params, &self.bloom_pong_view);
        let composite_params_buf =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bloom-composite-params"),
                    contents: bytemuck::bytes_of(&composite_params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
        let bloom_composite_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-composite-pass-bg"),
            layout: &self.bloom_composite_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: composite_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });

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
                pass.set_bind_group(0, &bloom_scene_bg, &[]);
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
                pass.set_bind_group(0, &bloom_ping_bg, &[]);
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
                pass.set_bind_group(0, &bloom_pong_bg, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_bind_group(0, &bloom_composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // (The SSR colour + depth snapshots that used to live here have
        // moved up to between pass A and the new plaque pass — see the
        // "SSR snapshot" block above. The smoke pass already maintains
        // its own `depth_copy_texture` copy, so nothing else needs the
        // end-of-frame depth dump.)

        // ── Overlay pass: 2D HUD text labels ────────────────────────────
        // Drawn AFTER the end-of-frame swapchain → scene_prev snapshot so
        // the text doesn't end up in next frame's SSR reflection sample.
        // The lacquered table reflects whatever's in scene_prev, and a
        // text label rasterised onto the plaque's screen rect would
        // otherwise appear as a phantom duplicate in the table reflection
        // immediately below the plaque (text doesn't write depth, so the
        // SSR ray hits the plaque's depth and samples the colour buffer
        // there — which has the text on top). Loading the swapchain (no
        // clear) lets us composite text on top of the just-finished scene.
        if ops.iter().any(|o| matches!(o, RenderOp::TextDraw(_))) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-overlay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
                    process_op(&mut pass, op);
                }
            }
        }

        // Flip the smoke TAA ping-pong for next frame — the slot we
        // just rendered into (and the composite just read) becomes
        // next frame's history input. Skipped when smoke is Off so we
        // don't mark undefined texture contents as valid history.
        if smoke_quality != crate::persistence::SmokeQuality::Off
            && let Some(fluid) = self.fluid.as_mut()
        {
            fluid.advance_taa_frame();
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
        let screenshot_path = self.pending_screenshot.take();
        let screenshot_staging = if let Some(ref path) = screenshot_path {
            log::info!("screenshot: encoding capture for {}", path.display());
            Some(self.encode_screenshot_copy(&mut encoder, frame_texture, path))
        } else {
            None
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
}
