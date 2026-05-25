//! Concentric-disc geometry for the post-run depth-well progression gauge.
//!
//! The well is a flat medallion viewed face-on: seven planar annular / disc
//! regions stacked in the XZ plane (normal = +Y), each with a distinct
//! albedo texture bound at render time.  The caller supplies a billboard
//! rotation so the face always turns toward the camera eye.
//!
//! Regions (outer → inner):
//!   Rim  ·  Step(0..N-1)  ·  Throat
//!
//! UV mapping: planar projection `u = x + 0.5`, `v = z + 0.5` where every
//! vertex lies in `-0.5..+0.5` on both X and Z.  Because the renderer
//! uniformly scales all axes by the supplied extents, the texture maps 1:1
//! to the full face square, and each region's image is a full-size circular
//! sprite with the ring artwork embedded.
//!
//! Tiny Y offsets between layers (innermost at y = 0, Rim at y ≈ 0.003)
//! prevent Z-fighting at shared boundary radii without visible separation.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;
use crate::render::theme::color;

// ── Region id ──────────────────────────────────────────────────────────────

/// Which concentric region of the depth-well medallion to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DepthWellRegionId {
    /// Outer decorative ring (filigree / compass ornaments).
    Rim,
    /// Step ring — `0` = outermost, `step_count - 1` = innermost.
    Step(u8),
    /// Central filled disc at the bottom of the well.
    Throat,
}

// ── Build config ───────────────────────────────────────────────────────────

/// Parameters that control the depth-well geometry.
pub struct DepthWellConfig {
    /// Number of step rings (= `POINTS_PER_LEVEL`).
    pub step_count: usize,
    /// Outer edge of the rim in normalised local coords (≤ 0.5).
    pub outer_radius: f32,
    /// Inner edge of the rim / outer edge of the step band.
    pub rim_inner_radius: f32,
    /// Inner edge of the innermost step / outer edge of the throat disc.
    pub throat_radius: f32,
    /// Circumference segment count (higher = smoother circles).
    pub segments: usize,
}

impl Default for DepthWellConfig {
    fn default() -> Self {
        Self {
            step_count: 5,
            outer_radius: 0.5,
            rim_inner_radius: 0.40,
            throat_radius: 0.07,
            segments: 64,
        }
    }
}

// ── Intermediate CPU mesh bundle ───────────────────────────────────────────

/// Pre-computed radii for each region.  Avoids re-running the config math
/// on every `extract_depth_well_region_mesh` call.
pub struct DepthWellCpu {
    rim_outer: f32,
    rim_inner: f32,
    /// `step_radii[i] = (inner_r, outer_r)` for step `i` (0 = outermost).
    step_radii: Vec<(f32, f32)>,
    throat_outer: f32,
    segments: usize,
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Pre-compute the radius layout from a config.  Cheap (no allocations
/// beyond `step_radii`).
pub fn build_depth_well_mesh(cfg: &DepthWellConfig) -> DepthWellCpu {
    let step_band = cfg.rim_inner_radius - cfg.throat_radius;
    let step_w = if cfg.step_count > 0 {
        step_band / cfg.step_count as f32
    } else {
        0.0
    };

    let step_radii = (0..cfg.step_count)
        .map(|i| {
            // Step 0 = outermost: inner_r is closest to rim.
            let inner = cfg.throat_radius + i as f32 * step_w;
            let outer = inner + step_w;
            (inner, outer)
        })
        .rev() // reverse so index 0 is outermost in the result vec
        .collect();

    DepthWellCpu {
        rim_outer: cfg.outer_radius,
        rim_inner: cfg.rim_inner_radius,
        step_radii,
        throat_outer: cfg.throat_radius,
        segments: cfg.segments,
    }
}

/// Build the [`MeshCpu`] for one region.  Each call allocates fresh vertex /
/// index buffers (called once at renderer init per region).
pub fn extract_depth_well_region_mesh(
    cpu: &DepthWellCpu,
    region: DepthWellRegionId,
) -> MeshCpu {
    let n = cpu.segments;
    let total = 2 + cpu.step_radii.len(); // Throat + steps + Rim

    match region {
        DepthWellRegionId::Throat => {
            build_disc(cpu.throat_outer, 0.0, n, default_plain())
        }
        DepthWellRegionId::Step(idx) => {
            // idx 0 = outermost: stored last in step_radii (we reversed above).
            let layer_from_inside = cpu.step_radii.len() - 1 - idx as usize;
            let y_off = (layer_from_inside + 1) as f32 * 0.0005;
            let (r_inner, r_outer) = cpu.step_radii[idx as usize];
            build_annulus(r_inner, r_outer, y_off, n, default_plain())
        }
        DepthWellRegionId::Rim => {
            let y_off = total as f32 * 0.0005;
            build_annulus(cpu.rim_inner, cpu.rim_outer, y_off, n, default_brass())
        }
    }
}

// ── Geometry helpers ───────────────────────────────────────────────────────

/// Annular ring (hollow disc) in the XZ plane at height `y_off`.
fn build_annulus(
    r_inner: f32,
    r_outer: f32,
    y_off: f32,
    n: usize,
    default_material: MaterialParams,
) -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity((n + 1) * 2);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);

    for i in 0..=n {
        let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
        let (sin_t, cos_t) = theta.sin_cos();

        // Outer vertex then inner vertex — interleaved so index arithmetic
        // below stays simple.
        vertices.push(vert(r_outer * cos_t, y_off, r_outer * sin_t));
        vertices.push(vert(r_inner * cos_t, y_off, r_inner * sin_t));
    }

    for i in 0..n as u32 {
        let o0 = i * 2;           // outer[i]
        let i0 = i * 2 + 1;       // inner[i]
        let o1 = (i + 1) * 2;     // outer[i+1]
        let i1 = (i + 1) * 2 + 1; // inner[i+1]

        // Two CCW triangles (facing +Y) per quad.
        indices.extend_from_slice(&[o0, o1, i0]);
        indices.extend_from_slice(&[i0, o1, i1]);
    }

    MeshCpu { vertices, indices, default_material }
}

/// Solid filled disc in the XZ plane at height `y_off`.
fn build_disc(r_outer: f32, y_off: f32, n: usize, default_material: MaterialParams) -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(n + 2);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 3);

    // Center vertex.
    vertices.push(vert(0.0, y_off, 0.0));

    for i in 0..=n {
        let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
        let (sin_t, cos_t) = theta.sin_cos();
        vertices.push(vert(r_outer * cos_t, y_off, r_outer * sin_t));
    }

    for i in 0..n as u32 {
        // CCW triangles facing +Y.
        indices.extend_from_slice(&[0, i + 1, i + 2]);
    }

    MeshCpu { vertices, indices, default_material }
}

// ── Vertex helper ──────────────────────────────────────────────────────────

/// Single vertex at `(x, y, z)` with +Y normal and planar UV from XZ.
#[inline]
fn vert(x: f32, y: f32, z: f32) -> Vertex3dTex {
    Vertex3dTex {
        position: [x, y, z],
        normal: [0.0, 1.0, 0.0],
        // Planar projection: maps local -0.5..+0.5 → 0..1.
        uv: [x + 0.5, z + 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    }
}

fn default_plain() -> MaterialParams {
    MaterialParams {
        kind: MaterialKind::Plain,
        base_color: [1.0, 1.0, 1.0, 1.0],
        specular_strength: 0.10,
        specular_power: 32.0,
    }
}

fn default_brass() -> MaterialParams {
    MaterialParams {
        kind: MaterialKind::Brass,
        base_color: color::BRASS,
        specular_strength: 0.72,
        specular_power: 128.0,
    }
}
