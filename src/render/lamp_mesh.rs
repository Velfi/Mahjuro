//! Procedural pendant lamp mesh — a hanging brass cone shade with a glass bulb.
//!
//! Built directly in **world space** (Z-up, right-handed): the lamp axis is **+Z**,
//! so no corrective rotation is needed at draw time. `obj.rotation = Mat4::IDENTITY`
//! is correct for a vertically hanging lamp.
//!
//! ```text
//!   z = +0.85  cord top   — ceiling attachment
//!   z =  0.00  shade apex — small end (top of cone, near ceiling)
//!   z = -0.85  shade rim  — wide open end (hangs toward counter) ✓
//! ```
//!
//! The cord is a thin cylinder along +Z. The shade is a frustum with:
//!   - apex (small, closed end) at z = SHADE_APEX_Z = 0.0
//!   - rim  (wide, open end)   at z = SHADE_RIM_Z  = -0.85
//!
//! Sub-meshes:
//! * [`build_lamp_body_mesh`] — cord + brass cone shade (`Metal`).
//! * [`build_lamp_bulb_mesh`] — small glass sphere inside the shade (`Glass`).
//! * [`build_bug_body_mesh`]  — chitin ellipsoid body for a hovering insect (`Ceramic`).
//! * [`build_bug_wing_mesh`]  — two flat wing quads fanned ~45° from body axis (`Glass`).

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const BODY_SEGS: usize = 24;
const BULB_SEGS: usize = 16;
const BULB_LAT:  usize = 12;

// ── Cord ──────────────────────────────────────────────────────────────────────
const CORD_R:  f32 = 0.014;
const CORD_Z0: f32 = 0.00;  // bottom of cord = shade apex
const CORD_Z1: f32 = 0.85;  // top of cord = ceiling attachment

// ── Shade ─────────────────────────────────────────────────────────────────────
// Apex (small, closed) at z=0 (top, near ceiling attachment).
// Rim  (wide, open)   at z=SHADE_RIM_Z (hangs downward toward counter).
const SHADE_APEX_Z: f32 =  0.00;  // top of shade (small end)
pub const SHADE_RIM_Z:  f32 = -0.55;  // bottom of shade (wide open end, hangs down)
const SHADE_APEX_R: f32 =  0.07;  // small finial radius at apex
pub const SHADE_RIM_R:  f32 =  0.72;  // wide open rim
const SHADE_RIM_T:  f32 =  0.018; // sheet-metal rim thickness

/// Returns the minimum horizontal radius (in mesh-local units, before scaling)
/// that a point at `local_z` must be at to lie *outside* the cone shade.
///
/// `local_z` should be ≤ 0 (below the apex). At or above the apex (z ≥ 0)
/// returns 0 (no constraint from the shade body). Below the rim returns the
/// rim radius (bugs clear the open end freely).
pub fn shade_exclusion_radius(local_z: f32) -> f32 {
    if local_z >= SHADE_APEX_Z { return 0.0; }
    // Clamp to the shade z-range.
    let t = (local_z / SHADE_RIM_Z).clamp(0.0, 1.0); // 0 at apex, 1 at rim
    SHADE_APEX_R + t * (SHADE_RIM_R - SHADE_APEX_R)
}

// ── Bulb ──────────────────────────────────────────────────────────────────────
// Sits inside the shade. z=0 is apex (near ceiling), z=SHADE_RIM_Z is open rim.
// BULB_CZ near apex so it's tucked inside and illuminates downward.
const BULB_CZ: f32 = -0.25; // center inside cone (upper third of shade)
const BULB_RH: f32 =  0.09; // horizontal radius (XY plane)
const BULB_RV: f32 =  0.10; // vertical radius (Z axis)

/// Exported: world-space Z of bulb center (relative to lamp anchor), used by
/// the scene to place the point light.
pub const BULB_Z: f32 = BULB_CZ;

// ─────────────────────────────────────────────────────────────────────────────
// Geometry helpers  (all in Z-up space: axis = Z, radial plane = XY)
// ─────────────────────────────────────────────────────────────────────────────

/// Cylinder wall along Z, constant radius.
fn add_cylinder_wall(
    verts: &mut Vec<Vertex3dTex>,
    idxs:  &mut Vec<u32>,
    r: f32, z0: f32, z1: f32, segs: usize,
) {
    let base = verts.len() as u32;
    for si in 0..=segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        let u = si as f32 / segs as f32;
        verts.push(Vertex3dTex { position: [r*c, r*s, z0], normal: [c, s, 0.0], uv: [u, 0.0] });
        verts.push(Vertex3dTex { position: [r*c, r*s, z1], normal: [c, s, 0.0], uv: [u, 1.0] });
    }
    for si in 0..segs {
        let i = base + si as u32 * 2;
        idxs.extend_from_slice(&[i, i+2, i+1,  i+1, i+2, i+3]);
    }
}

/// Frustum (truncated cone) wall along Z.
/// r0/z0 = first ring, r1/z1 = second ring.
/// `flip` reverses winding and normal sign (for inner wall).
fn add_frustum_wall(
    verts: &mut Vec<Vertex3dTex>,
    idxs:  &mut Vec<u32>,
    r0: f32, z0: f32, r1: f32, z1: f32, segs: usize, flip: bool,
) {
    let base = verts.len() as u32;
    // Outward normal for a frustum whose apex is at larger-z and rim at smaller-z.
    // The cone opens downward: r increases as z decreases.
    let dz = z1 - z0;
    let dr = r0 - r1; // positive when rim (r0) > apex (r1)
    let len = (dr*dr + dz*dz).sqrt().max(1e-6);
    // Axial component of normal (nz) = dr/len (positive = points away from narrow end).
    // Radial component (nr) = dz/len (positive when z1>z0 and cone opens toward z0).
    let (nz, nr) = (dr / len, dz / len);
    let ns = if flip { -1.0_f32 } else { 1.0 };
    for si in 0..=segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        let u = si as f32 / segs as f32;
        verts.push(Vertex3dTex {
            position: [r0*c, r0*s, z0],
            normal:   [ns*nr*c, ns*nr*s, ns*nz],
            uv: [u, 0.0],
        });
        verts.push(Vertex3dTex {
            position: [r1*c, r1*s, z1],
            normal:   [ns*nr*c, ns*nr*s, ns*nz],
            uv: [u, 1.0],
        });
    }
    for si in 0..segs {
        let i = base + si as u32 * 2;
        if flip {
            idxs.extend_from_slice(&[i, i+1, i+2,  i+1, i+3, i+2]);
        } else {
            idxs.extend_from_slice(&[i, i+2, i+1,  i+1, i+2, i+3]);
        }
    }
}

/// Disc (filled circle) perpendicular to Z.
fn add_disc(
    verts: &mut Vec<Vertex3dTex>,
    idxs:  &mut Vec<u32>,
    r: f32, z: f32, segs: usize, face_up: bool,
) {
    let nz = if face_up { 1.0_f32 } else { -1.0 };
    let center = verts.len() as u32;
    verts.push(Vertex3dTex { position: [0.0, 0.0, z], normal: [0.0, 0.0, nz], uv: [0.5, 0.5] });
    let rim = verts.len() as u32;
    for si in 0..segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        verts.push(Vertex3dTex {
            position: [r*c, r*s, z],
            normal: [0.0, 0.0, nz],
            uv: [0.5+0.5*c, 0.5+0.5*s],
        });
    }
    for si in 0..segs {
        let a = rim + si as u32;
        let b = rim + (si + 1) as u32 % segs as u32;
        if face_up { idxs.extend_from_slice(&[center, a, b]); }
        else        { idxs.extend_from_slice(&[center, b, a]); }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public mesh builders
// ─────────────────────────────────────────────────────────────────────────────

/// Brass body: thin cord + cone shade.
///
/// Apex (small, closed) at z=0 (near ceiling), rim (wide, open) at z=SHADE_RIM_Z (hangs down). ✓
pub fn build_lamp_body_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs:  Vec<u32>         = Vec::new();

    // ── Cord (thin cylinder along +Z, from apex up to ceiling) ───────────────
    add_cylinder_wall(&mut verts, &mut idxs, CORD_R, CORD_Z0, CORD_Z1, 8);
    add_disc(         &mut verts, &mut idxs, CORD_R, CORD_Z1, 8, true); // top cap

    // ── Shade outer wall: apex (z=0, small) → rim (z=-0.85, wide) ────────────
    add_frustum_wall(&mut verts, &mut idxs,
        SHADE_RIM_R,  SHADE_RIM_Z,   // r0/z0 = rim (wide, low)
        SHADE_APEX_R, SHADE_APEX_Z,  // r1/z1 = apex (small, high)
        BODY_SEGS, false,
    );
    // ── Shade inner wall (normals inward, lit by bulb) ────────────────────────
    add_frustum_wall(&mut verts, &mut idxs,
        SHADE_RIM_R  - SHADE_RIM_T,       SHADE_RIM_Z,
        SHADE_APEX_R - SHADE_RIM_T * 0.3, SHADE_APEX_Z,
        BODY_SEGS, true,
    );
    // ── Open-rim annulus at z=SHADE_RIM_Z (the hanging open end) ─────────────
    {
        let r_out = SHADE_RIM_R;
        let r_in  = SHADE_RIM_R - SHADE_RIM_T;
        let z     = SHADE_RIM_Z;
        let segs  = BODY_SEGS;
        let base  = verts.len() as u32;
        for si in 0..=segs {
            let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
            let (c, s) = (theta.cos(), theta.sin());
            let u = si as f32 / segs as f32;
            // Normal faces downward (−Z) — toward the counter.
            verts.push(Vertex3dTex { position: [r_out*c, r_out*s, z], normal: [0.0, 0.0, -1.0], uv: [u, 0.0] });
            verts.push(Vertex3dTex { position: [r_in *c, r_in *s, z], normal: [0.0, 0.0, -1.0], uv: [u, 1.0] });
        }
        for si in 0..segs {
            let i = base + si as u32 * 2;
            idxs.extend_from_slice(&[i, i+2, i+1,  i+1, i+2, i+3]);
        }
    }
    // ── Finial cap at apex (z=0, small, ceiling side) — faces up ─────────────
    add_disc(&mut verts, &mut idxs, SHADE_APEX_R, SHADE_APEX_Z, BODY_SEGS, true);

    MeshCpu {
        vertices: verts,
        indices:  idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            base_color: [0.82, 0.64, 0.30, 1.0], // aged brass
            specular_strength: 0.85,
            specular_power: 72.0,
        },
    }
}

/// Glass bulb — small ellipsoid inside the shade, rendered with `Glass`.
pub fn build_lamp_bulb_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs:  Vec<u32>         = Vec::new();

    let cz = BULB_CZ;
    for lat in 0..=BULB_LAT {
        let phi = std::f32::consts::PI * (lat as f32) / (BULB_LAT as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=BULB_SEGS {
            let theta = std::f32::consts::TAU * (lon as f32) / (BULB_SEGS as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            // Ellipsoid in Z-up: radial = XY, axial = Z.
            let x  = BULB_RH * sin_phi * cos_t;
            let y  = BULB_RH * sin_phi * sin_t;
            let z  = BULB_RV * cos_phi + cz;
            let nx = sin_phi * cos_t / BULB_RH;
            let ny = sin_phi * sin_t / BULB_RH;
            let nz = cos_phi / BULB_RV;
            let nlen = (nx*nx + ny*ny + nz*nz).sqrt().max(1e-6);
            verts.push(Vertex3dTex {
                position: [x, y, z],
                normal:   [nx/nlen, ny/nlen, nz/nlen],
                uv: [lon as f32 / BULB_SEGS as f32, lat as f32 / BULB_LAT as f32],
            });
        }
    }
    let row = BULB_SEGS + 1;
    for lat in 0..BULB_LAT {
        for lon in 0..BULB_SEGS {
            let i00 = (lat     * row + lon    ) as u32;
            let i01 = (lat     * row + lon + 1) as u32;
            let i10 = ((lat+1) * row + lon    ) as u32;
            let i11 = ((lat+1) * row + lon + 1) as u32;
            idxs.extend_from_slice(&[i00, i10, i01,  i01, i10, i11]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices:  idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Glass,
            base_color: [1.00, 0.92, 0.60, 1.0],
            specular_strength: 1.0,
            specular_power: 128.0,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bug meshes
// ─────────────────────────────────────────────────────────────────────────────
//
// Each bug is two Object3d draws: body + wings.
// The body is aligned along +X (the bug's forward axis). The scene rotates
// each bug so +X points tangent to its orbit, keeping it oriented naturally.
//
// Body proportions (unit scale, scene extents scale these up):
//   length (X) = 1.0, width (Y) = 0.38, height (Z) = 0.30
// Wing proportions:
//   span (Y each side) = 0.80, chord (X) = 0.55, flat in XY plane

const BUG_BODY_SEGS: usize = 14;
const BUG_BODY_LAT:  usize = 10;

/// Chitinous insect body — elongated ellipsoid along +X.
///
/// Unit bounding box: X ∈ [−0.5, +0.5], Y ∈ [−0.19, +0.19], Z ∈ [−0.15, +0.15].
/// The scene applies `extents` to scale it to world size.
pub fn build_bug_body_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs:  Vec<u32>         = Vec::new();

    // Ellipsoid: rx along X (length), ry along Y (width), rz along Z (height).
    let rx = 0.50_f32;
    let ry = 0.19_f32;
    let rz = 0.15_f32;

    for lat in 0..=BUG_BODY_LAT {
        let phi = std::f32::consts::PI * (lat as f32) / (BUG_BODY_LAT as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=BUG_BODY_SEGS {
            let theta = std::f32::consts::TAU * (lon as f32) / (BUG_BODY_SEGS as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            // Map spherical coords: polar axis = X, equatorial plane = YZ.
            let x = rx * cos_phi;
            let y = ry * sin_phi * cos_t;
            let z = rz * sin_phi * sin_t;
            // Ellipsoid normal (outward gradient of x²/rx²+y²/ry²+z²/rz²=1).
            let nx = cos_phi / rx;
            let ny = sin_phi * cos_t / ry;
            let nz = sin_phi * sin_t / rz;
            let nlen = (nx*nx + ny*ny + nz*nz).sqrt().max(1e-6);
            verts.push(Vertex3dTex {
                position: [x, y, z],
                normal:   [nx/nlen, ny/nlen, nz/nlen],
                uv: [lon as f32 / BUG_BODY_SEGS as f32, lat as f32 / BUG_BODY_LAT as f32],
            });
        }
    }
    let row = BUG_BODY_SEGS + 1;
    for lat in 0..BUG_BODY_LAT {
        for lon in 0..BUG_BODY_SEGS {
            let i00 = (lat     * row + lon    ) as u32;
            let i01 = (lat     * row + lon + 1) as u32;
            let i10 = ((lat+1) * row + lon    ) as u32;
            let i11 = ((lat+1) * row + lon + 1) as u32;
            idxs.extend_from_slice(&[i00, i10, i01,  i01, i10, i11]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices:  idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Enamel,
            // Dark iridescent chitin — deep olive-green, almost black.
            base_color: [0.08, 0.12, 0.06, 1.0],
            specular_strength: 0.90,
            specular_power: 64.0,
        },
    }
}

/// Two flat wing quads fanned symmetrically about the body's +X axis.
///
/// Wings lie in the XY plane (flat, no Z thickness). Each wing spans
/// Y = [0, 0.80] or [0, -0.80] and X = [-0.10, +0.45] in unit space.
/// The scene applies the same `extents` as the body.
pub fn build_bug_wing_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs:  Vec<u32>         = Vec::new();

    // Wing shape: slightly swept-back quad.
    // Defined in XY plane, normal = +Z. Two wings: +Y side and -Y side.
    // Wing root starts at body midpoint (x=0), tip sweeps back (x=-0.10).
    let wing_defs: &[(f32, f32)] = &[(1.0, 1.0), (-1.0, 1.0)]; // (y_sign, normal_z_sign)
    for &(ys, nz) in wing_defs {
        let base = verts.len() as u32;
        // Four corners: inner-front, inner-back, outer-front, outer-back.
        let pts = [
            [ 0.20_f32,  0.05 * ys, 0.0_f32], // inner leading edge (near head)
            [-0.10_f32,  0.10 * ys, 0.0_f32], // inner trailing edge
            [ 0.30_f32,  0.75 * ys, 0.0_f32], // outer leading edge (wing tip area)
            [-0.05_f32,  0.75 * ys, 0.0_f32], // outer trailing edge
        ];
        for (i, p) in pts.iter().enumerate() {
            let u = if i < 2 { 0.0 } else { 1.0 };
            let v = if i % 2 == 0 { 0.0 } else { 1.0 };
            verts.push(Vertex3dTex {
                position: *p,
                normal: [0.0, 0.0, nz],
                uv: [u, v],
            });
        }
        // Two triangles (quad). Winding matches nz sign.
        if nz > 0.0 {
            idxs.extend_from_slice(&[base, base+2, base+1,  base+1, base+2, base+3]);
        } else {
            idxs.extend_from_slice(&[base, base+1, base+2,  base+1, base+3, base+2]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices:  idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Glass,
            // Pale iridescent wings — warm translucent amber.
            base_color: [0.80, 0.90, 0.70, 0.6],
            specular_strength: 1.0,
            specular_power: 96.0,
        },
    }
}
