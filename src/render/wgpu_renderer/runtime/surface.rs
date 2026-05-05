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
        self.ssr_prev_depth_texture.destroy();
        let (sdt, sdv) = create_depth_copy(&self.device, new_size.width, new_size.height);
        self.ssr_prev_depth_texture = sdt;
        self.ssr_prev_depth_view = sdv;

        // SSR scene history texture follows the swapchain size; rebuild
        // the bind group so it points at the freshly allocated views.
        self.scene_prev_texture.destroy();
        let (spt, spv) = create_scene_prev(
            &self.device,
            SCENE_HDR_FORMAT,
            new_size.width,
            new_size.height,
        );
        self.scene_prev_texture = spt;
        self.scene_prev_view = spv;
        self.scene_color_texture.destroy();
        let (sct, scv) = create_scene_color(
            &self.device,
            SCENE_HDR_FORMAT,
            new_size.width,
            new_size.height,
        );
        self.scene_color_texture = sct;
        self.scene_color_view = scv;
        self.post_bloom_texture.destroy();
        let (pbt, pbv) = create_scene_color(
            &self.device,
            SCENE_HDR_FORMAT,
            new_size.width,
            new_size.height,
        );
        self.post_bloom_texture = pbt;
        self.post_bloom_view = pbv;
        self.journal_scene_texture.destroy();
        let (jst, jsv) = create_journal_scene(
            &self.device,
            SCENE_HDR_FORMAT,
            new_size.width,
            new_size.height,
        );
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
        let (cot, cov) = create_cascade_offscreen(
            &self.device,
            SCENE_HDR_FORMAT,
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
        self.lit_mesh_spot_ssr_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-spot-ssr-bg"),
            layout: &self.lit_mesh_spot_ssr_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.spot_lights_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.lit_mesh_ssr_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.scene_prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.ssr_prev_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
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

    }
}
