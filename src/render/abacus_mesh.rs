//! Procedural mesh for the shop counter's reroll prop — a Chinese
//! suanpan abacus.
//!
//! Frame is laid out in the local XZ plane (X = horizontal, Z = vertical).
//! Each bead is a **thin right cylinder with axis +Y** (circular profile in
//! XZ) sitting just in front of the frame — toward the camera after the
//! shop's [`crate::render::draw_cmd::camera_facing_euler_xyz_rad`] — so disks read
//! as round beads, not thick rectangular slabs.
//! Local space spans `-0.5..+0.5` on each axis; per-instance `extents` size it.
//!
//! The cost number for the restock action is **not** encoded in bead
//! positions — beads are baked at rest and the cost is shown on the
//! hover plaque.
//!
//! Suanpan layout:
//!   - 7 vertical bead rods (columns)
//!   - Horizontal reckoning beam splits the frame into upper "heaven"
//!     (2 beads/column) and lower "earth" (5 beads/column) decks
//!   - All beads at rest position: heaven beads pulled to top, earth
//!     beads pulled to bottom
//!
//! Heaven (upper) and earth (lower) beads are **two** registered meshes so
//! the shop can tint suanpan decks separately — traditional layouts often use
//! distinct glaze colours for the two decks.
//!
//! No decal texture is bound — the abacus is purely geometric.

use crate::render::lit_mesh::{
    Aabb, MaterialKind, MaterialParams, MeshCpu, push_box, push_cylinder_y,
};
use crate::render::theme::color;
use crate::render::tile_glb::Vertex3dTex;

/// Number of vertical bead rods (columns).
const N_RODS: usize = 7;
/// Beads in the upper "heaven" deck per column.
const HEAVEN_BEADS: usize = 2;
/// Beads in the lower "earth" deck per column.
const EARTH_BEADS: usize = 5;

/// Frame thickness (Y-axis): the frame's +Y face is the camera-facing
/// plane after `cam_rot`. Beads stick further out in +Y so they read
/// against the frame from a near-overhead camera.
const FRAME_HALF_Y: f32 = 0.11;
/// Frame rail width (the wood bordering the bead grid).
const RAIL_W: f32 = 0.06;
/// Reckoning beam thickness (Z-axis).
const BEAM_HALF_Z: f32 = 0.025;
/// Bead rod half-width (X-axis): rods are thin verticals.
const ROD_HALF_X: f32 = 0.008;
/// Beads are thin cylinders (axis +Y): radius in XZ, shallow thickness in Y.
const BEAD_RADIUS: f32 = 0.032;
/// Sit disks flush against the front (toward +Y) of the lacquered frame.
const BEAD_Y0: f32 = 0.112;
const BEAD_Y1: f32 = 0.146;
/// Triangle count around each bead — enough to read round at shop distance.
const BEAD_SEGMENTS: usize = 14;

/// Build the abacus frame mesh: outer rails, back-plate, reckoning beam, and
/// vertical rods (no beads — see [`build_abacus_heaven_beads_mesh`] /
/// [`build_abacus_earth_beads_mesh`]).
pub fn build_abacus_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Frame interior bounds (where beads + rods live).
    let x0 = -0.5 + RAIL_W;
    let x1 = 0.5 - RAIL_W;
    let z0 = -0.5 + RAIL_W;
    let z1 = 0.5 - RAIL_W;

    // ── Outer frame: four rails + a back-plate ──────────────────────────
    // Top rail (high Z).
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -FRAME_HALF_Y, FRAME_HALF_Y, z1, 0.5),
    );
    // Bottom rail (low Z).
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -FRAME_HALF_Y, FRAME_HALF_Y, -0.5, z0),
    );
    // Left stile (low X).
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, x0, -FRAME_HALF_Y, FRAME_HALF_Y, z0, z1),
    );
    // Right stile (high X).
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(x1, 0.5, -FRAME_HALF_Y, FRAME_HALF_Y, z0, z1),
    );
    // Thin back-plate behind the beads (at -Y) — gives the abacus visual
    // weight from the side and keeps the bead grid from looking like
    // floating dots when viewed at extreme angles.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(x0, x1, -FRAME_HALF_Y, -FRAME_HALF_Y * 0.5, z0, z1),
    );

    // ── Horizontal reckoning beam ───────────────────────────────────────
    // Splits the inner area into upper (heaven, 2 beads) and lower
    // (earth, 5 beads). Position it so the upper deck is ~2/7 of the
    // total bead height (matching real suanpan proportions).
    let inner_h = z1 - z0;
    let beam_z = z0 + inner_h * (EARTH_BEADS as f32 / (HEAVEN_BEADS + EARTH_BEADS) as f32);
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            x0,
            x1,
            -FRAME_HALF_Y,
            FRAME_HALF_Y,
            beam_z - BEAM_HALF_Z,
            beam_z + BEAM_HALF_Z,
        ),
    );

    // ── Bead rods (N_RODS thin vertical posts) ──────────────────────────
    let inner_w = x1 - x0;
    let rod_spacing = inner_w / (N_RODS as f32);
    for col in 0..N_RODS {
        let cx = x0 + rod_spacing * (col as f32 + 0.5);
        push_box(
            &mut vertices,
            &mut indices,
            Aabb::new(
                cx - ROD_HALF_X,
                cx + ROD_HALF_X,
                -FRAME_HALF_Y * 0.5,
                FRAME_HALF_Y * 0.5,
                z0,
                z1,
            ),
        );
    }

    // Zero all UVs — no decal texture for this mesh.
    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWoodFlat,
            base_color: [0.32, 0.18, 0.10, 1.0],
            specular_strength: 0.52,
            specular_power: 72.0,
        },
    }
}

fn push_heaven_beads(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    let x0 = -0.5 + RAIL_W;
    let x1 = 0.5 - RAIL_W;
    let z1 = 0.5 - RAIL_W;
    let inner_w = x1 - x0;
    let rod_spacing = inner_w / (N_RODS as f32);
    let bead_step_z = BEAD_RADIUS * 2.0 + 0.008;
    let heaven_z_top = z1 - BEAD_RADIUS * 0.35;
    for col in 0..N_RODS {
        let cx = x0 + rod_spacing * (col as f32 + 0.5);
        for row in 0..HEAVEN_BEADS {
            let cz = heaven_z_top - row as f32 * bead_step_z;
            push_cylinder_y(
                vertices,
                indices,
                cx,
                cz,
                BEAD_Y0,
                BEAD_Y1,
                BEAD_RADIUS,
                BEAD_SEGMENTS,
            );
        }
    }
}

fn push_earth_beads(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    let x0 = -0.5 + RAIL_W;
    let x1 = 0.5 - RAIL_W;
    let z0 = -0.5 + RAIL_W;
    let inner_w = x1 - x0;
    let rod_spacing = inner_w / (N_RODS as f32);
    let bead_step_z = BEAD_RADIUS * 2.0 + 0.008;
    let earth_z_bot = z0 + BEAD_RADIUS * 0.35;
    for col in 0..N_RODS {
        let cx = x0 + rod_spacing * (col as f32 + 0.5);
        for row in 0..EARTH_BEADS {
            let cz = earth_z_bot + row as f32 * bead_step_z;
            push_cylinder_y(
                vertices,
                indices,
                cx,
                cz,
                BEAD_Y0,
                BEAD_Y1,
                BEAD_RADIUS,
                BEAD_SEGMENTS,
            );
        }
    }
}

fn bead_mesh_cpu() -> MaterialParams {
    MaterialParams {
        kind: MaterialKind::Porcelain,
        base_color: color::PARCHMENT,
        specular_strength: 0.7,
        specular_power: 128.0,
    }
}

/// Upper ("heaven") deck — two beads per rod above the reckoning beam.
pub fn build_abacus_heaven_beads_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    push_heaven_beads(&mut vertices, &mut indices);
    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }
    MeshCpu {
        vertices,
        indices,
        default_material: bead_mesh_cpu(),
    }
}

/// Lower ("earth") deck — five beads per rod below the reckoning beam.
pub fn build_abacus_earth_beads_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    push_earth_beads(&mut vertices, &mut indices);
    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }
    MeshCpu {
        vertices,
        indices,
        default_material: bead_mesh_cpu(),
    }
}
