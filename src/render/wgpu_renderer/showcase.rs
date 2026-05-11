use super::*;
use wgpu::util::DeviceExt;

#[derive(Copy, Clone)]
pub(super) struct ShowcaseTileCtx<'a> {
    pub(super) device: &'a wgpu::Device,
    pub(super) layout: &'a wgpu::BindGroupLayout,
    pub(super) shadow_caster_layout: &'a wgpu::BindGroupLayout,
    pub(super) primitives: &'a [TilePrimitiveGpu],
    pub(super) decal_atlas: &'a crate::render::showcase_decal_atlas::ShowcaseDecalAtlasGpu,
}

pub(super) fn make_showcase_tile_gpu(
    ctx: &ShowcaseTileCtx<'_>,
    base_color_factor: [f32; 4],
    tile: &Tile,
) -> ShowcaseTileGpu {
    let ShowcaseTileCtx {
        device,
        layout,
        shadow_caster_layout,
        primitives,
        decal_atlas,
    } = *ctx;
    let key = (tile.suit, tile.rank, tile.enhancement, tile.debuffed_visual);
    let decal_atlas_uv = decal_atlas
        .lookup
        .get(&key)
        .copied()
        .unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let decal_view = &decal_atlas.view;
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            tile_seed: 0.0,
            decal_atlas_uv,
            hdr_tonemap: [0.0; 4],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

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
                        resource: wgpu::BindingResource::Sampler(&prim.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(decal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&prim.normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: prim.pbr_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&prim.metallic_roughness_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&prim.emissive_view),
                    },
                ],
            })
        })
        .collect();

    // Outline shell — always allocated so the bind group is stable.
    let initial_shadow = ShadowCasterUniform {
        light_view_proj: identity.to_cols_array(),
        model: identity.to_cols_array(),
    };
    let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-shadow-uniform"),
        contents: bytemuck::bytes_of(&initial_shadow),
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
        decal_atlas_uv,
        uniform_buffer,
        bind_groups,
        shadow_uniform_buffer,
        shadow_bind_group,
        cached_shadow_caster: initial_shadow,
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

pub(crate) fn make_debuff_marker_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> TileFaceOverlayGpu {
    const W: u32 = 192;
    const H: u32 = 256;
    let rgba = crate::render::decal::rasterize_debuff_marker_overlay(W, H);
    let (texture, view) = upload_rgba_texture(device, queue, "relic-debuff-marker", &rgba, W, H);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("relic-debuff-marker-bg"),
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

pub(super) fn make_prompt_icon_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    source: &crate::render::draw_cmd::PromptIconSource,
) -> Option<TileFaceOverlayGpu> {
    let (rgba, w, h) = match source {
        crate::render::draw_cmd::PromptIconSource::Embedded(asset_rel_path) => {
            crate::render::kenney_svg::rasterize_embedded_svg_rgba(asset_rel_path)?
        }
        crate::render::draw_cmd::PromptIconSource::Filesystem(path) => {
            crate::render::kenney_svg::rasterize_filesystem_svg_or_png_rgba(path)?
        }
    };
    let (texture, view) = upload_rgba_texture(device, queue, "kenney-prompt-icon", &rgba, w, h);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kenney-prompt-icon-bg"),
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
    Some(TileFaceOverlayGpu {
        _texture: texture,
        bind_group,
    })
}

// ---------------------------------------------------------------------------
// WgpuRenderer impl
// ---------------------------------------------------------------------------
