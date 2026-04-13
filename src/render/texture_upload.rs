//! Free functions extracted from `wgpu_renderer.rs` — texture upload helpers,
//! asset loaders, depth/scene-prev allocation, and per-tile GPU resource builders.
//! Not yet wired in — `wgpu_renderer.rs` still calls its own copies.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;

use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::render::decal::{rasterize_tile_face_decal, tile_short_label, tile_suit_emoji};
use crate::render::lit_mesh::ShadowCasterUniform;
use crate::scenes::BackgroundId;

use super::gpu_types::*;

// ---------------------------------------------------------------------------
// Texture helpers
// ---------------------------------------------------------------------------

pub(crate) fn upload_rgba_texture(
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

pub(crate) fn white_albedo(
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

/// Same as `upload_rgba_texture` but allocates the texture in **linear**
/// (non-sRGB) format. Used for data textures like heightmaps where the
/// stored byte value is a raw scalar, not a perceptual color.
pub(crate) fn upload_rgba_texture_linear(
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
pub(crate) fn load_coin_heightmap(
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
pub(crate) fn load_mirror_heightmap(
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
pub(crate) fn load_metal_heightmap(
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
            log::warn!("{label} asset missing at {path} — using flat fallback");
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
            log::warn!("failed to decode {label}: {e} — using flat fallback");
            upload_rgba_texture_linear(device, queue, flat_label, &[128, 128, 128, 255], 1, 1)
        }
    }
}

/// Decoded three-part zodiac ribbon textures (top cap, tileable middle, bottom cap).
/// One set of three textures per `ZodiacKind` in `ZodiacKind::all()` order.
pub(crate) struct ZodiacRibbonTextures {
    /// Keeps GPU textures alive so the views remain valid.
    #[allow(dead_code)]
    pub textures: Vec<wgpu::Texture>,
    pub top_views: Vec<wgpu::TextureView>,
    pub mid_views: Vec<wgpu::TextureView>,
    pub bot_views: Vec<wgpu::TextureView>,
}

/// Decode the zodiac silk ribbon PNGs (three parts each: `_top`, `_mid`, `_bot`)
/// and upload them as sRGB textures. Each missing or undecodeable file falls
/// back to a flat 1×1 white texture so the slot still renders (just untextured).
pub(crate) fn load_zodiac_ribbon_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> ZodiacRibbonTextures {
    use crate::core::zodiac::ZodiacKind;
    let zodiacs = ZodiacKind::all();
    let cap = zodiacs.len();
    let mut textures = Vec::with_capacity(cap * 3);
    let mut top_views = Vec::with_capacity(cap);
    let mut mid_views = Vec::with_capacity(cap);
    let mut bot_views = Vec::with_capacity(cap);

    let load_one = |textures: &mut Vec<wgpu::Texture>,
                    slug: &str,
                    part: &str|
     -> wgpu::TextureView {
        let path = format!("textures/zodiac_{}_{}.png", slug, part);
        let label = format!("zodiac-ribbon-{}-{}", slug, part);
        let (tex, view) = match crate::asset_path::get(&path) {
            Some(file) => match image::load_from_memory(&file.data) {
                Ok(img) => {
                    let rgba = img.into_rgba8();
                    let (w, h) = rgba.dimensions();
                    upload_rgba_texture(device, queue, &label, &rgba.into_raw(), w, h)
                }
                Err(e) => {
                    log::warn!("failed to decode {label}: {e} — using flat white fallback");
                    upload_rgba_texture(device, queue, &label, &[255, 255, 255, 255], 1, 1)
                }
            },
            None => {
                log::warn!("zodiac ribbon texture missing at {path} — using flat white fallback");
                upload_rgba_texture(device, queue, &label, &[255, 255, 255, 255], 1, 1)
            }
        };
        textures.push(tex);
        view
    };

    for &z in zodiacs {
        let slug = z.slug();
        top_views.push(load_one(&mut textures, slug, "top"));
        mid_views.push(load_one(&mut textures, slug, "mid"));
        bot_views.push(load_one(&mut textures, slug, "bot"));
    }
    ZodiacRibbonTextures {
        textures,
        top_views,
        mid_views,
        bot_views,
    }
}

/// Spawn a background thread that decodes all relic PNGs and sends the RGBA
/// data back over a channel.  The main thread uploads to the GPU as results
/// arrive (see `poll_relic_textures`).
pub(crate) fn spawn_relic_loader() -> mpsc::Receiver<DecodedRelicImage> {
    use crate::core::relic::all_relic_defs;

    let (tx, rx) = mpsc::channel();

    // Collect the static data we need before moving into the thread.
    let defs: Vec<(RelicId, &'static str, String)> = all_relic_defs()
        .iter()
        .map(|d| {
            let asset_path = format!("textures/relics/{}", d.id.asset_filename());
            (d.id, d.name, asset_path)
        })
        .collect();

    std::thread::Builder::new()
        .name("relic-loader".into())
        .spawn(move || {
            let t_thread = Instant::now();
            let mut decoded = 0usize;
            let mut decode_time = std::time::Duration::ZERO;
            for (id, name, asset_path) in defs {
                let bytes = match crate::asset_path::get(&asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("relic icon not found in embedded assets: {asset_path}");
                        continue;
                    }
                };
                let t_decode = Instant::now();
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode relic icon {asset_path}: {e}");
                        continue;
                    }
                };
                decode_time += t_decode.elapsed();
                decoded += 1;
                let (w, h) = img.dimensions();
                let msg = DecodedRelicImage {
                    id,
                    name,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break; // receiver dropped, renderer shut down
                }
            }
            log::info!(
                "relic-loader thread finished: decoded {decoded} images in {decode_time:?} (thread total {:?})",
                t_thread.elapsed(),
            );
        })
        .expect("failed to spawn relic-loader thread");

    rx
}

/// Load tile-pack box art textures synchronously at init. There are at most 7
/// packs and only a handful have art, so the blocking decode is trivial.
pub(crate) fn load_pack_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    text_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> HashMap<TilePackKind, RelicTextureGpu> {
    let mut map = HashMap::new();
    for &kind in TilePackKind::all() {
        let asset_path = format!("textures/packs/{}", kind.asset_filename());
        let bytes = match crate::asset_path::get(&asset_path) {
            Some(file) => file.data.to_vec(),
            None => {
                log::debug!("pack texture not found (optional): {asset_path}");
                continue;
            }
        };
        let img = match image::load_from_memory(&bytes) {
            Ok(img) => img.into_rgba8(),
            Err(e) => {
                log::warn!("failed to decode pack texture {asset_path}: {e}");
                continue;
            }
        };
        let (w, h) = img.dimensions();
        let (tex, view) = upload_rgba_texture(device, queue, kind.name(), img.as_raw(), w, h);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(kind.name()),
            layout: text_layout,
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
        map.insert(
            kind,
            RelicTextureGpu {
                view,
                texture: tex,
                bind_group,
            },
        );
    }
    log::info!("loaded {} pack textures synchronously", map.len());
    map
}

/// Spawn a background thread that decodes all background PNGs and sends the RGBA
/// data back over a channel.
pub(crate) fn spawn_background_loader() -> mpsc::Receiver<DecodedBackgroundImage> {
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

pub(crate) fn create_depth(
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
pub(crate) fn create_scene_prev(
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

/// Sibling depth texture used as a sampleable snapshot of the scene depth
/// between the pre-smoke and post-smoke render passes.
pub(crate) fn create_depth_copy(
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

// ---------------------------------------------------------------------------
// Per-tile GPU resource builder (free function avoids double-borrow of `self`)
// ---------------------------------------------------------------------------

pub(crate) fn make_hand_tile_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    primitives: &[TilePrimitiveGpu],
    sampler: &wgpu::Sampler,
    base_color_factor: [f32; 4],
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile: &Tile,
    tile_set: Option<&str>,
) -> HandTileGpu {
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // The tile face is 0.734 wide × 1.0 tall in local coords (see tile_3d.wgsl
    // — local Z is the short on-screen-horizontal axis, local X is the long
    // on-screen-vertical axis). Match that aspect in the texture so the GPU
    // stretching doesn't distort the rasterised glyphs.
    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba = rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set);
    let (decal_texture, decal_view) =
        upload_rgba_texture(device, queue, "hand-tile-decal", &rgba, DECAL_W, DECAL_H);

    let bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand-tile-bg"),
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

    // Outline shell uniform + matching bind groups. The outline pipeline
    // only samples binding 0 (camera uniform) but we have to provide the
    // texture/sampler bindings to satisfy the shared layout.
    let outline_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-outline-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let outline_bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand-tile-outline-bg"),
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

    // Per-tile shadow caster uniform — written each frame the tile is
    // visible with the same model matrix as the main uniform.
    let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-shadow-uniform"),
        contents: bytemuck::bytes_of(&ShadowCasterUniform {
            light_view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hand-tile-shadow-bg"),
        layout: shadow_caster_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: shadow_uniform_buffer.as_entire_binding(),
        }],
    });

    let symbol = tile_short_label(tile);
    let suit_emoji = tile_suit_emoji(tile).to_string();
    let suit_color = tile.suit_color();
    HandTileGpu {
        uniform_buffer,
        bind_groups,
        outline_uniform_buffer,
        outline_bind_groups,
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (
            tile.suit,
            tile.rank,
            tile.enhancement,
            tile.debuffed_visual,
            tile.face_down_visual,
        ),
        symbol,
        suit_emoji,
        suit_color,
        decal_texture,
    }
}

pub(crate) fn make_showcase_tile_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    primitives: &[TilePrimitiveGpu],
    sampler: &wgpu::Sampler,
    base_color_factor: [f32; 4],
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile: &Tile,
    tile_set: Option<&str>,
) -> ShowcaseTileGpu {
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba = rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set);
    let (decal_texture, decal_view) = upload_rgba_texture(
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
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (
            tile.suit,
            tile.rank,
            tile.enhancement,
            tile.debuffed_visual,
            tile.face_down_visual,
        ),
        decal_texture,
    }
}

// ---------------------------------------------------------------------------
// Score panel instance builder
// ---------------------------------------------------------------------------

pub fn build_instances_from_layout(
    score: (f32, f32, f32, f32),
    _modifier: (f32, f32, f32, f32),
    _anim_scale_score: f32,
    plays: u32,
    plays_max: u32,
    discards: u32,
    discards_max: u32,
) -> Vec<GpuInstance> {
    use crate::render::theme::color as themec;

    // The score cartouche backplane is replaced by the hanging wooden plaque
    // (`DrawCmd::Plaque`) pushed by the gameplay scene. This function now
    // only emits the plays/discards pip indicators that float at the right
    // edge of the score panel — phase 4 of the skeuomorphic UI redesign
    // replaces these with a physical peg block.
    let (sx, sy, sw, sh) = (score.0, score.1, score.2, score.3);
    let mut v: Vec<GpuInstance> = Vec::new();

    // Pip indicators — two stacked rows of jade/amber pills floating at the
    // right edge of the logical score-panel region (NOT over the cartouche).
    let pip = (sh * 0.22).clamp(8.0, 28.0);
    let gap = pip * 0.25;
    let margin = pip * 0.9;
    let row_gap = pip * 0.3;

    let total_h = pip + row_gap + pip;
    let row1_y = sy + (sh - total_h) * 0.5;
    let row2_y = row1_y + pip + row_gap;

    let plays_row_w = plays_max as f32 * pip + (plays_max.saturating_sub(1)) as f32 * gap;
    let plays_x0 = sx + sw - margin - plays_row_w;
    for i in 0..plays_max {
        let x = plays_x0 + i as f32 * (pip + gap);
        let filled = i < plays;
        v.push(GpuInstance {
            rect: [x, row1_y, pip, pip],
            color: if filled {
                themec::JADE
            } else {
                themec::alpha(themec::JADE, 0.25)
            },
        });
    }

    let disc_row_w = discards_max as f32 * pip + (discards_max.saturating_sub(1)) as f32 * gap;
    let disc_x0 = sx + sw - margin - disc_row_w;
    for i in 0..discards_max {
        let x = disc_x0 + i as f32 * (pip + gap);
        let filled = i < discards;
        v.push(GpuInstance {
            rect: [x, row2_y, pip, pip],
            color: if filled {
                themec::AMBER
            } else {
                themec::alpha(themec::AMBER, 0.25)
            },
        });
    }

    v
}
