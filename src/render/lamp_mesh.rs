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
//! * [`build_bug_wing_mesh`]       — flat moth-shape wing, flapped per-frame (`Glass`).
//! * [`build_bug_wing_blur_mesh`]  — swept-fan motion-blur surrogate for the flapping wing (`Glass`).

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const BODY_SEGS: usize = 24;
const BULB_SEGS: usize = 16;
const BULB_LAT: usize = 12;

// ── Cord ──────────────────────────────────────────────────────────────────────
const CORD_R: f32 = 0.014;
const CORD_Z0: f32 = 0.00; // bottom of cord = shade apex
const CORD_Z1: f32 = 0.85; // top of cord = ceiling attachment

// ── Shade ─────────────────────────────────────────────────────────────────────
// Apex (small, closed) at z=0 (top, near ceiling attachment).
// Rim  (wide, open)   at z=SHADE_RIM_Z (hangs downward toward counter).
const SHADE_APEX_Z: f32 = 0.00; // top of shade (small end)
pub const SHADE_RIM_Z: f32 = -0.55; // bottom of shade (wide open end, hangs down)
const SHADE_APEX_R: f32 = 0.07; // small finial radius at apex
pub const SHADE_RIM_R: f32 = 0.72; // wide open rim
const SHADE_RIM_T: f32 = 0.018; // sheet-metal rim thickness

/// Returns the minimum horizontal radius (in mesh-local units, before scaling)
/// that a point at `local_z` must be at to lie *outside* the cone shade.
///
/// `local_z` should be ≤ 0 (below the apex). At or above the apex (z ≥ 0)
/// returns 0 (no constraint from the shade body). Below the rim returns the
/// rim radius (bugs clear the open end freely).
pub fn shade_exclusion_radius(local_z: f32) -> f32 {
    if local_z >= SHADE_APEX_Z {
        return 0.0;
    }
    // Clamp to the shade z-range.
    let t = (local_z / SHADE_RIM_Z).clamp(0.0, 1.0); // 0 at apex, 1 at rim
    SHADE_APEX_R + t * (SHADE_RIM_R - SHADE_APEX_R)
}

// ── Bulb ──────────────────────────────────────────────────────────────────────
// Sits inside the shade. z=0 is apex (near ceiling), z=SHADE_RIM_Z is open rim.
// BULB_CZ near apex so it's tucked inside and illuminates downward.
const BULB_CZ: f32 = -0.25; // center inside cone (upper third of shade)
const BULB_RH: f32 = 0.09; // horizontal radius (XY plane)
const BULB_RV: f32 = 0.10; // vertical radius (Z axis)

/// Exported: world-space Z of bulb center (relative to lamp anchor), used by
/// the scene to place the point light.
pub const BULB_Z: f32 = BULB_CZ;

// ─────────────────────────────────────────────────────────────────────────────
// Geometry helpers  (all in Z-up space: axis = Z, radial plane = XY)
// ─────────────────────────────────────────────────────────────────────────────

/// Cylinder wall along Z, constant radius.
fn add_cylinder_wall(
    verts: &mut Vec<Vertex3dTex>,
    idxs: &mut Vec<u32>,
    r: f32,
    z0: f32,
    z1: f32,
    segs: usize,
) {
    let base = verts.len() as u32;
    for si in 0..=segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        let u = si as f32 / segs as f32;
        verts.push(Vertex3dTex {
            position: [r * c, r * s, z0],
            normal: [c, s, 0.0],
            uv: [u, 0.0],
        });
        verts.push(Vertex3dTex {
            position: [r * c, r * s, z1],
            normal: [c, s, 0.0],
            uv: [u, 1.0],
        });
    }
    for si in 0..segs {
        let i = base + si as u32 * 2;
        idxs.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
    }
}

/// Frustum (truncated cone) wall along Z.
/// r0/z0 = first ring, r1/z1 = second ring.
/// `flip` reverses winding and normal sign (for inner wall).
fn add_frustum_wall(
    verts: &mut Vec<Vertex3dTex>,
    idxs: &mut Vec<u32>,
    r0: f32,
    z0: f32,
    r1: f32,
    z1: f32,
    segs: usize,
    flip: bool,
) {
    let base = verts.len() as u32;
    // Outward normal for a frustum whose apex is at larger-z and rim at smaller-z.
    // The cone opens downward: r increases as z decreases.
    let dz = z1 - z0;
    let dr = r0 - r1; // positive when rim (r0) > apex (r1)
    let len = (dr * dr + dz * dz).sqrt().max(1e-6);
    // Axial component of normal (nz) = dr/len (positive = points away from narrow end).
    // Radial component (nr) = dz/len (positive when z1>z0 and cone opens toward z0).
    let (nz, nr) = (dr / len, dz / len);
    let ns = if flip { -1.0_f32 } else { 1.0 };
    for si in 0..=segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        let u = si as f32 / segs as f32;
        verts.push(Vertex3dTex {
            position: [r0 * c, r0 * s, z0],
            normal: [ns * nr * c, ns * nr * s, ns * nz],
            uv: [u, 0.0],
        });
        verts.push(Vertex3dTex {
            position: [r1 * c, r1 * s, z1],
            normal: [ns * nr * c, ns * nr * s, ns * nz],
            uv: [u, 1.0],
        });
    }
    for si in 0..segs {
        let i = base + si as u32 * 2;
        if flip {
            idxs.extend_from_slice(&[i, i + 1, i + 2, i + 1, i + 3, i + 2]);
        } else {
            idxs.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
    }
}

/// Disc (filled circle) perpendicular to Z.
fn add_disc(
    verts: &mut Vec<Vertex3dTex>,
    idxs: &mut Vec<u32>,
    r: f32,
    z: f32,
    segs: usize,
    face_up: bool,
) {
    let nz = if face_up { 1.0_f32 } else { -1.0 };
    let center = verts.len() as u32;
    verts.push(Vertex3dTex {
        position: [0.0, 0.0, z],
        normal: [0.0, 0.0, nz],
        uv: [0.5, 0.5],
    });
    let rim = verts.len() as u32;
    for si in 0..segs {
        let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
        let (c, s) = (theta.cos(), theta.sin());
        verts.push(Vertex3dTex {
            position: [r * c, r * s, z],
            normal: [0.0, 0.0, nz],
            uv: [0.5 + 0.5 * c, 0.5 + 0.5 * s],
        });
    }
    for si in 0..segs {
        let a = rim + si as u32;
        let b = rim + (si + 1) as u32 % segs as u32;
        if face_up {
            idxs.extend_from_slice(&[center, a, b]);
        } else {
            idxs.extend_from_slice(&[center, b, a]);
        }
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
    let mut idxs: Vec<u32> = Vec::new();

    // ── Cord (thin cylinder along +Z, from apex up to ceiling) ───────────────
    add_cylinder_wall(&mut verts, &mut idxs, CORD_R, CORD_Z0, CORD_Z1, 8);
    add_disc(&mut verts, &mut idxs, CORD_R, CORD_Z1, 8, true); // top cap

    // ── Shade outer wall: apex (z=0, small) → rim (z=-0.85, wide) ────────────
    add_frustum_wall(
        &mut verts,
        &mut idxs,
        SHADE_RIM_R,
        SHADE_RIM_Z, // r0/z0 = rim (wide, low)
        SHADE_APEX_R,
        SHADE_APEX_Z, // r1/z1 = apex (small, high)
        BODY_SEGS,
        false,
    );
    // ── Shade inner wall (normals inward, lit by bulb) ────────────────────────
    add_frustum_wall(
        &mut verts,
        &mut idxs,
        SHADE_RIM_R - SHADE_RIM_T,
        SHADE_RIM_Z,
        SHADE_APEX_R - SHADE_RIM_T * 0.3,
        SHADE_APEX_Z,
        BODY_SEGS,
        true,
    );
    // ── Open-rim annulus at z=SHADE_RIM_Z (the hanging open end) ─────────────
    {
        let r_out = SHADE_RIM_R;
        let r_in = SHADE_RIM_R - SHADE_RIM_T;
        let z = SHADE_RIM_Z;
        let segs = BODY_SEGS;
        let base = verts.len() as u32;
        for si in 0..=segs {
            let theta = (si as f32) / (segs as f32) * std::f32::consts::TAU;
            let (c, s) = (theta.cos(), theta.sin());
            let u = si as f32 / segs as f32;
            // Normal faces downward (−Z) — toward the counter.
            verts.push(Vertex3dTex {
                position: [r_out * c, r_out * s, z],
                normal: [0.0, 0.0, -1.0],
                uv: [u, 0.0],
            });
            verts.push(Vertex3dTex {
                position: [r_in * c, r_in * s, z],
                normal: [0.0, 0.0, -1.0],
                uv: [u, 1.0],
            });
        }
        for si in 0..segs {
            let i = base + si as u32 * 2;
            idxs.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
    }
    // ── Finial cap at apex (z=0, small, ceiling side) — faces up ─────────────
    add_disc(
        &mut verts,
        &mut idxs,
        SHADE_APEX_R,
        SHADE_APEX_Z,
        BODY_SEGS,
        true,
    );

    MeshCpu {
        vertices: verts,
        indices: idxs,
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
    let mut idxs: Vec<u32> = Vec::new();

    let cz = BULB_CZ;
    for lat in 0..=BULB_LAT {
        let phi = std::f32::consts::PI * (lat as f32) / (BULB_LAT as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=BULB_SEGS {
            let theta = std::f32::consts::TAU * (lon as f32) / (BULB_SEGS as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            // Ellipsoid in Z-up: radial = XY, axial = Z.
            let x = BULB_RH * sin_phi * cos_t;
            let y = BULB_RH * sin_phi * sin_t;
            let z = BULB_RV * cos_phi + cz;
            let nx = sin_phi * cos_t / BULB_RH;
            let ny = sin_phi * sin_t / BULB_RH;
            let nz = cos_phi / BULB_RV;
            let nlen = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            verts.push(Vertex3dTex {
                position: [x, y, z],
                normal: [nx / nlen, ny / nlen, nz / nlen],
                uv: [lon as f32 / BULB_SEGS as f32, lat as f32 / BULB_LAT as f32],
            });
        }
    }
    let row = BULB_SEGS + 1;
    for lat in 0..BULB_LAT {
        for lon in 0..BULB_SEGS {
            let i00 = (lat * row + lon) as u32;
            let i01 = (lat * row + lon + 1) as u32;
            let i10 = ((lat + 1) * row + lon) as u32;
            let i11 = ((lat + 1) * row + lon + 1) as u32;
            idxs.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices: idxs,
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
//   length (X) = 1.0, width (Y) = 0.22, height (Z) = 0.22
//   — slender cylindrical moth thorax/abdomen (roughly round in
//   cross-section), not the flattened beetle ellipsoid that an
//   earlier revision used.
// Wing proportions:
//   span (Y each side) = 1.10, chord (X) = 0.75, flat in XY plane
//   — roughly 2.8× wingspan:body-length, typical for moths.

const BUG_BODY_SEGS: usize = 14;
const BUG_BODY_LAT: usize = 10;

/// Chitinous insect body — elongated ellipsoid along +X.
///
/// Unit bounding box: X ∈ [−0.5, +0.5], Y ∈ [−0.11, +0.11], Z ∈ [−0.11, +0.11].
/// The scene applies `extents` to scale it to world size.
pub fn build_bug_body_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs: Vec<u32> = Vec::new();

    // Ellipsoid: rx along X (length), ry along Y (width), rz along Z (height).
    // Slender, near-circular cross-section — the moth's fuzzy tube-shaped
    // abdomen-plus-thorax reads more naturally than the squat ellipsoid an
    // earlier revision used (which looked beetle-like in side view).
    let rx = 0.50_f32;
    let ry = 0.11_f32;
    let rz = 0.11_f32;

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
            let nlen = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            verts.push(Vertex3dTex {
                position: [x, y, z],
                normal: [nx / nlen, ny / nlen, nz / nlen],
                uv: [
                    lon as f32 / BUG_BODY_SEGS as f32,
                    lat as f32 / BUG_BODY_LAT as f32,
                ],
            });
        }
    }
    let row = BUG_BODY_SEGS + 1;
    for lat in 0..BUG_BODY_LAT {
        for lon in 0..BUG_BODY_SEGS {
            let i00 = (lat * row + lon) as u32;
            let i01 = (lat * row + lon + 1) as u32;
            let i10 = ((lat + 1) * row + lon) as u32;
            let i11 = ((lat + 1) * row + lon + 1) as u32;
            idxs.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices: idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Enamel,
            // Dark iridescent chitin — deep olive-green, almost black.
            base_color: [0.08, 0.12, 0.06, 1.0],
            specular_strength: 0.90,
            specular_power: 64.0,
        },
    }
}

/// The moth-wing silhouette, sampled as a closed outline in the
/// (X, Y) plane. Y = 0 is the hinge edge (root); the wing extends
/// into +Y. Shared between the live wing mesh and the motion-blur
/// swept-fan mesh so their silhouettes match exactly.
fn moth_wing_outline() -> &'static [[f32; 2]] {
    // Scaled for a realistic moth wingspan:body ratio of ~2.8:1 —
    // body length is 1.0 unit (X ∈ [−0.5, 0.5]), each wing extends
    // ~1.13 units outward in +Y, so full wingspan ≈ 2.26 units.
    &[
        // --- forewing leading edge (forward, +X side) ---
        [0.30, 0.00], // root, forward corner (hinge, +X end)
        [0.40, 0.16],
        [0.51, 0.38],
        [0.59, 0.62],
        [0.61, 0.84],
        [0.54, 1.03],
        // --- forewing apex & outer margin ---
        [0.41, 1.13], // forewing apex (pointed tip, swept forward)
        [0.22, 1.11],
        [0.03, 1.03],
        [-0.14, 0.89],
        // --- notch between forewing and hindwing ---
        [-0.19, 0.73],
        [-0.14, 0.62],
        // --- hindwing outer lobe ---
        [-0.27, 0.54],
        [-0.38, 0.41],
        [-0.41, 0.27],
        [-0.35, 0.16],
        [-0.27, 0.08],
        // --- close at root (aft corner) ---
        [-0.19, 0.00],
    ]
}

/// A single moth wing in the +Y half-plane, hinged at Y = 0.
///
/// The mesh is a flat fan of triangles traced around a realistic
/// moth-wing silhouette: a larger triangular forewing with a swept-
/// forward apex and a smaller rounded hindwing lobe, joined by a
/// shallow notch. Spans roughly Y ∈ [0, 1.13] and X ∈ [-0.41, 0.61]
/// in unit space. All
/// verts sit on Z = 0 so the shape is two-sided (both faces emitted
/// via duplicated triangles with opposite winding and flipped normals).
///
/// The scene draws this mesh twice per bug — once for the left wing
/// (identity Y) and once for the right wing (transform with −Y scale),
/// each rotated about the body's +X axis by a per-frame flap angle.
/// Keeping both wings as copies of the same mesh lets the renderer
/// share one vertex buffer while still animating each wing independently.
pub fn build_bug_wing_mesh() -> MeshCpu {
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs: Vec<u32> = Vec::new();

    // Silhouette shared with the motion-blur fan; see `moth_wing_outline`.
    // Axis convention: +X is forward along the body, -X is aft, and +Y
    // spans outward from the hinge at Y = 0. The shape is traced as a
    // triangle fan from the root midpoint so every triangle shares the
    // hinge — a rotation around +X (flapping) keeps the root stationary.
    let outline = moth_wing_outline();
    // Fan anchor at the hinge midpoint (between the two root corners).
    let anchor: [f32; 2] = [0.06, 0.00];

    // Emit two copies of the fan: one with +Z normal (top face), one
    // with -Z normal (bottom face) and reversed winding. Both share the
    // same outline so the wing reads from both sides.
    for nz in [1.0_f32, -1.0_f32] {
        let base = verts.len() as u32;
        // Anchor vertex (index 0 relative to `base`).
        verts.push(Vertex3dTex {
            position: [anchor[0], anchor[1], 0.0],
            normal: [0.0, 0.0, nz],
            uv: [0.5, 0.0],
        });
        // Outline vertices (indices 1..=outline.len() relative to base).
        for p in outline {
            // UV: U along body axis (mapped to [0,1]), V along span.
            verts.push(Vertex3dTex {
                position: [p[0], p[1], 0.0],
                normal: [0.0, 0.0, nz],
                uv: [(p[0] + 0.41) / 1.02, p[1] / 1.13],
            });
        }
        // Fan triangles. Winding flips between top and bottom faces so
        // each side is outward-facing.
        let n = outline.len() as u32;
        for k in 0..(n - 1) {
            let a = base; // anchor
            let b = base + 1 + k; // outline[k]
            let c = base + 1 + k + 1; // outline[k+1]
            if nz > 0.0 {
                idxs.extend_from_slice(&[a, b, c]);
            } else {
                idxs.extend_from_slice(&[a, c, b]);
            }
        }
    }

    MeshCpu {
        vertices: verts,
        indices: idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Glass,
            // Pale iridescent wings — warm translucent amber.
            base_color: [0.80, 0.90, 0.70, 0.6],
            specular_strength: 1.0,
            specular_power: 96.0,
        },
    }
}

/// The wing-arc swept volume — a motion-blur stand-in for a flapping
/// wing that the eye can't resolve at real moth wingbeat rates
/// (~25 Hz). Where [`build_bug_wing_mesh`] is a flat plane hinged at
/// Y = 0 and rotated per-frame about +X, this mesh is the *union of
/// every position* that plane passes through during a full flap
/// stroke, drawn once at rest.
///
/// Construction: the same moth outline is swept around the +X axis
/// (the flap hinge) from `-FLAP_AMP` to `+FLAP_AMP`. Each outline
/// point at local `(x, y, 0)` traces an arc of radius `y` in the Y-Z
/// plane at constant X, so the swept surface looks like a curved
/// ribbon following the wing's arc of travel. We emit quads between
/// adjacent (outline-index, angle-step) pairs, with both faces so the
/// shape is visible from above and below. Normals point radially
/// outward from the +X axis, giving a soft shaded look when lit.
///
/// The scene draws this mesh at full opacity scaled by an alpha that
/// tracks the wing's angular speed — near the flap turnarounds the
/// wing is momentarily still and the live mesh reads crisply; near
/// mid-stroke the wing is invisible in flight and the swept fan
/// takes over. This is how 1/60 s shutter exposures of moths actually
/// look: a static-looking body flanked by blurred wing fans.
pub fn build_bug_wing_blur_mesh() -> MeshCpu {
    // Match the live wing's amplitude. Slightly padded so the blur fan
    // visibly extends past the crisp wing's extremes even at rest.
    const FLAP_AMP: f32 = 1.15;
    // Angle subdivisions across the sweep. 12 steps over ~130° is ~11°
    // per step — fine enough that the fan silhouette reads as a smooth
    // curve rather than a polyhedron, while keeping the mesh small.
    const ANGLE_STEPS: usize = 12;

    let outline = moth_wing_outline();
    let mut verts: Vec<Vertex3dTex> = Vec::new();
    let mut idxs: Vec<u32> = Vec::new();

    // Pre-compute the rotated positions for every (outline_i, step_j)
    // pair. Laid out as a flat array indexed as `i * (ANGLE_STEPS+1) + j`.
    let steps = ANGLE_STEPS + 1; // inclusive endpoints
    let n = outline.len();
    for (i, p) in outline.iter().enumerate() {
        for j in 0..steps {
            let t = j as f32 / ANGLE_STEPS as f32; // 0..=1
            let theta = -FLAP_AMP + t * (2.0 * FLAP_AMP);
            let (s, c) = theta.sin_cos();
            let x = p[0];
            let y = p[1] * c;
            let z = p[1] * s;
            // Normal: perpendicular to the swept surface. The surface
            // is generated by rotating a planar outline around +X, so
            // at each point the normal lies in the Y-Z plane and is
            // radially outward from the +X axis. For y = 0 (hinge
            // points) this would be singular; fall back to +Z there.
            let normal = if p[1].abs() < 1.0e-4 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, c, s]
            };
            // UV: U is the outline parameter (how far around the
            // silhouette), V is the sweep angle (0 at -amp, 1 at +amp).
            // This gives the fan a consistent gradient for any future
            // alpha-textured treatment.
            let u = i as f32 / (n - 1) as f32;
            let v = t;
            verts.push(Vertex3dTex {
                position: [x, y, z],
                normal,
                uv: [u, v],
            });
        }
    }

    // Quads between adjacent outline segments and angle steps.
    // Emit both face windings so the fan is visible from both sides —
    // at every flap angle the viewer could be looking at the "top"
    // or "bottom" of the sweep.
    let idx = |i: usize, j: usize| -> u32 { (i * steps + j) as u32 };
    for i in 0..(n - 1) {
        for j in 0..ANGLE_STEPS {
            let a = idx(i, j);
            let b = idx(i + 1, j);
            let c = idx(i + 1, j + 1);
            let d = idx(i, j + 1);
            // Front face
            idxs.extend_from_slice(&[a, b, c, a, c, d]);
            // Back face (reversed winding)
            idxs.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }

    MeshCpu {
        vertices: verts,
        indices: idxs,
        default_material: MaterialParams {
            kind: MaterialKind::Glass,
            // Same warm amber as the live wing but extra-translucent;
            // per-instance alpha is further scaled by flap speed at
            // draw time. The fan mostly reads as a shimmer.
            base_color: [0.80, 0.90, 0.70, 0.18],
            specular_strength: 0.4,
            specular_power: 48.0,
        },
    }
}
