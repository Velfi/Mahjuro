//! Procedural mesh for a small carved bone tablet — used by the gameplay
//! scene's yaku selectors and (later) cascade score tokens.
//!
//! The tablet is a rounded cuboid ("pillow"): a rounded-rectangle
//! cross-section swept along Y with a smooth pillowed top and bottom.
//! Local space spans `-0.5..+0.5` on each axis; per-instance scale
//! supplies the actual size. All side/shoulder normals are interpolated
//! across a shared vertex ring so the surface shades as a smooth glossy
//! pillow rather than a faceted chamfer.

use crate::cap_extrude::planar_y_cap_uv_xz;
use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::theme::color;
use crate::tile_glb::Vertex3dTex;

/// Corner radius as a fraction of the full extent. Drives the in-plane
/// corner rounding of the side wall *and* the pillow bulge on the top
/// and bottom caps, so the whole silhouette reads as one continuous
/// porcelain pebble. Larger values give a rounder, softer silhouette
/// at the cost of cap area — the engraved decal sits on the inset cap
/// quad, so values much above 0.25 start clipping multi-syllable yaku
/// names (e.g. "Chiitoitsu").
const RADIUS: f32 = 0.22;

/// Quadrant tessellation for the rounded vertical corners. 10 segments
/// per 90° (40 around the full perimeter) keeps the rounded corners
/// smooth even at close inspection.
const CORNER_SEGMENTS: usize = 10;

/// Number of latitude layers from the belly (vertical side) to the
/// cap rim. More layers = smoother shoulder transition where the side
/// curves up into the flat top.
const PILLOW_LAYERS: usize = 9;

/// Build the bone tablet mesh as a smooth rounded pillow.
///
/// The top cap is a flat rounded rectangle (so the engraved decal has a
/// well-defined square-ish surface). UVs use [`planar_y_cap_uv_xz`] (+u = +X, +v = +Z).
pub fn build_bone_tablet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // One perimeter sample ring (counter-clockwise when viewed from +Y).
    // Each sample carries its in-plane outward normal.
    let perimeter = build_rounded_rect_ring();
    let ring_len = perimeter.len() as u32;

    // Top half: sweep the ring along a quarter-circle profile from the
    // belly (vertical wall, full width) up to the cap rim (horizontal
    // face, inset by RADIUS).
    let top_profile = build_profile(1.0);
    let first_top = vertices.len() as u32;
    for layer in &top_profile {
        emit_ring(&mut vertices, &perimeter, layer);
    }
    stitch_rings(&mut indices, first_top, PILLOW_LAYERS, ring_len, false);

    // Bottom half: mirrored profile, flipped winding.
    let bot_profile = build_profile(-1.0);
    let first_bot = vertices.len() as u32;
    for layer in &bot_profile {
        emit_ring(&mut vertices, &perimeter, layer);
    }
    stitch_rings(&mut indices, first_bot, PILLOW_LAYERS, ring_len, true);

    // ── Top cap: triangle fan from the cap centre out to the final
    //    ring of the top profile. Cap vertices are re-emitted with +Y
    //    normals (and proper UVs) so the flat face shades crisply and
    //    the decal samples correctly. ─────────────────────────────────
    {
        let cap_ring_start = first_top + (PILLOW_LAYERS as u32 - 1) * ring_len;
        emit_cap(
            &mut vertices,
            &mut indices,
            &perimeter,
            &top_profile[PILLOW_LAYERS - 1],
            cap_ring_start,
            true,
        );
    }

    // ── Bottom cap: same, winding flipped so -Y faces outward. ──────
    {
        let cap_ring_start = first_bot + (PILLOW_LAYERS as u32 - 1) * ring_len;
        emit_cap(
            &mut vertices,
            &mut indices,
            &perimeter,
            &bot_profile[PILLOW_LAYERS - 1],
            cap_ring_start,
            false,
        );
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: color::PARCHMENT,
            specular_strength: 0.65,
            specular_power: 96.0,
        },
    }
}

/// One sample on the in-plane rounded-rectangle perimeter.
struct RingSample {
    x: f32,
    z: f32,
    /// Outward in-plane unit normal at this sample.
    nx: f32,
    nz: f32,
}

/// Build a counter-clockwise perimeter of a rounded rectangle spanning
/// `[-0.5, 0.5]` in X and Z, with corner radius `RADIUS`. Corners are
/// tessellated by `CORNER_SEGMENTS` per quadrant. Straight edges are
/// represented only by the arc endpoints that bound them — no extra
/// midpoints, because the normal is constant along each edge and the
/// stitched quads interpolate position linearly. The returned ring is
/// open (first != last); stitching wraps modulo the length.
fn build_rounded_rect_ring() -> Vec<RingSample> {
    let r = RADIUS;
    let p = 0.5 - r;

    // The four corner arc centres, each paired with the angle at which
    // the arc starts when walking the perimeter CCW from +Y view. The
    // walk begins at the +X,-Z corner, arcing from angle -π/2 up to 0
    // (the +X edge), then along the straight +X edge to the +X,+Z
    // corner, etc.
    let corners = [
        // (+X, -Z) corner: sweeps from -π/2 → 0
        ([p, -p], -std::f32::consts::FRAC_PI_2),
        // (+X, +Z) corner: 0 → π/2
        ([p, p], 0.0),
        // (-X, +Z) corner: π/2 → π
        ([-p, p], std::f32::consts::FRAC_PI_2),
        // (-X, -Z) corner: π → 3π/2
        ([-p, -p], std::f32::consts::PI),
    ];

    let mut ring: Vec<RingSample> = Vec::new();
    for (centre, start) in corners.iter() {
        // Emit CORNER_SEGMENTS + 1 samples along this arc. The last
        // sample is the *first* of the next straight edge, and will be
        // shared with the next corner's first sample — but because the
        // next corner's start angle matches this one's end angle, the
        // two are identical and we can just skip the duplicate. We
        // omit the final sample (k = CORNER_SEGMENTS) so ring walks
        // cleanly with no repeated vertices.
        for k in 0..CORNER_SEGMENTS {
            let t = k as f32 / CORNER_SEGMENTS as f32;
            let a = start + t * std::f32::consts::FRAC_PI_2;
            let nx = a.cos();
            let nz = a.sin();
            let x = centre[0] + r * nx;
            let z = centre[1] + r * nz;
            ring.push(RingSample { x, z, nx, nz });
        }
    }
    ring
}

/// One Y-layer of the vertical profile: Y position, in-plane scale
/// (1.0 = belly, shrinks toward the cap), and `tilt ∈ [0, 1]` where 0
/// means the vertex normal is purely horizontal and 1 means purely
/// vertical.
struct ProfileLayer {
    y: f32,
    scale: f32,
    tilt: f32,
}

/// Quarter-circle pillow profile. `sign = +1` builds the top half
/// (belly → top cap rim); `sign = -1` builds the bottom half.
fn build_profile(sign: f32) -> Vec<ProfileLayer> {
    let belly_y = sign * (0.5 - RADIUS);
    let mut layers = Vec::with_capacity(PILLOW_LAYERS);
    for i in 0..PILLOW_LAYERS {
        let t = i as f32 / (PILLOW_LAYERS - 1) as f32;
        let theta = t * std::f32::consts::FRAC_PI_2;
        let sin_t = theta.sin();
        // Scale shrinks the full perimeter so the ring contracts onto
        // the cap inset. At θ=π/2 the ring sits on the cap rim, which
        // is a rounded rectangle spanning [-p, p] in X/Z (same corner
        // radius as the belly, just scaled down). Because the belly
        // perimeter already spans [-0.5, 0.5], we scale by `p/0.5 = 1 - 2R`…
        // but that would collapse straight edges too much. Instead we
        // inset by `R * sin θ` on each side, giving a cap ring of half-
        // extent `0.5 - R*sinθ` — so the cap outline remains a rounded
        // rectangle with proportional corners.
        //
        // To express that as a single `scale` we need the perimeter
        // *including* the corner radius to shrink uniformly, which is
        // exact when `scale = 1 - 2R*sinθ`. At θ=π/2 the belly
        // (half-extent 0.5) becomes half-extent `0.5 - R`, and the
        // corner radius becomes `R(1 - 2R)` — slightly smaller than R,
        // which reads cleanly as "the rim's corners are tighter than
        // the belly's, like a real pillow."
        let scale = 1.0 - 2.0 * RADIUS * sin_t;
        let y = belly_y + sign * RADIUS * sin_t;
        layers.push(ProfileLayer {
            y,
            scale,
            tilt: sin_t,
        });
    }
    layers
}

/// Emit one perimeter ring of vertices for `layer`, with smooth
/// normals blended between the in-plane outward direction and ±Y.
fn emit_ring(vertices: &mut Vec<Vertex3dTex>, ring: &[RingSample], layer: &ProfileLayer) {
    let up_sign = if layer.y >= 0.0 { 1.0 } else { -1.0 };
    let horiz = (1.0 - layer.tilt).max(0.0);
    let vert = layer.tilt;
    for sample in ring {
        let x = sample.x * layer.scale;
        let z = sample.z * layer.scale;
        let nx = sample.nx * horiz;
        let ny = up_sign * vert;
        let nz = sample.nz * horiz;
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-5);
        vertices.push(Vertex3dTex {
            position: [x, layer.y, z],
            normal: [nx / len, ny / len, nz / len],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
}

/// Stitch `layers` successive rings (each `ring_len` vertices wide)
/// into a tube of quads. The ring is closed, so we wrap modulo
/// `ring_len`. `flip = true` reverses winding for the bottom half so
/// the outward face points away from the body.
fn stitch_rings(indices: &mut Vec<u32>, base: u32, layers: usize, ring_len: u32, flip: bool) {
    for layer in 0..(layers as u32 - 1) {
        let row0 = base + layer * ring_len;
        let row1 = base + (layer + 1) * ring_len;
        for i in 0..ring_len {
            let i1 = (i + 1) % ring_len;
            let a = row0 + i;
            let b = row0 + i1;
            let c = row1 + i1;
            let d = row1 + i;
            if flip {
                indices.extend_from_slice(&[a, c, b, a, d, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
}

/// Cap the top or bottom of the pillow with a triangle fan: a centre
/// vertex at `(0, layer.y, 0)` with a pure ±Y normal, plus a new ring
/// of vertices at the cap rim. The rim vertices use *smoothly blended*
/// normals matching the final side ring, so the shading transition
/// from curved shoulder into flat cap is continuous and the tablet
/// reads as one rounded pebble rather than a chiclet with a hard
/// shoulder seam. The cap geometry stays planar (so the engraved decal
/// still maps cleanly to a square top) — only the shading interpolation
/// is curved.
///
/// The rim samples share positions with the final side ring but with
/// different UVs (decal-mapped on the top cap), so we still emit fresh
/// vertices here.
fn emit_cap(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    ring: &[RingSample],
    layer: &ProfileLayer,
    _shared_ring_start: u32,
    up: bool,
) {
    let ring_len = ring.len() as u32;
    let centre_normal = if up {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, -1.0, 0.0]
    };
    // Rim normals match the side-ring blend at this layer (mostly ±Y
    // with a horizontal nudge proportional to `1 - tilt`). At the cap
    // layer `tilt` is at its maximum (sin(π/2) = 1), so the horizontal
    // component is small but non-zero — exactly what we want so light
    // wraps around the rim instead of cliff-shadowing into a hard edge.
    let up_sign = if up { 1.0 } else { -1.0 };
    let horiz = (1.0 - layer.tilt).max(0.0);
    let vert = layer.tilt;

    // UVs on the top cap: [`planar_y_cap_uv_xz`]. Bottom cap UVs are zeroed (never used).
    let centre_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, layer.y, 0.0],
        normal: centre_normal,
        uv: if up { [0.5, 0.5] } else { [0.0, 0.0] },
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });

    let rim_base = vertices.len() as u32;
    for sample in ring {
        let x = sample.x * layer.scale;
        let z = sample.z * layer.scale;
        let nx = sample.nx * horiz;
        let ny = up_sign * vert;
        let nz = sample.nz * horiz;
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-5);
        let uv = if up {
            planar_y_cap_uv_xz(x, z)
        } else {
            [0.0, 0.0]
        };
        vertices.push(Vertex3dTex {
            position: [x, layer.y, z],
            normal: [nx / len, ny / len, nz / len],
            uv,
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }

    for i in 0..ring_len {
        let i1 = (i + 1) % ring_len;
        let a = rim_base + i;
        let b = rim_base + i1;
        if up {
            indices.extend_from_slice(&[centre_idx, a, b]);
        } else {
            // Bottom cap: reverse winding so the triangle faces -Y.
            indices.extend_from_slice(&[centre_idx, b, a]);
        }
    }
}
