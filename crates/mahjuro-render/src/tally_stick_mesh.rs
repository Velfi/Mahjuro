//! Procedural meshes for tally sticks — the draws/discards counter fans that
//! stand in front of the Play (mirror) and Discard (river) actions.
//!
//! Each stick is split into two sibling meshes so the body and tip cap can
//! render with distinct per-instance colors:
//!   - [`build_tally_stick_base_mesh`]: the lower segment (from the narrow
//!     base up to `1 - TIP_FRAC` along the stick).
//!   - [`build_tally_stick_tip_mesh`]: the upper cap (from `1 - TIP_FRAC` up
//!     to the wide tip).
//!
//! Local space: pivot at the narrow base, stick extends along `+Y` from
//! `y=0` to `y=1`. Cross-section is rectangular with thickness along `Z`;
//! the stick tapers linearly from `STICK_NARROW_HALF` at `y=0` to
//! `STICK_WIDE_HALF` at `y=1`. Local bounds are
//! `x ∈ [-STICK_WIDE_HALF, +STICK_WIDE_HALF]`, `y ∈ [0, 1]`,
//! `z ∈ [-STICK_THICKNESS_HALF, +STICK_THICKNESS_HALF]`. A per-instance
//! scale of `(w, len, t)` maps directly to a stick of wide-end width `w`,
//! length `len`, and thickness `t` in world units.

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_quad};
use crate::tile_glb::Vertex3dTex;

/// Local half-width at the narrow (base) end. Half the wide end → 2:1 taper.
pub const STICK_NARROW_HALF: f32 = 0.25;
/// Local half-width at the wide (tip) end. `0.5` so identity X-scale spans
/// unit width at the tip and per-instance scale reads as world-space width.
pub const STICK_WIDE_HALF: f32 = 0.5;
/// Local half-thickness (depth along Z). `0.5` so identity Z-scale reads
/// as world-space thickness.
pub const STICK_THICKNESS_HALF: f32 = 0.5;

/// Fraction of stick length covered by the polychrome tip cap, measured from the
/// wide end. A value of `0.35` means the top 35% of each stick is painted.
pub const TIP_FRAC: f32 = 0.35;

/// Linear interpolation of the half-width at local `y ∈ [0, 1]`.
fn half_width_at(y: f32) -> f32 {
    STICK_NARROW_HALF + (STICK_WIDE_HALF - STICK_NARROW_HALF) * y
}

/// Build a tapered segment mesh between `y0` and `y1` along the stick's
/// local Y axis. The segment keeps the stick's linear taper, so two adjacent
/// segments joined at the same `y` meet seamlessly.
fn build_segment_mesh(y0: f32, y1: f32, base_color: [f32; 4]) -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let h0 = half_width_at(y0);
    let h1 = half_width_at(y1);
    let t = STICK_THICKNESS_HALF;

    // Eight corners: four at `y0` (narrower), four at `y1` (wider).
    let b00 = [-h0, y0, -t];
    let b10 = [h0, y0, -t];
    let b11 = [h0, y0, t];
    let b01 = [-h0, y0, t];
    let t00 = [-h1, y1, -t];
    let t10 = [h1, y1, -t];
    let t11 = [h1, y1, t];
    let t01 = [-h1, y1, t];

    // Six faces, winding CCW when viewed from outside.
    push_quad(
        &mut vertices,
        &mut indices,
        b10,
        t10,
        t11,
        b11,
        [1.0, 0.0, 0.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        b01,
        t01,
        t00,
        b00,
        [-1.0, 0.0, 0.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        b11,
        t11,
        t01,
        b01,
        [0.0, 0.0, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        b00,
        t00,
        t10,
        b10,
        [0.0, 0.0, -1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        t00,
        t01,
        t11,
        t10,
        [0.0, 1.0, 0.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        b00,
        b10,
        b11,
        b01,
        [0.0, -1.0, 0.0],
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color,
            // Bone/ivory sheen — matches the `bone_tablet_mesh` kit so the
            // sticks read as part of the same physical set.
            specular_strength: 0.30,
            specular_power: 32.0,
        },
    }
}

/// Lower body segment of a tally stick: from the narrow base up to the line
/// where the tip cap begins. White mesh albedo; per-instance body tint applied
/// at draw time.
pub fn build_tally_stick_base_mesh() -> MeshCpu {
    build_segment_mesh(0.0, 1.0 - TIP_FRAC, [1.0, 1.0, 1.0, 1.0])
}

/// Upper tip cap of a tally stick: from the tint line up to the wide tip.
/// White mesh albedo; per-instance polychrome tint applied at draw time.
pub fn build_tally_stick_tip_mesh() -> MeshCpu {
    build_segment_mesh(1.0 - TIP_FRAC, 1.0, [1.0, 1.0, 1.0, 1.0])
}
