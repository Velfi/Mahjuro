use super::*;

pub(super) struct TileFaceOverlayGpuParams<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub layout: &'a wgpu::BindGroupLayout,
    pub sampler: &'a wgpu::Sampler,
    pub ui_font: Option<&'a fontdue::Font>,
    pub emoji_font: Option<&'a fontdue::Font>,
    pub tile: &'a Tile,
    pub tile_set: Option<&'a str>,
}

pub(super) fn make_tile_face_overlay_gpu(p: &TileFaceOverlayGpuParams<'_>) -> TileFaceOverlayGpu {
    let TileFaceOverlayGpuParams {
        device,
        queue,
        layout,
        sampler,
        ui_font,
        emoji_font,
        tile,
        tile_set,
    } = *p;
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
    let rgba = crate::decal::rasterize_debuff_marker_overlay(W, H);
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

pub(super) fn make_image_quad_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    source: &crate::draw_cmd::ImageQuadSource,
) -> Option<TileFaceOverlayGpu> {
    if let crate::draw_cmd::ImageQuadSource::Asset { path } = source {
        let payload = crate::baked_texture::load_baked_texture(path).ok()?;
        let (texture, view, _) = crate::baked_texture::upload_payload(
            device,
            queue,
            "image-quad",
            &payload,
            crate::baked_texture::bc7_supported(device),
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image-quad-bg"),
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
        return Some(TileFaceOverlayGpu {
            _texture: texture,
            bind_group,
        });
    }
    if let crate::draw_cmd::ImageQuadSource::RawAsset { path } = source {
        let (rgba, w, h) = crate::baked_texture::load_rgba_for_cpu(path).ok()?;
        let (texture, view) = upload_rgba_texture(device, queue, "image-quad", &rgba, w, h);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image-quad-bg"),
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
        return Some(TileFaceOverlayGpu {
            _texture: texture,
            bind_group,
        });
    }

    let (rgba, w, h) = match source {
        crate::draw_cmd::ImageQuadSource::AtlasSprite { sheet, name } => {
            crate::kenney_atlas::extract_sprite_rgba(sheet, name)?
        }
        crate::draw_cmd::ImageQuadSource::PackedAtlas { sheet, name } => {
            crate::temptation_atlas::extract_sprite_rgba(sheet, name)?
        }
        crate::draw_cmd::ImageQuadSource::Asset { .. } => return None,
        crate::draw_cmd::ImageQuadSource::RawAsset { .. } => return None,
        crate::draw_cmd::ImageQuadSource::Relic(id) => {
            crate::relic_pipeline::decode_relic_icon_rgba(*id)?
        }
        crate::draw_cmd::ImageQuadSource::DebuffMarker => {
            const W: u32 = 192;
            const H: u32 = 256;
            let rgba = crate::decal::rasterize_debuff_marker_overlay(W, H);
            (rgba, W, H)
        }
    };
    let (texture, view) = upload_rgba_texture(device, queue, "image-quad", &rgba, w, h);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("image-quad-bg"),
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
