//! Early boot splash: production logo sequence + SDF "loading..." + progress bar
//! while the full renderer initializes.

use std::sync::OnceLock;

use wgpu::util::DeviceExt;

use super::loading_screen::{self, LoadingAlphas};
use crate::wgpu_renderer::embedded_wgsl;
use crate::wgpu_renderer::targets::RenderTarget;
use crate::wgpu_renderer::ui_instances::GpuInstance;

#[derive(Debug, serde::Deserialize)]
struct BootLoadingMeta {
    spread_px: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BootGlobals {
    screen: [f32; 2],
    gamma: f32,
    spread: f32,
    msdf_uv_min: [f32; 2],
    msdf_uv_max: [f32; 2],
}

static BOOT_META: OnceLock<BootLoadingMeta> = OnceLock::new();

fn boot_meta() -> &'static BootLoadingMeta {
    BOOT_META.get_or_init(|| {
        let json = mahjuro_assets::asset_path::get("data/boot_loading_msdf.json")
            .expect("boot_loading_msdf.json missing; run scripts/bake_boot_loading_msdf.py");
        serde_json::from_slice(&json.data).expect("boot_loading_msdf.json parse")
    })
}

fn boot_png_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES
        .get_or_init(|| {
            mahjuro_assets::asset_path::get("textures/boot_loading_msdf.png")
                .expect("boot_loading_msdf.png missing; run scripts/bake_boot_loading_msdf.py")
                .data
                .to_vec()
        })
        .as_slice()
}

fn logo_png_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES
        .get_or_init(|| {
            mahjuro_assets::asset_path::get(loading_screen::LOADING_LOGO_ASSET)
                .expect("zelda_built_this.png missing")
                .data
                .to_vec()
        })
        .as_slice()
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    png: &[u8],
) -> anyhow::Result<wgpu::Texture> {
    let img = image::load_from_memory(png)?.to_rgba8();
    let (tw, th) = img.dimensions();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: tw.max(1),
            height: th.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &img,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * tw),
            rows_per_image: Some(th),
        },
        wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
    );
    Ok(texture)
}

/// Live boot presenter — created after `early_gpu`, dropped when full init finishes.
#[cfg(feature = "windowed")]
pub struct BootSplash<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globals_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    screen_w: f32,
    screen_h: f32,
}

#[cfg(feature = "windowed")]
impl<'a> BootSplash<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        format: wgpu::TextureFormat,
        screen_w: u32,
        screen_h: u32,
    ) -> anyhow::Result<Self> {
        let meta = boot_meta();
        let msdf_texture =
            upload_rgba_texture(device, queue, "boot-loading-msdf", boot_png_bytes())?;
        let logo_texture =
            upload_rgba_texture(device, queue, "boot-loading-logo", logo_png_bytes())?;
        let msdf_view = msdf_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let logo_view = logo_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("boot-atlas-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("boot-globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("boot-atlas-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boot-globals"),
            contents: bytemuck::bytes_of(&BootGlobals {
                screen: [screen_w.max(1) as f32, screen_h.max(1) as f32],
                gamma: 1.0,
                spread: meta.spread_px,
                msdf_uv_min: [0.0, 0.0],
                msdf_uv_max: [1.0, 1.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("boot-globals-bg"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("boot-atlas-bg"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&msdf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&logo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("boot-splash-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::BOOT_SPLASH.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("boot-splash-layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: 8,
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

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("boot-splash-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout, instance_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let quad_v: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boot-quad-verts"),
            contents: bytemuck::cast_slice(&quad_v),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boot-quad-idx"),
            contents: bytemuck::cast_slice(&idx),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("boot-instances"),
            size: (std::mem::size_of::<GpuInstance>() * 5) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            globals_bind_group,
            atlas_bind_group,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            screen_w: screen_w.max(1) as f32,
            screen_h: screen_h.max(1) as f32,
        })
    }

    /// Present one boot frame. `progress` is in `[0, 1]`.
    pub fn present(
        &mut self,
        target: &RenderTarget,
        config: &wgpu::SurfaceConfiguration,
        progress: f32,
        alphas: LoadingAlphas,
    ) -> anyhow::Result<()> {
        let RenderTarget::Surface(surface) = target else {
            return Ok(());
        };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated => {
                surface.configure(self.device, config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let instances = self.build_instances(progress.clamp(0.0, 1.0), alphas);
        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("boot-splash"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("boot-splash-pass"),
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..6, 0, 0..instances.len() as u32);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }

    fn build_instances(&self, progress: f32, alphas: LoadingAlphas) -> Vec<GpuInstance> {
        let layout = loading_screen::layout_with_msdf_label(self.screen_w, self.screen_h);
        let mut out = Vec::with_capacity(5);

        if alphas.logo > 0.004 {
            out.push(GpuInstance {
                rect: layout.logo_rect,
                color: [1.0, 1.0, 1.0, alphas.logo],
                user: 2,
            });
        }

        if alphas.loading_ui > 0.004 {
            let ui_a = alphas.loading_ui;
            out.push(GpuInstance {
                rect: layout.bar_rect,
                color: [
                    layout.track_color[0],
                    layout.track_color[1],
                    layout.track_color[2],
                    layout.track_color[3] * ui_a,
                ],
                user: 0,
            });

            let fill_w = (layout.bar_rect[2] * progress).max(0.0);
            if fill_w > 0.5 {
                out.push(GpuInstance {
                    rect: [
                        layout.bar_rect[0],
                        layout.bar_rect[1],
                        fill_w,
                        layout.bar_rect[3],
                    ],
                    color: [
                        layout.fill_color[0],
                        layout.fill_color[1],
                        layout.fill_color[2],
                        layout.fill_color[3] * ui_a,
                    ],
                    user: 0,
                });
            }

            out.push(GpuInstance {
                rect: layout.label_rect,
                color: [
                    layout.text_color[0],
                    layout.text_color[1],
                    layout.text_color[2],
                    layout.text_color[3] * ui_a,
                ],
                user: 1,
            });
        }

        out
    }
}

#[cfg(feature = "windowed")]
pub(super) fn boot_present<'a>(
    splash: &mut Option<BootSplash<'a>>,
    target: &RenderTarget,
    config: &wgpu::SurfaceConfiguration,
    boot_progress: f32,
    input_poll: &mut Option<&mut dyn FnMut()>,
) {
    loading_screen::touch_boot_frame();
    loading_screen::set_boot_progress(boot_progress);
    if let Some(poll) = input_poll.as_mut() {
        poll();
    }
    let alphas = loading_screen::current_boot_alphas();
    let progress = loading_screen::combined_progress(0.0);
    if let Some(s) = splash.as_mut() {
        let _ = s.present(target, config, progress, alphas);
    }
}
