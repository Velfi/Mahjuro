//! Procedural mesh for the shop counter's leave prop — reads closer to a cast
//! bonshō temple bell: latitudinal cast ridges (tatsuki-like), bosses (nyū),
//! a reinforced striking panel (tsuki-za), simplified ryūzu horns at the crown,
//! and a bronze suspension stem above the kasagata dome.
//!
//! Bell axis-of-symmetry is local +Z. Lip opens toward −Z; hanger runs toward +Z.
//! Local coordinates span roughly `−0.5..+0.5`; [`crate::render::draw_cmd::Object3d`]
//! extents scale the mesh.

use glam::Vec3;

use crate::render::lit_mesh::{CylinderZParams, LitMeshBuffers, push_cylinder_z};
use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Z of the outer lip ring — exported so the shop scene can anchor the silk
/// tassel to the mouth center ([`super::bell_tassel_mesh`]).
pub const LIP_Z: f32 = -0.30;

/// Azimuth of the outward-facing striking plate (+X), matching shrine bells.
const TSUKI_THETA: f32 = 0.0;

/// Number of azimuthal segments around the bell axis.
const SEGS: usize = 24;

/// Vertical bands of the dome (excluding hanger geometry above the kasagata).
const BANDS: usize = 8;

/// Z position of the dome cap (top of bell shell before crown horns).
const CAP_Z: f32 = 0.30;

/// Outer radius at the lip (widest point).
const LIP_R: f32 = 0.46;
/// Outer radius at the shoulder (narrowing).
const SHOULDER_R: f32 = 0.28;
/// Outer radius where the kasagata dome meets the suspension stem.
const CAP_R: f32 = 0.11;

/// Inner lip radius for the underside annulus.
const LIP_INNER_DR: f32 = 0.04;

/// Smooth bonshō shoulder curve — wide mouth, inward taper, small crown.
fn radius_base(t: f32) -> f32 {
    debug_assert!((0.0..=1.0).contains(&t));
    if t < 0.55 {
        let s = t / 0.55;
        LIP_R + (SHOULDER_R - LIP_R) * (s * s * (3.0 - 2.0 * s))
    } else {
        let s = (t - 0.55) / 0.45;
        SHOULDER_R + (CAP_R - SHOULDER_R) * (s * s * (3.0 - 2.0 * s))
    }
}

/// Latitudinal raised rings — evoke cast mold joints / decorative tatsuki bands.
fn latitudinal_ridges(t: f32) -> f32 {
    let g = |center: f32, sigma: f32, amp: f32| {
        let d = (t - center) / sigma;
        (-d * d).exp() * amp
    };
    g(0.16, 0.045, 0.017) + g(0.38, 0.042, 0.014) + g(0.58, 0.038, 0.011)
}

fn radius_at(t: f32) -> f32 {
    let base = radius_base(t) + latitudinal_ridges(t);
    base.max(0.05)
}

fn z_at(t: f32) -> f32 {
    LIP_Z + (CAP_Z - LIP_Z) * t
}

fn angle(seg: usize) -> f32 {
    (seg as f32) / (SEGS as f32) * std::f32::consts::TAU
}

/// Low-detail UV sphere — bosses / horn knobs.
fn push_sphere(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    radius: f32,
    lat_rings: usize,
    lon_segs: usize,
) {
    let lat_rings = lat_rings.max(3);
    let lon_segs = lon_segs.max(4);
    let row = (lon_segs + 1) as u32;
    let base = vertices.len() as u32;
    for lat in 0..=lat_rings {
        let phi = std::f32::consts::PI * (lat as f32) / (lat_rings as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=lon_segs {
            let theta = std::f32::consts::TAU * (lon as f32) / (lon_segs as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            let nx = sin_phi * cos_t;
            let ny = sin_phi * sin_t;
            let nz = cos_phi;
            vertices.push(Vertex3dTex {
                position: [
                    center[0] + radius * nx,
                    center[1] + radius * ny,
                    center[2] + radius * nz,
                ],
                normal: [nx, ny, nz],
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }
    for lat in 0..lat_rings as u32 {
        for lon in 0..lon_segs as u32 {
            let i00 = base + lat * row + lon;
            let i01 = base + lat * row + lon + 1;
            let i10 = base + (lat + 1) * row + lon;
            let i11 = base + (lat + 1) * row + lon + 1;
            indices.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }
}

pub fn build_shop_bell_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // ── Dome outer surface ───────────────────────────────────────────────
    for band in 0..BANDS {
        let t0 = band as f32 / BANDS as f32;
        let t1 = (band + 1) as f32 / BANDS as f32;
        let r0 = radius_at(t0);
        let r1 = radius_at(t1);
        let z0 = z_at(t0);
        let z1 = z_at(t1);
        for seg in 0..SEGS {
            let a0 = angle(seg);
            let a1 = angle(seg + 1);
            let (cx0, cy0) = (a0.cos(), a0.sin());
            let (cx1, cy1) = (a1.cos(), a1.sin());

            let dr = r1 - r0;
            let dz = z1 - z0;
            let hyp = (dr * dr + dz * dz).sqrt().max(1e-6);
            let nr = dz / hyp;
            let nz = -dr / hyp;
            let n00 = [nr * cx0, nr * cy0, nz];
            let n01 = [nr * cx1, nr * cy1, nz];

            let base = vertices.len() as u32;
            vertices.push(Vertex3dTex {
                position: [r0 * cx0, r0 * cy0, z0],
                normal: n00,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [r0 * cx1, r0 * cy1, z0],
                normal: n01,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [r1 * cx1, r1 * cy1, z1],
                normal: n01,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [r1 * cx0, r1 * cy0, z1],
                normal: n00,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    // ── Bottom lip annulus ────────────────────────────────────────────────
    let lip_inner_r = radius_at(0.0) - LIP_INNER_DR;
    for seg in 0..SEGS {
        let a0 = angle(seg);
        let a1 = angle(seg + 1);
        let r_out = radius_at(0.0);
        let (ox0, oy0) = (r_out * a0.cos(), r_out * a0.sin());
        let (ox1, oy1) = (r_out * a1.cos(), r_out * a1.sin());
        let (ix0, iy0) = (lip_inner_r * a0.cos(), lip_inner_r * a0.sin());
        let (ix1, iy1) = (lip_inner_r * a1.cos(), lip_inner_r * a1.sin());
        let n = [0.0, 0.0, -1.0];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ox0, oy0, LIP_Z],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, oy1, LIP_Z],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, iy1, LIP_Z],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, iy0, LIP_Z],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }

    // ── Cap disc (+Z) closes kasagata crown ───────────────────────────────
    {
        let center = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [0.0, 0.0, CAP_Z],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        for seg in 0..SEGS {
            let a = angle(seg);
            vertices.push(Vertex3dTex {
                position: [CAP_R * a.cos(), CAP_R * a.sin(), CAP_Z],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for seg in 0..SEGS {
            let a = center + 1 + seg as u32;
            let b = center + 1 + ((seg as u32 + 1) % SEGS as u32);
            indices.extend_from_slice(&[center, a, b]);
        }
    }

    // ── Nyū — hemispherical bosses around upper chichi-no-ma ───────────────
    const N_BOSS: usize = 12;
    let t_nyu = 0.415_f32;
    let z_nyu = z_at(t_nyu);
    let r_nyu = radius_at(t_nyu);
    let boss_r = 0.036_f32;
    for i in 0..N_BOSS {
        let theta = std::f32::consts::TAU * (i as f32) / (N_BOSS as f32);
        let (ct, st) = (theta.cos(), theta.sin());
        let sx = r_nyu * ct;
        let sy = r_nyu * st;
        let sz = z_nyu;
        let surf = Vec3::new(sx, sy, sz);
        let n = surf.normalize();
        let c = surf + n * (boss_r * 0.88);
        push_sphere(&mut vertices, &mut indices, [c.x, c.y, c.z], boss_r, 5, 8);
    }

    // ── Tsuki-za — slightly raised bronze plate on outer striking sector ──
    {
        let t_mid = 0.44_f32;
        let z_lo = z_at(t_mid - 0.075);
        let z_hi = z_at(t_mid + 0.065);
        let r_mid = radius_at(t_mid);
        let thick = 0.032_f32;
        let cos_t = TSUKI_THETA.cos();
        let sin_t = TSUKI_THETA.sin();
        let x0 = r_mid * cos_t;
        let y0 = r_mid * sin_t;
        let x1 = (r_mid + thick) * cos_t;
        let y1 = (r_mid + thick) * sin_t;
        let plate_w = 0.14_f32;
        let px = (x0 + x1) * 0.5;
        let py = (y0 + y1) * 0.5;
        let nx = cos_t;
        let ny = sin_t;
        let tx = -sin_t;
        let ty = cos_t;
        let hw = plate_w * 0.5;
        let corners = [
            [px + nx * 0.018 + tx * hw, py + ny * 0.018 + ty * hw, z_lo],
            [px + nx * 0.018 - tx * hw, py + ny * 0.018 - ty * hw, z_lo],
            [px + nx * 0.018 - tx * hw, py + ny * 0.018 - ty * hw, z_hi],
            [px + nx * 0.018 + tx * hw, py + ny * 0.018 + ty * hw, z_hi],
        ];
        let base = vertices.len() as u32;
        let nrm = [nx, ny, 0.0];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: nrm,
                uv: *uv,
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Ryūzu pair — stub dragon-head knobs (highly stylised) ────────────
    let horn_z = CAP_Z + 0.045;
    push_sphere(
        &mut vertices,
        &mut indices,
        [-0.076, 0.0, horn_z],
        0.052,
        4,
        8,
    );
    push_sphere(
        &mut vertices,
        &mut indices,
        [0.076, 0.0, horn_z],
        0.052,
        4,
        8,
    );

    // ── Kasagata bead + suspension stem (bronze rod along +Z) ────────────
    let mut buffers = LitMeshBuffers {
        vertices: &mut vertices,
        indices: &mut indices,
    };
    push_cylinder_z(
        &mut buffers,
        &CylinderZParams {
            cx: 0.0,
            cy: 0.0,
            z0: CAP_Z + 0.002,
            z1: CAP_Z + 0.095,
            radius: CAP_R * 0.72,
            segments: 14,
        },
    );

    // ── Shu-moku cord stub (thin braided hemp toward lamp smoke) ─────────
    push_cylinder_z(
        &mut buffers,
        &CylinderZParams {
            cx: 0.0,
            cy: 0.0,
            z0: CAP_Z + 0.095,
            z1: CAP_Z + 0.26,
            radius: 0.022,
            segments: 10,
        },
    );

    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.58, 0.42, 0.20, 1.0],
            specular_strength: 0.88,
            specular_power: 104.0,
        },
    }
}
