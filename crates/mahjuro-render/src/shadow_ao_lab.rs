//! Programmatic shadow + contact-AO test rig for the debug lab overlay.
//!
//! Builds a known box corridor in world space (Z-up), software-rasterizes a synthetic
//! `.msh` depth field from the same layout, and exposes CPU probes for punctual shadow
//! and contact AO.

use std::sync::{Arc, OnceLock};

use glam::{Mat4, Vec3, Vec4};

use crate::draw_cmd::ScenePunctualLight;
use crate::draw_cmd::{CameraParams, Object3d, Object3dKind};
use crate::primitive::{MaterialSpec, MeshId};
use crate::projected_light_shadow::punctual_light_world;
use crate::room_gi_bake::RoomGiRoom;
use crate::room_shadow_bake::{
    self, ContactAoWorldProbe, RoomShadowBake, bake_contact_ao_from_depth,
};
use crate::wgpu_renderer::PointLight;
use crate::world_space::surface_anchor_from_world_xyz;

/// Synthetic lab geometry lives in absolute world units (no room-env height scale).
pub const CONTACT_AO_WORLD_SCALE: f32 = 1.0;

const BAKE_W: u32 = 256;
const BAKE_H: u32 = 256;

/// Which synthetic layout the lab draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowAoLabLayout {
    /// Floor / walls / ceiling + thin horizontal bar (ceiling-edge AO stress test).
    HorizontalBand,
    /// Same room shell without the bar — punctual-only baseline.
    CleanCorridor,
}

impl ShadowAoLabLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::HorizontalBand => "Horizontal band",
            Self::CleanCorridor => "Clean corridor",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::HorizontalBand => Self::CleanCorridor,
            Self::CleanCorridor => Self::HorizontalBand,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LabAabb {
    pub center: Vec3,
    pub half: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct LabMesh {
    aabb: LabAabb,
    color: [f32; 4],
    casts_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowAoLabCamera {
    Corridor,
    FarWall,
    Orbit,
}

impl ShadowAoLabCamera {
    pub fn label(self) -> &'static str {
        match self {
            Self::Corridor => "Corridor",
            Self::FarWall => "Far wall",
            Self::Orbit => "Orbit",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Corridor => Self::FarWall,
            Self::FarWall => Self::Orbit,
            Self::Orbit => Self::Corridor,
        }
    }
}

/// CPU punctual-shadow estimate + contact-AO probe for one world point.
#[derive(Clone, Copy, Debug)]
pub struct ShadowAoLabProbe {
    pub label: &'static str,
    pub world: Vec3,
    pub analytic_shadow: f32,
    pub contact_ao: Option<ContactAoWorldProbe>,
}

fn shell_meshes(layout: ShadowAoLabLayout) -> Vec<LabMesh> {
    let mut meshes = vec![
        LabMesh {
            aabb: LabAabb {
                center: Vec3::new(0.0, 2800.0, 25.0),
                half: Vec3::new(800.0, 3200.0, 25.0),
            },
            color: [0.35, 0.32, 0.30, 1.0],
            casts_shadow: true,
        },
        LabMesh {
            aabb: LabAabb {
                center: Vec3::new(0.0, 6000.0, 1200.0),
                half: Vec3::new(800.0, 25.0, 1200.0),
            },
            color: [0.42, 0.38, 0.34, 1.0],
            casts_shadow: true,
        },
        LabMesh {
            aabb: LabAabb {
                center: Vec3::new(-775.0, 2800.0, 1200.0),
                half: Vec3::new(25.0, 3200.0, 1200.0),
            },
            color: [0.38, 0.34, 0.32, 1.0],
            casts_shadow: true,
        },
        LabMesh {
            aabb: LabAabb {
                center: Vec3::new(775.0, 2800.0, 1200.0),
                half: Vec3::new(25.0, 3200.0, 1200.0),
            },
            color: [0.38, 0.34, 0.32, 1.0],
            casts_shadow: true,
        },
        LabMesh {
            aabb: LabAabb {
                center: Vec3::new(0.0, 2800.0, 2400.0),
                half: Vec3::new(800.0, 3200.0, 25.0),
            },
            color: [0.30, 0.28, 0.26, 1.0],
            casts_shadow: true,
        },
    ];
    if layout == ShadowAoLabLayout::HorizontalBand {
        meshes.push(LabMesh {
            aabb: LabAabb {
                center: Vec3::new(0.0, 4375.0, 1625.0),
                half: Vec3::new(720.0, 60.0, 260.0),
            },
            color: [0.55, 0.52, 0.48, 1.0],
            casts_shadow: true,
        });
    }
    meshes
}

/// Lab ceiling lamp position in world space (Z-up).
pub fn light_world() -> Vec3 {
    Vec3::new(0.0, 2800.0, 2050.0)
}

fn probe_world_points() -> [(&'static str, Vec3); 3] {
    [
        ("back_lo", Vec3::new(0.0, 5950.0, 600.0)),
        ("back_mid", Vec3::new(0.0, 5950.0, 1200.0)),
        ("back_hi", Vec3::new(0.0, 5950.0, 1800.0)),
    ]
}

fn fit_corners(meshes: &[LabMesh]) -> Vec<Vec3> {
    let mut corners = Vec::with_capacity(meshes.len() * 8);
    for m in meshes {
        let c = m.aabb.center;
        let h = m.aabb.half;
        for &sx in &[-1.0f32, 1.0] {
            for &sy in &[-1.0, 1.0] {
                for &sz in &[-1.0, 1.0] {
                    corners.push(c + Vec3::new(sx * h.x, sy * h.y, sz * h.z));
                }
            }
        }
    }
    corners
}

/// World AABB corners for punctual shadow frustum fitting in the lab scene.
pub fn fit_corners_world(layout: ShadowAoLabLayout) -> Vec<Vec3> {
    fit_corners(&shell_meshes(layout))
}

fn ray_aabb(origin: Vec3, dir: Vec3, box_center: Vec3, half: Vec3) -> Option<f32> {
    let inv = dir.recip();
    let min_b = box_center - half;
    let max_b = box_center + half;
    let t1 = (min_b - origin) * inv;
    let t2 = (max_b - origin) * inv;
    let tmin = t1.min(t2);
    let tmax = t1.max(t2);
    let t_enter = tmin.x.max(tmin.y).max(tmin.z);
    let t_exit = tmax.x.min(tmax.y).min(tmax.z);
    if t_enter > t_exit || t_exit < 0.0 {
        return None;
    }
    let t = if t_enter >= 0.0 { t_enter } else { t_exit };
    if t >= 0.0 { Some(t) } else { None }
}

/// World-space ray from light → probe blocked by any shadow-casting mesh?
pub fn analytic_punctual_shadow(light: Vec3, probe: Vec3, layout: ShadowAoLabLayout) -> f32 {
    let dir = probe - light;
    let dist = dir.length();
    if dist < 1e-4 {
        return 1.0;
    }
    let d = dir / dist;
    for m in shell_meshes(layout) {
        if !m.casts_shadow {
            continue;
        }
        if let Some(t) = ray_aabb(light, d, m.aabb.center, m.aabb.half)
            && t > 1e-3
            && t < dist - 1e-3
        {
            return 0.0;
        }
    }
    1.0
}

/// Aim point for live punctual-shadow frusta (must stay in sync with [`light_view_proj`]).
pub fn punctual_shadow_look_at() -> Vec3 {
    Vec3::new(0.0, 4500.0, 1200.0)
}

pub const PUNCTUAL_SHADOW_FALLBACK_HALF: f32 = 900.0;
pub const PUNCTUAL_SHADOW_FALLBACK_DEPTH: f32 = 8000.0;

/// Punctual-shadow LVP used by the synthetic contact-AO bake and CPU probes.
pub fn punctual_light_view_proj(layout: ShadowAoLabLayout) -> Mat4 {
    light_view_proj(layout)
}

fn light_view_proj(layout: ShadowAoLabLayout) -> Mat4 {
    let corners = fit_corners(&shell_meshes(layout));
    crate::projected_light_shadow::point_light_shadow_view_proj_with_fit(
        light_world(),
        &corners,
        PUNCTUAL_SHADOW_FALLBACK_HALF,
        PUNCTUAL_SHADOW_FALLBACK_DEPTH,
        punctual_shadow_look_at(),
    )
    .0
}

fn software_depth_bake(lvp: Mat4, meshes: &[LabMesh]) -> Vec<u8> {
    let w = BAKE_W as usize;
    let h = BAKE_H as usize;
    let inv_lvp = lvp.inverse();
    let mut depth = vec![1.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let ndc_x = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
            let ndc_y = 1.0 - (y as f32 + 0.5) / h as f32 * 2.0;
            let near_h = inv_lvp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
            let far_h = inv_lvp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
            let p0 = near_h.truncate() / near_h.w.max(1e-8);
            let p1 = far_h.truncate() / far_h.w.max(1e-8);
            let ray_dir = (p1 - p0).normalize_or_zero();
            if ray_dir.length_squared() < 1e-10 {
                continue;
            }
            let mut best = 1.0f32;
            for m in meshes {
                if let Some(t) = ray_aabb(p0, ray_dir, m.aabb.center, m.aabb.half) {
                    let hit = p0 + ray_dir * t;
                    let clip = lvp * hit.extend(1.0);
                    let z = (clip.z / clip.w).clamp(0.0, 1.0);
                    best = best.min(z);
                }
            }
            depth[y * w + x] = best;
        }
    }
    depth.into_iter().flat_map(f32::to_le_bytes).collect()
}

fn build_synthetic_bake(layout: ShadowAoLabLayout) -> RoomShadowBake {
    let lvp = light_view_proj(layout);
    let depth_bytes: Arc<[u8]> = Arc::from(software_depth_bake(lvp, &shell_meshes(layout)));
    let ao_bytes = Arc::from(bake_contact_ao_from_depth(BAKE_W, BAKE_H, &depth_bytes));
    RoomShadowBake {
        room: RoomGiRoom::Hallway,
        width: BAKE_W,
        height: BAKE_H,
        light_view_proj: lvp.to_cols_array(),
        depth_bias: 0.005,
        depth_bytes,
        ao_bytes: Some(ao_bytes),
    }
}

static SYNTHETIC_BAKES: OnceLock<[Arc<RoomShadowBake>; 2]> = OnceLock::new();

/// Committed synthetic `.msh` for the active lab layout (cached).
pub fn synthetic_bake(layout: ShadowAoLabLayout) -> Arc<RoomShadowBake> {
    let all = SYNTHETIC_BAKES.get_or_init(|| {
        [
            Arc::new(build_synthetic_bake(ShadowAoLabLayout::HorizontalBand)),
            Arc::new(build_synthetic_bake(ShadowAoLabLayout::CleanCorridor)),
        ]
    });
    match layout {
        ShadowAoLabLayout::HorizontalBand => all[0].clone(),
        ShadowAoLabLayout::CleanCorridor => all[1].clone(),
    }
}

pub fn probe_layout(layout: ShadowAoLabLayout, _window_h: f32) -> Vec<ShadowAoLabProbe> {
    let bake = synthetic_bake(layout);
    let light = light_world();
    probe_world_points()
        .into_iter()
        .map(|(label, world)| ShadowAoLabProbe {
            label,
            world,
            analytic_shadow: analytic_punctual_shadow(light, world, layout),
            contact_ao: room_shadow_bake::probe_contact_ao_at_world(
                &bake,
                world,
                CONTACT_AO_WORLD_SCALE,
            ),
        })
        .collect()
}

pub fn camera(preset: ShadowAoLabCamera, orbit_yaw: f32) -> CameraParams {
    match preset {
        ShadowAoLabCamera::Corridor => CameraParams {
            eye: [0.0, -800.0, 1100.0],
            target: [0.0, 4500.0, 900.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 52.0,
            clip_near: None,
            clip_far: None,
        },
        ShadowAoLabCamera::FarWall => CameraParams {
            eye: [0.0, 2000.0, 950.0],
            target: [0.0, 5950.0, 1200.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 48.0,
            clip_near: None,
            clip_far: None,
        },
        ShadowAoLabCamera::Orbit => {
            let target = Vec3::new(0.0, 3800.0, 1100.0);
            let offset = Vec3::new(0.0, -2200.0, 600.0);
            let c = orbit_yaw.cos();
            let s = orbit_yaw.sin();
            let rotated = Vec3::new(
                offset.x * c - offset.y * s,
                offset.x * s + offset.y * c,
                offset.z,
            );
            let eye = target + rotated;
            CameraParams {
                eye: eye.to_array(),
                target: target.to_array(),
                up: [0.0, 0.0, 1.0],
                fovy_deg: 50.0,
                clip_near: None,
                clip_far: None,
            }
        }
    }
}

fn world_anchor(window_w: f32, window_h: f32, world: Vec3) -> [f32; 3] {
    surface_anchor_from_world_xyz(window_w, window_h, world)
}

pub fn build_object3ds(layout: ShadowAoLabLayout, window_w: f32, window_h: f32) -> Vec<Object3d> {
    let material = MaterialSpec::plain();
    shell_meshes(layout)
        .into_iter()
        .map(|m| {
            let e = m.aabb.half * 2.0;
            Object3d {
                pos: world_anchor(window_w, window_h, m.aabb.center),
                extents: [e.x, e.y, e.z],
                rotation: [0.0, 0.0, 0.0],
                color: m.color,
                kind: Object3dKind::Primitive {
                    shape: MeshId::Cube,
                    material: material.clone(),
                    pick_id: None,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
            }
        })
        .collect()
}

pub fn build_point_lights(window_w: f32, window_h: f32) -> Vec<PointLight> {
    // Smooth (quadratic) falloff — inverse-square needs GLB inv_doc_scale and vanishes at
    // lab kilo-unit distances when `embedded_gltf_punctual` is off.
    let radius = window_h * 3.5;
    vec![PointLight {
        pos: world_anchor(window_w, window_h, light_world()),
        radius,
        color: [1.0, 0.92, 0.78],
        intensity: 6.0,
    }]
}

pub fn punctual_shadow_ndc_at_probe(
    layout: ShadowAoLabLayout,
    window_w: f32,
    window_h: f32,
    probe: Vec3,
) -> Option<Vec3> {
    let lvp = light_view_proj(layout);
    let lights = build_point_lights(window_w, window_h);
    let entry = ScenePunctualLight::InverseSquare(lights[0]);
    let light_w = punctual_light_world(window_w, window_h, &entry, None, false);
    let _ = light_w;
    let clip = lvp * probe.extend(1.0);
    if clip.w.abs() < 1e-8 {
        return None;
    }
    Some(clip.truncate() / clip.w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_band_back_wall_never_applies_mis_mapped_ao() {
        let bake = synthetic_bake(ShadowAoLabLayout::HorizontalBand);
        let world = Vec3::new(0.0, 5950.0, 1200.0);
        if let Some(probe) = room_shadow_bake::probe_contact_ao_world(&bake, world, 1080.0) {
            if probe.ao < 240 {
                assert!(
                    !probe.applies,
                    "mis-mapped dark AO must be rejected by depth coherence"
                );
            }
        }
    }

    #[test]
    fn synthetic_bake_has_dark_contact_ao_texels() {
        let bake = synthetic_bake(ShadowAoLabLayout::HorizontalBand);
        let ao = bake.ao_bytes.as_ref().expect("ao");
        assert!(
            ao.iter().any(|&b| b < 200),
            "expected depth-edge darkening in synthetic AO field"
        );
    }

    #[test]
    fn horizontal_bar_casts_analytic_shadow_on_back_wall() {
        let s = analytic_punctual_shadow(
            light_world(),
            Vec3::new(0.0, 5950.0, 1200.0),
            ShadowAoLabLayout::HorizontalBand,
        );
        assert_eq!(s, 0.0);
    }

    #[test]
    fn clean_corridor_no_analytic_bar_shadow() {
        let s = analytic_punctual_shadow(
            light_world(),
            Vec3::new(0.0, 5950.0, 1200.0),
            ShadowAoLabLayout::CleanCorridor,
        );
        assert_eq!(s, 1.0);
    }

    #[test]
    fn horizontal_band_back_wall_probe_summary() {
        let probes = probe_layout(ShadowAoLabLayout::HorizontalBand, 1080.0);
        for p in probes {
            let ao = p
                .contact_ao
                .map(|c| (c.ao, c.applies))
                .unwrap_or((255, false));
            eprintln!(
                "{} shadow={} ao={} applies={}",
                p.label, p.analytic_shadow, ao.0, ao.1
            );
        }
        let lo = analytic_punctual_shadow(
            light_world(),
            Vec3::new(0.0, 5950.0, 600.0),
            ShadowAoLabLayout::HorizontalBand,
        );
        let hi = analytic_punctual_shadow(
            light_world(),
            Vec3::new(0.0, 5950.0, 1800.0),
            ShadowAoLabLayout::HorizontalBand,
        );
        assert_eq!(lo, 1.0, "back_lo should be lit (above/below bar)");
        assert_eq!(hi, 1.0, "back_hi should be lit");
    }

    #[test]
    fn z_up_shadow_view_up_uses_world_z() {
        let up = crate::projected_light_shadow::z_up_shadow_view_up(
            Vec3::new(0.0, 1.0, -0.2).normalize(),
        );
        assert!(up.z.abs() > 0.99);
    }
}
