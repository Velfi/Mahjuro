use rustc_hash::FxHashMap;
use std::sync::mpsc;
use std::time::Instant;

use super::*;

use crate::core::tile_pack::TilePackKind;
use crate::render::gpu_types::RelicTextureGpu;
use crate::render::theme::color;

/// Linear HDR color format for the main 3D scene, bloom chain, and journal
/// GPU scene target — independent of swapchain (SDR vs HDR).
pub(crate) const SCENE_HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Backgrounds decoded in `spawn_background_loader`. Keep in sync with that thread.
pub(crate) const ASYNC_LOADED_BACKGROUNDS: &[BackgroundId] = &[];

/// Pre-loaded background texture + bind group for the image pipeline.
pub(crate) struct BackgroundTextureGpu {
    pub bind_group: wgpu::BindGroup,
}

/// Decoded background image data sent from the background loader thread.
pub(crate) struct DecodedBackgroundImage {
    pub id: BackgroundId,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub(super) fn relic_material_params(
    relic_id: RelicId,
    base_color: [f32; 4],
    glow: f32,
) -> MaterialParams {
    let visual = relic_visual(relic_id);
    let g = glow.clamp(0.0, 1.0);

    // Each metal tier is `RELIC_<METAL>` (the rarity-keyed body color, see
    // `theme::color`) plus a small per-channel admixture of the per-relic
    // `base_color`, so individual relics in the same tier shift slightly
    // around the canonical metal hue without leaving the palette.
    //
    // Per-channel scales bias each tier toward its character: copper-bronze
    // and gold push red harder and cap blue, silver lifts blue a touch less,
    // iron is symmetric. Pulled from the original literals so the visual
    // baseline is unchanged by the token switch.
    let (metal, scale, spec_base, spec_glow, spec_pow) = match visual.material {
        RelicRenderMaterial::Iron => (color::RELIC_IRON, [0.14, 0.14, 0.14], 0.38, 0.18, 26.0),
        RelicRenderMaterial::Copper => (color::RELIC_COPPER, [0.16, 0.14, 0.10], 0.52, 0.22, 34.0),
        RelicRenderMaterial::Silver => (color::RELIC_SILVER, [0.14, 0.14, 0.12], 0.78, 0.22, 64.0),
        RelicRenderMaterial::Gold => (color::RELIC_GOLD, [0.14, 0.14, 0.10], 0.88, 0.24, 80.0),
    };
    MaterialParams {
        kind: MaterialKind::Enamel,
        base_color: [
            metal[0] + base_color[0] * scale[0],
            metal[1] + base_color[1] * scale[1],
            metal[2] + base_color[2] * scale[2],
            base_color[3],
        ],
        specular_strength: spec_base + spec_glow * g,
        specular_power: spec_pow,
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

/// 1×1 linear RGBA encoding tangent-space normal (0, 0, 1) → `(128, 128, 255, 255)`.
pub(super) fn flat_normal_map_ts(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture_linear(
        device,
        queue,
        "tile-flat-normal-ts",
        &[128, 128, 255, 255],
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

pub(super) struct TextureUploadParams<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub label: String,
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub mips: bool,
}

/// Upload RGBA8 with optional CPU-generated mip chain (box filter).
pub(super) fn upload_rgba_texture_with_mips(
    p: &TextureUploadParams<'_>,
) -> (wgpu::Texture, wgpu::TextureView) {
    use crate::render::gltf_helpers::{cpu_mip_chain_rgba8, mip_level_count};
    let mip_levels = if p.mips && p.width.max(p.height) > 1 {
        mip_level_count(p.width, p.height)
    } else {
        1
    };
    let tex = p.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(p.label.as_str()),
        size: wgpu::Extent3d {
            width: p.width,
            height: p.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: p.format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if mip_levels == 1 {
        p.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            p.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(p.width * 4),
                rows_per_image: Some(p.height),
            },
            wgpu::Extent3d {
                width: p.width,
                height: p.height,
                depth_or_array_layers: 1,
            },
        );
    } else {
        let chain = cpu_mip_chain_rgba8(p.rgba.to_vec(), p.width, p.height);
        for (level, (data, mw, mh)) in chain.into_iter().enumerate() {
            let level = level as u32;
            p.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mw * 4),
                    rows_per_image: Some(mh),
                },
                wgpu::Extent3d {
                    width: mw,
                    height: mh,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Load a loose PNG from the asset pack for room-env `decal_tex` binds (e.g. shop candle SSS bake).
pub(super) fn load_room_env_png_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_path: &str,
    label: &str,
    format: wgpu::TextureFormat,
) -> Option<(wgpu::Texture, wgpu::TextureView)> {
    let file = crate::asset_path::get(asset_path)?;
    let img = image::load_from_memory(&file.data).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let mips = w > 1 && h > 1;
    Some(upload_rgba_texture_with_mips(&TextureUploadParams {
        device,
        queue,
        label: label.to_string(),
        rgba: rgba.as_raw(),
        width: w,
        height: h,
        format,
        mips,
    }))
}

/// glTF default metallic–roughness texel: roughness = 1 (G), metallic = 0 (B).
pub(super) fn default_metallic_roughness_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture_linear(device, queue, "gltf-default-mr", &[255, 255, 0, 255], 1, 1)
}

/// glTF: with no `emissiveTexture`, the texture factor is **1** for RGB (not 0).
/// A black 1×1 would zero `emissiveTexture × emissiveFactor` for factor-only materials
/// (typical Blender lamps with emission strength, no emissive map).
pub(super) fn default_emissive_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture(
        device,
        queue,
        "gltf-default-emissive",
        &[255, 255, 255, 255],
        1,
        1,
    )
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
    let bytes_opt = crate::asset_path::get(path);
    let bytes = match &bytes_opt {
        Some(file) => file.data.as_ref(),
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
    match image::load_from_memory(bytes) {
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

/// Decoded full-ribbon zodiac textures — one tall portrait image per zodiac.
/// Indexed by position in `ZodiacKind::all()`.
pub(super) struct ZodiacRibbonTextures {
    /// Keeps GPU textures alive so the views remain valid.
    #[allow(dead_code)]
    pub textures: Vec<wgpu::Texture>,
    pub views: Vec<wgpu::TextureView>,
}

/// Decode the zodiac silk ribbon PNGs (one tall portrait per zodiac at
/// `textures/zodiacs/zodiac_<slug>.png`) and upload them as sRGB textures.
/// Missing or undecodeable files fall back to a flat 1×1 white texture so
/// the slot still renders (just untextured).
pub(super) fn load_zodiac_ribbon_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> ZodiacRibbonTextures {
    use crate::core::zodiac::ZodiacKind;
    let zodiacs = ZodiacKind::all();
    let cap = zodiacs.len();
    let mut textures = Vec::with_capacity(cap);
    let mut views = Vec::with_capacity(cap);

    for &z in zodiacs {
        let slug = z.slug();
        let path = format!("textures/zodiacs/zodiac_{}.png", slug);
        let label = format!("zodiac-ribbon-{}", slug);
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
        views.push(view);
    }
    ZodiacRibbonTextures { textures, views }
}

/// Load tile-pack box art textures synchronously at init. There are at most 7
/// packs and only a handful have art, so the blocking decode is trivial.
pub(super) fn load_pack_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    default_relief_view: &wgpu::TextureView,
) -> FxHashMap<TilePackKind, RelicTextureGpu> {
    let mut map = FxHashMap::default();
    for &kind in TilePackKind::all() {
        let asset_path = format!("textures/tile_packs/{}", kind.asset_filename());
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
        let (_tex, view) = upload_rgba_texture(device, queue, kind.name(), img.as_raw(), w, h);
        map.insert(
            kind,
            RelicTextureGpu {
                view,
                relief_view: default_relief_view.clone(),
            },
        );
    }
    log::debug!("loaded {} pack textures synchronously", map.len());
    map
}

/// Spawn a background thread that decodes all background PNGs and sends the RGBA
/// data back over a channel.
pub(super) fn spawn_background_loader() -> mpsc::Receiver<DecodedBackgroundImage> {
    let (tx, rx) = mpsc::channel();

    let backgrounds: Vec<(BackgroundId, &'static str)> = ASYNC_LOADED_BACKGROUNDS
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
            log::debug!(
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
///
/// Allocated at half the visible resolution (`width / 2`, `height / 2`)
/// — see `scene_color_downsample.wgsl`. The lit_mesh SSR sampler reads
/// it with normalized UVs so any size works. Texture is a render
/// attachment because the downsample blit writes to it directly
/// (the old full-res `copy_texture_to_texture` path is gone).
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Returns the `(width, height)` to allocate `scene_prev_texture` at,
/// given the swapchain's full size. Centralised so the init path and
/// the resize path can't drift.
pub(super) fn scene_prev_size(full_width: u32, full_height: u32) -> (u32, u32) {
    ((full_width / 2).max(1), (full_height / 2).max(1))
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

/// Fullscreen offscreen target for the embedded yaku-journal scene's
/// real GPU render. The shop's open-book mesh samples this view in
/// screen space so the journal content reads as a window cut through
/// the page region rather than a flat decal stuck on the page mesh.
/// Window-sized so its content lines up 1:1 with the post-transition
/// `YakuJournalScene` after handoff.
pub(super) fn create_journal_scene(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("journal-scene-target"),
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

/// Depth texture usable as a shader-sampled snapshot (e.g. SSR history).
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
