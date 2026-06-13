use super::*;

impl WgpuRenderer {
    /// Reconfigure the swapchain and tonemap output pipeline when HDR is toggled
    /// in Options, without restarting the app.
    pub fn set_hdr_enabled(&mut self, hdr_enabled: bool) {
        let new_format = if hdr_enabled && self.swapchain_hdr_available {
            wgpu::TextureFormat::Rgba16Float
        } else {
            self.swapchain_sdr_format
        };
        if new_format == self.config.format {
            return;
        }
        log::info!(
            "swapchain format {:?} → {:?} (HDR {})",
            self.config.format,
            new_format,
            if hdr_enabled { "on" } else { "off" }
        );
        self.config.format = new_format;
        if let RenderTarget::Surface(surface) = &self.target {
            surface.configure(&self.device, &self.config);
        }
        self.tonemap_pipeline =
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("tonemap-pipeline"),
                    layout: Some(&self.tonemap_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.tonemap_shader_module,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.tonemap_shader_module,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: new_format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });

        // `text_pipeline` / `image_pipeline` target the swapchain format at init.
        // Recreate them here so HUD text and tile-face decals stay compatible
        // after toggling HDR (e.g. `Rgba16Float` ↔ `Bgra8UnormSrgb`).
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        };
        let depth_ui = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        self.text_pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("text-pipeline"),
                layout: Some(&self.text_overlay_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.text_shader_module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[vertex_layout.clone(), instance_layout.clone()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.text_shader_module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: new_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        self.image_pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("image-pipeline"),
                layout: Some(&self.text_overlay_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.image_shader_module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[vertex_layout, instance_layout],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.image_shader_module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: new_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
    }

    /// Returns whether a screenshot path is still queued for the next frame
    /// after calling `queue_screenshot`. Peeks without consuming the queue
    /// (`Cell::take` + restore) so the next `render` can still fulfill it.
    /// Used by the headless screenshot harness to detect a dropped draw where
    /// the capture never ran.
    pub fn screenshot_pending(&self) -> bool {
        let p = self.pending_screenshot.take();
        let pending = p.is_some();
        // Restore (Cell::take() removes the value; put it back so the
        // next draw can fulfil it).
        self.pending_screenshot.set(p);
        pending
    }

    pub fn resize(&mut self, new_size: crate::physical_size::PhysicalSize) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        let new_size = super::super::clamp_render_physical_size(new_size);
        self.size = new_size;
        self.render_size = super::super::constants::scaled_render_size(new_size, self.render_scale);
        let rs = self.render_size;
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
        let (dt, dv) = create_depth(&self.device, rs.width, rs.height);
        self.depth_texture = dt;
        self.depth_view = dv;
        self.overlay_depth_texture.destroy();
        let (odt, odv) = create_depth(&self.device, new_size.width, new_size.height);
        self.overlay_depth_texture = odt;
        self.overlay_depth_view = odv;
        self.depth_r32_snapshot_texture.destroy();
        let (drt, drv) =
            create_depth_r32_snapshot(&self.device, rs.width, rs.height, "depth-r32-snapshot");
        self.depth_r32_snapshot_texture = drt;
        self.depth_r32_snapshot_view = drv;
        self.depth_copy_staging_buffer.destroy();
        self.depth_copy_staging_buffer =
            create_depth_copy_staging(&self.device, rs.width, rs.height);

        self.scene_color_texture.destroy();
        let (sct, scv) = create_scene_color(&self.device, SCENE_HDR_FORMAT, rs.width, rs.height);
        self.scene_color_texture = sct;
        self.scene_color_view = scv;
        self.room_emissive_texture.destroy();
        let (re_t, re_v) = create_scene_color(&self.device, SCENE_HDR_FORMAT, rs.width, rs.height);
        self.room_emissive_texture = re_t;
        self.room_emissive_view = re_v;
        self.post_bloom_texture.destroy();
        let (pbt, pbv) = create_scene_color(&self.device, SCENE_HDR_FORMAT, rs.width, rs.height);
        self.post_bloom_texture = pbt;
        self.post_bloom_view = pbv;
        self.journal_scene_texture.destroy();
        let (jst, jsv) = create_journal_scene(&self.device, SCENE_HDR_FORMAT, rs.width, rs.height);
        self.journal_scene_texture = jst;
        self.journal_scene_view = jsv;
        // Bump the journal-scene view generation so any cached bind
        // group bound against the now-destroyed previous view forces
        // itself to rebind before the next draw. The book body's slot-3
        // binding points at this view; without the bump, the first draw
        // after resize faults with "Texture with 'journal-scene-target'
        // label has been destroyed."
        self.journal_scene_view_generation = self.journal_scene_view_generation.wrapping_add(1);
        self.cascade_offscreen_texture.destroy();
        let (cot, cov) =
            create_cascade_offscreen(&self.device, SCENE_HDR_FORMAT, rs.width, rs.height);
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
        let bloom_w = (rs.width.max(1) / 2).max(1);
        let bloom_h = (rs.height.max(1) / 2).max(1);
        let (bpt, bpv) = create_post_texture(
            &self.device,
            SCENE_HDR_FORMAT,
            bloom_w,
            bloom_h,
            "bloom-ping",
        );
        self.bloom_ping_texture = bpt;
        self.bloom_ping_view = bpv;
        let (bot, bov) = create_post_texture(
            &self.device,
            SCENE_HDR_FORMAT,
            bloom_w,
            bloom_h,
            "bloom-pong",
        );
        self.bloom_pong_texture = bot;
        self.bloom_pong_view = bov;
        self.lit_mesh_spot_frame_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lit-mesh-spot-frame-bg"),
                layout: &self.lit_mesh_spot_frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.spot_lights_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.lit_mesh_frame_buffer.as_entire_binding(),
                    },
                ],
            });
        self.bloom_scene_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-scene-bg"),
            layout: &self.bloom_extract_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_extract_params_buffer.as_entire_binding(),
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
                    resource: self.bloom_blur_h_params_buffer.as_entire_binding(),
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
                    resource: self.bloom_blur_v_params_buffer.as_entire_binding(),
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
                        resource: self.bloom_composite_params_buffer.as_entire_binding(),
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
        self.tonemap_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap-pass-bg"),
            layout: &self.tonemap_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.tonemap_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.post_bloom_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.tonemap_bind_group_scene = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap-pass-scene-bg"),
            layout: &self.tonemap_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.tonemap_params_buffer.as_entire_binding(),
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

        let globals = Globals {
            screen: [new_size.width as f32, new_size.height as f32],
            time: self.creation_time.elapsed().as_secs_f32(),
            // Gamma will be re-uploaded on the next render() call.
            gamma: 1.0,
            cursor_pos: [new_size.width as f32 * 0.5, new_size.height as f32 * 0.5],
            transition_progress: 0.0,
            quality_level: 2.0,
            moon_phase: self.main_menu_moon_phase_debug.resolved_phase(),
            _globals_pad: [
                0.0,
                if crate::main_menu_glb::main_menu_pride_rainbow_active(
                    self.main_menu_pride_rainbow_debug,
                ) {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
        };
        self.queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));
        self.queue.write_buffer(
            &self.globals_scene_hdr_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );
        let inv_bw = 1.0 / bloom_w as f32;
        let inv_bh = 1.0 / bloom_h as f32;
        let data0 = [1.1_f32, 0.0, inv_bw, inv_bh];
        self.queue.write_buffer(
            &self.bloom_extract_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0,
                data1: [0.0; 4],
            }),
        );
        self.queue.write_buffer(
            &self.bloom_blur_h_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0,
                data1: [1.0, 0.0, 0.0, 0.0],
            }),
        );
        self.queue.write_buffer(
            &self.bloom_blur_v_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0,
                data1: [0.0, 1.0, 0.0, 0.0],
            }),
        );
        self.queue.write_buffer(
            &self.bloom_composite_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0,
                data1: [0.0; 4],
            }),
        );
    }
}
