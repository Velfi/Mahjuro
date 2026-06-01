use rustc_hash::FxHashMap;
use std::sync::mpsc;
use std::time::Instant;

use super::*;

use mahjuro_core::core::tile_pack::TilePackKind;
use crate::gpu_types::RelicTextureGpu;
use crate::theme::color;

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

/// Build GPU resources for the rain-hit debug overlay (merged `rain_hit_*` shells).
pub(super) fn init_main_menu_rain_hit_debug(
    device: &wgpu::Device,
    material_layout: &wgpu::BindGroupLayout,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    albedo_view: &wgpu::TextureView,
    relief_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> (Option<LitMeshGpu>, Option<LitMeshInstance>) {
    use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
    use crate::tile_glb::Vertex3dTex;

    let meshes = crate::main_menu_glb::main_menu_rain_surface_meshes();
    if meshes.is_empty() {
        return (None, None);
    }
    let tri_count: usize = meshes.iter().map(|m| m.triangles.len()).sum();
    let mut vertices = Vec::with_capacity(tri_count * 3);
    let mut indices = Vec::with_capacity(tri_count * 3);
    for mesh in &meshes {
        for tri in &mesh.triangles {
            let base = vertices.len() as u32;
            for v in tri {
                vertices.push(Vertex3dTex::new(
                    v.to_array(),
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0],
                    Vertex3dTex::DEFAULT_TANGENT,
                ));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }
    if indices.is_empty() {
        return (None, None);
    }
    let cpu = MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Emissive,
            base_color: [1.0, 0.06, 0.06, 0.95],
            specular_strength: 2.5,
            specular_power: 0.0,
        },
    };
    let gpu = LitMeshGpu::new(device, &cpu, "main-menu-rain-hit-debug");
    let inst = LitMeshInstance::new(
        device,
        material_layout,
        shadow_caster_layout,
        albedo_view,
        relief_view,
        sampler,
    );
    (Some(gpu), Some(inst))
}

/// Flat texture read for boss-icon medallions (matches 2D atlas; no scene lighting).
pub(super) fn ordeal_icon_material_params(base_color: [f32; 4], glow: f32) -> MaterialParams {
    let g = glow.clamp(0.0, 1.0);
    let target = [1.55, 1.32, 0.78, base_color[3]];
    MaterialParams {
        kind: MaterialKind::Unshaded,
        base_color: [
            base_color[0] + (target[0] - base_color[0]) * g,
            base_color[1] + (target[1] - base_color[1]) * g,
            base_color[2] + (target[2] - base_color[2]) * g,
            base_color[3],
        ],
        specular_strength: 0.0,
        specular_power: 0.0,
    }
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

pub(crate) struct TextureUploadParams<'a> {
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
pub(crate) fn upload_rgba_texture_with_mips(
    p: &TextureUploadParams<'_>,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture_with_mip_chain(p, None)
}

/// Like [`upload_rgba_texture_with_mips`], but reuses a precomputed chain from room GLB decode.
pub(super) fn upload_rgba_texture_with_mip_chain(
    p: &TextureUploadParams<'_>,
    precomputed: Option<&[(Vec<u8>, u32, u32)]>,
) -> (wgpu::Texture, wgpu::TextureView) {
    use crate::gltf_helpers::{cpu_mip_chain_rgba8, mip_level_count};
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
        let chain: Vec<(Vec<u8>, u32, u32)> = match precomputed {
            Some(levels) => levels.to_vec(),
            None => cpu_mip_chain_rgba8(p.rgba.to_vec(), p.width, p.height),
        };
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

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct RoomEnvTextureCacheKey {
    content_hash: u64,
    width: u32,
    height: u32,
    format_tag: u8,
    mips: bool,
}

fn room_env_texture_cache_key(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    mips: bool,
) -> RoomEnvTextureCacheKey {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    let step = (rgba.len() / 8192).max(4);
    for (i, b) in rgba.iter().enumerate().step_by(step) {
        i.hash(&mut hasher);
        b.hash(&mut hasher);
    }
    let format_tag = match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => 0,
        wgpu::TextureFormat::Rgba8Unorm => 1,
        _ => 2,
    };
    RoomEnvTextureCacheKey {
        content_hash: hasher.finish(),
        width,
        height,
        format_tag,
        mips,
    }
}

/// Dedupes identical room-env texture uploads during renderer init (shared glTF images).
pub(super) struct RoomEnvTextureCache {
    views: rustc_hash::FxHashMap<RoomEnvTextureCacheKey, wgpu::TextureView>,
    textures: Vec<wgpu::Texture>,
}

impl RoomEnvTextureCache {
    pub fn new() -> Self {
        Self {
            views: rustc_hash::FxHashMap::default(),
            textures: Vec::new(),
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: String,
        rgba: &[u8],
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        mips: bool,
        mip_chain: Option<&[(Vec<u8>, u32, u32)]>,
    ) -> wgpu::TextureView {
        let key = room_env_texture_cache_key(rgba, width, height, format, mips);
        if let Some(view) = self.views.get(&key) {
            return view.clone();
        }
        let (tex, view) = upload_rgba_texture_with_mip_chain(
            &TextureUploadParams {
                device,
                queue,
                label,
                rgba,
                width,
                height,
                format,
                mips,
            },
            mip_chain,
        );
        self.textures.push(tex);
        self.views.insert(key, view.clone());
        view
    }

    pub fn upload_slot(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: String,
        rgba: Option<&(Vec<u8>, u32, u32)>,
        mip_chain: Option<&[(Vec<u8>, u32, u32)]>,
        format: wgpu::TextureFormat,
        mips: bool,
        fallback: &wgpu::TextureView,
    ) -> wgpu::TextureView {
        match rgba {
            Some((rgba, w, h)) => {
                self.upload(device, queue, label, rgba, *w, *h, format, mips, mip_chain)
            }
            None => fallback.clone(),
        }
    }
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
/// embedded assets and uploads as a linear (non-sRGB) RGBA8 texture.
pub(super) fn load_metal_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &str,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let bytes_opt = mahjuro_assets::asset_path::get(path);
    let bytes = match &bytes_opt {
        Some(file) => file.data.as_ref(),
        None => panic!("{label} asset missing at {path}"),
    };
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            upload_rgba_texture_linear(device, queue, label, &rgba.into_raw(), w, h)
        }
        Err(e) => panic!("failed to decode {label} at {path}: {e}"),
    }
}

/// Decoded full-ribbon zodiac textures — one tall portrait image per zodiac.
/// Indexed by position in `ZodiacKind::all()`.
pub(super) struct ZodiacRibbonTextures {
    /// Keeps GPU textures alive so the views remain valid.
    pub _textures: Vec<wgpu::Texture>,
    pub views: Vec<wgpu::TextureView>,
}

/// Decode zodiac silk ribbon PNGs (one tall portrait per zodiac at
/// `textures/zodiacs/zodiac_<slug>.png`) and upload them as sRGB textures.
pub(super) fn load_zodiac_ribbon_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> ZodiacRibbonTextures {
    use mahjuro_core::core::zodiac::ZodiacKind;
    let zodiacs = ZodiacKind::all();
    let cap = zodiacs.len();
    let mut textures = Vec::with_capacity(cap);
    let mut views = Vec::with_capacity(cap);

    for &z in zodiacs {
        let slug = z.slug();
        let path = format!("textures/zodiacs/zodiac_{}.png", slug);
        let label = format!("zodiac-ribbon-{}", slug);
        let (tex, view) = match mahjuro_assets::asset_path::get(&path) {
            Some(file) => {
                let img = image::load_from_memory(&file.data)
                    .unwrap_or_else(|e| panic!("failed to decode {label} at {path}: {e}"))
                    .into_rgba8();
                let (w, h) = img.dimensions();
                upload_rgba_texture(device, queue, &label, &img.into_raw(), w, h)
            }
            None => panic!("zodiac ribbon texture missing at {path}"),
        };
        textures.push(tex);
        views.push(view);
    }
    ZodiacRibbonTextures { _textures: textures, views }
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
        let bytes = match mahjuro_assets::asset_path::get(&asset_path) {
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
                let bytes = match mahjuro_assets::asset_path::get(asset_path) {
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

/// R32 depth snapshot for `textureLoad` in SSR / emissive-probe shaders.
/// GLES/llvmpipe cannot compile WGSL `textureLoad` on `texture_depth_2d`.
pub(crate) fn create_depth_r32_snapshot(
    device: &wgpu::Device,
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
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

pub(crate) fn depth_copy_buffer_size(width: u32, height: u32) -> u64 {
    let bytes_per_row = depth_copy_bytes_per_row(width);
    bytes_per_row as u64 * height.max(1) as u64
}

fn depth_copy_bytes_per_row(width: u32) -> u32 {
    let unpadded = width.max(1) * 4;
    unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

pub(crate) fn create_depth_copy_staging(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("depth-copy-staging"),
        size: depth_copy_buffer_size(width, height),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Copy a hardware depth attachment into an R32Float snapshot via staging.
pub(crate) fn encode_copy_depth_to_r32float(
    encoder: &mut wgpu::CommandEncoder,
    staging: &wgpu::Buffer,
    src_depth: &wgpu::Texture,
    dst_r32: &wgpu::Texture,
    width: u32,
    height: u32,
) {
    let width = width.max(1);
    let height = height.max(1);
    let bytes_per_row = depth_copy_bytes_per_row(width);
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let buffer_layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(bytes_per_row),
        rows_per_image: Some(height),
    };
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: src_depth,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: staging,
            layout: buffer_layout,
        },
        extent,
    );
    encoder.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo {
            buffer: staging,
            layout: buffer_layout,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst_r32,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        extent,
    );
}

/// Row stride for `queue.write_texture` of tightly packed RGBA8 (`width` × 4 bytes).
pub(crate) fn rgba8_copy_bytes_per_row(width: u32) -> u32 {
    let unpadded = width.max(1) * 4;
    unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Upload tightly packed RGBA8 (`width * height * 4` bytes) into an `Rgba8*` texture.
pub(crate) fn write_rgba8_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    tight_rgba: &[u8],
) {
    let width = width.max(1);
    let height = height.max(1);
    let unpadded = width * 4;
    let bytes_per_row = rgba8_copy_bytes_per_row(width);
    let expected = (unpadded * height) as usize;
    assert_eq!(
        tight_rgba.len(),
        expected,
        "write_rgba8_texture: expected {expected} bytes, got {}",
        tight_rgba.len()
    );
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(bytes_per_row),
        rows_per_image: Some(height),
    };
    let tex_info = wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    };
    if bytes_per_row == unpadded {
        queue.write_texture(tex_info, tight_rgba, layout, extent);
        return;
    }
    let mut padded = vec![0u8; (bytes_per_row * height) as usize];
    for y in 0..height {
        let src = (y * unpadded) as usize;
        let dst = (y * bytes_per_row) as usize;
        padded[dst..dst + unpadded as usize].copy_from_slice(&tight_rgba[src..src + unpadded as usize]);
    }
    queue.write_texture(tex_info, &padded, layout, extent);
}

#[cfg(test)]
mod rgba8_upload_tests {
    use super::rgba8_copy_bytes_per_row;

    #[test]
    fn rgba8_copy_bytes_per_row_aligns_wide_archive_sign_decals() {
        // 320px tall sign face with ~3.26:1 aspect → width 1045 (see decal_dimensions).
        assert_eq!(rgba8_copy_bytes_per_row(1045), 4352);
        assert_eq!(rgba8_copy_bytes_per_row(256), 1024);
    }
}
