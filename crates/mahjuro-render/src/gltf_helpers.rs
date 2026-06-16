//! glTF ↔ wgpu helpers: mip generation, sampler descriptors, UV transforms.

use gltf::material::AlphaMode;
use gltf::texture::{MagFilter, MinFilter, Sampler, WrappingMode};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GltfPbrUniform {
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub alpha_cutoff: f32,
    pub _pad0: f32,
    pub emissive_factor: [f32; 4],
    pub alpha_mode: u32,
    /// Explicit room-env feature flags consumed by `room_glb.wgsl`.
    pub flags: u32,
    pub _pad1: [u32; 2],
}

pub const GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT: u32 = 1 << 0;
pub const GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL: u32 = 1 << 1;
pub const GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE: u32 = 1 << 2;
pub const GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW: u32 = 1 << 3;
pub const GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME: u32 = 1 << 4;
pub const GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO: u32 = 1 << 5;
pub const GLTF_PBR_FLAG_ROOM_CANDLE_WAX: u32 = 1 << 6;
pub const GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER: u32 = 1 << 7;
pub const GLTF_PBR_FLAG_ROOM_READABLE_SURFACE: u32 = 1 << 8;

impl GltfPbrUniform {
    pub fn from_loaded(
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        alpha_mode: GltfAlphaMode,
        alpha_cutoff: f32,
    ) -> Self {
        Self {
            metallic_factor,
            roughness_factor,
            alpha_cutoff,
            _pad0: 0.0,
            emissive_factor: [
                emissive_factor[0],
                emissive_factor[1],
                emissive_factor[2],
                0.0,
            ],
            alpha_mode: alpha_mode as u32,
            flags: 0,
            _pad1: [0; 2],
        }
    }

    #[inline]
    pub fn add_flags(&mut self, flags: u32) {
        self.flags |= flags;
    }
}

/// Linear RGB emissive factor from glTF, including [`KHR_materials_emissive_strength`]
/// when present (defaults to strength `1`).
pub fn effective_gltf_emissive_rgb(material: &gltf::Material<'_>) -> [f32; 3] {
    let f = material.emissive_factor();
    let s = material
        .emissive_strength()
        .filter(|v| v.is_finite())
        .unwrap_or(1.0)
        .max(0.0);
    [f[0] * s, f[1] * s, f[2] * s]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GltfAlphaMode {
    Opaque = 0,
    Mask = 1,
    Blend = 2,
}

impl From<AlphaMode> for GltfAlphaMode {
    fn from(m: AlphaMode) -> Self {
        match m {
            AlphaMode::Opaque => Self::Opaque,
            AlphaMode::Mask => Self::Mask,
            AlphaMode::Blend => Self::Blend,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GltfSamplerCpu {
    pub wrap_s: WrappingMode,
    pub wrap_t: WrappingMode,
    pub mag_filter: Option<MagFilter>,
    pub min_filter: Option<MinFilter>,
}

pub fn sampler_cpu_from_material(material: &gltf::Material<'_>) -> GltfSamplerCpu {
    let pbr = material.pbr_metallic_roughness();
    if let Some(tex_info) = pbr.base_color_texture() {
        return sampler_cpu_from_gltf(tex_info.texture().sampler());
    }
    if let Some(nt) = material.normal_texture() {
        return sampler_cpu_from_gltf(nt.texture().sampler());
    }
    if let Some(mr) = pbr.metallic_roughness_texture() {
        return sampler_cpu_from_gltf(mr.texture().sampler());
    }
    if let Some(em) = material.emissive_texture() {
        return sampler_cpu_from_gltf(em.texture().sampler());
    }
    GltfSamplerCpu {
        wrap_s: WrappingMode::Repeat,
        wrap_t: WrappingMode::Repeat,
        mag_filter: None,
        min_filter: None,
    }
}

pub fn sampler_cpu_from_gltf(s: Sampler<'_>) -> GltfSamplerCpu {
    GltfSamplerCpu {
        wrap_s: s.wrap_s(),
        wrap_t: s.wrap_t(),
        mag_filter: s.mag_filter(),
        min_filter: s.min_filter(),
    }
}

#[inline]
pub fn wrap_to_address(w: WrappingMode) -> wgpu::AddressMode {
    match w {
        WrappingMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        WrappingMode::Repeat => wgpu::AddressMode::Repeat,
        WrappingMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

pub fn wants_mipmaps(min: Option<MinFilter>) -> bool {
    matches!(
        min,
        Some(
            MinFilter::NearestMipmapNearest
                | MinFilter::LinearMipmapNearest
                | MinFilter::NearestMipmapLinear
                | MinFilter::LinearMipmapLinear
        )
    )
}

pub fn build_sampler_descriptor(
    cpu: GltfSamplerCpu,
    label: Option<&'static str>,
) -> wgpu::SamplerDescriptor<'static> {
    let mag = cpu
        .mag_filter
        .map(|m| match m {
            MagFilter::Nearest => wgpu::FilterMode::Nearest,
            MagFilter::Linear => wgpu::FilterMode::Linear,
        })
        .unwrap_or(wgpu::FilterMode::Linear);
    let min = cpu
        .min_filter
        .map(|m| match m {
            MinFilter::Nearest | MinFilter::NearestMipmapNearest => wgpu::FilterMode::Nearest,
            MinFilter::Linear
            | MinFilter::LinearMipmapNearest
            | MinFilter::NearestMipmapLinear
            | MinFilter::LinearMipmapLinear => wgpu::FilterMode::Linear,
        })
        .unwrap_or(wgpu::FilterMode::Linear);
    let mipmap_filter = match cpu.min_filter {
        Some(MinFilter::NearestMipmapNearest | MinFilter::LinearMipmapNearest) => {
            wgpu::MipmapFilterMode::Nearest
        }
        Some(MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear) => {
            wgpu::MipmapFilterMode::Linear
        }
        _ => wgpu::MipmapFilterMode::Nearest,
    };

    wgpu::SamplerDescriptor {
        label,
        address_mode_u: wrap_to_address(cpu.wrap_s),
        address_mode_v: wrap_to_address(cpu.wrap_t),
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: mag,
        min_filter: min,
        mipmap_filter,
        ..Default::default()
    }
}

/// Apply KHR_texture_transform to each UV in-place.
pub fn apply_texture_transform(uvs: &mut [[f32; 2]], tex_info: &gltf::texture::Info<'_>) {
    let Some(t) = tex_info.texture_transform() else {
        return;
    };
    let [ox, oy] = t.offset();
    let [sx, sy] = t.scale();
    let r = t.rotation();
    let (sin_r, cos_r) = r.sin_cos();
    for uv in uvs.iter_mut() {
        let u = uv[0];
        let v = uv[1];
        uv[0] = u * sx * cos_r - v * sy * sin_r + ox;
        uv[1] = u * sx * sin_r + v * sy * cos_r + oy;
    }
}

pub fn mip_level_count(w: u32, h: u32) -> u32 {
    let d = w.max(h);
    if d <= 1 {
        1
    } else {
        u32::BITS - d.leading_zeros()
    }
}

fn sample_rgba(src: &[u8], sw: u32, _sh: u32, x: u32, y: u32) -> [f32; 4] {
    let i = ((y * sw + x) * 4) as usize;
    [
        src[i] as f32,
        src[i + 1] as f32,
        src[i + 2] as f32,
        src[i + 3] as f32,
    ]
}

/// Half-resolution box filter for RGBA8.
pub fn downsample_rgba8_box(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for dy in 0..dh {
        for dx in 0..dw {
            let x0 = dx * 2;
            let y0 = dy * 2;
            let mut acc = [0.0f32; 4];
            let mut count = 0.0f32;
            for oy in 0..2 {
                for ox in 0..2 {
                    let sx = (x0 + ox).min(sw - 1);
                    let sy = (y0 + oy).min(sh - 1);
                    let p = sample_rgba(src, sw, sh, sx, sy);
                    for c in 0..4 {
                        acc[c] += p[c];
                    }
                    count += 1.0;
                }
            }
            let base = ((dy * dw + dx) * 4) as usize;
            for c in 0..4 {
                out[base + c] = ((acc[c] / count).round()).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

pub fn cpu_mip_chain_rgba8(mut rgba: Vec<u8>, mut w: u32, mut h: u32) -> Vec<(Vec<u8>, u32, u32)> {
    let mut levels = Vec::new();
    levels.push((rgba.clone(), w, h));
    while w > 1 || h > 1 {
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        rgba = downsample_rgba8_box(&levels.last().unwrap().0, w, h, nw, nh);
        levels.push((rgba.clone(), nw, nh));
        w = nw;
        h = nh;
    }
    levels
}
