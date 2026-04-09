//! Procedural mesh for the discard river — a small stone trough holding a
//! flowing water surface. Replaces the older lacquered bowl: tiles get
//! "thrown into the river", splash, drift downstream, and fade out, which
//! reads as a discard target far more clearly than a static bowl.
//!
//! The mesh is a single primitive (one draw call) using
//! `MaterialKind::Water`. The shader branches per fragment on `uv.y`:
//!
//!   - `uv.y > 0.5` → animated indigo water surface (scrolling normals)
//!   - `uv.y <= 0.5` → dark stone trough (walls, floor, underside)
//!
//! Local space spans `[-0.5, +0.5]` on X (the channel runs along +X) and
//! `[-0.5, +0.5]` on Z (channel width). Vertical extents are `[-0.15, +0.05]`.
//! Per-instance scale supplies the actual table size — currently driven
//! by the same `bw.min(bh)` diameter the bowl used, so the river drops
//! into the bowl's footprint without retouching gameplay layout code.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

// Channel bounds (X = flow axis, Z = width axis).
const CHAN_X: f32 = 0.46; // outer half-extent along flow
const CHAN_Z: f32 = 0.30; // outer half-extent across flow
const WALL_THICK: f32 = 0.05;
// Vertical band: `FLOOR_Y` is the stone floor, `WATER_Y` is the water
// surface plane (slightly recessed inside the rim), `RIM_Y` is the top of
// the stone walls.
const FLOOR_Y: f32 = -0.15;
const WATER_Y: f32 = -0.02;
const RIM_Y: f32 = 0.05;

/// Local-space AABB half-extents tight to the river mesh, used by the
/// gameplay raycast picker (the `DiscardBowl` pick variant still routes
/// through this AABB; renaming the variant is a separate cleanup).
pub const RIVER_LOCAL_HALF: [f32; 3] = [CHAN_X, (RIM_Y - FLOOR_Y) * 0.5, CHAN_Z];
pub const RIVER_LOCAL_CENTER_Y: f32 = (RIM_Y + FLOOR_Y) * 0.5;

/// Build the stone trough + water surface mesh. UV.y encodes the
/// per-fragment material switch (water vs stone) consumed by
/// `lit_mesh.wgsl`'s `Water` branch.
pub fn build_river_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // ── Stone underside disc (one large box, capped just below FLOOR_Y so
    //    the trough has a solid bottom that catches shadows).
    push_stone_box(
        &mut vertices,
        &mut indices,
        -CHAN_X,
        CHAN_X,
        FLOOR_Y - 0.04,
        FLOOR_Y,
        -CHAN_Z,
        CHAN_Z,
    );

    // ── Stone walls — four boxes around the perimeter, leaving a
    //    rectangular well in the middle for the water surface.
    let inner_x = CHAN_X - WALL_THICK;
    let inner_z = CHAN_Z - WALL_THICK;
    // -X end cap
    push_stone_box(
        &mut vertices,
        &mut indices,
        -CHAN_X,
        -inner_x,
        FLOOR_Y,
        RIM_Y,
        -CHAN_Z,
        CHAN_Z,
    );
    // +X end cap
    push_stone_box(
        &mut vertices,
        &mut indices,
        inner_x,
        CHAN_X,
        FLOOR_Y,
        RIM_Y,
        -CHAN_Z,
        CHAN_Z,
    );
    // -Z side wall (between the end caps)
    push_stone_box(
        &mut vertices,
        &mut indices,
        -inner_x,
        inner_x,
        FLOOR_Y,
        RIM_Y,
        -CHAN_Z,
        -inner_z,
    );
    // +Z side wall
    push_stone_box(
        &mut vertices,
        &mut indices,
        -inner_x,
        inner_x,
        FLOOR_Y,
        RIM_Y,
        inner_z,
        CHAN_Z,
    );

    // ── Water surface — a single quad slightly inside the well lip. This
    //    is the only part of the mesh that uses uv.y = 1.0, which the
    //    shader reads as the "this is water" flag and runs the scrolling
    //    surface animation on.
    let wx = inner_x - 0.005;
    let wz = inner_z - 0.005;
    push_water_quad(
        &mut vertices,
        &mut indices,
        -wx,
        wx,
        WATER_Y,
        -wz,
        wz,
    );

    MeshCpu {
        vertices,
        indices,
        // Indigo base for the water; the shader's water branch overrides
        // albedo per-fragment with a procedural foam tint, so this is
        // mostly a fallback for any future stone-only fragments that want
        // a uniform base. Kept dark so the trough silhouette reads from
        // across the table.
        default_material: MaterialParams {
            kind: MaterialKind::Water,
            base_color: [0.06, 0.09, 0.18, 1.0],
            specular_strength: 0.85,
            specular_power: 220.0,
        },
    }
}

/// Append a stone box to the mesh. Reuses [`push_box`] (the shared box
/// helper used by plaque/ofuda/tablets) but stamps every vertex's UV.y to
/// `0.0` so the shader treats the fragments as stone, not water.
fn push_stone_box(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    z0: f32,
    z1: f32,
) {
    let start = vertices.len();
    push_box(vertices, indices, x0, x1, y0, y1, z0, z1);
    for v in &mut vertices[start..] {
        // UV.y = 0.0 → stone branch in the water shader. UV.x stays as
        // `push_box` set it (used only as a stable input for procedural
        // noise on stone, not for sampling).
        v.uv[1] = 0.0;
    }
}

/// Append the water surface quad. Two triangles, all four vertices
/// stamped with UV.y = 1.0 so the shader takes the animated-water branch.
/// UV.x maps the flow axis 0..1 across the channel — the shader uses
/// this to scroll the surface in the +X direction.
fn push_water_quad(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    x0: f32,
    x1: f32,
    y: f32,
    z0: f32,
    z1: f32,
) {
    let n = [0.0, 1.0, 0.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [x0, y, z0],
        normal: n,
        uv: [0.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [x1, y, z0],
        normal: n,
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [x1, y, z1],
        normal: n,
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [x0, y, z1],
        normal: n,
        uv: [0.0, 1.0],
    });
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}
