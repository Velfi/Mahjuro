use super::*;

#[derive(Copy, Clone)]
pub(super) struct ShowcaseTileCtx<'a> {
    pub(super) device: &'a wgpu::Device,
    pub(super) queue: &'a wgpu::Queue,
    pub(super) layout: &'a wgpu::BindGroupLayout,
    pub(super) shadow_caster_layout: &'a wgpu::BindGroupLayout,
    pub(super) primitives: &'a [TilePrimitiveGpu],
    pub(super) sampler: &'a wgpu::Sampler,
    pub(super) ui_font: Option<&'a fontdue::Font>,
    pub(super) emoji_font: Option<&'a fontdue::Font>,
}

pub(super) fn make_showcase_tile_gpu(
    ctx: &ShowcaseTileCtx<'_>,
    base_color_factor: [f32; 4],
    tile: &Tile,
    tile_set: Option<&str>,
) -> ShowcaseTileGpu {
    let ShowcaseTileCtx {
        device,
        queue,
        layout,
        shadow_caster_layout,
        primitives,
        sampler,
        ui_font,
        emoji_font,
    } = *ctx;
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            tile_seed: 0.0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    // Use `true` (hand-tile quality) so hand-strip tiles get the same
    // full-resolution decal as the old HandTileGpu path did.
    let rgba =
        rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, true);
    let (_decal_texture, decal_view) = upload_rgba_texture(
        device,
        queue,
        "showcase-tile-decal",
        &rgba,
        DECAL_W,
        DECAL_H,
    );

    let bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("showcase-tile-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&decal_view),
                    },
                ],
            })
        })
        .collect();

    // Outline shell — always allocated so the bind group is stable.
    let outline_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-outline-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            tile_seed: 0.0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let outline_bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("showcase-tile-outline-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: outline_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&decal_view),
                    },
                ],
            })
        })
        .collect();

    let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-shadow-uniform"),
        contents: bytemuck::bytes_of(&ShadowCasterUniform {
            light_view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("showcase-tile-shadow-bg"),
        layout: shadow_caster_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: shadow_uniform_buffer.as_entire_binding(),
        }],
    });

    ShowcaseTileGpu {
        uniform_buffer,
        bind_groups,
        outline_uniform_buffer,
        outline_bind_groups,
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (tile.suit, tile.rank, tile.enhancement, tile.debuffed_visual),
    }
}

pub(super) fn make_tile_face_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile: &Tile,
    tile_set: Option<&str>,
) -> TileFaceOverlayGpu {
    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba =
        rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, false);
    let (texture, view) =
        upload_rgba_texture(device, queue, "tile-face-overlay", &rgba, DECAL_W, DECAL_H);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-face-overlay-bg"),
        layout,
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
    TileFaceOverlayGpu {
        _texture: texture,
        bind_group,
    }
}

// ---------------------------------------------------------------------------
// WgpuRenderer impl
// ---------------------------------------------------------------------------
