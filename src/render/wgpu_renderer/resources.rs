use super::*;

/// Pre-loaded background texture + bind group for the image pipeline.
pub(super) struct BackgroundTextureGpu {
    pub(super) bind_group: wgpu::BindGroup,
}

/// Decoded background image data sent from the background loader thread.
pub(super) struct DecodedBackgroundImage {
    pub(super) id: BackgroundId,
    pub(super) rgba: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn relic_material_params(
    relic_id: RelicId,
    base_color: [f32; 4],
    glow: f32,
) -> MaterialParams {
    let visual = relic_visual(relic_id);
    let g = glow.clamp(0.0, 1.0);
    match visual.material {
        RelicRenderMaterial::Iron => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.42 + base_color[0] * 0.14,
                0.44 + base_color[1] * 0.14,
                0.48 + base_color[2] * 0.14,
                base_color[3],
            ],
            specular_strength: 0.38 + 0.18 * g,
            specular_power: 26.0,
        },
        RelicRenderMaterial::Copper => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.78 + base_color[0] * 0.16,
                0.46 + base_color[1] * 0.14,
                0.26 + base_color[2] * 0.10,
                base_color[3],
            ],
            specular_strength: 0.52 + 0.22 * g,
            specular_power: 34.0,
        },
        RelicRenderMaterial::Silver => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.82 + base_color[0] * 0.14,
                0.84 + base_color[1] * 0.14,
                0.88 + base_color[2] * 0.12,
                base_color[3],
            ],
            specular_strength: 0.78 + 0.22 * g,
            specular_power: 64.0,
        },
        RelicRenderMaterial::Gold => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.94 + base_color[0] * 0.14,
                0.78 + base_color[1] * 0.14,
                0.28 + base_color[2] * 0.10,
                base_color[3],
            ],
            specular_strength: 0.88 + 0.24 * g,
            specular_power: 80.0,
        },
    }
}

pub(super) fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(super) fn white_albedo(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture(
        device,
        queue,
        "tile-albedo-white",
        &[255, 255, 255, 255],
        1,
        1,
    )
}

/// 1×1 mid-gray linear texture — default `relief_tex` for lit meshes that
/// don't use a separate height map (enamel shader reads ~0.5 → flat relief).
pub(super) fn flat_relief_height(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture_linear(
        device,
        queue,
        "lit-relief-flat",
        &[128, 128, 128, 255],
        1,
        1,
    )
}

/// Same as `upload_rgba_texture` but allocates the texture in **linear**
/// (non-sRGB) format. Used for data textures like heightmaps where the
/// stored byte value is a raw scalar, not a perceptual color.
pub(super) fn upload_rgba_texture_linear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
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
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Decode the embedded coin face heightmap PNG and upload it as a linear
/// data texture. Falls back to a flat mid-gray 1×1 if the asset is missing
/// or fails to decode (so the coin still renders, just without engraving).
pub(super) fn load_coin_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    load_metal_heightmap(
        device,
        queue,
        "textures/coin_heightmap.png",
        "coin-heightmap",
    )
}

/// Decode the embedded bronze mirror heightmap PNG and upload it as a
/// linear data texture. Bound at slot 1 of every gameplay mirror instance;
/// the metal branch in lit_mesh.wgsl samples it as a heightfield to perturb
/// the polished face's surface normal so the cast four-spirit relief catches
/// the candle highlights. Same fallback behavior as the coin loader.
pub(super) fn load_mirror_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    load_metal_heightmap(
        device,
        queue,
        "textures/mirror_heightmap.png",
        "mirror-heightmap",
    )
}

/// Shared body for the per-asset heightmap loaders. Reads `path` from the
/// embedded assets, decodes it, and uploads as a linear (non-sRGB) RGBA8
/// texture. Falls back to a flat mid-gray 1×1 on any failure so the
/// metal-perturbation branch degrades gracefully to a smooth surface.
pub(super) fn load_metal_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &str,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let flat_label: &'static str = match label {
        "coin-heightmap" => "coin-heightmap-flat",
        "mirror-heightmap" => "mirror-heightmap-flat",
        _ => "metal-heightmap-flat",
    };
    let bytes = match crate::asset_path::get(path) {
        Some(file) => file.data.to_vec(),
        None => {
            log::warn!("{label} asset missing at {path} - using flat fallback");
            return upload_rgba_texture_linear(
                device,
                queue,
                flat_label,
                &[128, 128, 128, 255],
                1,
                1,
            );
        }
    };
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            upload_rgba_texture_linear(device, queue, label, &rgba.into_raw(), w, h)
        }
        Err(e) => {
            log::warn!("failed to decode {label}: {e} - using flat fallback");
            upload_rgba_texture_linear(device, queue, flat_label, &[128, 128, 128, 255], 1, 1)
        }
    }
}

/// Load the three-part zodiac silk ribbon textures (top/mid/bot per zodiac).
pub(super) fn load_zodiac_ribbon_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> crate::render::texture_upload::ZodiacRibbonTextures {
    crate::render::texture_upload::load_zodiac_ribbon_textures(device, queue)
}

/// Spawn a background thread that decodes all background PNGs and sends the RGBA
/// data back over a channel.
pub(super) fn spawn_background_loader() -> mpsc::Receiver<DecodedBackgroundImage> {
    let (tx, rx) = mpsc::channel();

    let backgrounds: Vec<(BackgroundId, &'static str)> = [BackgroundId::Menu, BackgroundId::Score]
        .iter()
        .filter_map(|id| id.asset_path().map(|p| (*id, p)))
        .collect();

    std::thread::Builder::new()
        .name("bg-loader".into())
        .spawn(move || {
            let t_thread = Instant::now();
            let mut decoded = 0usize;
            let mut decode_time = std::time::Duration::ZERO;
            for (id, asset_path) in backgrounds {
                let bytes = match crate::asset_path::get(asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("background image not found: {asset_path}");
                        continue;
                    }
                };
                let t_decode = Instant::now();
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode background {asset_path}: {e}");
                        continue;
                    }
                };
                decode_time += t_decode.elapsed();
                decoded += 1;
                let (w, h) = img.dimensions();
                let msg = DecodedBackgroundImage {
                    id,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
            log::info!(
                "bg-loader thread finished: decoded {decoded} images in {decode_time:?} (thread total {:?})",
                t_thread.elapsed(),
            );
        })
        .expect("failed to spawn bg-loader thread");

    rx
}

pub(super) fn create_depth(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Snapshot of the previous frame's swapchain colour. Bound by the lacquered
/// floor as the source for screen-space reflections — the table is drawn
/// before the candles each frame, so it has to reflect *last* frame's
/// composited candles + flames + tiles. The camera is fixed, so the
/// one-frame stale image is essentially correct.
pub(super) fn create_scene_prev(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene-prev"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(super) fn create_scene_color(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene-color"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Half-resolution offscreen target for the shooting-star cascade shader.
/// The shader is heavy per-pixel, so it runs at 1/2 × 1/2 = 1/4 pixel count
/// and is additively composited up to the main scene target.
pub(super) fn create_cascade_offscreen(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    full_width: u32,
    full_height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let w = (full_width / 2).max(1);
    let h = (full_height / 2).max(1);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cascade-offscreen"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(super) fn create_post_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Sibling depth texture used as a sampleable snapshot of the scene depth
/// between the pre-smoke and post-smoke render passes.
pub(super) fn create_depth_copy(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-copy"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}
