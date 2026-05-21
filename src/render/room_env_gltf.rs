//! Shared glTF **room environment** decode: PBR meshes, bounds, embedded perspective cameras,
//! [`KHR_lights_punctual`], collision triangle soups, and world-space framing helpers.
//!
//! [`crate::render::room_glb`] and [`crate::render::hallway_glb`] load different assets but share
//! this pipeline and GPU path (`room_glb.wgsl`).

use std::sync::Arc;

use rustc_hash::FxHashMap;

use anyhow::Context;
use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::render::draw_cmd::CameraParams;
use crate::render::gltf_helpers::{
    apply_texture_transform, cpu_mip_chain_rgba8, sampler_cpu_from_material,
};
use crate::render::tile_glb::{
    GltfAlphaMode, LoadedPrimitive, Vertex3dTex, compute_vertex_tangents,
    gltf_image_to_rgba8_capped, multiply_rgba8_by_factor, solid_albedo_rgba8,
};

/// Longest edge for shop / hallway / archive / main-menu room glTF textures (tiles stay 256).
pub const ROOM_ENV_TEXTURE_MAX_DIMENSION: u32 = 1024;

#[inline]
fn room_env_gltf_image_to_rgba8(img: &gltf::image::Data) -> Option<(Vec<u8>, u32, u32)> {
    gltf_image_to_rgba8_capped(img, ROOM_ENV_TEXTURE_MAX_DIMENSION)
}

/// One glTF image decoded to ≤[`ROOM_ENV_TEXTURE_MAX_DIMENSION`] RGBA8 with a precomputed mip chain.
pub struct CappedGltfImage {
    pub base: (Vec<u8>, u32, u32),
    /// Level 0 = [`Self::base`]; includes all mips down to 1×1.
    pub mip_chain: Arc<Vec<(Vec<u8>, u32, u32)>>,
}

/// Decode every embedded image once (cap + mips). Room env walks index into this table.
pub fn cap_room_gltf_images(images: &[gltf::image::Data]) -> Vec<Option<CappedGltfImage>> {
    images
        .iter()
        .map(|img| {
            room_env_gltf_image_to_rgba8(img).map(|(rgba, w, h)| {
                let mip_chain = Arc::new(cpu_mip_chain_rgba8(rgba.clone(), w, h));
                CappedGltfImage {
                    base: (rgba, w, h),
                    mip_chain,
                }
            })
        })
        .collect()
}

#[inline]
fn capped_image_base(img: &CappedGltfImage) -> (Vec<u8>, u32, u32) {
    (img.base.0.clone(), img.base.1, img.base.2)
}

#[inline]
fn capped_image_at<'a>(
    capped: &'a [Option<CappedGltfImage>],
    index: usize,
) -> Option<&'a CappedGltfImage> {
    capped.get(index).and_then(|o| o.as_ref())
}

/// `COLOR_0.a` tag: archive sign description samples [`decal_tex`](../../shaders/room_glb.wgsl) at `uv`.
pub const ROOM_ENV_COLOR_A_ARCHIVE_DECAL: f32 = 2.0;
/// `COLOR_0.a` tag: shop candle wax samples baked SSS at `uv` (glTF `TEXCOORD_1`).
pub const ROOM_ENV_COLOR_A_CANDLE_SSS_BAKE: f32 = 4.0;

/// glTF node prefix for shop environment candle **meshes** (not `light_candle*` punctuals).
pub const SHOP_CANDLE_WAX_NODE_PREFIX: &str = "Candle";
/// Blender material name on exported shop candle geometry.
pub const SHOP_CANDLE_WAX_MATERIAL_NAME: &str = "Wax SSS translucent shader";
/// Baked subsurface pass (linear RGB) sampled with `TEXCOORD_1` — rebake after moving candles.
pub const SHOP_CANDLE_SSS_BAKE_TEXTURE: &str = "textures/shop/candle_sss.png";

/// Shop [`shop.glb`](../../../assets/3d/shop.glb) candle votive mesh (wax body).
#[inline]
pub fn is_shop_candle_wax_mesh(gltf_node_name: &str, material_name: Option<&str>) -> bool {
    gltf_node_name.starts_with(SHOP_CANDLE_WAX_NODE_PREFIX)
        || material_name == Some(SHOP_CANDLE_WAX_MATERIAL_NAME)
}

#[inline]
pub fn is_shop_candle_wax_node_name(node: &str) -> bool {
    node.starts_with(SHOP_CANDLE_WAX_NODE_PREFIX)
}

/// One environment mesh primitive plus embedded glTF sampler parameters for GPU samplers.
pub struct RoomEnvPrimitiveCpu {
    /// glTF node name for this primitive (per-node mesh), when known.
    pub gltf_node_name: Option<String>,
    pub mesh: LoadedPrimitive,
}

/// CPU triangle soup for one named GLB node (typically invisible anchor geometry).
#[derive(Clone)]
pub struct RoomCollisionMesh {
    pub node_name: String,
    pub triangles: Vec<[Vec3; 3]>,
}

/// Axis-aligned bounds of decoded environment vertices (glTF document space).
#[derive(Clone, Copy, Debug)]
pub struct RoomEnvironmentBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl RoomEnvironmentBounds {
    #[inline]
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn corners(self) -> [Vec3; 8] {
        let mn = self.min;
        let mx = self.max;
        [
            Vec3::new(mn.x, mn.y, mn.z),
            Vec3::new(mx.x, mn.y, mn.z),
            Vec3::new(mn.x, mx.y, mn.z),
            Vec3::new(mx.x, mx.y, mn.z),
            Vec3::new(mn.x, mn.y, mx.z),
            Vec3::new(mx.x, mn.y, mx.z),
            Vec3::new(mn.x, mx.y, mx.z),
            Vec3::new(mx.x, mx.y, mx.z),
        ]
    }
}

/// [`KHR_lights_punctual`] point light — positions in **document units** (same as mesh).
#[derive(Clone, Debug)]
pub struct RoomGltfEmbeddedPointLight {
    pub node_name: String,
    pub pos_doc: Vec3,
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub is_lantern: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
}

/// [`KHR_lights_punctual`] spot light — cone aims along node **−Z** in document space.
#[derive(Clone, Copy, Debug)]
pub struct RoomGltfEmbeddedSpotLight {
    pub pos_doc: Vec3,
    pub dir_doc: Vec3,
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub is_lantern: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
    pub inner_cone_rad: f32,
    pub outer_cone_rad: f32,
}

/// Perspective camera baked into a room glTF (positions in **document units**).
#[derive(Clone, Copy, Debug)]
pub struct RoomGltfEmbeddedCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fovy_deg: f32,
}

impl RoomGltfEmbeddedCamera {
    pub fn to_camera_params(
        self,
        window_h: f32,
        env_height_scale: f32,
        center_doc: Vec3,
    ) -> CameraParams {
        let s = room_env_world_scale(window_h, env_height_scale);
        let up = self.up.normalize_or_zero();
        let up = if up.length_squared() > 1e-12 {
            up
        } else {
            Vec3::Z
        };
        CameraParams {
            eye: ((self.eye - center_doc) * s).to_array(),
            target: ((self.target - center_doc) * s).to_array(),
            up: up.to_array(),
            fovy_deg: self.fovy_deg,
            clip_near: None,
            clip_far: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct EmbeddedCameraHarvest {
    named: Option<RoomGltfEmbeddedCamera>,
    fallback: Option<RoomGltfEmbeddedCamera>,
    /// Lowercase glTF node names → perspective cameras (hallway `default` / `boss`, etc.).
    by_name: FxHashMap<String, RoomGltfEmbeddedCamera>,
}

impl EmbeddedCameraHarvest {
    pub(crate) fn insert(&mut self, name: &str, cam: RoomGltfEmbeddedCamera) {
        let key = name.to_ascii_lowercase();
        if self.by_name.insert(key.clone(), cam).is_some() {
            log::warn!("room glTF: duplicate embedded camera `{name}` — using last");
        }
        let preferred = matches!(key.as_str(), "camera" | "shopcamera" | "shop_camera");
        if preferred {
            if self.named.replace(cam).is_some() {
                log::warn!("room glTF: multiple preferred camera node names — using last");
            }
        } else if self.fallback.is_none() {
            self.fallback = Some(cam);
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Option<RoomGltfEmbeddedCamera>,
        FxHashMap<String, RoomGltfEmbeddedCamera>,
    ) {
        let picked = self.named.or(self.fallback);
        (picked, self.by_name)
    }
}

#[inline]
pub fn room_env_world_scale(window_h: f32, height_scale: f32) -> f32 {
    window_h.max(1e-6) * height_scale
}

/// KHR punctual `range` in **world units** for point/spot `pos.w` (`room_glb.wgsl`).
#[inline]
pub fn glb_punctual_range_world_upload(window_h: f32, scale: f32, range_doc: Option<f32>) -> f32 {
    match range_doc {
        None => (window_h * 24.0).max(scale * 40.0),
        Some(r) if r.is_finite() && r > 0.0 => r * scale,
        Some(_) => 0.0,
    }
}

#[inline]
pub fn room_env_model_matrix(window_h: f32, height_scale: f32, center_doc: Vec3) -> Mat4 {
    let s = room_env_world_scale(window_h, height_scale);
    Mat4::from_translation(-center_doc * s) * Mat4::from_scale(Vec3::splat(s))
}

#[inline]
pub fn room_env_model_matrix_from_bounds_doc(
    window_h: f32,
    height_scale: f32,
    environment_bounds_doc: Option<RoomEnvironmentBounds>,
) -> Mat4 {
    let c = environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    room_env_model_matrix(window_h, height_scale, c)
}

fn compute_room_environment_bounds(prims: &[RoomEnvPrimitiveCpu]) -> Option<RoomEnvironmentBounds> {
    let mut min_v = Vec3::splat(f32::INFINITY);
    let mut max_v = Vec3::splat(f32::NEG_INFINITY);
    for p in prims {
        for vtx in &p.mesh.vertices {
            let pos = Vec3::from(vtx.position);
            min_v = min_v.min(pos);
            max_v = max_v.max(pos);
        }
    }
    if !min_v.x.is_finite() || !max_v.x.is_finite() {
        return None;
    }
    Some(RoomEnvironmentBounds {
        min: min_v,
        max: max_v,
    })
}

pub fn room_environment_bounds(prims: &[RoomEnvPrimitiveCpu]) -> Option<RoomEnvironmentBounds> {
    compute_room_environment_bounds(prims)
}

/// World-unit slack in front of the AABB entry when the camera is **outside** the room.
pub const ROOM_CAMERA_CLIP_NEAR_INSET: f32 = 400.0;
/// Minimum near plane in world units after room scale (also used when the camera sits inside the AABB).
pub const ROOM_CAMERA_CLIP_NEAR_MIN: f32 = 80.0;
/// World-unit slack beyond the farthest bounds exit / corner along the view ray.
pub const ROOM_CAMERA_CLIP_FAR_PAD: f32 = 1200.0;
/// Cap `far / near` so long corridors keep usable depth precision.
pub const ROOM_CAMERA_CLIP_MAX_RATIO: f32 = 2000.0;

fn aabb_from_corners(corners_world: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for p in corners_world {
        mn = mn.min(*p);
        mx = mx.max(*p);
    }
    if !mn.x.is_finite() || !mx.x.is_finite() {
        return None;
    }
    Some((mn, mx))
}

/// Ray–slab entry / exit distances along `dir` (world units). Returns `None` when the ray misses.
fn ray_aabb_t_enter_exit(origin: Vec3, dir: Vec3, bmin: Vec3, bmax: Vec3) -> Option<(f32, f32)> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;
    for i in 0..3 {
        let o = origin[i];
        let d = dir[i];
        let mn = bmin[i];
        let mx = bmax[i];
        if d.abs() < 1e-8 {
            if o < mn || o > mx {
                return None;
            }
            continue;
        }
        let t1 = (mn - o) / d;
        let t2 = (mx - o) / d;
        let (t_near, t_far) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        t_min = t_min.max(t_near);
        t_max = t_max.min(t_far);
    }
    if t_min > t_max || t_max < 0.0 {
        return None;
    }
    Some((t_min, t_max))
}

/// Tighten [`CameraParams::clip_near`] / [`CameraParams::clip_far`] from the room AABB.
/// Embedded GLB rooms scale ~with `window_h`; the legacy `near = 1` / `far = h × 32` range
/// wastes depth precision. When the authored camera sits **inside** the bounds (hallway),
/// AABB corners are all behind or beyond the nearby walls — keep a low near plane and fit
/// far to the forward ray exit instead.
pub fn room_camera_fit_clip_planes(mut cam: CameraParams, corners_world: &[Vec3]) -> CameraParams {
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    let forward = (target - eye).normalize_or_zero();
    if forward.length_squared() < 1e-12 || corners_world.is_empty() {
        return cam;
    }

    let Some((bmin, bmax)) = aabb_from_corners(corners_world) else {
        return cam;
    };
    let Some((t_enter, t_exit)) = ray_aabb_t_enter_exit(eye, forward, bmin, bmax) else {
        return cam;
    };

    let inside = eye.x >= bmin.x
        && eye.x <= bmax.x
        && eye.y >= bmin.y
        && eye.y <= bmax.y
        && eye.z >= bmin.z
        && eye.z <= bmax.z;

    let near = if inside || t_enter <= 0.0 {
        ROOM_CAMERA_CLIP_NEAR_MIN
    } else {
        (t_enter - ROOM_CAMERA_CLIP_NEAR_INSET).max(ROOM_CAMERA_CLIP_NEAR_MIN)
    };

    let mut max_corner_d = f32::NEG_INFINITY;
    for p in corners_world {
        let d = (*p - eye).dot(forward);
        if d > 0.01 {
            max_corner_d = max_corner_d.max(d);
        }
    }
    let mut far = (t_exit + ROOM_CAMERA_CLIP_FAR_PAD).max(max_corner_d + ROOM_CAMERA_CLIP_FAR_PAD);
    if !far.is_finite() {
        far = t_exit + ROOM_CAMERA_CLIP_FAR_PAD;
    }
    far = far.max(near + 1000.0);
    far = far.min(near * ROOM_CAMERA_CLIP_MAX_RATIO);

    cam.clip_near = Some(near);
    cam.clip_far = Some(far);
    cam
}

/// World-space AABB corners after centering and scale (for FOV fitting).
pub fn room_world_bounds_corners_centered(
    bounds_doc: RoomEnvironmentBounds,
    window_h: f32,
    env_height_scale: f32,
) -> Vec<Vec3> {
    let s = room_env_world_scale(window_h, env_height_scale);
    let c = bounds_doc.center();
    bounds_doc.corners().iter().map(|p| (*p - c) * s).collect()
}

/// Widen vertical FOV (only upward) so corners **in front of** the camera project inside `±margin_ndc`.
pub fn room_camera_fit_fovy_for_corners(
    window_w: f32,
    window_h: f32,
    mut cam: CameraParams,
    corners_world: &[Vec3],
    margin_ndc: f32,
) -> CameraParams {
    if corners_world.is_empty() {
        return cam;
    }
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    let forward = (target - eye).normalize_or_zero();
    if forward.length_squared() < 1e-12 {
        return cam;
    }
    let test_pts: Vec<Vec3> = corners_world
        .iter()
        .copied()
        .filter(|p| (*p - eye).dot(forward) > 0.25)
        .collect();
    if test_pts.len() < 4 {
        return cam;
    }

    let h = window_h.max(1e-6);
    let aspect = window_w / h;
    let (near_p, far_p) = cam.clip_planes(h);

    let projects_ok = |fovy_deg: f32| -> bool {
        let fov_y = fovy_deg.to_radians();
        let up = Vec3::from_array(cam.up);
        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(fov_y, aspect, near_p, far_p);
        let vp = proj * view;
        for p in &test_pts {
            let clip = vp * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 0.01 {
                return false;
            }
            let inv_w = 1.0 / clip.w;
            let nx = clip.x * inv_w;
            let ny = clip.y * inv_w;
            if nx.abs() > margin_ndc || ny.abs() > margin_ndc {
                return false;
            }
        }
        true
    };

    if projects_ok(cam.fovy_deg) {
        return cam;
    }

    let mut lo = cam.fovy_deg;
    let mut hi = 170.0_f32;
    if !projects_ok(hi) {
        return cam;
    }

    for _ in 0..24 {
        if hi - lo < 0.05 {
            break;
        }
        let mid = (lo + hi) * 0.5;
        if projects_ok(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    cam.fovy_deg = hi;
    cam
}

pub fn merge_marker_mesh_bounds(
    map: &mut FxHashMap<String, RoomEnvironmentBounds>,
    node_name: &str,
    prim: &RoomEnvPrimitiveCpu,
) {
    let mut min_v = Vec3::splat(f32::INFINITY);
    let mut max_v = Vec3::splat(f32::NEG_INFINITY);
    for vtx in &prim.mesh.vertices {
        let p = Vec3::from(vtx.position);
        min_v = min_v.min(p);
        max_v = max_v.max(p);
    }
    if !min_v.x.is_finite() {
        return;
    }
    use std::collections::hash_map::Entry;
    match map.entry(node_name.to_string()) {
        Entry::Vacant(e) => {
            e.insert(RoomEnvironmentBounds {
                min: min_v,
                max: max_v,
            });
        }
        Entry::Occupied(mut e) => {
            let b = e.get_mut();
            b.min = b.min.min(min_v);
            b.max = b.max.max(max_v);
        }
    }
}

pub fn decode_collision_triangles(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[Vec<u8>],
) -> anyhow::Result<Vec<[Vec3; 3]>> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .context("collision primitive has no POSITION")?
        .collect();
    let indices: Vec<u32> = if let Some(ids) = reader.read_indices() {
        ids.into_u32().collect()
    } else {
        (0..positions.len() as u32).collect()
    };
    let mut out = Vec::with_capacity(indices.len() / 3);
    for tri in indices.chunks_exact(3) {
        let p0 = node_world.transform_point3(Vec3::from(positions[tri[0] as usize]));
        let p1 = node_world.transform_point3(Vec3::from(positions[tri[1] as usize]));
        let p2 = node_world.transform_point3(Vec3::from(positions[tri[2] as usize]));
        out.push([p0, p1, p2]);
    }
    Ok(out)
}

pub fn room_embedded_camera_from_node(
    world: Mat4,
    cam: gltf::Camera<'_>,
) -> Option<RoomGltfEmbeddedCamera> {
    let gltf::camera::Projection::Perspective(p) = cam.projection() else {
        return None;
    };
    let fovy_deg = p.yfov().to_degrees();
    let z_axis = world.z_axis.truncate();
    let y_axis = world.y_axis.truncate();
    let eye = world.w_axis.truncate();
    let z_len = z_axis.length();
    let y_len = y_axis.length();
    if !(z_len > 1e-20 && y_len > 1e-20) {
        return None;
    }
    let forward = (-z_axis / z_len).normalize();
    let up = (y_axis / y_len).normalize();
    let target = eye + forward;
    Some(RoomGltfEmbeddedCamera {
        eye,
        target,
        up,
        fovy_deg,
    })
}

fn apply_normal_scale_rgba8(pixels: &mut [u8], scale: f32) {
    if (scale - 1.0).abs() < 1e-5 {
        return;
    }
    for chunk in pixels.chunks_exact_mut(4) {
        let x = chunk[0] as f32 / 255.0 * 2.0 - 1.0;
        let y = chunk[1] as f32 / 255.0 * 2.0 - 1.0;
        let z = chunk[2] as f32 / 255.0 * 2.0 - 1.0;
        let nx = x * scale;
        let ny = y * scale;
        let nz = z;
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-8);
        let nx = (nx / len) * 0.5 + 0.5;
        let ny = (ny / len) * 0.5 + 0.5;
        let nz = (nz / len) * 0.5 + 0.5;
        chunk[0] = (nx.clamp(0.0, 1.0) * 255.0).round() as u8;
        chunk[1] = (ny.clamp(0.0, 1.0) * 255.0).round() as u8;
        chunk[2] = (nz.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

pub fn decode_env_primitive(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[Vec<u8>],
    capped_images: &[Option<CappedGltfImage>],
    log_asset_label: &str,
    gltf_node_name: &str,
) -> anyhow::Result<RoomEnvPrimitiveCpu> {
    let normal_xform = node_world.inverse().transpose();
    let material = primitive.material();
    let material_name = material.name();
    let is_candle_wax = is_shop_candle_wax_mesh(gltf_node_name, material_name);
    let sampler_cpu = sampler_cpu_from_material(&material);

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
    let base_tex_coord = pbr.base_color_texture().map(|t| t.tex_coord()).unwrap_or(0);
    let mr_tex_coord = pbr
        .metallic_roughness_texture()
        .map(|t| t.tex_coord())
        .unwrap_or(base_tex_coord);
    let normal_tex_coord = material
        .normal_texture()
        .map(|t| t.tex_coord())
        .unwrap_or(mr_tex_coord);

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

    // `room_glb.wgsl` samples MR / normal / emissive at `uv_emr` (see `VsOut.uv_emr`).
    let emr_tex_coord = if normal_tex_coord == mr_tex_coord {
        mr_tex_coord
    } else {
        log::warn!(
            "{log_asset_label} {gltf_node_name}: normal texCoord {normal_tex_coord} != MR {mr_tex_coord} — using MR set"
        );
        mr_tex_coord
    };

    let mut uv_emr: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(emr_tex_coord) {
        tc.into_f32().collect()
    } else {
        uvs.clone()
    };
    anyhow::ensure!(
        uv_emr.len() == positions_local.len(),
        "EMR TEXCOORD count does not match POSITION count"
    );
    if let Some(tex_info) = pbr.metallic_roughness_texture() {
        apply_texture_transform(&mut uv_emr, &tex_info);
    }

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

    if material.normal_texture().is_some() && normal_tex_coord != emr_tex_coord {
        tangents_local.clear();
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

    // Candle wax: `uv` = lightmap (`TEXCOORD_1`) for baked SSS; `uv_emr` stays on set 0 for defaults.
    if is_candle_wax {
        if let Some(tc1) = reader.read_tex_coords(1) {
            let uv1: Vec<[f32; 2]> = tc1.into_f32().collect();
            if uv1.len() == positions_local.len() {
                uvs = uv1;
            } else {
                log::warn!(
                    "{log_asset_label} {gltf_node_name}: TEXCOORD_1 count mismatch for candle SSS"
                );
            }
        }
        uv_emr = if let Some(tc0) = reader.read_tex_coords(0) {
            tc0.into_f32().collect()
        } else {
            vec![[0.0, 0.0]; positions_local.len()]
        };
    }

    let colors: Vec<[f32; 4]> = if let Some(iter) = reader.read_colors(0) {
        iter.into_rgba_f32().collect()
    } else {
        Vec::new()
    };

    // Archive `sign_description_*` boards: pack the decal sampler onto the mesh's UV bounding
    // box so the description maps exactly once across whatever UV layout the asset ships with
    // (Repeat sampler + UVs > 1 would otherwise tile the text — see screenshot in PR thread).
    let is_archive_sign = matches!(
        gltf_node_name,
        "sign_description_left" | "sign_description_right"
    );
    let uv_remap = if is_archive_sign {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for uv in &uvs {
            min = min.min(Vec2::from_array(*uv));
            max = max.max(Vec2::from_array(*uv));
        }
        let span = max - min;
        let inv = Vec2::new(
            if span.x.abs() > 1e-6 {
                1.0 / span.x
            } else {
                0.0
            },
            if span.y.abs() > 1e-6 {
                1.0 / span.y
            } else {
                0.0
            },
        );
        Some((min, inv))
    } else {
        None
    };

    let mut vertices: Vec<Vertex3dTex> = (0..positions_local.len())
        .map(|i| {
            let p = node_world.transform_point3(Vec3::from(positions_local[i]));
            let n = normal_xform
                .transform_vector3(Vec3::from(normals_local[i]))
                .normalize_or_zero();
            let tl = tangents_local[i];
            let t_loc = Vec3::new(tl[0], tl[1], tl[2]);
            let w = tl[3];
            let t_w = node_world.transform_vector3(t_loc).normalize_or_zero();
            let color = colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let uv = if let Some((min, inv)) = uv_remap {
                let u = Vec2::from_array(uvs[i]);
                let n = (u - min) * inv;
                [n.x, n.y]
            } else {
                uvs[i]
            };
            Vertex3dTex {
                position: p.into(),
                normal: n.into(),
                uv,
                tangent: [t_w.x, t_w.y, t_w.z, w],
                uv_emr: uv_emr[i],
                color,
            }
        })
        .collect();
    // `room_glb.wgsl` composites `@binding(3)` decal_tex when `COLOR_0.a` matches tags below.
    if is_archive_sign {
        for v in &mut vertices {
            v.color[3] = ROOM_ENV_COLOR_A_ARCHIVE_DECAL;
        }
    } else if is_candle_wax {
        for v in &mut vertices {
            v.color[3] = ROOM_ENV_COLOR_A_CANDLE_SSS_BAKE;
        }
    }
    let factor = pbr.base_color_factor();

    let albedo_src = pbr.base_color_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let mut albedo_rgba = albedo_src.map(capped_image_base);
    let albedo_mip_chain = albedo_src.map(|c| Arc::clone(&c.mip_chain));

    if albedo_rgba.is_none() && pbr.base_color_texture().is_some() {
        log::warn!(
            "{log_asset_label} primitive {}: base color texture present but image decode failed",
            primitive.index()
        );
    }

    match &mut albedo_rgba {
        Some((pix, _, _)) => multiply_rgba8_by_factor(pix, &factor),
        None => {
            let want_fallback_tex = is_candle_wax
                || factor != [1.0, 1.0, 1.0, 1.0]
                || pbr.base_color_texture().is_some();
            if want_fallback_tex {
                let wax_factor = if is_candle_wax {
                    [0.94, 0.86, 0.62, 1.0]
                } else {
                    factor
                };
                albedo_rgba = Some(solid_albedo_rgba8(&wax_factor));
            }
        }
    }

    let normal_src = material.normal_texture().and_then(|nt| {
        let img_index = nt.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let normal_rgba = normal_src.map(|img| {
        let mut tex = capped_image_base(img);
        if let Some(nt) = material.normal_texture() {
            apply_normal_scale_rgba8(&mut tex.0, nt.scale());
        }
        tex
    });
    let normal_mip_chain = normal_src.and_then(|c| {
        let scale = material.normal_texture().map(|nt| nt.scale()).unwrap_or(1.0);
        (scale == 1.0).then(|| Arc::clone(&c.mip_chain))
    });

    if normal_rgba.is_none() && material.normal_texture().is_some() {
        log::warn!(
            "{log_asset_label} primitive {}: normal texture present but image decode failed",
            primitive.index()
        );
    }

    let mr_src = pbr.metallic_roughness_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let metallic_roughness_rgba = mr_src.map(capped_image_base);
    let metallic_roughness_mip_chain = mr_src.map(|c| Arc::clone(&c.mip_chain));

    let emissive_src = material.emissive_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        capped_image_at(capped_images, img_index)
    });
    let emissive_rgba = emissive_src.map(capped_image_base);
    let emissive_mip_chain = emissive_src.map(|c| Arc::clone(&c.mip_chain));

    let alpha_mode = GltfAlphaMode::from(material.alpha_mode());
    let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);
    let (metallic_factor, roughness_factor) = if is_candle_wax {
        (0.0, 0.88)
    } else {
        (pbr.metallic_factor(), pbr.roughness_factor())
    };
    let emissive_factor = if is_candle_wax {
        [0.0, 0.0, 0.0]
    } else {
        crate::render::gltf_helpers::effective_gltf_emissive_rgb(&material)
    };

    Ok(RoomEnvPrimitiveCpu {
        gltf_node_name: if gltf_node_name.is_empty() {
            None
        } else {
            Some(gltf_node_name.to_string())
        },
        mesh: LoadedPrimitive {
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
            metallic_factor,
            roughness_factor,
            emissive_factor,
            alpha_mode,
            alpha_cutoff,
            double_sided: material.double_sided(),
            sampler: sampler_cpu,
        },
    })
}

pub fn harvest_khr_punctual_light(
    world: Mat4,
    light: gltf::khr_lights_punctual::Light<'_>,
    node_name: &str,
    candle_node_prefix: &str,
    lantern_node_prefix: &str,
    log_asset_label: &str,
    points: &mut Vec<RoomGltfEmbeddedPointLight>,
    spots: &mut Vec<RoomGltfEmbeddedSpotLight>,
) {
    use gltf::khr_lights_punctual::Kind;

    let color_linear = light.color();
    let is_candle = node_name.starts_with(candle_node_prefix);
    let is_lantern = node_name.starts_with(lantern_node_prefix);
    let intensity = light.intensity();
    let range_doc = light.range();

    match light.kind() {
        Kind::Point => {
            let pos_doc = world.transform_point3(Vec3::ZERO);
            points.push(RoomGltfEmbeddedPointLight {
                node_name: node_name.to_string(),
                pos_doc,
                color_linear,
                is_candle,
                is_lantern,
                intensity,
                range_doc,
            });
        }
        Kind::Spot {
            inner_cone_angle,
            outer_cone_angle,
        } => {
            let z_axis = world.z_axis.truncate();
            let z_len = z_axis.length();
            if z_len < 1e-20 {
                log::warn!(
                    "{log_asset_label}: spot light {:?} has degenerate orientation — skipping",
                    node_name
                );
                return;
            }
            let dir_doc = (-z_axis / z_len).normalize();
            let pos_doc = world.transform_point3(Vec3::ZERO);
            let outer_rad = outer_cone_angle.max(1e-4);
            let inner_rad = inner_cone_angle.min(outer_rad).max(0.0);
            spots.push(RoomGltfEmbeddedSpotLight {
                pos_doc,
                dir_doc,
                color_linear,
                is_candle,
                is_lantern,
                intensity,
                range_doc,
                inner_cone_rad: inner_rad,
                outer_cone_rad: outer_rad,
            });
        }
        Kind::Directional => {
            log::debug!(
                "{log_asset_label}: skipping directional light on node {:?}",
                node_name
            );
        }
    }
}

/// Boolean-operand meshes that stay in the glTF export but must not draw at runtime.
#[inline]
pub fn skip_room_env_authoring_mesh_node_name(name: &str) -> bool {
    name == "subtractor"
}

/// How mesh geometry under a node contributes to environment draw + picking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomMeshPolicy {
    /// Decode each primitive as a visible environment mesh.
    EnvironmentDraw,
    /// Skip draw; if the node is a marker, decode collision triangles only (invisible anchor volumes).
    SkipDrawCollisionIfMarker,
    /// Visible mesh plus collision tris + marker bounds union (diegetic buttons).
    EnvironmentDrawWithCollision,
    /// Invisible rain-impact shell (`rain_hit_*` nodes): collision tris only, no draw.
    RainSurfaceCollision,
}

/// Per-asset rules for [`walk_room_env_node`].
pub trait RoomEnvWalkHooks {
    fn is_marker(&self, name: &str) -> bool;
    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy;
    fn log_asset_label(&self) -> &'static str;
    /// Authoring-only instances (e.g. scattered foliage cards) that must not draw or affect bounds.
    fn skip_env_mesh(&self, _name: &str) -> bool {
        false
    }
}

/// Mutable harvest targets for [`walk_room_env_node`].
pub struct RoomEnvWalkState<'a> {
    pub candle_node_prefix: &'a str,
    pub lantern_node_prefix: &'a str,
    pub markers: &'a mut FxHashMap<String, Mat4>,
    pub env_primitives: &'a mut Vec<RoomEnvPrimitiveCpu>,
    pub marker_mesh_bounds_doc: &'a mut FxHashMap<String, RoomEnvironmentBounds>,
    pub collision_meshes: &'a mut Vec<RoomCollisionMesh>,
    pub rain_surface_meshes: &'a mut Vec<RoomCollisionMesh>,
    pub embedded_cameras: &'a mut EmbeddedCameraHarvest,
    pub embedded_point_lights: &'a mut Vec<RoomGltfEmbeddedPointLight>,
    pub embedded_spot_lights: &'a mut Vec<RoomGltfEmbeddedSpotLight>,
    pub buffers: &'a [Vec<u8>],
    pub capped_images: &'a [Option<CappedGltfImage>],
}

pub fn walk_room_env_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    hooks: &impl RoomEnvWalkHooks,
    state: &mut RoomEnvWalkState<'_>,
) -> anyhow::Result<()> {
    let label = hooks.log_asset_label();
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    let name = node.name().unwrap_or("");

    if let Some(light) = node.light() {
        harvest_khr_punctual_light(
            world,
            light,
            name,
            state.candle_node_prefix,
            state.lantern_node_prefix,
            label,
            state.embedded_point_lights,
            state.embedded_spot_lights,
        );
    }

    if let Some(cam) = node.camera() {
        match cam.projection() {
            gltf::camera::Projection::Perspective(_) => {
                if let Some(ec) = room_embedded_camera_from_node(world, cam) {
                    state.embedded_cameras.insert(name, ec);
                }
            }
            gltf::camera::Projection::Orthographic(_) => {
                log::debug!("{label}: skipping orthographic camera on node {:?}", name);
            }
        }
    }

    if hooks.is_marker(name) && state.markers.insert(name.to_string(), world).is_some() {
        log::warn!(
            "{label}: duplicate marker node name {:?} — using last transform",
            name
        );
    }

    if let Some(mesh) = node.mesh() {
        if !skip_room_env_authoring_mesh_node_name(name) && !hooks.skip_env_mesh(name) {
            match hooks.mesh_policy(name) {
                RoomMeshPolicy::SkipDrawCollisionIfMarker => {
                    if hooks.is_marker(name) {
                        let mut tris = Vec::new();
                        for prim in mesh.primitives() {
                            match decode_collision_triangles(prim, world, state.buffers) {
                                Ok(chunk) => tris.extend(chunk),
                                Err(e) => log::warn!("{label} node {:?} collision: {e:#}", name),
                            }
                        }
                        if !tris.is_empty() {
                            state.collision_meshes.push(RoomCollisionMesh {
                                node_name: name.to_string(),
                                triangles: tris,
                            });
                        }
                    }
                }
                RoomMeshPolicy::EnvironmentDrawWithCollision => {
                    let mut tris: Vec<[Vec3; 3]> = Vec::new();
                    for prim in mesh.primitives() {
                        match decode_collision_triangles(prim.clone(), world, state.buffers) {
                            Ok(chunk) => tris.extend(chunk),
                            Err(e) => log::warn!("{label} node {:?} collision: {e:#}", name),
                        }
                        let decoded = decode_env_primitive(
                            prim,
                            world,
                            state.buffers,
                            state.capped_images,
                            label,
                            name,
                        )?;
                        merge_marker_mesh_bounds(state.marker_mesh_bounds_doc, name, &decoded);
                        state.env_primitives.push(decoded);
                    }
                    if !tris.is_empty() {
                        state.collision_meshes.push(RoomCollisionMesh {
                            node_name: name.to_string(),
                            triangles: tris,
                        });
                    }
                }
                RoomMeshPolicy::EnvironmentDraw => {
                    for prim in mesh.primitives() {
                        let decoded = decode_env_primitive(
                            prim,
                            world,
                            state.buffers,
                            state.capped_images,
                            label,
                            name,
                        )?;
                        if hooks.is_marker(name) {
                            merge_marker_mesh_bounds(state.marker_mesh_bounds_doc, name, &decoded);
                        }
                        state.env_primitives.push(decoded);
                    }
                }
                RoomMeshPolicy::RainSurfaceCollision => {
                    let mut tris = Vec::new();
                    for prim in mesh.primitives() {
                        match decode_collision_triangles(prim, world, state.buffers) {
                            Ok(chunk) => tris.extend(chunk),
                            Err(e) => log::warn!("{label} node {:?} rain surface: {e:#}", name),
                        }
                    }
                    if !tris.is_empty() {
                        state.rain_surface_meshes.push(RoomCollisionMesh {
                            node_name: name.to_string(),
                            triangles: tris,
                        });
                    }
                }
            }
        }
    }

    for child in node.children() {
        walk_room_env_node(child, world, hooks, state)?;
    }
    Ok(())
}

/// Document-space marker origin minus environment AABB center (multiply by [`room_env_world_scale`]
/// for world space consistent with the centered room model matrix).
pub fn marker_translation_doc(
    markers: &FxHashMap<String, Mat4>,
    environment_bounds_doc: Option<RoomEnvironmentBounds>,
    name: &str,
) -> Option<Vec3> {
    let center_doc = environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    markers
        .get(name)
        .map(|m| m.transform_point3(Vec3::ZERO) - center_doc)
}
