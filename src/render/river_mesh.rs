//! Procedural mesh for the discard river — a miniature stream with a
//! meandering centerline, variable width, and scattered rocky banks.
//!
//! Design notes:
//! - The water surface is not a rectangle. The centerline wanders in Z
//!   as a smooth function of X (a low-frequency sinusoid), and the
//!   half-width pulses along X so the stream visibly constricts and
//!   widens. The surface is built as a triangle strip of quads across
//!   many slices along the flow axis.
//! - Banks are NOT four walls. They are a scatter of ~30 small stone
//!   boxes of varying height, width, and depth, tucked just outside the
//!   moving water edge. This reads as a pebble-lined bank rather than a
//!   clean trough.
//! - The river runs off both ends of the mesh: the water surface
//!   reaches `±CHAN_X` on the flow axis and the ends are not capped.
//!   A few larger rocks are still placed at the outlets so the
//!   silhouette has visible framing without walling the water in.
//! - The stone bed beneath the water has small dome bumps so underwater
//!   silhouettes through the surface read as stones, not a flat floor.
//!
//! The mesh is still a single draw-call primitive using
//! `MaterialKind::Water`. The shader continues to branch on `uv.y`:
//! `uv.y > 0.5` is water, otherwise stone.
//!
//! Local space still spans `[-0.5, +0.5]` on X and Z so the gameplay
//! raycast AABB stays compatible. Scene code supplies non-uniform
//! extents (X longer than Z) to make the river read as a stream.

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

// Flow-axis half-extent (local X). The water reaches nearly to the
// local bounds so the stream runs off both ends of the footprint.
const CHAN_X: f32 = 0.48;
// Total cross-flow half-extent (local Z). The bank rocks extend out
// toward this bound; the water itself stays well inside.
const CHAN_Z: f32 = 0.42;
// Water surface parameters. `BASE_HALF_W` is the nominal half-width of
// the water strip; actual width modulates above and below this.
const BASE_HALF_W: f32 = 0.15;
const WIDTH_VARIATION: f32 = 0.055;
// Centerline meander amplitude (Z offset) and wavelength count along X.
const MEANDER_AMP: f32 = 0.085;
const MEANDER_FREQ: f32 = 1.6; // ~1.6 full wobbles across the length
// Vertical stack: floor (with bumps), water surface slightly above,
// top of the bank rocks above that.
const FLOOR_Y: f32 = -0.15;
const WATER_Y: f32 = -0.04;
const BANK_MAX_Y: f32 = 0.06;

// Number of slices used to tessellate the meandering water surface
// along the flow axis. Higher = smoother meander silhouette.
const WATER_SLICES: usize = 48;

/// Local-space AABB half-extents used by the gameplay raycast picker.
/// Tight to the water surface strip — NOT the full bank-rock footprint.
/// The outer rocks extend to `±CHAN_Z` in local space, but the player
/// clicks the *river*, so the hit region should track the water channel.
/// Using the full mesh bounds here made the river's world-space AABB
/// reach sideways far enough to swallow the adjacent bronze mirror's
/// pick zone, blocking mirror hover/labels.
pub const RIVER_LOCAL_HALF: [f32; 3] = [
    CHAN_X,
    (BANK_MAX_Y - FLOOR_Y) * 0.5,
    // Widest water half-width — the spring pool at t=0 is the broadest
    // point — plus the meander amplitude budget so the hit rect matches
    // where the water surface actually appears on-screen. Still well
    // under CHAN_Z, so the picker does not overlap the bronze mirror.
    BASE_HALF_W + WIDTH_VARIATION + SPRING_BULGE + MEANDER_AMP,
];
pub const RIVER_LOCAL_CENTER_Y: f32 = (BANK_MAX_Y + FLOOR_Y) * 0.5;

/// Centerline Z offset as a function of the flow-axis parameter t in [0, 1].
/// A smooth low-frequency sinusoid plus a tiny secondary term so the
/// meander is not perfectly symmetric.
fn centerline_z(t: f32) -> f32 {
    let phase = t * std::f32::consts::TAU * MEANDER_FREQ;
    let primary = (phase).sin() * MEANDER_AMP;
    let secondary = (phase * 2.3 + 0.7).sin() * (MEANDER_AMP * 0.22);
    primary + secondary
}

/// Half-width of the water strip at flow-axis parameter t in [0, 1].
fn half_width(t: f32) -> f32 {
    // Two slow sinusoids at incommensurate frequencies so the width
    // variation doesn't align with the meander period.
    let phase = t * std::f32::consts::TAU;
    let a = (phase * 1.1 + 1.3).sin();
    let b = (phase * 0.6 - 0.4).sin();
    let base = BASE_HALF_W + WIDTH_VARIATION * (a * 0.6 + b * 0.4);
    // Source-end bulge. At t=0 the stream opens into a roughly circular
    // spring pool; by t=SPRING_T the width tapers back into the normal
    // stream width. Smoothstep keeps the transition smooth.
    let bulge = SPRING_BULGE * (1.0 - smoothstep01(t / SPRING_T));
    base + bulge
}

/// Cubic smoothstep on [0, 1].
fn smoothstep01(x: f32) -> f32 {
    let c = x.clamp(0.0, 1.0);
    c * c * (3.0 - 2.0 * c)
}

/// Flow-axis fraction over which the spring pool tapers into the
/// regular stream. Kept in sync with the shader constant of the same
/// name so the visual spring-effect region matches the geometric bulge.
pub const SPRING_T: f32 = 0.18;
/// Extra half-width added to the water strip at the source.
pub const SPRING_BULGE: f32 = 0.11;

/// Deterministic hash → [0, 1). Lets the rocks be stable across runs
/// without pulling in an RNG dependency.
fn hash01(i: u32, seed: u32) -> f32 {
    let mut x = i
        .wrapping_mul(747796405)
        .wrapping_add(seed.wrapping_mul(2891336453));
    x ^= x >> 16;
    x = x.wrapping_mul(2246822519);
    x ^= x >> 13;
    x = x.wrapping_mul(3266489917);
    x ^= x >> 16;
    (x & 0x00FFFFFF) as f32 / 16_777_216.0
}

/// Build the full river mesh.
pub fn build_river_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // ── Stone bed ───────────────────────────────────────────────────
    // A thin slab under the whole footprint catches shadows and sits
    // beneath the water. Kept narrow to the rock scatter bounds so it
    // doesn't peek out past the banks.
    push_stone_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            -CHAN_X,
            CHAN_X,
            FLOOR_Y - 0.04,
            FLOOR_Y,
            -CHAN_Z * 0.78,
            CHAN_Z * 0.78,
        ),
    );

    // Ring of taller rocks framing the spring pool at the -X source.
    scatter_spring_ring(&mut vertices, &mut indices);

    // A handful of dome-ish stones on the bed so shallow water shows
    // silhouettes through it. These are just short wide boxes — the
    // shader's stone branch adds speckle, which is plenty of detail at
    // this scale.
    for i in 0..7u32 {
        let t = (i as f32 + 0.5) / 7.0;
        let x = -CHAN_X + t * 2.0 * CHAN_X;
        let cz = centerline_z(t);
        let jitter_z = (hash01(i, 0xBEEF) - 0.5) * 0.04;
        let jx = (hash01(i, 0xCAFE) - 0.5) * 0.02;
        let w = 0.025 + hash01(i, 0x1234) * 0.02;
        let d = 0.02 + hash01(i, 0x5678) * 0.015;
        let h = 0.015 + hash01(i, 0x9ABC) * 0.015;
        push_stone_box(
            &mut vertices,
            &mut indices,
            Aabb::new(
                x + jx - w,
                x + jx + w,
                FLOOR_Y,
                FLOOR_Y + h,
                cz + jitter_z - d,
                cz + jitter_z + d,
            ),
        );
    }

    // ── Water surface ───────────────────────────────────────────────
    // Tessellate the meandering strip into a long ribbon of quads. UVs
    // set uv.y = 1.0 (water branch) and uv.x stores the normalized
    // flow-axis parameter so the shader can do band shading if it wants.
    push_water_ribbon(&mut vertices, &mut indices);

    // ── Bank rocks ──────────────────────────────────────────────────
    // Scatter rocks just outside the water edge on both banks. Sizes
    // vary; taller ones dominate the silhouette near the middle while
    // the outlets stay low and open so the river reads as flowing
    // through. Two passes: a back row (close to the water) and an
    // outer row (tucked against the local-Z bounds).
    scatter_bank_rocks(&mut vertices, &mut indices, Bank::Near, 0);
    scatter_bank_rocks(&mut vertices, &mut indices, Bank::Far, 1);
    scatter_outer_rocks(&mut vertices, &mut indices);

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Water,
            base_color: [0.06, 0.09, 0.18, 1.0],
            specular_strength: 0.85,
            specular_power: 220.0,
        },
    }
}

#[derive(Copy, Clone)]
enum Bank {
    /// -Z side of the river (the "near" bank relative to the generator).
    Near,
    /// +Z side.
    Far,
}

/// Tessellate the water surface. Each slice along X generates two
/// vertices (one on each edge of the stream at that X). UV.y = 1.0
/// flags water; UV.x stores the flow-axis parameter in [0, 1] so the
/// shader can derive flow-aligned quantities if it ever needs them.
fn push_water_ribbon(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    let n = [0.0, 1.0, 0.0];
    let base = vertices.len() as u32;
    for i in 0..=WATER_SLICES {
        let t = i as f32 / WATER_SLICES as f32;
        let x = -CHAN_X + t * 2.0 * CHAN_X;
        let cz = centerline_z(t);
        let hw = half_width(t);
        let u = t;
        vertices.push(Vertex3dTex {
            position: [x, WATER_Y, cz - hw],
            normal: n,
            uv: [u, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x, WATER_Y, cz + hw],
            normal: n,
            uv: [u, 1.0],
        });
    }
    // Two triangles per quad between consecutive slices.
    for i in 0..WATER_SLICES as u32 {
        let a = base + i * 2;
        let b = base + i * 2 + 1;
        let c = base + i * 2 + 2;
        let d = base + i * 2 + 3;
        indices.extend_from_slice(&[a, c, d, a, d, b]);
    }
}

/// Scatter small rocks along one bank of the river, just outside the
/// water edge. Rocks are jittered in Z to form a cluttered shoreline,
/// and in height/size to produce varied silhouettes. `seed` lets the
/// two banks generate different patterns.
fn scatter_bank_rocks(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    bank: Bank,
    seed_base: u32,
) {
    const COUNT: u32 = 26;
    let sign: f32 = match bank {
        Bank::Near => -1.0,
        Bank::Far => 1.0,
    };
    for i in 0..COUNT {
        let t = (i as f32 + hash01(i, 0x111 + seed_base) * 0.6) / COUNT as f32;
        // Leave the spring source region clear — those rocks are placed
        // separately as a ring so they frame the pool.
        if t < SPRING_T {
            continue;
        }
        let x = -CHAN_X + t * 2.0 * CHAN_X;
        let cz = centerline_z(t);
        let hw = half_width(t);
        // Rock sits just outside the water edge with a small outward
        // offset that grows slightly toward the middle, so the banks
        // thicken where the river is pinched and thin out at the
        // outlets (keeping the ends open).
        let openness = 1.0 - (t - 0.5).abs() * 2.0; // 0 at ends, 1 at middle
        let outward = 0.012 + hash01(i, 0x222 + seed_base) * 0.040 * openness;
        let z_edge = cz + sign * (hw + outward);
        // Rock box size. Taller rocks near the middle for silhouette,
        // shorter near the outlets so the stream reads as continuing.
        let h_max = 0.04 + openness * 0.10;
        let h = 0.018 + hash01(i, 0x333 + seed_base) * h_max;
        let w = 0.022 + hash01(i, 0x444 + seed_base) * 0.030; // along X
        let d = 0.022 + hash01(i, 0x555 + seed_base) * 0.030; // along Z
        // Small lateral jitter, plus a touch of X jitter so rocks
        // don't line up on even spacing.
        let jx = (hash01(i, 0x666 + seed_base) - 0.5) * 0.02;
        let jz = (hash01(i, 0x777 + seed_base) - 0.5) * 0.015;
        let cx = x + jx;
        let cz_rock = z_edge + sign * d + jz; // rock sits entirely on the bank side
        // Clamp so rocks don't poke past the local-Z footprint.
        let z0 = (cz_rock - d).clamp(-CHAN_Z, CHAN_Z);
        let z1 = (cz_rock + d).clamp(-CHAN_Z, CHAN_Z);
        if (z1 - z0).abs() < 0.002 {
            continue;
        }
        push_stone_box(
            vertices,
            indices,
            Aabb::new(cx - w, cx + w, FLOOR_Y, FLOOR_Y + h, z0.min(z1), z0.max(z1)),
        );
    }
}

/// Local-space center of the spring pool (on the water surface). The
/// shader reads the same constants (via hard-coded literals in the
/// `is_spring` branch) to align the radial ripples with the geometry.
pub const SPRING_CX: f32 = -CHAN_X + SPRING_POOL_R + 0.02;
pub const SPRING_POOL_R: f32 = BASE_HALF_W + WIDTH_VARIATION + SPRING_BULGE;

/// Place a ring of taller rocks around the spring pool. The ring
/// wraps roughly 220° of a circle — leaving an opening on the +X
/// downstream side so the spring clearly feeds the river.
fn scatter_spring_ring(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    const COUNT: u32 = 16;
    // Spring pool center matches the geometry: just inside the -X
    // boundary on the water centerline (t=0 puts cz at centerline_z(0)).
    let cx = SPRING_CX;
    let cz = centerline_z(0.0);
    // Arc from ~120° to ~240° (measured from +X), wrapping around the
    // -X, -Z, and +Z sides. The +X face of the pool stays open so the
    // stream can exit into the channel proper.
    let a0: f32 = 120.0_f32.to_radians();
    let a1: f32 = 360.0_f32.to_radians() - a0;
    for i in 0..COUNT {
        let t = (i as f32 + hash01(i, 0xEEEE) * 0.4) / COUNT as f32;
        let ang = a0 + (a1 - a0) * t;
        // Rocks sit just outside the pool radius, with a little radial
        // jitter so the ring doesn't read as a perfect circle.
        let r_out = SPRING_POOL_R + 0.020 + hash01(i, 0xFFF1) * 0.035;
        let rx = ang.cos();
        let rz = ang.sin();
        let rock_cx = cx + r_out * rx;
        let rock_cz = cz + r_out * rz;
        // Larger and taller than the regular bank rocks: this is the
        // dramatic framing for the spring.
        let w = 0.028 + hash01(i, 0xFFF2) * 0.040;
        let d = 0.028 + hash01(i, 0xFFF3) * 0.040;
        let h = 0.055 + hash01(i, 0xFFF4) * 0.080;
        // Tip each rock slightly off axis-aligned by using slightly
        // different half-widths in the two axes of its footprint — cheap
        // way to get varied silhouettes without rotating boxes.
        let w_x = w * (0.8 + hash01(i, 0xFFF5) * 0.4);
        let d_z = d * (0.8 + hash01(i, 0xFFF6) * 0.4);
        // Clamp so a rock near the edge of the local footprint doesn't
        // poke outside the mesh bounds.
        let x0 = (rock_cx - w_x).max(-CHAN_X);
        let x1 = (rock_cx + w_x).max(-CHAN_X + 0.002);
        let z0 = (rock_cz - d_z).clamp(-CHAN_Z, CHAN_Z);
        let z1 = (rock_cz + d_z).clamp(-CHAN_Z, CHAN_Z);
        if (x1 - x0).abs() < 0.003 || (z1 - z0).abs() < 0.003 {
            continue;
        }
        push_stone_box(
            vertices,
            indices,
            Aabb::new(
                x0.min(x1),
                x0.max(x1),
                FLOOR_Y,
                FLOOR_Y + h,
                z0.min(z1),
                z0.max(z1),
            ),
        );
    }
    // A few extra small "splash" rocks just inside the pool lip — the
    // kind of half-submerged stones a real spring collects around its
    // eye. These poke up a bit out of the water surface.
    for i in 0..5u32 {
        let ang = a0 + (a1 - a0) * ((i as f32 + 0.5) / 5.0);
        let r_in = SPRING_POOL_R * (0.55 + hash01(i, 0xFFF7) * 0.25);
        let rx = ang.cos();
        let rz = ang.sin();
        let px = cx + r_in * rx;
        let pz = cz + r_in * rz;
        let w = 0.016 + hash01(i, 0xFFF8) * 0.018;
        let d = 0.016 + hash01(i, 0xFFF9) * 0.018;
        let h = 0.028 + hash01(i, 0xFFFA) * 0.030;
        push_stone_box(
            vertices,
            indices,
            Aabb::new(px - w, px + w, FLOOR_Y, FLOOR_Y + h, pz - d, pz + d),
        );
    }
}

/// A thinner outer ring of small rocks near the local-Z edges to frame
/// the footprint without looking like a wall.
fn scatter_outer_rocks(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    const COUNT: u32 = 18;
    for side in 0..2u32 {
        let sign: f32 = if side == 0 { -1.0 } else { 1.0 };
        for i in 0..COUNT {
            let t = (i as f32 + hash01(i, 0x888 + side) * 0.5) / COUNT as f32;
            let x = -CHAN_X + t * 2.0 * CHAN_X;
            let w = 0.018 + hash01(i, 0x999 + side) * 0.020;
            let d = 0.018 + hash01(i, 0xAAA + side) * 0.022;
            let h = 0.012 + hash01(i, 0xBBB + side) * 0.030;
            let jz = hash01(i, 0xCCC + side) * 0.05;
            let z_edge = sign * (CHAN_Z - 0.02 - jz);
            let cz_rock = z_edge - sign * d;
            let jx = (hash01(i, 0xDDD + side) - 0.5) * 0.04;
            let cx = x + jx;
            push_stone_box(
                vertices,
                indices,
                Aabb::new(
                    cx - w,
                    cx + w,
                    FLOOR_Y,
                    FLOOR_Y + h,
                    cz_rock - d,
                    cz_rock + d,
                ),
            );
        }
    }
}

/// Append a stone box to the mesh. Reuses [`push_box`] and stamps
/// every vertex's UV.y to `0.0` so the shader treats the fragments as
/// stone, not water. UV.x encodes a stable per-vertex coordinate used
/// by the shader's stone speckle noise.
fn push_stone_box(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>, aabb: Aabb) {
    let start = vertices.len();
    push_box(vertices, indices, aabb);
    for v in &mut vertices[start..] {
        v.uv[1] = 0.0;
    }
}
