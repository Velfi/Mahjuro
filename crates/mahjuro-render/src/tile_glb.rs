//! Load mesh + PBR data from a GLB by walking the **default scene** (same idea as
//! [`crate::room_glb`]): every node that carries a mesh contributes **all** of that mesh's
//! primitives, each transformed by the node's accumulated world matrix.
//!
//! Decodes align with glTF 2.0 where practical: base color (+ factor), normal map (+ scale),
//! metallic–roughness (+ factors), emissive (+ factor), alpha mode/cutoff, `doubleSided`,
//! `KHR_texture_transform` on textures, optional `COLOR_0`, and sampler-driven mips/wrap/filter
//! for GPU upload (see [`crate::gltf_helpers`]).
//!
//! [`crate::room_glb`] shares [`LoadedPrimitive`] / [`Vertex3dTex`] for the shop room mesh.
//!
//! **Blender:** **Z-up** (default) + built-in glTF 2.0 exporter. The file is still stored **Y-up**
//! per glTF. Depending on object rotation / apply, thickness can end up on glTF **+Z** instead of
//! **+Y**; [`normalize_mesh`] rotates that case once (+90° about +X) so
//! [`crate::table_transform::tile_mesh_local_to_world`] always sees thickness on local +Y.

use std::sync::Arc;

use anyhow::Context;
use glam::{Mat4, Vec2, Vec3};
use gltf::image::Format;
use mahjuro_gfx_types::TileMaterial;
use rustc_hash::FxHashMap;

pub use crate::gltf_helpers::{
    GltfAlphaMode, GltfSamplerCpu, apply_texture_transform, sampler_cpu_from_material,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex3dTex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// xyz = tangent (mesh space), w = handedness sign for bitangent (`cross(n, t) * w`).
    pub tangent: [f32; 4],
    /// UV for normal / metallic-roughness / emissive when glTF uses another TEXCOORD set.
    pub uv_emr: [f32; 2],
    /// Linear multiplier from glTF `COLOR_0` (default white).
    pub color: [f32; 4],
}

impl Vertex3dTex {
    pub const DEFAULT_TANGENT: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    /// Canonical constructor (fills `uv_emr = uv`, white `color`); kept for new call sites.
    #[inline]
    pub fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2], tangent: [f32; 4]) -> Self {
        Self {
            position,
            normal,
            uv,
            tangent,
            uv_emr: uv,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// Shared CPU RGBA8 texture payload (cap + optional factor bake); refcounted across primitives.
pub type RgbaTextureCpu = Arc<(Vec<u8>, u32, u32)>;

/// One glTF image decoded to a capped RGBA8 base + precomputed mip chain.
pub struct CappedGltfImage {
    pub base: RgbaTextureCpu,
    /// Level 0 = [`Self::base`]; includes all mips down to 1×1.
    pub mip_chain: Arc<Vec<(Vec<u8>, u32, u32)>>,
    pub source_format: &'static str,
}

/// Memoizes per-image factor/normal bakes and 1×1 solid albedo during one GLB decode.
#[derive(Default)]
pub struct TextureBakeCache {
    albedo_factored: FxHashMap<(usize, [u32; 4]), RgbaTextureCpu>,
    normal_scaled: FxHashMap<(usize, u32), RgbaTextureCpu>,
    solid_albedo: FxHashMap<[u32; 4], RgbaTextureCpu>,
}

impl TextureBakeCache {
    pub fn shared_texture(capped: &CappedGltfImage) -> RgbaTextureCpu {
        Arc::clone(&capped.base)
    }

    pub fn albedo_from_capped(
        &mut self,
        image_index: usize,
        capped: &CappedGltfImage,
        factor: &[f32; 4],
    ) -> RgbaTextureCpu {
        if *factor == [1.0, 1.0, 1.0, 1.0] {
            return Self::shared_texture(capped);
        }
        let key = (image_index, factor_key(factor));
        self.albedo_factored
            .entry(key)
            .or_insert_with(|| {
                let mut pixels = capped.base.0.clone();
                multiply_rgba8_by_factor(&mut pixels, factor);
                Arc::new((pixels, capped.base.1, capped.base.2))
            })
            .clone()
    }

    pub fn normal_from_capped(
        &mut self,
        image_index: usize,
        capped: &CappedGltfImage,
        scale: f32,
    ) -> RgbaTextureCpu {
        if (scale - 1.0).abs() <= 1e-6 {
            return Self::shared_texture(capped);
        }
        let key = (image_index, scale.to_bits());
        self.normal_scaled
            .entry(key)
            .or_insert_with(|| {
                let mut pixels = capped.base.0.clone();
                apply_normal_scale_rgba8(&mut pixels, scale);
                Arc::new((pixels, capped.base.1, capped.base.2))
            })
            .clone()
    }

    pub fn solid_albedo(&mut self, factor: &[f32; 4]) -> RgbaTextureCpu {
        let key = factor_key(factor);
        self.solid_albedo
            .entry(key)
            .or_insert_with(|| Arc::new(solid_albedo_rgba8_unshared(factor)))
            .clone()
    }
}

#[inline]
fn factor_key(factor: &[f32; 4]) -> [u32; 4] {
    [
        factor[0].to_bits(),
        factor[1].to_bits(),
        factor[2].to_bits(),
        factor[3].to_bits(),
    ]
}

#[inline]
pub(crate) fn capped_image_at(
    capped: &[Option<CappedGltfImage>],
    index: usize,
) -> Option<&CappedGltfImage> {
    capped.get(index).and_then(|o| o.as_ref())
}

/// Decode every embedded image once (cap + mips). Scene walks index into this table.
pub fn cap_gltf_images(
    images: &[gltf::image::Data],
    max_dimension: u32,
) -> Vec<Option<CappedGltfImage>> {
    images
        .iter()
        .map(|img| {
            gltf_image_to_rgba8_capped(img, max_dimension).map(|(rgba, w, h)| {
                let base = Arc::new((rgba.clone(), w, h));
                let mip_chain = Arc::new(crate::gltf_helpers::cpu_mip_chain_rgba8(rgba, w, h));
                CappedGltfImage {
                    base,
                    mip_chain,
                    source_format: gltf_image_format_label(img.format),
                }
            })
        })
        .collect()
}

fn gltf_image_format_label(format: gltf::image::Format) -> &'static str {
    use gltf::image::Format as F;
    match format {
        F::R8 => "R8",
        F::R8G8 => "R8G8",
        F::R8G8B8 => "R8G8B8",
        F::R8G8B8A8 => "R8G8B8A8",
        F::R16 => "R16",
        F::R16G16 => "R16G16",
        F::R16G16B16 => "R16G16B16",
        F::R16G16B16A16 => "R16G16B16A16",
        F::R32G32B32FLOAT => "R32G32B32FLOAT",
        F::R32G32B32A32FLOAT => "R32G32B32A32FLOAT",
    }
}

/// One material-slot from the GLB (maps to one glTF primitive).
pub struct LoadedPrimitive {
    pub vertices: Vec<Vertex3dTex>,
    pub indices: Vec<u32>,
    /// Decoded RGBA8, row-major (shared across primitives via [`RgbaTextureCpu`]).
    pub albedo_rgba: Option<RgbaTextureCpu>,
    /// Precomputed mips for [`Self::albedo_rgba`] when sourced from a shared glTF image.
    pub albedo_mip_chain: Option<Arc<Vec<(Vec<u8>, u32, u32)>>>,
    /// Optional tangent-space normal map (linear RGBA8, +X +Y +Z in tangent frame).
    pub normal_rgba: Option<RgbaTextureCpu>,
    pub normal_mip_chain: Option<Arc<Vec<(Vec<u8>, u32, u32)>>>,
    /// Metallic (B) + roughness (G) in linear RGBA8.
    pub metallic_roughness_rgba: Option<RgbaTextureCpu>,
    pub metallic_roughness_mip_chain: Option<Arc<Vec<(Vec<u8>, u32, u32)>>>,
    /// sRGB emissive texture (matches base-color encoding for candle-lit output).
    pub emissive_rgba: Option<RgbaTextureCpu>,
    pub emissive_mip_chain: Option<Arc<Vec<(Vec<u8>, u32, u32)>>>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub emissive_factor: [f32; 3],
    pub alpha_mode: GltfAlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub sampler: GltfSamplerCpu,
}

/// Drop mesh and decoded texture blobs after GPU upload; keeps factors, alpha, sampler, etc.
pub(crate) fn release_loaded_primitive_gpu_source_buffers(prim: &mut LoadedPrimitive) {
    prim.vertices = Vec::new();
    prim.indices = Vec::new();
    prim.albedo_rgba = None;
    prim.albedo_mip_chain = None;
    prim.normal_rgba = None;
    prim.normal_mip_chain = None;
    prim.metallic_roughness_rgba = None;
    prim.metallic_roughness_mip_chain = None;
    prim.emissive_rgba = None;
    prim.emissive_mip_chain = None;
}

/// All decoded primitives from the default scene (order: depth-first scene traversal).
pub struct LoadedTile {
    pub primitives: Vec<LoadedPrimitive>,
}

/// glTF asset path under `assets/` for each player tile material mesh.
pub fn tile_glb_asset_path(material: TileMaterial) -> &'static str {
    match material {
        TileMaterial::Bamboo => "3d/tile_bamboo_and_ivory.glb",
        TileMaterial::Plastic => "3d/tile_plastic.glb",
        TileMaterial::TortoiseShell => "3d/tile_tortoise_shell.glb",
    }
}

/// Index into [`TileMaterial`] mesh tables (`[Bamboo, Plastic, TortoiseShell]`).
pub fn tile_material_index(material: TileMaterial) -> usize {
    match material {
        TileMaterial::Bamboo => 0,
        TileMaterial::Plastic => 1,
        TileMaterial::TortoiseShell => 2,
    }
}

pub const TILE_MATERIAL_MESH_COUNT: usize = 3;

/// Material slot that receives the projected mahjong face decal.
///
/// Authoring convention: glTF material named **`Face`** on a flat quad (see
/// `tile_bamboo_and_ivory.glb`: 4-vertex primitive, zero thickness). Blender may
/// list three material **slots**, but the glTF exporter only writes materials that
/// are assigned to faces — an empty slot 3 does not appear in the `.glb`.
pub fn is_tile_face_material_name(name: Option<&str>) -> bool {
    name.is_some_and(|name| name.eq_ignore_ascii_case("face"))
}

fn normalize_uv01(uvs: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for uv in uvs {
        min[0] = min[0].min(uv[0]);
        max[0] = max[0].max(uv[0]);
        min[1] = min[1].min(uv[1]);
        max[1] = max[1].max(uv[1]);
    }
    let du = (max[0] - min[0]).max(1e-6);
    let dv = (max[1] - min[1]).max(1e-6);
    uvs.iter()
        .map(|uv| [(uv[0] - min[0]) / du, (uv[1] - min[1]) / dv])
        .collect()
}

/// Longest edge (width or height) allowed for glTF-decoded textures. Larger images are
/// downsampled with a 2×2 box filter (repeat halving) before GPU upload — cheap, chunky, PS2-ish.
pub const GLTF_TEXTURE_MAX_DIMENSION: u32 = 256;

fn box_downsample_half_rgba8(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let w_us = w as usize;
    let h_us = h as usize;
    let ow = w_us.div_ceil(2) as u32;
    let oh = h_us.div_ceil(2) as u32;
    let ow_us = ow as usize;
    let oh_us = oh as usize;
    let mut dst = vec![0u8; ow_us.saturating_mul(oh_us).saturating_mul(4)];
    for oy in 0..oh_us {
        for ox in 0..ow_us {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for dy in 0..2 {
                let y = oy * 2 + dy;
                if y >= h_us {
                    continue;
                }
                for dx in 0..2 {
                    let x = ox * 2 + dx;
                    if x >= w_us {
                        continue;
                    }
                    let i = (y * w_us + x) * 4;
                    acc[0] += src[i] as u32;
                    acc[1] += src[i + 1] as u32;
                    acc[2] += src[i + 2] as u32;
                    acc[3] += src[i + 3] as u32;
                    n += 1;
                }
            }
            let oi = (oy * ow_us + ox) * 4;
            if n > 0 {
                dst[oi] = (acc[0] / n) as u8;
                dst[oi + 1] = (acc[1] / n) as u8;
                dst[oi + 2] = (acc[2] / n) as u8;
                dst[oi + 3] = (acc[3] / n) as u8;
            }
        }
    }
    (dst, ow, oh)
}

#[inline]
pub fn clamp_gltf_rgba8_max_dimension_with_cap(
    mut pixels: Vec<u8>,
    mut w: u32,
    mut h: u32,
    cap: u32,
) -> (Vec<u8>, u32, u32) {
    if cap == 0 || (w <= cap && h <= cap) {
        return (pixels, w, h);
    }
    while w > cap || h > cap {
        let (next, nw, nh) = box_downsample_half_rgba8(&pixels, w, h);
        pixels = next;
        w = nw;
        h = nh;
    }
    (pixels, w, h)
}

#[inline]
fn scale_u16_to_u8(v: u16) -> u8 {
    ((v as u32 * 255 + 32767) / 65535).min(255) as u8
}

#[inline]
fn scale_f32_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Convert imported glTF image to RGBA8 for GPU upload.
///
/// Decoded images larger than `max_dimension` on either axis are halved with a 2×2 box filter
/// until both dimensions fit (preserves aspect ratio in a mip-like way).
pub fn gltf_image_to_rgba8_capped(
    img: &gltf::image::Data,
    max_dimension: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let w = img.width;
    let h = img.height;
    let px = w as usize * h as usize;
    let decoded = match img.format {
        Format::R8G8B8A8 => Some((img.pixels.clone(), w, h)),
        Format::R8G8B8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for chunk in img.pixels.chunks(3) {
                v.extend_from_slice(chunk);
                v.push(255);
            }
            Some((v, w, h))
        }
        Format::R8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for &g in &img.pixels {
                v.extend_from_slice(&[g, g, g, 255]);
            }
            Some((v, w, h))
        }
        Format::R8G8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for chunk in img.pixels.chunks_exact(2) {
                v.push(chunk[0]);
                v.push(chunk[1]);
                v.push(0);
                v.push(255);
            }
            Some((v, w, h))
        }
        Format::R16 => {
            if img.pixels.len() != px * 2 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(2) {
                    let g = scale_u16_to_u8(u16::from_ne_bytes([ch[0], ch[1]]));
                    v.extend_from_slice(&[g, g, g, 255]);
                }
                Some((v, w, h))
            }
        }
        Format::R16G16 => {
            if img.pixels.len() != px * 4 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(4) {
                    let r = scale_u16_to_u8(u16::from_ne_bytes([ch[0], ch[1]]));
                    let g = scale_u16_to_u8(u16::from_ne_bytes([ch[2], ch[3]]));
                    v.extend_from_slice(&[r, g, 0, 255]);
                }
                Some((v, w, h))
            }
        }
        Format::R16G16B16 => {
            if img.pixels.len() != px * 6 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(6) {
                    let r = scale_u16_to_u8(u16::from_ne_bytes([ch[0], ch[1]]));
                    let g = scale_u16_to_u8(u16::from_ne_bytes([ch[2], ch[3]]));
                    let b = scale_u16_to_u8(u16::from_ne_bytes([ch[4], ch[5]]));
                    v.extend_from_slice(&[r, g, b, 255]);
                }
                Some((v, w, h))
            }
        }
        Format::R16G16B16A16 => {
            if img.pixels.len() != px * 8 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(8) {
                    let r = scale_u16_to_u8(u16::from_ne_bytes([ch[0], ch[1]]));
                    let g = scale_u16_to_u8(u16::from_ne_bytes([ch[2], ch[3]]));
                    let b = scale_u16_to_u8(u16::from_ne_bytes([ch[4], ch[5]]));
                    let a = scale_u16_to_u8(u16::from_ne_bytes([ch[6], ch[7]]));
                    v.extend_from_slice(&[r, g, b, a]);
                }
                Some((v, w, h))
            }
        }
        Format::R32G32B32FLOAT => {
            if img.pixels.len() != px * 12 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(12) {
                    let r = scale_f32_to_u8(f32::from_ne_bytes([ch[0], ch[1], ch[2], ch[3]]));
                    let g = scale_f32_to_u8(f32::from_ne_bytes([ch[4], ch[5], ch[6], ch[7]]));
                    let b = scale_f32_to_u8(f32::from_ne_bytes([ch[8], ch[9], ch[10], ch[11]]));
                    v.extend_from_slice(&[r, g, b, 255]);
                }
                Some((v, w, h))
            }
        }
        Format::R32G32B32A32FLOAT => {
            if img.pixels.len() != px * 16 {
                None
            } else {
                let mut v = Vec::with_capacity(px * 4);
                for ch in img.pixels.chunks_exact(16) {
                    let r = scale_f32_to_u8(f32::from_ne_bytes([ch[0], ch[1], ch[2], ch[3]]));
                    let g = scale_f32_to_u8(f32::from_ne_bytes([ch[4], ch[5], ch[6], ch[7]]));
                    let b = scale_f32_to_u8(f32::from_ne_bytes([ch[8], ch[9], ch[10], ch[11]]));
                    let a = scale_f32_to_u8(f32::from_ne_bytes([ch[12], ch[13], ch[14], ch[15]]));
                    v.extend_from_slice(&[r, g, b, a]);
                }
                Some((v, w, h))
            }
        }
    };
    decoded.map(|(p, ww, hh)| clamp_gltf_rgba8_max_dimension_with_cap(p, ww, hh, max_dimension))
}

/// Tile / prop glTF decode — capped at [`GLTF_TEXTURE_MAX_DIMENSION`].
pub fn gltf_image_to_rgba8(img: &gltf::image::Data) -> Option<(Vec<u8>, u32, u32)> {
    gltf_image_to_rgba8_capped(img, GLTF_TEXTURE_MAX_DIMENSION)
}

#[inline]
pub(crate) fn multiply_rgba8_by_factor(pixels: &mut [u8], factor: &[f32; 4]) {
    if *factor == [1.0, 1.0, 1.0, 1.0] {
        return;
    }
    for chunk in pixels.chunks_exact_mut(4) {
        for i in 0..4 {
            let v = chunk[i] as f32 / 255.0 * factor[i];
            chunk[i] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

#[inline]
fn solid_albedo_rgba8_unshared(factor: &[f32; 4]) -> (Vec<u8>, u32, u32) {
    let mut px = [0u8; 4];
    for i in 0..4 {
        px[i] = (factor[i].clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    (px.to_vec(), 1, 1)
}

/// MikkTSpace-style tangent accumulation for glTF meshes (positions + normals + UVs in **local** space).
pub fn compute_vertex_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let n_v = positions.len();
    let mut tan1 = vec![Vec3::ZERO; n_v];
    let mut tan2 = vec![Vec3::ZERO; n_v];
    for tri in indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        let p0 = Vec3::from_array(positions[i0]);
        let p1 = Vec3::from_array(positions[i1]);
        let p2 = Vec3::from_array(positions[i2]);
        let uv0 = Vec2::from_array(uvs[i0]);
        let uv1 = Vec2::from_array(uvs[i1]);
        let uv2 = Vec2::from_array(uvs[i2]);
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let du1 = uv1.x - uv0.x;
        let du2 = uv2.x - uv0.x;
        let dv1 = uv1.y - uv0.y;
        let dv2 = uv2.y - uv0.y;
        let denom = du1 * dv2 - du2 * dv1;
        let r = if denom.abs() < 1e-12 {
            0.0
        } else {
            1.0 / denom
        };
        let sdir = Vec3::new(
            (dv2 * e1.x - dv1 * e2.x) * r,
            (dv2 * e1.y - dv1 * e2.y) * r,
            (dv2 * e1.z - dv1 * e2.z) * r,
        );
        let tdir = Vec3::new(
            (-du2 * e1.x + du1 * e2.x) * r,
            (-du2 * e1.y + du1 * e2.y) * r,
            (-du2 * e1.z + du1 * e2.z) * r,
        );
        tan1[i0] += sdir;
        tan1[i1] += sdir;
        tan1[i2] += sdir;
        tan2[i0] += tdir;
        tan2[i1] += tdir;
        tan2[i2] += tdir;
    }
    let mut out = Vec::with_capacity(n_v);
    for i in 0..n_v {
        let nrm = Vec3::from_array(normals[i]).normalize_or_zero();
        let t = tan1[i];
        let t = (t - nrm * nrm.dot(t)).normalize_or_zero();
        let b = nrm.cross(t);
        let b_ref = tan2[i];
        let w = if b.dot(b_ref) < 0.0 { -1.0 } else { 1.0 };
        out.push([t.x, t.y, t.z, w]);
    }
    out
}

/// Apply a rotation to every vertex attribute on all primitives.
fn apply_rotation_to_tile(tile: &mut LoadedTile, r: Mat4) {
    for prim in &mut tile.primitives {
        for v in &mut prim.vertices {
            let p = r.transform_point3(Vec3::from_array(v.position));
            v.position = p.to_array();
            let n = r
                .transform_vector3(Vec3::from_array(v.normal))
                .normalize_or_zero();
            v.normal = n.to_array();
            let t3 = r.transform_vector3(Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]));
            let tv = t3.normalize_or_zero();
            v.tangent = [tv.x, tv.y, tv.z, v.tangent[3]];
        }
    }
}

/// After [`apply_rotation_to_tile`], flip 180° about +X when the **Face** slot points down.
fn ensure_face_normal_points_up(tile: &mut LoadedTile) {
    let mut sum_y = 0.0f32;
    let mut count = 0u32;
    for prim in &tile.primitives {
        for v in &prim.vertices {
            if v.color[3] > 0.5 {
                sum_y += v.normal[1];
                count += 1;
            }
        }
    }
    if count == 0 || sum_y / count as f32 >= 0.0 {
        return;
    }
    apply_rotation_to_tile(tile, Mat4::from_rotation_x(std::f32::consts::PI));
}

/// [`tile_mesh_local_to_world`](crate::table_transform::tile_mesh_local_to_world) assumes
/// **local +Y** is the thin thickness axis / face normal. Z-up Blender glTF exports usually
/// store thickness on **+Z** with the face normal on **+Z**; map that with **−90° about +X**
/// so **+Y** is up. No-op when **+Y** is already the thinnest extent.
fn ensure_tile_engine_local_axes(tile: &mut LoadedTile) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut any = false;
    for prim in &tile.primitives {
        for v in &prim.vertices {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
    }
    if !any {
        return;
    }
    let ex = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let i_min = if ex[0] <= ex[1] && ex[0] <= ex[2] {
        0
    } else if ex[1] <= ex[2] {
        1
    } else {
        2
    };
    let z_is_thinnest_or_tied = ex[2] <= ex[0] + 1e-5 && ex[2] <= ex[1] + 1e-5;
    let rotate_z_thickness_to_y = i_min == 2 || (z_is_thinnest_or_tied && i_min != 1);
    if rotate_z_thickness_to_y {
        apply_rotation_to_tile(tile, Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    }
    ensure_face_normal_points_up(tile);
}

/// Recenter mesh vertices so the AABB center sits at the local origin.
pub fn center_mesh_at_origin(tile: &mut LoadedTile) {
    let Some(center) = mesh_aabb_center(tile) else {
        return;
    };
    for prim in &mut tile.primitives {
        for v in &mut prim.vertices {
            v.position[0] -= center[0];
            v.position[1] -= center[1];
            v.position[2] -= center[2];
        }
    }
}

fn mesh_aabb_center(tile: &LoadedTile) -> Option<[f32; 3]> {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut any = false;
    for prim in &tile.primitives {
        for v in &prim.vertices {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
    }
    if !any {
        return None;
    }
    Some([
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ])
}

/// Center mesh at origin and scale so the largest AABB extent is 1.0.
pub fn normalize_mesh(tile: &mut LoadedTile) {
    ensure_tile_engine_local_axes(tile);

    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut any = false;

    for prim in &tile.primitives {
        for v in &prim.vertices {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
    }

    if !any {
        return;
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];

    let extent = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(1e-6);

    let s = 1.0 / extent;
    for prim in &mut tile.primitives {
        for v in &mut prim.vertices {
            v.position[0] = (v.position[0] - center[0]) * s;
            v.position[1] = (v.position[1] - center[1]) * s;
            v.position[2] = (v.position[2] - center[2]) * s;
        }
    }
}

/// Rotate decoded mesh data into the engine's expected local frame
/// (**+Y** is thickness / face-normal axis) without recentering or
/// renormalizing authored scale.
///
/// Use this when caller wants Blender-authored dimensions to pass
/// through unchanged.
pub fn reorient_mesh_to_engine_axes(tile: &mut LoadedTile) {
    ensure_tile_engine_local_axes(tile);
}

/// Half-extents of the mesh AABB after load-time reorientation/centering.
pub fn mesh_local_half_extents(tile: &LoadedTile) -> [f32; 3] {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut any = false;
    for prim in &tile.primitives {
        for v in &prim.vertices {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
    }
    if !any {
        return [0.5, 0.5, 0.5];
    }
    [
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    ]
}

fn decode_tile_primitive(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[gltf::buffer::Data],
    capped_images: &[Option<CappedGltfImage>],
    bake_cache: &mut TextureBakeCache,
) -> anyhow::Result<LoadedPrimitive> {
    let normal_xform = node_world.inverse().transpose();
    let material = primitive.material();
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions_local: Vec<[f32; 3]> = reader
        .read_positions()
        .context("primitive has no POSITION attribute")?
        .collect();

    let normals_local: Vec<[f32; 3]> = if let Some(n) = reader.read_normals() {
        n.collect()
    } else {
        vec![[0.0, 1.0, 0.0]; positions_local.len()]
    };

    anyhow::ensure!(
        normals_local.len() == positions_local.len(),
        "NORMAL count does not match POSITION count"
    );

    let pbr = material.pbr_metallic_roughness();
    let is_face = is_tile_face_material_name(material.name());

    let base_tex_coord = pbr.base_color_texture().map(|t| t.tex_coord()).unwrap_or(0);
    let mut uvs: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(base_tex_coord) {
        tc.into_f32().collect()
    } else {
        vec![[0.0, 0.0]; positions_local.len()]
    };

    anyhow::ensure!(
        uvs.len() == positions_local.len(),
        "TEXCOORD count does not match POSITION count"
    );

    if let Some(tex_info) = pbr.base_color_texture() {
        apply_texture_transform(&mut uvs, &tex_info);
    }

    let face_uvs = reader
        .read_tex_coords(1)
        .map(|tc| tc.into_f32().collect::<Vec<[f32; 2]>>());

    let mut uv_emr = uvs.clone();
    let mut tangents_local: Vec<[f32; 4]> = if let Some(t_iter) = reader.read_tangents() {
        let t: Vec<[f32; 4]> = t_iter.map(|a| [a[0], a[1], a[2], a[3]]).collect();
        anyhow::ensure!(
            t.len() == positions_local.len(),
            "TANGENT count does not match POSITION count"
        );
        t
    } else {
        Vec::new()
    };

    if let Some(nt) = material.normal_texture() {
        let set = nt.tex_coord();
        if set != base_tex_coord {
            uv_emr = if let Some(tc) = reader.read_tex_coords(set) {
                tc.into_f32().collect()
            } else {
                uvs.clone()
            };
            anyhow::ensure!(
                uv_emr.len() == positions_local.len(),
                "normal TEXCOORD count does not match POSITION count"
            );
            tangents_local.clear();
        }
    }

    if is_face && let Some(mut face) = face_uvs {
        anyhow::ensure!(
            face.len() == positions_local.len(),
            "TEXCOORD_1 count does not match POSITION count on face primitive"
        );
        face = normalize_uv01(&face);
        uv_emr = face;
    }

    let indices: Vec<u32> = if let Some(ids) = reader.read_indices() {
        ids.into_u32().collect()
    } else {
        (0..positions_local.len() as u32).collect()
    };

    if tangents_local.is_empty() {
        tangents_local =
            compute_vertex_tangents(&positions_local, &normals_local, &uv_emr, &indices);
    }

    let colors: Vec<[f32; 4]> = if let Some(iter) = reader.read_colors(0) {
        iter.into_rgba_f32().collect()
    } else {
        Vec::new()
    };

    let face_marker_a = if is_face { 1.0 } else { 0.0 };

    let vertices: Vec<Vertex3dTex> = (0..positions_local.len())
        .map(|i| {
            let p = node_world.transform_point3(Vec3::from(positions_local[i]));
            let n = normal_xform
                .transform_vector3(Vec3::from(normals_local[i]))
                .normalize_or_zero();
            let tl = tangents_local[i];
            let t_loc = Vec3::new(tl[0], tl[1], tl[2]);
            let w = tl[3];
            let t_w = node_world.transform_vector3(t_loc).normalize_or_zero();
            let mut color = colors
                .get(i)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, face_marker_a]);
            if colors.is_empty() {
                color[3] = face_marker_a;
            } else if is_face {
                color[3] = 1.0;
            } else {
                color[3] = 0.0;
            }
            Vertex3dTex {
                position: p.into(),
                normal: n.into(),
                uv: uvs[i],
                tangent: [t_w.x, t_w.y, t_w.z, w],
                uv_emr: uv_emr[i],
                color,
            }
        })
        .collect();

    let factor = pbr.base_color_factor();

    let albedo_src = pbr.base_color_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let mut albedo_rgba = albedo_src.map(|img| {
        let img_index = pbr
            .base_color_texture()
            .expect("albedo_src implies texture")
            .texture()
            .source()
            .index();
        bake_cache.albedo_from_capped(img_index, img, &factor)
    });
    let albedo_mip_chain = albedo_src.map(|c| Arc::clone(&c.mip_chain));

    if albedo_rgba.is_none() && pbr.base_color_texture().is_some() {
        log::warn!(
            "primitive {}: base color texture present but image could not be decoded",
            primitive.index()
        );
    }

    if albedo_rgba.is_none() {
        let want_fallback_tex =
            factor != [1.0, 1.0, 1.0, 1.0] || pbr.base_color_texture().is_some();
        if want_fallback_tex {
            albedo_rgba = Some(bake_cache.solid_albedo(&factor));
        }
    }

    let mr_src = pbr.metallic_roughness_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let metallic_roughness_rgba = mr_src.map(TextureBakeCache::shared_texture);
    let metallic_roughness_mip_chain = mr_src.map(|c| Arc::clone(&c.mip_chain));

    let emissive_src = material.emissive_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let emissive_rgba = emissive_src.map(TextureBakeCache::shared_texture);
    let emissive_mip_chain = emissive_src.map(|c| Arc::clone(&c.mip_chain));

    let normal_src = material.normal_texture().and_then(|nt| {
        let img_index = nt.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let normal_scale = material
        .normal_texture()
        .map(|nt| nt.scale())
        .unwrap_or(1.0);
    let normal_rgba = normal_src.map(|img| {
        let img_index = material
            .normal_texture()
            .expect("normal_src implies texture")
            .texture()
            .source()
            .index();
        bake_cache.normal_from_capped(img_index, img, normal_scale)
    });
    let normal_mip_chain =
        normal_src.and_then(|c| (normal_scale == 1.0).then(|| Arc::clone(&c.mip_chain)));

    if normal_rgba.is_none() && material.normal_texture().is_some() {
        log::warn!(
            "primitive {}: normal texture present but image could not be decoded",
            primitive.index()
        );
    }

    let alpha_mode = GltfAlphaMode::from(material.alpha_mode());
    let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);

    Ok(LoadedPrimitive {
        vertices,
        indices,
        albedo_rgba,
        albedo_mip_chain,
        normal_rgba,
        normal_mip_chain,
        metallic_roughness_rgba,
        metallic_roughness_mip_chain,
        emissive_rgba,
        emissive_mip_chain,
        metallic_factor: pbr.metallic_factor(),
        roughness_factor: pbr.roughness_factor(),
        emissive_factor: crate::gltf_helpers::effective_gltf_emissive_rgb(&material),
        alpha_mode,
        alpha_cutoff,
        double_sided: material.double_sided(),
        sampler: sampler_cpu_from_material(&material),
    })
}

fn walk_tile_scene_nodes_filtered(
    node: gltf::Node<'_>,
    parent: Mat4,
    node_name: Option<&str>,
    out: &mut Vec<LoadedPrimitive>,
    buffers: &[gltf::buffer::Data],
    capped_images: &[Option<CappedGltfImage>],
    bake_cache: &mut TextureBakeCache,
) -> anyhow::Result<()> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    let include_mesh = node_name.is_none_or(|name| node.name() == Some(name));
    if include_mesh && let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            out.push(decode_tile_primitive(
                prim,
                world,
                buffers,
                capped_images,
                bake_cache,
            )?);
        }
    }
    for child in node.children() {
        walk_tile_scene_nodes_filtered(
            child,
            world,
            node_name,
            out,
            buffers,
            capped_images,
            bake_cache,
        )?;
    }
    Ok(())
}

pub fn load_glb_tile_from_bytes(data: &[u8]) -> anyhow::Result<LoadedTile> {
    load_glb_tile_from_node_name(data, None)
}

/// Decode mesh primitives from the default scene, optionally keeping only
/// nodes whose glTF name matches `node_name`.
pub fn load_glb_tile_from_node_name(
    data: &[u8],
    node_name: Option<&str>,
) -> anyhow::Result<LoadedTile> {
    let (document, buffers, images) =
        gltf::import_slice(data).context("gltf::import_slice(tile mesh glb)")?;

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context("GLB has no scenes")?;

    let capped_images = cap_gltf_images(&images, GLTF_TEXTURE_MAX_DIMENSION);
    let mut bake_cache = TextureBakeCache::default();
    let mut primitives = Vec::new();
    for node in scene.nodes() {
        walk_tile_scene_nodes_filtered(
            node,
            Mat4::IDENTITY,
            node_name,
            &mut primitives,
            &buffers,
            &capped_images,
            &mut bake_cache,
        )?;
    }

    anyhow::ensure!(
        !primitives.is_empty(),
        "default scene has no mesh primitives{}",
        node_name
            .map(|n| format!(" for node `{n}`"))
            .unwrap_or_default()
    );
    Ok(LoadedTile { primitives })
}

/// Scale decoded tangent-space normal map texels (linear RGBA8).
fn apply_normal_scale_rgba8(pixels: &mut [u8], scale: f32) {
    if scale == 1.0 {
        return;
    }
    for px in pixels.chunks_exact_mut(4) {
        let nx = (px[0] as f32 / 255.0) * 2.0 - 1.0;
        let ny = (px[1] as f32 / 255.0) * 2.0 - 1.0;
        let nz = (px[2] as f32 / 255.0) * 2.0 - 1.0;
        let snx = nx * scale;
        let sny = ny * scale;
        let len = (snx * snx + sny * sny + nz * nz).sqrt().max(1e-6);
        px[0] = (((snx / len) * 0.5 + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        px[1] = (((sny / len) * 0.5 + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        px[2] = (((nz / len) * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahjuro_gfx_types::TileMaterial;

    fn face_primitive_count(mesh: &LoadedTile) -> usize {
        mesh.primitives
            .iter()
            .filter(|p| p.vertices.iter().any(|v| v.color[3] > 0.5))
            .count()
    }

    fn face_points_up(mesh: &LoadedTile) -> bool {
        for prim in &mesh.primitives {
            for v in &prim.vertices {
                if v.color[3] > 0.5 && v.normal[1] > 0.5 {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn player_tile_glbs_load_with_one_face_primitive_each() {
        for material in [
            TileMaterial::Bamboo,
            TileMaterial::Plastic,
            TileMaterial::TortoiseShell,
        ] {
            let path = tile_glb_asset_path(material);
            let bytes = match path {
                "3d/tile_bamboo_and_ivory.glb" => {
                    include_bytes!("../../../assets/3d/tile_bamboo_and_ivory.glb").as_slice()
                }
                "3d/tile_plastic.glb" => {
                    include_bytes!("../../../assets/3d/tile_plastic.glb").as_slice()
                }
                "3d/tile_tortoise_shell.glb" => {
                    include_bytes!("../../../assets/3d/tile_tortoise_shell.glb").as_slice()
                }
                other => panic!("unexpected tile glb path: {other}"),
            };
            let mut mesh = load_glb_tile_from_bytes(bytes).expect(path);
            normalize_mesh(&mut mesh);
            assert!(
                mesh.primitives.len() >= 2,
                "{path}: expected body + face primitives"
            );
            assert_eq!(
                face_primitive_count(&mesh),
                1,
                "{path}: expected exactly one Face-marked primitive"
            );
            assert!(
                face_points_up(&mesh),
                "{path}: face normal should point up (+local Y) after normalize_mesh"
            );
        }
    }
}
