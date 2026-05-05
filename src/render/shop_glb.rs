//! Load [`Shop.glb`](../../../assets/Shop.glb): named empties/meshes for UI anchors + merged environment geometry.
//!
//! Marker object names (Blender object names → glTF node names):
//! - `exit_btn`, `restock_btn`, `journal_btn`
//! - `shop_spawn_relic_00` … `shop_spawn_relic_08`
//! - `shop_player_relic_00` … `shop_player_relic_04`
//! - `shop_player_consumable_00`, `shop_player_consumable_01`
//!
//! **Spawn / inventory anchor** nodes (`shop_spawn_relic_*`, `shop_player_*`) may carry mesh
//! geometry that exists only for authoring (invisible hit volumes). That mesh is **skipped** at
//! decode time so it does not draw, but it is still decoded into **[`ShopCollisionMesh`]** triangle
//! soups for cursor ray picking (`pick_shop_object`). **Shop buttons** (`exit_btn`, `restock_btn`,
//! `journal_btn`) still record marker transforms **and** decode their meshes for drawing.
//!
//! ## Materials
//! Each primitive uses glTF PBR **base color texture** (if present) and multiplies by
//! **`baseColorFactor`** on the CPU. Factor-only materials become a 1×1 uploaded texture.
//! **Normal maps** (`material.normalTexture`) are decoded as linear RGBA; **`scale`** is baked
//! into texels. Tangents come from the glTF `TANGENT` attribute when present, otherwise from
//! [`crate::render::tile_glb::compute_vertex_tangents`] using the normal map TEXCOORD when it
//! differs from base color. Metallic–roughness, emissive, alpha modes, `COLOR_0`, and glTF sampler
//! settings follow [`crate::render::tile_glb::LoadedPrimitive`] (shared with `Tile.glb`).
//!
//! ## Export (Blender / glTF)
//! Ship **`Shop.glb` without Draco** (`KHR_draco_mesh_compression`). This crate uses
//! [`gltf::import_slice`](https://docs.rs/gltf), which does not decode Draco — compressed files fail
//! validation (`accessor.bufferView: Missing data`, unsupported extension).
//!
//! ## Scale
//! The GPU applies `model = UniformScale(window_h * height_scale)` each frame so the room tracks
//! resolution like the shop camera (`eye` / `target` use `h *` fractions). Marker positions use the
//! same factor. Default multiplier is [`SHOP_ENV_HEIGHT_SCALE`]; Debug → Tuning → **Shop Env & Lighting…**
//! overrides height scale and [`ShopEnvLightingTune`] fields live (typical height range `0.001`–`2.0`).
//!
//! ## Optional perspective camera
//! If the default scene contains a **perspective** camera node, the shop uses it for
//! [`crate::render::draw_cmd::CameraParams`] (eye / target / up / vertical FOV). Transforms are read
//! in glTF camera convention (−Z forward, +Y up); positions are scaled by [`shop_env_world_scale`]
//! like marker geometry. If multiple cameras exist, a node named `ShopCamera`, `shop_camera`, or
//! `Camera` wins; otherwise the first perspective camera in depth-first order is used. Orthographic
//! cameras are ignored (hardcoded fallback camera applies).
//!
//! ## `KHR_lights_punctual`
//! **Point** and **spot** lights on scene nodes drive shop lighting when present: hardcoded lamp +
//! fill point lights are omitted so only glTF punctual lights apply (hover highlights may still add
//! extras). **Directional** lights are skipped. With embedded lights, the room draws through
//! `shop_glb.wgsl`: inverse-square attenuation (Khronos range window),
//! metallic–roughness, ACES, and [`SHOP_ENV_LINEAR_EXPOSURE`] ([`SHOP_ENV_AMBIENT_SCALE`] is `0` for this interior).
//! Intensity is scaled by [`SHOP_GLTF_LIGHT_INTENSITY_SCALE`]. Shop punctual points use a separate
//! uniform buffer, bound as group 1 binding 0 for [`shop_glb.wgsl`] and binding 2 for [`lit_mesh.wgsl`]
//! (inverse-square on props; stays within WebGPU `max_bind_groups` on Metal).
//! Punctual lights on nodes whose names
//! start with [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`] get [`SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL`]; all
//! other lights keep glTF-authored color. `range` maps to glTF max distance (`0` = infinite).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::render::draw_cmd::CameraParams;
use crate::render::gltf_helpers::{apply_texture_transform, sampler_cpu_from_material};
use crate::render::tile_glb::{
    GltfAlphaMode, LoadedPrimitive, Vertex3dTex, compute_vertex_tangents, gltf_image_to_rgba8,
    multiply_rgba8_by_factor, solid_albedo_rgba8,
};
use anyhow::Context;
use glam::{Mat4, Vec3};

static SHOP_GLB_CPU: OnceLock<Option<ShopGlbCpu>> = OnceLock::new();

/// Default height multiplier for [`shop_env_world_scale`] when no debug override is active.
pub const SHOP_ENV_HEIGHT_SCALE: f32 = 1.0;

/// Multiplies glTF punctual **intensity** (candela) before upload. `shop_glb.wgsl` uses physical-style
/// inverse-square falloff + tonemap; leave at `1.0` and tune [`SHOP_ENV_LINEAR_EXPOSURE`] first.
pub const SHOP_GLTF_LIGHT_INTENSITY_SCALE: f32 = 1.0;

/// Linear HDR exposure before ACES in `shop_glb.wgsl` (stored in `CameraUniform.tile_seed` for shop draws).
pub const SHOP_ENV_LINEAR_EXPOSURE: f32 = 1.0;

/// Scales hemispheric ambient fill in `shop_glb.wgsl` (`CameraUniform.decal_atlas_uv.x`). `0` for a
/// punctual-only interior (no fake ambient).
pub const SHOP_ENV_AMBIENT_SCALE: f32 = 0.0;

/// Applied in `lit_mesh.wgsl` as `shop_gltf_point_lights.extras.w` when
/// [`crate::render::draw_cmd::UiFrame::shop_env_gltf_punctual`] is set (`shop_glb.wgsl` ignores it).
/// Defaults to `1` so embedded punctual lights match the room; debug tuning may lower it.
pub const SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE: f32 = 1.0;

/// glTF **node** name prefix for punctual lights that should read as warm candles (`light_candle_00`, …).
pub const SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX: &str = "light_candle_";

/// Linear RGB multiplier for punctual lights on nodes matching [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`].
/// Other lights keep glTF linear RGB from the file (typically white for new fill lights).
pub const SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL: [f32; 3] = [1.0, 0.91, 0.74];

/// Runtime shop lighting matching the `SHOP_*` source constants. Carried on [`DrawCtx`](crate::scenes::DrawCtx)
/// and editable from the debug overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShopEnvLightingTune {
    pub gltf_light_intensity_scale: f32,
    pub linear_exposure: f32,
    pub ambient_scale: f32,
    pub lit_mesh_gltf_punctual_scale: f32,
    pub candle_light_color_mul: [f32; 3],
}

impl Default for ShopEnvLightingTune {
    fn default() -> Self {
        Self::SOURCE_DEFAULTS
    }
}

impl ShopEnvLightingTune {
    pub const SOURCE_DEFAULTS: Self = Self {
        gltf_light_intensity_scale: SHOP_GLTF_LIGHT_INTENSITY_SCALE,
        linear_exposure: SHOP_ENV_LINEAR_EXPOSURE,
        ambient_scale: SHOP_ENV_AMBIENT_SCALE,
        lit_mesh_gltf_punctual_scale: SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE,
        candle_light_color_mul: SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL,
    };
}

/// [`KHR_lights_punctual`] point light — positions in **document units** (same as mesh).
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedPointLight {
    pub pos_doc: Vec3,
    /// Linear RGB from glTF before candle tint (see [`ShopGlbEmbeddedPointLight::is_candle`]).
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
}

/// [`KHR_lights_punctual`] spot light — cone aims along node **−Z** in document space.
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedSpotLight {
    pub pos_doc: Vec3,
    /// Unit vector from light toward illuminated surfaces (world/doc −Z axis).
    pub dir_doc: Vec3,
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
    pub inner_cone_rad: f32,
    pub outer_cone_rad: f32,
}

/// Pick ids for Leave / Reroll props — must stay aligned with [`crate::scenes::shop`] (`PICK_*`).
pub const SHOP_GLTF_PICK_LEAVE_PROP: u32 = 6;
pub const SHOP_GLTF_PICK_REROLL_PROP: u32 = 7;

#[inline]
pub fn shop_env_world_scale(window_h: f32, height_scale: f32) -> f32 {
    window_h.max(1e-6) * height_scale
}

/// One environment mesh primitive plus embedded glTF sampler parameters for GPU samplers.
pub struct ShopEnvPrimitiveCpu {
    pub mesh: LoadedPrimitive,
}

/// CPU triangle soup for one named GLB node (typically invisible anchor geometry). Vertices are
/// in the same pre-GPU-scale space as uploaded shop environment meshes (node transform applied).
#[derive(Clone)]
pub struct ShopCollisionMesh {
    pub node_name: String,
    pub triangles: Vec<[Vec3; 3]>,
}

/// Perspective camera baked into `Shop.glb` (positions in **document units**, same as mesh verts).
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fovy_deg: f32,
}

impl ShopGlbEmbeddedCamera {
    pub fn to_camera_params(&self, window_h: f32, env_height_scale: f32) -> CameraParams {
        let s = shop_env_world_scale(window_h, env_height_scale);
        let up = self.up.normalize_or_zero();
        let up = if up.length_squared() > 1e-12 {
            up
        } else {
            Vec3::Z
        };
        CameraParams {
            eye: (self.eye * s).to_array(),
            target: (self.target * s).to_array(),
            up: up.to_array(),
            fovy_deg: self.fovy_deg,
        }
    }
}

#[derive(Default)]
struct EmbeddedCameraHarvest {
    named: Option<ShopGlbEmbeddedCamera>,
    fallback: Option<ShopGlbEmbeddedCamera>,
}

impl EmbeddedCameraHarvest {
    fn pick(self) -> Option<ShopGlbEmbeddedCamera> {
        self.named.or(self.fallback)
    }

    fn insert(&mut self, name: &str, cam: ShopGlbEmbeddedCamera) {
        let key = name.to_ascii_lowercase();
        let preferred = matches!(key.as_str(), "camera" | "shopcamera" | "shop_camera");
        if preferred {
            if self.named.replace(cam).is_some() {
                log::warn!("Shop.glb: multiple preferred camera node names — using last");
            }
        } else if self.fallback.is_none() {
            self.fallback = Some(cam);
        }
    }
}

pub struct ShopGlbCpu {
    pub markers: HashMap<String, Mat4>,
    pub environment_primitives: Vec<ShopEnvPrimitiveCpu>,
    /// Trimesh colliders for skipped-draw marker meshes (`shop_spawn_*`, `shop_player_*`).
    pub collision_meshes: Vec<ShopCollisionMesh>,
    /// First eligible perspective camera from the default scene, if any.
    pub embedded_perspective_camera: Option<ShopGlbEmbeddedCamera>,
    pub embedded_point_lights: Vec<ShopGlbEmbeddedPointLight>,
    pub embedded_spot_lights: Vec<ShopGlbEmbeddedSpotLight>,
}

pub fn shop_glb_cpu() -> Option<&'static ShopGlbCpu> {
    SHOP_GLB_CPU
        .get_or_init(|| {
            let Some(file) = crate::asset_path::get("Shop.glb") else {
                log::debug!("Shop.glb not embedded; using PNG storeroom backdrop");
                return None;
            };
            match load_shop_glb_from_bytes(&file.data) {
                Ok(cpu) => {
                    log::info!(
                        "Shop.glb: {} marker node(s), {} draw primitive(s), {} collision mesh(es)",
                        cpu.markers.len(),
                        cpu.environment_primitives.len(),
                        cpu.collision_meshes.len(),
                    );
                    if cpu.embedded_perspective_camera.is_some()
                        || !cpu.embedded_point_lights.is_empty()
                        || !cpu.embedded_spot_lights.is_empty()
                    {
                        log::info!(
                            "Shop.glb scene extras: perspective_camera={} point_lights={} spot_lights={}",
                            cpu.embedded_perspective_camera.is_some(),
                            cpu.embedded_point_lights.len(),
                            cpu.embedded_spot_lights.len(),
                        );
                    }
                    Some(cpu)
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    log::warn!("Shop.glb failed to load: {msg}");
                    if msg.contains("KHR_draco_mesh_compression") {
                        log::warn!(
                            "Re-export Shop.glb with Draco compression disabled (Blender glTF: turn off mesh compression / Draco)."
                        );
                    }
                    None
                }
            }
        })
        .as_ref()
}

#[inline]
pub fn spawn_relic_marker_name(slot: usize) -> String {
    format!("shop_spawn_relic_{slot:02}")
}

#[inline]
pub fn player_relic_marker_name(slot: usize) -> String {
    format!("shop_player_relic_{slot:02}")
}

#[inline]
pub fn player_consumable_marker_name(slot: usize) -> String {
    format!("shop_player_consumable_{slot:02}")
}

fn is_marker_name(name: &str) -> bool {
    matches!(name, "exit_btn" | "restock_btn" | "journal_btn" | "Dish")
        || name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

/// Environment draw skip: anchor nodes often have collision/helper meshes that should not render.
/// Button markers are excluded — their mesh is a visible control and may bind focus UI.
fn skip_shop_env_mesh_for_node_name(name: &str) -> bool {
    name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

fn decode_collision_triangles(
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

fn shop_embedded_camera_from_node(world: Mat4, cam: gltf::Camera<'_>) -> Option<ShopGlbEmbeddedCamera> {
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
    Some(ShopGlbEmbeddedCamera {
        eye,
        target,
        up,
        fovy_deg,
    })
}

/// Bake glTF `normalTexture.scale` into linear-ish RGBA normal texels.
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

fn decode_primitive(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[Vec<u8>],
    images: &[gltf::image::Data],
) -> anyhow::Result<ShopEnvPrimitiveCpu> {
    let normal_xform = node_world.inverse().transpose();
    let material = primitive.material();
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
            let color = colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
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

    let mut albedo_rgba = pbr.base_color_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    if albedo_rgba.is_none() && pbr.base_color_texture().is_some() {
        log::warn!(
            "Shop.glb primitive {}: base color texture present but image decode failed",
            primitive.index()
        );
    }

    match &mut albedo_rgba {
        Some((pix, _, _)) => multiply_rgba8_by_factor(pix, &factor),
        None => {
            let want_fallback_tex =
                factor != [1.0, 1.0, 1.0, 1.0] || pbr.base_color_texture().is_some();
            if want_fallback_tex {
                albedo_rgba = Some(solid_albedo_rgba8(&factor));
            }
        }
    }

    let normal_rgba = material.normal_texture().and_then(|nt| {
        let scale = nt.scale();
        let img_index = nt.texture().source().index();
        images
            .get(img_index)
            .and_then(gltf_image_to_rgba8)
            .map(|mut tex| {
                apply_normal_scale_rgba8(&mut tex.0, scale);
                tex
            })
    });

    if normal_rgba.is_none() && material.normal_texture().is_some() {
        log::warn!(
            "Shop.glb primitive {}: normal texture present but image decode failed",
            primitive.index()
        );
    }

    let metallic_roughness_rgba = pbr.metallic_roughness_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    let emissive_rgba = material.emissive_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    let alpha_mode = GltfAlphaMode::from(material.alpha_mode());
    let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);

    Ok(ShopEnvPrimitiveCpu {
        mesh: LoadedPrimitive {
            vertices,
            indices,
            albedo_rgba,
            normal_rgba,
            metallic_roughness_rgba,
            emissive_rgba,
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            emissive_factor: material.emissive_factor(),
            alpha_mode,
            alpha_cutoff,
            double_sided: material.double_sided(),
            sampler: sampler_cpu,
        },
    })
}

fn harvest_khr_light(
    world: Mat4,
    light: gltf::khr_lights_punctual::Light<'_>,
    node_name: &str,
    points: &mut Vec<ShopGlbEmbeddedPointLight>,
    spots: &mut Vec<ShopGlbEmbeddedSpotLight>,
) {
    use gltf::khr_lights_punctual::Kind;

    let color_linear = light.color();
    let is_candle = node_name.starts_with(SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX);
    let intensity = light.intensity();
    let range_doc = light.range();

    match light.kind() {
        Kind::Point => {
            let pos_doc = world.transform_point3(Vec3::ZERO);
            points.push(ShopGlbEmbeddedPointLight {
                pos_doc,
                color_linear,
                is_candle,
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
                    "Shop.glb: spot light {:?} has degenerate orientation — skipping",
                    node_name
                );
                return;
            }
            let dir_doc = (-z_axis / z_len).normalize();
            let pos_doc = world.transform_point3(Vec3::ZERO);
            let outer_rad = outer_cone_angle.max(1e-4);
            let inner_rad = inner_cone_angle.min(outer_rad).max(0.0);
            spots.push(ShopGlbEmbeddedSpotLight {
                pos_doc,
                dir_doc,
                color_linear,
                is_candle,
                intensity,
                range_doc,
                inner_cone_rad: inner_rad,
                outer_cone_rad: outer_rad,
            });
        }
        Kind::Directional => {
            log::debug!(
                "Shop.glb: skipping directional light on node {:?}",
                node_name
            );
        }
    }
}

fn walk_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    markers: &mut HashMap<String, Mat4>,
    env_primitives: &mut Vec<ShopEnvPrimitiveCpu>,
    collision_meshes: &mut Vec<ShopCollisionMesh>,
    embedded_cameras: &mut EmbeddedCameraHarvest,
    embedded_point_lights: &mut Vec<ShopGlbEmbeddedPointLight>,
    embedded_spot_lights: &mut Vec<ShopGlbEmbeddedSpotLight>,
    buffers: &[Vec<u8>],
    images: &[gltf::image::Data],
) -> anyhow::Result<()> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    let name = node.name().unwrap_or("");

    if let Some(light) = node.light() {
        harvest_khr_light(
            world,
            light,
            name,
            embedded_point_lights,
            embedded_spot_lights,
        );
    }

    if let Some(cam) = node.camera() {
        match cam.projection() {
            gltf::camera::Projection::Perspective(_) => {
                if let Some(ec) = shop_embedded_camera_from_node(world, cam) {
                    embedded_cameras.insert(name, ec);
                }
            }
            gltf::camera::Projection::Orthographic(_) => {
                log::debug!(
                    "Shop.glb: skipping orthographic camera on node {:?}",
                    name
                );
            }
        }
    }

    if is_marker_name(name) {
        if markers.insert(name.to_string(), world).is_some() {
            log::warn!(
                "Shop.glb: duplicate marker node name {:?} — using last transform",
                name
            );
        }
    }

    if let Some(mesh) = node.mesh() {
        if skip_shop_env_mesh_for_node_name(name) {
            if is_marker_name(name) {
                let mut tris = Vec::new();
                for prim in mesh.primitives() {
                    match decode_collision_triangles(prim, world, buffers) {
                        Ok(chunk) => tris.extend(chunk),
                        Err(e) => log::warn!("Shop.glb node {:?} collision: {e:#}", name),
                    }
                }
                if !tris.is_empty() {
                    collision_meshes.push(ShopCollisionMesh {
                        node_name: name.to_string(),
                        triangles: tris,
                    });
                }
            }
        } else {
            for prim in mesh.primitives() {
                env_primitives.push(decode_primitive(prim, world, buffers, images)?);
            }
        }
    }

    for child in node.children() {
        walk_node(
            child,
            world,
            markers,
            env_primitives,
            collision_meshes,
            embedded_cameras,
            embedded_point_lights,
            embedded_spot_lights,
            buffers,
            images,
        )?;
    }
    Ok(())
}

pub fn load_shop_glb_from_bytes(data: &[u8]) -> anyhow::Result<ShopGlbCpu> {
    let (document, buffers_vec, images) =
        gltf::import_slice(data).context("gltf::import_slice(Shop.glb)")?;

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context("Shop.glb has no scenes")?;

    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();

    let mut markers = HashMap::new();
    let mut environment_primitives = Vec::new();
    let mut collision_meshes = Vec::new();
    let mut embedded_cameras = EmbeddedCameraHarvest::default();
    let mut embedded_point_lights = Vec::new();
    let mut embedded_spot_lights = Vec::new();

    for node in scene.nodes() {
        walk_node(
            node,
            Mat4::IDENTITY,
            &mut markers,
            &mut environment_primitives,
            &mut collision_meshes,
            &mut embedded_cameras,
            &mut embedded_point_lights,
            &mut embedded_spot_lights,
            &buffers,
            &images,
        )?;
    }

    let embedded_perspective_camera = embedded_cameras.pick();

    Ok(ShopGlbCpu {
        markers,
        environment_primitives,
        collision_meshes,
        embedded_perspective_camera,
        embedded_point_lights,
        embedded_spot_lights,
    })
}

/// Shop camera from embedded GLB perspective camera, scaled like marker geometry.
#[inline]
pub fn shop_camera_from_glb_if_present(window_h: f32, env_height_scale: f32) -> Option<CameraParams> {
    shop_glb_cpu()?.embedded_perspective_camera.map(|c| {
        c.to_camera_params(window_h, env_height_scale)
    })
}

/// Best-effort marker translation for layout (world space, Z-up game frame).
pub fn marker_translation(cpu: &ShopGlbCpu, name: &str) -> Option<Vec3> {
    cpu.markers
        .get(name)
        .map(|m| m.transform_point3(Vec3::ZERO))
}
