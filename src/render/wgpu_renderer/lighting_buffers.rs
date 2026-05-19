use crate::render::draw_cmd::ScenePunctualLight;
use crate::render::world_space::pixel_to_world;

use super::constants::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS, MAX_TILE_OCCLUDERS};

/// `PointLightGpu.params.x` — must match WGSL `lights.lights[i].params.x`.
pub(crate) const SCENE_POINT_KIND_SMOOTH: f32 = 0.0;
pub(crate) const SCENE_POINT_KIND_INVERSE_SQUARE: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileOccluderGpu {
    /// xyz = world-space AABB center, w = unused.
    pub center: [f32; 4],
    /// xyz = world-space AABB half-extents, w = unused.
    pub half_extents: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileOccludersBuf {
    /// `count.x` = number of active occluders; rest is std140 padding.
    pub count: [u32; 4],
    pub boxes: [TileOccluderGpu; MAX_TILE_OCCLUDERS],
}

impl TileOccludersBuf {
    pub(crate) fn empty() -> Self {
        Self {
            count: [0; 4],
            boxes: [TileOccluderGpu {
                center: [0.0; 4],
                half_extents: [0.0; 4],
            }; MAX_TILE_OCCLUDERS],
        }
    }

    /// World-space AABBs from imported room collision meshes for punctual ray tests in
    /// `room_glb.wgsl` (largest volumes first, capped at [`MAX_TILE_OCCLUDERS`]).
    pub(crate) fn from_room_collision_meshes(
        model: glam::Mat4,
        meshes: &[crate::render::room_env_gltf::RoomCollisionMesh],
    ) -> Self {
        let mut ranked: Vec<(f32, TileOccluderGpu)> = Vec::new();
        for mesh in meshes {
            if mesh.node_name.starts_with("light_") {
                continue;
            }
            let (center, half) = room_collision_mesh_world_aabb(mesh, model);
            if !half.is_finite() || half.max_element() < 1e-5 {
                continue;
            }
            let volume = half.x * half.y * half.z;
            ranked.push((
                volume,
                TileOccluderGpu {
                    center: [center.x, center.y, center.z, 0.0],
                    half_extents: [half.x, half.y, half.z, 0.0],
                },
            ));
        }
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut occ = Self::empty();
        let take = ranked.len().min(MAX_TILE_OCCLUDERS);
        for (i, (_, b)) in ranked.into_iter().take(take).enumerate() {
            occ.boxes[i] = b;
        }
        occ.count[0] = take as u32;
        occ
    }
}

#[inline]
fn room_collision_mesh_world_aabb(
    mesh: &crate::render::room_env_gltf::RoomCollisionMesh,
    model: glam::Mat4,
) -> (glam::Vec3, glam::Vec3) {
    let mut lo = glam::Vec3::splat(f32::INFINITY);
    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
    for tri in &mesh.triangles {
        for p in tri {
            let w = model.transform_point3(*p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
    }
    if !lo.is_finite() {
        return (glam::Vec3::ZERO, glam::Vec3::ZERO);
    }
    let half = (hi - lo) * 0.5 * 1.03;
    let center = (lo + hi) * 0.5;
    (center, half)
}

/// CPU-side description of a point light. Scenes add these via
/// [`crate::render::draw_cmd::SceneLighting`]; the renderer translates
/// them into [`PointLightGpu`] each frame.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// World-space position of the light. The first two components match the
    /// pixel-space coordinate system used for tile model matrices (with the
    /// usual `y → -y` flip the renderer applies); `z` lets candle wicks sit in
    /// front of the table plane so 3D meshes catch the light correctly.
    pub pos: [f32; 3],
    /// Falloff radius in pixels. Outside this distance the light contributes
    /// nothing.
    pub radius: f32,
    /// Linear-space RGB tint.
    pub color: [f32; 3],
    /// Brightness multiplier. >1.0 is fine — the tile shader is unclamped.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PointLightGpu {
    /// xyz = world-space position, w = smooth radius **or** inverse-square range.
    pub pos: [f32; 4],
    /// rgb = colour, a = intensity.
    pub color: [f32; 4],
    /// x = kind ([`SCENE_POINT_KIND_SMOOTH`] or [`SCENE_POINT_KIND_INVERSE_SQUARE`]).
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PointLightsBuf {
    /// `count.x` = number of active lights; rest is std140 padding.
    pub count: [u32; 4],
    /// Frame-wide extras shared with shaders that bind this buffer:
    /// `extras.x` = display gamma (used to gamma-correct 3D fragments
    /// that don't have access to the screen-space `Globals` uniform).
    /// `extras.y` = wall-clock time in seconds (used by `MaterialKind::Water`
    /// to scroll the river surface and animate foam crests).
    /// `extras.z` = candle flame height in world units (for shaders that
    /// key flame envelope size off the shared point-light buffer).
    /// `extras.w` = inverse-square intensity scale in `lit_mesh` when embedded GLB punctual is active.
    pub extras: [f32; 4],
    pub lights: [PointLightGpu; MAX_POINT_LIGHTS],
}

pub(crate) struct PunctualLightBakeParams<'a> {
    pub src: &'a [ScenePunctualLight],
    pub candle_count: u32,
    pub flame_height_world: f32,
    pub lit_mesh_punctual_intensity_scale: f32,
    pub screen_w: f32,
    pub screen_h: f32,
    pub gamma: f32,
    pub time: f32,
}

pub(crate) struct PunctualLightBakeShopCameraParams<'a> {
    pub bake: &'a PunctualLightBakeParams<'a>,
    pub cam: &'a crate::render::draw_cmd::CameraParams,
}

/// CPU-side description of a spotlight. A spotlight has a direction + cone
/// half-angle, so it pools light onto a specific surface region rather than
/// radiating omnidirectionally. Used to draw focused visual-highlight pools
/// on specific tiles (hint indicators). Scenes push these into
/// [`crate::render::draw_cmd::UiFrame::spot_lights`]; the renderer translates
/// them into [`SpotLightGpu`] each frame. Sampled by the tile pipeline and
/// `lit_mesh`.
#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    /// Pixel-space position (same convention as `PointLight`). `z` is the
    /// vertical lift above the felt (Z-up world).
    pub pos: [f32; 3],
    /// World-space direction the light points, FROM the light TOWARD the
    /// illuminated surface. Does not need to be normalized; the GPU side
    /// normalises. Typical use: `[0.0, 0.0, -1.0]` for straight-down.
    pub dir: [f32; 3],
    /// Falloff radius in pixels. Outside this distance the light contributes
    /// nothing.
    pub radius: f32,
    /// Cosine of the outer cone half-angle. Outside this angle, contribution
    /// drops to zero. `cos(30°) ≈ 0.866` for a 60°-wide cone.
    pub cos_outer: f32,
    /// Cosine of the inner cone half-angle. Inside this angle, contribution
    /// is full. Between inner and outer the factor smoothsteps. Must be
    /// greater than or equal to `cos_outer` (inner angle ≤ outer angle).
    pub cos_inner: f32,
    /// Linear-space RGB tint.
    pub color: [f32; 3],
    /// Brightness multiplier.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SpotLightGpu {
    /// xyz = world-space position, w = radius.
    pub pos: [f32; 4],
    /// xyz = world-space direction (normalized), w = cos_outer.
    pub dir: [f32; 4],
    /// rgb = colour, a = intensity.
    pub color: [f32; 4],
    /// x = cos_inner, y/z/w reserved.
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SpotLightsBuf {
    /// `count.x` = number of active spotlights; rest is std140 padding.
    pub count: [u32; 4],
    pub lights: [SpotLightGpu; MAX_SPOT_LIGHTS],
}

impl SpotLightsBuf {
    pub(crate) fn empty() -> Self {
        Self {
            count: [0; 4],
            lights: [SpotLightGpu {
                pos: [0.0; 4],
                dir: [0.0, 0.0, -1.0, 1.0],
                color: [0.0; 4],
                params: [1.0; 4],
            }; MAX_SPOT_LIGHTS],
        }
    }

    /// Build the std140 spotlight buffer. Positions are mapped from pixel
    /// space to world (Z-up) via `pixel_to_world`, or via
    /// [`crate::render::world_space::world_on_camera_ray_plane_z`] when `pos_cam`
    /// is set (perspective shop / pack celebration). Direction is taken as-is
    /// in world space (already Z-up) and normalised on the GPU side — we
    /// normalise here too to keep the uniform sane to inspect.
    pub(crate) fn from_lights(
        src: &[SpotLight],
        screen_w: f32,
        screen_h: f32,
        pos_cam: Option<&crate::render::draw_cmd::CameraParams>,
    ) -> Self {
        let mut lights = [SpotLightGpu {
            pos: [0.0; 4],
            dir: [0.0, 0.0, -1.0, 1.0],
            color: [0.0; 4],
            params: [1.0; 4],
        }; MAX_SPOT_LIGHTS];
        let n = src.len().min(MAX_SPOT_LIGHTS);
        for (i, l) in src.iter().take(n).enumerate() {
            let p = if let Some(cam) = pos_cam {
                crate::render::world_space::world_on_camera_ray_plane_z(
                    screen_w, screen_h, cam, l.pos[0], l.pos[1], l.pos[2],
                )
            } else {
                pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2])
            };
            let d = glam::Vec3::from(l.dir).normalize_or_zero();
            let d = if d.length_squared() < 0.5 {
                glam::Vec3::new(0.0, 0.0, -1.0)
            } else {
                d
            };
            lights[i] = SpotLightGpu {
                pos: [p.x, p.y, p.z, l.radius],
                dir: [d.x, d.y, d.z, l.cos_outer],
                color: [l.color[0], l.color[1], l.color[2], l.intensity],
                params: [l.cos_inner, 0.0, 0.0, 0.0],
            };
        }
        Self {
            count: [n as u32, 0, 0, 0],
            lights,
        }
    }
}

impl PointLightsBuf {
    fn push_scene_punctual_entry(
        lights: &mut [PointLightGpu; MAX_POINT_LIGHTS],
        i: usize,
        p: &PointLight,
        world_xyz: glam::Vec3,
        kind: f32,
    ) {
        lights[i] = PointLightGpu {
            pos: [world_xyz.x, world_xyz.y, world_xyz.z, p.radius],
            color: [p.color[0], p.color[1], p.color[2], p.intensity],
            params: [kind, 0.0, 0.0, 0.0],
        };
    }

    /// Unified punctual upload (smooth + inverse-square in one buffer).
    pub(crate) fn from_scene_punctual(p: &PunctualLightBakeParams<'_>) -> Self {
        let mut lights = [PointLightGpu {
            pos: [0.0; 4],
            color: [0.0; 4],
            params: [0.0; 4],
        }; MAX_POINT_LIGHTS];
        let n = p.src.len().min(MAX_POINT_LIGHTS);
        for (i, ent) in p.src.iter().take(n).enumerate() {
            match ent {
                ScenePunctualLight::Smooth(l) => {
                    let world =
                        pixel_to_world(p.screen_w, p.screen_h, l.pos[0], l.pos[1], l.pos[2]);
                    Self::push_scene_punctual_entry(
                        &mut lights,
                        i,
                        l,
                        world,
                        SCENE_POINT_KIND_SMOOTH,
                    );
                }
                ScenePunctualLight::InverseSquare(l) => {
                    let world =
                        pixel_to_world(p.screen_w, p.screen_h, l.pos[0], l.pos[1], l.pos[2]);
                    Self::push_scene_punctual_entry(
                        &mut lights,
                        i,
                        l,
                        world,
                        SCENE_POINT_KIND_INVERSE_SQUARE,
                    );
                }
            }
        }
        Self {
            count: [n as u32, p.candle_count.min(n as u32), 0, 0],
            extras: [
                p.gamma.max(0.01),
                p.time,
                p.flame_height_world,
                p.lit_mesh_punctual_intensity_scale,
            ],
            lights,
        }
    }

    /// Same as [`Self::from_scene_punctual`] but smooth lights use the shop camera ray /
    /// horizontal-plane hit; inverse-square lights keep `pixel_to_world` (embedded anchors).
    pub(crate) fn from_scene_punctual_shop_camera(
        p: &PunctualLightBakeShopCameraParams<'_>,
    ) -> Self {
        let PunctualLightBakeShopCameraParams { bake, cam } = p;
        let mut lights = [PointLightGpu {
            pos: [0.0; 4],
            color: [0.0; 4],
            params: [0.0; 4],
        }; MAX_POINT_LIGHTS];
        let n = bake.src.len().min(MAX_POINT_LIGHTS);
        for (i, ent) in bake.src.iter().take(n).enumerate() {
            match ent {
                ScenePunctualLight::Smooth(l) => {
                    let p = crate::render::world_space::world_on_camera_ray_plane_z(
                        bake.screen_w,
                        bake.screen_h,
                        cam,
                        l.pos[0],
                        l.pos[1],
                        l.pos[2],
                    );
                    Self::push_scene_punctual_entry(&mut lights, i, l, p, SCENE_POINT_KIND_SMOOTH);
                }
                ScenePunctualLight::InverseSquare(l) => {
                    let p =
                        pixel_to_world(bake.screen_w, bake.screen_h, l.pos[0], l.pos[1], l.pos[2]);
                    Self::push_scene_punctual_entry(
                        &mut lights,
                        i,
                        l,
                        p,
                        SCENE_POINT_KIND_INVERSE_SQUARE,
                    );
                }
            }
        }
        Self {
            count: [n as u32, bake.candle_count.min(n as u32), 0, 0],
            extras: [
                bake.gamma.max(0.01),
                bake.time,
                bake.flame_height_world,
                bake.lit_mesh_punctual_intensity_scale,
            ],
            lights,
        }
    }
}
