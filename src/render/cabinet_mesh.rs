//! Procedural mesh for the collection scene's hexagonal display cabinet.
//!
//! A vertical hexagonal column with overhanging cap and base, intended as
//! a "curio cabinet tower" the player rotates to browse different
//! collection categories. The 6 side faces are flat lacquered wood —
//! large enough that future iterations can decal artifact rows directly
//! onto them, or position separate 3D objects in front of each face.
//!
//! Local-space convention (matches the rest of the codebase):
//! - Cube spans `-0.5..+0.5` on each axis. Per-instance scale matrix
//!   sizes it via `Object3d.extents` → `(width, depth, height)` mapped to
//!   `(mesh_X, mesh_Y, mesh_Z)`.
//! - Mesh **+Z is up** (matches world Z-up). The hex faces wrap around
//!   the local Z axis.
//!
//! ```text
//!   +Z up
//!     ┌──────────┐                  ← hex cap (overhangs body)
//!     ├──────────┤                  ← cap underside
//!     │  body    │
//!     │  (6 hex  │                  ← 6 lacquered-wood side faces
//!     │   faces) │
//!     ├──────────┤                  ← base top
//!     └──────────┘                  ← hex base (overhangs body)
//! ```

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_quad};
use crate::render::tile_glb::Vertex3dTex;

/// Hexagonal column body radius (apothem-style: distance from local Z
/// axis to the center of each side face). Body height matches the local
/// `-0.5..+0.5` Z range minus the cap/base thickness.
const BODY_RADIUS: f32 = 0.46;

/// Cap and base radius — slightly larger than `BODY_RADIUS` so they
/// overhang and read as ornamental terminations rather than a flush
/// continuation of the column.
const CAP_RADIUS: f32 = 0.50;

/// Vertical thickness of the cap and base slabs (each), in local units.
const CAP_THICKNESS: f32 = 0.06;

/// Number of horizontal shelf grooves carved into the body. Each groove
/// is a thin recessed band that visually divides the column into shelf
/// rows, even before any artifact geometry is layered onto the faces.
const SHELF_GROOVE_COUNT: usize = 3;

/// How deep each shelf groove is recessed from the face plane (local
/// units). Grooves go inward by this amount.
const GROOVE_DEPTH: f32 = 0.012;

/// Vertical thickness of each shelf-groove band.
const GROOVE_HEIGHT: f32 = 0.025;

/// Build the cabinet mesh.
///
/// All faces are explicit quads with hand-set normals so the lit-mesh
/// shader gets correct shading without any cross-face normal averaging.
/// Single material — see `build_cabinet_rails_mesh` for the brass corner
/// rails (separate object so they can use `MaterialKind::Metal`).
pub fn build_cabinet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Z extents of the body (between cap and base).
    let body_z0 = -0.5 + CAP_THICKNESS;
    let body_z1 = 0.5 - CAP_THICKNESS;

    // Compute the Z slabs. With N grooves we get N+1 wood bands
    // separated by the grooves, evenly distributed across the body.
    // The bands are full-radius; the grooves recess inward.
    let body_h = body_z1 - body_z0;
    let band_pitch = body_h / (SHELF_GROOVE_COUNT as f32 + 1.0);
    let mut z_segments: Vec<(f32, f32, bool)> = Vec::new(); // (z0, z1, is_groove)
    let mut z_cursor = body_z0;
    for i in 0..SHELF_GROOVE_COUNT {
        let band_top = body_z0 + (i as f32 + 1.0) * band_pitch - GROOVE_HEIGHT * 0.5;
        z_segments.push((z_cursor, band_top, false));
        z_segments.push((band_top, band_top + GROOVE_HEIGHT, true));
        z_cursor = band_top + GROOVE_HEIGHT;
    }
    z_segments.push((z_cursor, body_z1, false));

    // ── Body: 6 side faces × N segments ─────────────────────────────
    // Each side face spans one of 6 hexagonal sectors and is split
    // vertically into the segments above. Groove segments use a smaller
    // radius so they read as recessed bands.
    //
    // UVs map each face's non-groove segments to a slice of a 6-cell
    // horizontal decal strip — face `i` samples columns
    // `[i/6, (i+1)/6]`. Groove segments and all other geometry (cap,
    // base, rims, groove top/bottom strips) sample UV `[0, 0]` which
    // is held transparent in the rasterised strip texture so the wood
    // material renders unchanged there.
    let vertex_radius = BODY_RADIUS / (std::f32::consts::PI / 6.0).cos();
    let groove_vertex_radius = (BODY_RADIUS - GROOVE_DEPTH) / (std::f32::consts::PI / 6.0).cos();
    for i in 0..6 {
        let theta_center = i as f32 * std::f32::consts::TAU / 6.0;
        let theta_left = theta_center + std::f32::consts::PI / 6.0;
        let theta_right = theta_center - std::f32::consts::PI / 6.0;
        let normal = [theta_center.cos(), theta_center.sin(), 0.0];

        let u_left = i as f32 / 6.0;
        let u_right = (i + 1) as f32 / 6.0;

        for &(z0, z1, is_groove) in &z_segments {
            let r = if is_groove {
                groove_vertex_radius
            } else {
                vertex_radius
            };
            let v_left = [theta_left.cos() * r, theta_left.sin() * r];
            let v_right = [theta_right.cos() * r, theta_right.sin() * r];
            let base_idx = vertices.len();
            push_quad(
                &mut vertices,
                &mut indices,
                [v_right[0], v_right[1], z0],
                [v_right[0], v_right[1], z1],
                [v_left[0], v_left[1], z1],
                [v_left[0], v_left[1], z0],
                normal,
            );
            if !is_groove {
                // Map this segment's quad onto the strip cell. V is
                // relative to the segment's Z range within the body
                // (V=0 at top, V=1 at bottom).
                //
                // The face's `v_left` and `v_right` corners are named
                // for theta_center ± 30° in the local frame, but once
                // the cabinet yaws around world Z those map to the
                // *opposite* sides of the camera. Empirically: U
                // increasing left-to-right in the strip lands on
                // `v_left` (math-CCW corner) when viewed from the
                // camera. Tested by reading the rasterised label
                // upright on the front face.
                let v_top = 1.0 - (z1 - body_z0) / body_h;
                let v_bot = 1.0 - (z0 - body_z0) / body_h;
                vertices[base_idx].uv = [u_left, v_bot]; // bottom-right (face local) → texture left
                vertices[base_idx + 1].uv = [u_left, v_top]; // top-right (face local) → texture left
                vertices[base_idx + 2].uv = [u_right, v_top]; // top-left (face local) → texture right
                vertices[base_idx + 3].uv = [u_right, v_bot]; // bottom-left (face local) → texture right
            }
        }

        // Top and bottom of each groove: thin horizontal strips that
        // close the gap between the groove's recessed face and the
        // adjacent full-radius bands. Without these the groove looks
        // like a hole rather than a recessed band.
        for &(_, z_top, is_groove) in &z_segments {
            if !is_groove {
                continue;
            }
            // Top of groove: face up, connects groove face to band above.
            let v_left_in = [
                theta_left.cos() * groove_vertex_radius,
                theta_left.sin() * groove_vertex_radius,
            ];
            let v_right_in = [
                theta_right.cos() * groove_vertex_radius,
                theta_right.sin() * groove_vertex_radius,
            ];
            let v_left_out = [
                theta_left.cos() * vertex_radius,
                theta_left.sin() * vertex_radius,
            ];
            let v_right_out = [
                theta_right.cos() * vertex_radius,
                theta_right.sin() * vertex_radius,
            ];
            push_quad(
                &mut vertices,
                &mut indices,
                [v_right_in[0], v_right_in[1], z_top],
                [v_right_out[0], v_right_out[1], z_top],
                [v_left_out[0], v_left_out[1], z_top],
                [v_left_in[0], v_left_in[1], z_top],
                [0.0, 0.0, 1.0],
            );
            // Bottom of groove: face down.
            let z_bot = z_top - GROOVE_HEIGHT;
            push_quad(
                &mut vertices,
                &mut indices,
                [v_right_out[0], v_right_out[1], z_bot],
                [v_right_in[0], v_right_in[1], z_bot],
                [v_left_in[0], v_left_in[1], z_bot],
                [v_left_out[0], v_left_out[1], z_bot],
                [0.0, 0.0, -1.0],
            );
        }
    }

    // Snapshot the vertex count so cap/base/rim UVs can be zeroed
    // after construction — `push_hex_disc` deliberately spreads UVs
    // across the texture for per-vertex normals/lighting variety, but
    // we want those geometries to sample the strip texture's
    // transparent corner so no tab name leaks onto them.
    let pre_cap_idx = vertices.len();

    // ── Cap: top hex slab ───────────────────────────────────────────
    // Top face (+Z), bottom face (-Z, the cap's underside), and 6 side
    // faces around the rim.
    push_hex_disc(&mut vertices, &mut indices, CAP_RADIUS, 0.5, true);
    push_hex_disc(
        &mut vertices,
        &mut indices,
        CAP_RADIUS,
        0.5 - CAP_THICKNESS,
        false,
    );
    push_hex_rim(
        &mut vertices,
        &mut indices,
        CAP_RADIUS,
        0.5 - CAP_THICKNESS,
        0.5,
    );

    // ── Base: bottom hex slab (mirror of cap) ───────────────────────
    push_hex_disc(
        &mut vertices,
        &mut indices,
        CAP_RADIUS,
        -0.5 + CAP_THICKNESS,
        true,
    );
    push_hex_disc(&mut vertices, &mut indices, CAP_RADIUS, -0.5, false);
    push_hex_rim(
        &mut vertices,
        &mut indices,
        CAP_RADIUS,
        -0.5,
        -0.5 + CAP_THICKNESS,
    );

    // Zero out cap/base/rim UVs so they sample the strip texture's
    // transparent corner (alpha = 0 → wood material renders unchanged).
    for v in &mut vertices[pre_cap_idx..] {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            // Flat variant: the table-tuned wood displacement amplitude
            // would push vertices through the slim cap/base slabs.
            kind: MaterialKind::LacqueredWoodFlat,
            base_color: [1.0, 1.0, 1.0, 1.0],
            specular_strength: 0.45,
            specular_power: 64.0,
        },
    }
}

/// Half-width of each brass corner rail (square cross-section, in local
/// units). Sized to read at typical scene scale — too thin and the
/// rails become sub-pixel trim under perspective foreshortening.
const RAIL_HALF: f32 = 0.035;

/// How far past `BODY_RADIUS` the rails sit. Slight outward offset so
/// they sit proud of the wood faces and read as inlaid hardware.
const RAIL_OUTSET: f32 = 0.012;

/// Build the brass corner rails mesh — 6 thin vertical square prisms at
/// the hexagon's vertex columns, running the full body height. Drawn
/// alongside `build_cabinet_mesh` by the renderer's Cabinet dispatcher
/// so the rails get `MaterialKind::Metal` shading while the body keeps
/// `LacqueredWoodFlat`.
///
/// Local-space convention matches `build_cabinet_mesh` (cube
/// −0.5..+0.5, mesh +Z up). Rails are aligned to the body's vertex
/// circumradius so they trace each edge of the hexagon column.
pub fn build_cabinet_rails_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let body_z0 = -0.5 + CAP_THICKNESS;
    let body_z1 = 0.5 - CAP_THICKNESS;
    let circumradius = BODY_RADIUS / (std::f32::consts::PI / 6.0).cos() + RAIL_OUTSET;

    // Each rail at hex vertex angle 30° + i*60° (matching the wood
    // body's vertex positions). Build it as an axis-aligned box at the
    // origin, then transform into place by rotating around Z and
    // translating outward.
    for i in 0..6 {
        let theta = (i as f32 + 0.5) * std::f32::consts::TAU / 6.0;
        let cx = theta.cos() * circumradius;
        let cy = theta.sin() * circumradius;
        // Box axis-aligned to the rail's local frame: long axis = Z,
        // square cross-section = ±RAIL_HALF in X and Y. Then rotate by
        // theta about Z so the cross-section's "outward" face points
        // along (cos θ, sin θ, 0) and translate to (cx, cy).
        push_rotated_box(
            &mut vertices,
            &mut indices,
            cx,
            cy,
            theta,
            RAIL_HALF,
            RAIL_HALF,
            body_z0,
            body_z1,
        );
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            // Warm brass tint; the Metal shader drives Schlick Fresnel
            // against this base colour for the conductor look.
            base_color: [0.86, 0.65, 0.32, 1.0],
            specular_strength: 0.85,
            specular_power: 96.0,
        },
    }
}

/// Push a Z-aligned rectangular prism centered at `(cx, cy)` in XY,
/// rotated by `theta` about world Z, with half-extents `(hx, hy)` in
/// the prism's local frame and Z spanning `z0..z1`. Used by the rails
/// builder so each rail's outward face has the correct world-XY
/// orientation.
fn push_rotated_box(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    cx: f32,
    cy: f32,
    theta: f32,
    hx: f32,
    hy: f32,
    z0: f32,
    z1: f32,
) {
    let cos_t = theta.cos();
    let sin_t = theta.sin();
    // Helper: take a local-frame (lx, ly) and project to world XY.
    let world = |lx: f32, ly: f32| -> [f32; 2] {
        [cx + lx * cos_t - ly * sin_t, cy + lx * sin_t + ly * cos_t]
    };

    // Four corners in local XY: (+x, +y), (+x, -y), (-x, -y), (-x, +y).
    let p_pp = world(hx, hy);
    let p_pm = world(hx, -hy);
    let p_mm = world(-hx, -hy);
    let p_mp = world(-hx, hy);

    // Outward-face normals in world frame, computed by rotating local
    // axis directions.
    let n_outx = [cos_t, sin_t, 0.0]; // local +X (outward)
    let n_inx = [-cos_t, -sin_t, 0.0]; // local -X (inward)
    let n_outy = [-sin_t, cos_t, 0.0]; // local +Y (tangent +)
    let n_iny = [sin_t, -cos_t, 0.0]; // local -Y (tangent -)

    // +X (outward) face. CCW from outside.
    push_quad(
        vertices,
        indices,
        [p_pm[0], p_pm[1], z0],
        [p_pp[0], p_pp[1], z0],
        [p_pp[0], p_pp[1], z1],
        [p_pm[0], p_pm[1], z1],
        n_outx,
    );
    // -X (inward) face.
    push_quad(
        vertices,
        indices,
        [p_mp[0], p_mp[1], z0],
        [p_mm[0], p_mm[1], z0],
        [p_mm[0], p_mm[1], z1],
        [p_mp[0], p_mp[1], z1],
        n_inx,
    );
    // +Y (tangent) face.
    push_quad(
        vertices,
        indices,
        [p_pp[0], p_pp[1], z0],
        [p_mp[0], p_mp[1], z0],
        [p_mp[0], p_mp[1], z1],
        [p_pp[0], p_pp[1], z1],
        n_outy,
    );
    // -Y (tangent) face.
    push_quad(
        vertices,
        indices,
        [p_mm[0], p_mm[1], z0],
        [p_pm[0], p_pm[1], z0],
        [p_pm[0], p_pm[1], z1],
        [p_mm[0], p_mm[1], z1],
        n_iny,
    );
    // +Z cap.
    push_quad(
        vertices,
        indices,
        [p_mm[0], p_mm[1], z1],
        [p_pm[0], p_pm[1], z1],
        [p_pp[0], p_pp[1], z1],
        [p_mp[0], p_mp[1], z1],
        [0.0, 0.0, 1.0],
    );
    // -Z cap.
    push_quad(
        vertices,
        indices,
        [p_pm[0], p_pm[1], z0],
        [p_mm[0], p_mm[1], z0],
        [p_mp[0], p_mp[1], z0],
        [p_pp[0], p_pp[1], z0],
        [0.0, 0.0, -1.0],
    );
}

/// Push a flat hexagonal disc at `z` with given `radius`. `face_up = true`
/// emits a +Z-facing disc (CCW from above); `false` emits -Z-facing.
///
/// The hexagon is fan-triangulated from its center — six triangles each
/// spanning a 60° wedge. This adds 7 vertices (1 center + 6 perimeter)
/// and 18 indices per call.
fn push_hex_disc(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    radius: f32,
    z: f32,
    face_up: bool,
) {
    let normal = if face_up {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 0.0, -1.0]
    };
    let center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, z],
        normal,
        uv: [0.5, 0.5],
    });
    let perim_base = vertices.len() as u32;
    // Hexagon vertices: angular offsets 30°, 90°, 150°, 210°, 270°, 330°
    // so flats face along ±X and ±Y rotated by 30°. Match the body face
    // orientation: face 0 has its outward normal along +X with vertices
    // at ±30° — i.e. the hex perimeter vertices are at 30°, 90°, etc.
    for i in 0..6 {
        let theta = (i as f32 + 0.5) * std::f32::consts::TAU / 6.0;
        vertices.push(Vertex3dTex {
            position: [theta.cos() * radius, theta.sin() * radius, z],
            normal,
            uv: [0.5 + 0.5 * theta.cos(), 0.5 + 0.5 * theta.sin()],
        });
    }
    // Fan triangulation. Wind CCW from the side `normal` points to.
    for i in 0..6 {
        let a = perim_base + i;
        let b = perim_base + (i + 1) % 6;
        if face_up {
            indices.extend_from_slice(&[center_idx, a, b]);
        } else {
            indices.extend_from_slice(&[center_idx, b, a]);
        }
    }
}

/// Push the 6 vertical rim faces between two stacked hexagons at z0
/// (lower) and z1 (upper). Each rim face is a quad with its outward
/// normal pointing radially out from the column axis.
fn push_hex_rim(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    radius: f32,
    z0: f32,
    z1: f32,
) {
    for i in 0..6 {
        let theta_a = (i as f32 + 0.5) * std::f32::consts::TAU / 6.0;
        let theta_b = ((i + 1) as f32 + 0.5) * std::f32::consts::TAU / 6.0;
        let theta_mid = (theta_a + theta_b) * 0.5;
        let normal = [theta_mid.cos(), theta_mid.sin(), 0.0];
        let a = [theta_a.cos() * radius, theta_a.sin() * radius];
        let b = [theta_b.cos() * radius, theta_b.sin() * radius];
        push_quad(
            vertices,
            indices,
            [a[0], a[1], z0],
            [b[0], b[1], z0],
            [b[0], b[1], z1],
            [a[0], a[1], z1],
            normal,
        );
    }
}
