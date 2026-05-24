//! Procedural meshes for the on-table relic dish + the placeholder relic boxes
//! that sit inside it. Both are simple axis-aligned boxes built once at startup
//! and instanced via [`crate::render::lit_mesh::LitMeshInstance`].
//!
//! The dish is a flat low box centered at local (0,0,0); the relic placeholder
//! is a unit cube spanning -0.5..+0.5 on each axis so a per-instance scale can
//! turn it into rectangular prisms of varying sizes.

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::theme::color;
use crate::render::tile_glb::Vertex3dTex;

use lyon_path::Path;
use lyon_path::math::Point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};

/// Rounded-rectangle card mesh (-0.5..+0.5) for **tile booster packs**,
/// oriented as an upright card: broad faces on ±Y (front/back, toward/away
/// from camera), width on ±X, height on ±Z. Callers scale via
/// `Object3d.extents` with `[width, thickness, height]` where
/// `width = height * aspect_w_over_h`.
///
/// Corners of the silhouette are rounded (`CORNER_R` in local units) with
/// `CORNER_SEG` segments each, so the side strip curves smoothly around
/// the card instead of showing sharp 90° edges. Front and back faces are
/// triangulated as fans from the center of the rounded polygon.
///
/// UVs on the -Y (camera-facing) face map the full texture: U along +X,
/// V along -Z, so the image reads upright with (0,0) at the top-left.
/// The +Y back mirrors the front. The side strip samples the texture
/// corner (0,0) — it's a few pixels on screen, so a solid sliver reads
/// cleanly without art spilling around the edge.
pub fn build_pack_mesh() -> MeshCpu {
    // Corner radius in local units, tuned to match a standard playing
    // card (≈6% of the short side). Half-extent is 0.5, so 0.03 reads
    // as a tight card-style round rather than a soft pillow.
    const CORNER_R: f32 = 0.03;
    const CORNER_SEG: usize = 4;

    // Silhouette: counter-clockwise polygon in the XZ plane (viewed from
    // -Y / camera side). Builds the four straight edges joined by quarter
    // circles at each corner.
    //
    // Corner centers sit `CORNER_R` inset from each rectangle corner;
    // each arc sweeps 90° from one straight edge to the next. Using a
    // single loop keeps the polygon in a known order for both fan
    // triangulation and side-strip extrusion.
    let mut silhouette: Vec<[f32; 2]> = Vec::with_capacity(4 * (CORNER_SEG + 1));
    // Order around the silhouette: each arc's end point lies on the same
    // rectangle edge as the next arc's start point, so the implicit
    // straight segments run along the edges of the card (not across its
    // interior). Going CCW as viewed from -Y: bottom-right → top-right →
    // top-left → bottom-left.
    let corners: [([f32; 2], f32); 4] = [
        (
            [0.5 - CORNER_R, -0.5 + CORNER_R],
            -std::f32::consts::FRAC_PI_2,
        ), // bottom-right: arc from bottom edge to right edge
        ([0.5 - CORNER_R, 0.5 - CORNER_R], 0.0), // top-right: arc from right edge to top edge
        (
            [-0.5 + CORNER_R, 0.5 - CORNER_R],
            std::f32::consts::FRAC_PI_2,
        ), // top-left: arc from top edge to left edge
        ([-0.5 + CORNER_R, -0.5 + CORNER_R], std::f32::consts::PI), // bottom-left: arc from left edge to bottom edge
    ];
    for (center, start_angle) in corners.iter() {
        for i in 0..=CORNER_SEG {
            let t = i as f32 / CORNER_SEG as f32;
            let a = start_angle + t * std::f32::consts::FRAC_PI_2;
            silhouette.push([
                center[0] + CORNER_R * a.cos(),
                center[1] + CORNER_R * a.sin(),
            ]);
        }
    }
    // The corner arcs share endpoints with the adjacent straight edges;
    // the implicit straight segments are the chord between arc end and
    // next arc start, which the side-strip extrusion handles naturally.

    // UV from silhouette xz: U = x + 0.5, V = 0.5 - z (matches the
    // original -Y face mapping so pack textures read upright).
    let uv_of = |p: [f32; 2]| [p[0] + 0.5, 0.5 - p[1]];

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // ── Front face (-Y) ──────────────────────────────────────────────
    // Fan triangulation from the polygon center. Center vertex first.
    let front_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, -0.5, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let front_ring_start = vertices.len() as u32;
    for p in silhouette.iter() {
        vertices.push(Vertex3dTex {
            position: [p[0], -0.5, p[1]],
            normal: [0.0, -1.0, 0.0],
            uv: uv_of(*p),
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    let n = silhouette.len() as u32;
    for i in 0..n {
        let next = (i + 1) % n;
        indices.extend_from_slice(&[
            front_center_idx,
            front_ring_start + i,
            front_ring_start + next,
        ]);
    }

    // ── Back face (+Y) ───────────────────────────────────────────────
    // Same fan but with the winding reversed so the outside is +Y.
    let back_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.5, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let back_ring_start = vertices.len() as u32;
    for p in silhouette.iter() {
        vertices.push(Vertex3dTex {
            position: [p[0], 0.5, p[1]],
            normal: [0.0, 1.0, 0.0],
            // Mirror U so the back reads upright when rotated 180° about Z.
            uv: [1.0 - (p[0] + 0.5), 0.5 - p[1]],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let next = (i + 1) % n;
        indices.extend_from_slice(&[back_center_idx, back_ring_start + next, back_ring_start + i]);
    }

    // ── Side strip ────────────────────────────────────────────────────
    // Two verts per silhouette point (bottom at -Y=-0.5, top at +Y=+0.5)
    // with the outward normal. Straight edges and arcs share the same
    // extrusion math; arc segments naturally get smooth per-vertex
    // normals pointing radially outward from their corner center.
    //
    // Normal at each silhouette vertex: for arc segments, it's the
    // radial direction from corner center; for chord endpoints it falls
    // out the same way because the endpoints coincide with the arc
    // start/end (normal aligns with the straight edge's outward axis).
    //
    // Since each silhouette vertex belongs to exactly one arc (built
    // above as arc-per-corner), we can compute its normal as the
    // normalized direction from the nearest corner center — which is
    // just the vertex-relative offset we used during generation.
    //
    // We regenerate the offset by recovering which corner each index
    // belongs to: indices [c*(SEG+1)..(c+1)*(SEG+1)] → corner c.
    for (c, (center, _)) in corners.iter().enumerate() {
        for i in 0..=CORNER_SEG {
            let vi = c * (CORNER_SEG + 1) + i;
            let p = silhouette[vi];
            let dx = p[0] - center[0];
            let dz = p[1] - center[1];
            let len = (dx * dx + dz * dz).sqrt().max(1e-6);
            let n = [dx / len, 0.0, dz / len];
            // Side strip UV: U wraps around the silhouette (0..1), V spans
            // thickness (0 = front, 1 = back). The edge texture is a
            // solid sliver, so exact wrapping doesn't matter; we just
            // keep the UVs in the (0,0) corner so the decal shader
            // masks the side strip out (front_face smoothstep on
            // local_pos.y).
            vertices.push(Vertex3dTex {
                position: [p[0], -0.5, p[1]],
                normal: n,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [p[0], 0.5, p[1]],
                normal: n,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }
    // Strip triangulation: for each silhouette edge (i → i+1), emit a
    // quad using the two bottom + two top verts.
    let strip_base = back_ring_start + n;
    for i in 0..n {
        let next = (i + 1) % n;
        let b0 = strip_base + 2 * i;
        let t0 = b0 + 1;
        let b1 = strip_base + 2 * next;
        let t1 = b1 + 1;
        // Outside winding: looking from outside (along +normal), the
        // quad reads bottom-left → bottom-right → top-right → top-left
        // with the silhouette going CCW in XZ (viewed from -Y). So:
        // bottom edge goes b0 → b1; top edge goes t1 → t0.
        indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.85, 0.78, 0.55, 1.0],
            specular_strength: 0.35,
            specular_power: 24.0,
        },
    }
}

/// Build the shop action prop mesh: a flat rectangular block (Plain material)
/// whose `+Y` face shows the button label decal.
///
/// UV layout: U runs along local `+X` (the wide axis), V along local `+Z`,
/// so a landscape 512×192 label texture reads left-to-right when the prop
/// sits on the counter facing the camera (cam_rot applied, +Y = up).
/// All other faces have UV (0,0) so the decal only appears on top.
pub fn build_shop_action_prop_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.5, 0.5, -0.5, 0.5),
    );

    // push_box emits 6 faces × 4 verts in order: +X(0-3), -X(4-7), +Y(8-11),
    // -Y(12-15), +Z(16-19), -Z(20-23).  Zero UVs on every face except +Y.
    for i in (0..24).filter(|i| !(8..12).contains(i)) {
        vertices[i].uv = [0.0, 0.0];
    }
    // +Y face corners emitted by push_box:
    //   v8  (-0.5, top, -0.5)  local -X, -Z  →  screen-left,  screen-top   → [0, 0]
    //   v9  (-0.5, top, +0.5)  local -X, +Z  →  screen-left,  screen-bot   → [0, 1]
    //   v10 (+0.5, top, +0.5)  local +X, +Z  →  screen-right, screen-bot   → [1, 1]
    //   v11 (+0.5, top, -0.5)  local +X, -Z  →  screen-right, screen-top   → [1, 0]
    // With cam_rot: camera-right = local +X, camera-up = local -Z, so
    // U = 0 at left, U = 1 at right; V = 0 at top, V = 1 at bottom. ✓
    vertices[8].uv = [0.0, 0.0];
    vertices[9].uv = [0.0, 1.0];
    vertices[10].uv = [1.0, 1.0];
    vertices[11].uv = [1.0, 0.0];

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.85, 0.78, 0.55, 1.0],
            specular_strength: 0.4,
            specular_power: 32.0,
        },
    }
}

/// Build the dish mesh: a shallow brass tray with a raised rim around its
/// top edge and a recessed floor in the middle. Local units span -0.5..0.5
/// on each axis (same convention as the unit box) so callers can scale it
/// independently in x/y/z to choose the dish footprint and rim height.
///
/// Geometry: outer footprint is the full -0.5..0.5 cube. The top face is
/// replaced by an annular rim (a flat ring at y=+0.5) plus four inward-facing
/// inner walls dropping down to a recessed floor at y=`RECESS_FLOOR`. Things
/// placed "on" the dish (coins, relic placeholders) still sit on top of the
/// rim because callers anchor them at +half_height — the recess only affects
/// what the player sees inside the lip.
pub fn build_dish_mesh() -> MeshCpu {
    // Wall thickness in xz (fraction of full extent) and recess floor height
    // (in local y, where the rim top sits at +0.5 and the dish base at -0.5).
    const RIM_THICK: f32 = 0.10;
    const RECESS_FLOOR: f32 = 0.20;
    let inner = 0.5 - RIM_THICK;
    let rim_top = 0.5_f32;

    // Each face is a quad with explicit corners listed CCW when viewed from
    // along the normal. Same convention as `build_pack_mesh`.
    let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
        // ── Outer shell ────────────────────────────────────────────────
        // Bottom (-Y)
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // Outer +X
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // Outer -X
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, -0.5],
            ],
        ),
        // Outer +Z
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        // Outer -Z
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        // ── Rim top: annular ring at y=+0.5 (4 strips) ────────────────
        // North strip (+Z side of the rim)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, inner],
                [-0.5, rim_top, 0.5],
                [0.5, rim_top, 0.5],
                [0.5, rim_top, inner],
            ],
        ),
        // South strip (-Z side of the rim)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, -0.5],
                [-0.5, rim_top, -inner],
                [0.5, rim_top, -inner],
                [0.5, rim_top, -0.5],
            ],
        ),
        // East strip (+X side of the rim, between the N/S strips)
        (
            [0.0, 1.0, 0.0],
            [
                [inner, rim_top, -inner],
                [inner, rim_top, inner],
                [0.5, rim_top, inner],
                [0.5, rim_top, -inner],
            ],
        ),
        // West strip (-X side of the rim, between the N/S strips)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, -inner],
                [-0.5, rim_top, inner],
                [-inner, rim_top, inner],
                [-inner, rim_top, -inner],
            ],
        ),
        // ── Inner walls (face inward toward the recess) ───────────────
        // Inner +X wall at x=+inner, normal -X
        (
            [-1.0, 0.0, 0.0],
            [
                [inner, RECESS_FLOOR, inner],
                [inner, rim_top, inner],
                [inner, rim_top, -inner],
                [inner, RECESS_FLOOR, -inner],
            ],
        ),
        // Inner -X wall at x=-inner, normal +X
        (
            [1.0, 0.0, 0.0],
            [
                [-inner, RECESS_FLOOR, -inner],
                [-inner, rim_top, -inner],
                [-inner, rim_top, inner],
                [-inner, RECESS_FLOOR, inner],
            ],
        ),
        // Inner +Z wall at z=+inner, normal -Z
        (
            [0.0, 0.0, -1.0],
            [
                [-inner, RECESS_FLOOR, inner],
                [-inner, rim_top, inner],
                [inner, rim_top, inner],
                [inner, RECESS_FLOOR, inner],
            ],
        ),
        // Inner -Z wall at z=-inner, normal +Z
        (
            [0.0, 0.0, 1.0],
            [
                [inner, RECESS_FLOOR, -inner],
                [inner, rim_top, -inner],
                [-inner, rim_top, -inner],
                [-inner, RECESS_FLOOR, -inner],
            ],
        ),
        // ── Recess floor at y=RECESS_FLOOR, normal +Y ─────────────────
        (
            [0.0, 1.0, 0.0],
            [
                [-inner, RECESS_FLOOR, -inner],
                [-inner, RECESS_FLOOR, inner],
                [inner, RECESS_FLOOR, inner],
                [inner, RECESS_FLOOR, -inner],
            ],
        ),
    ];

    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(faces.len() * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(faces.len() * 6);
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: *normal,
                uv: *uv,
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        // Darker brass tint — matches the Walnut, Brass & Felt palette
        // (see `COLOR_THEME.md`) and reads as a recessed metal tray under
        // candlelight.
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.32, 0.22, 0.10, 1.0],
            specular_strength: 0.55,
            specular_power: 48.0,
        },
    }
}

/// Round variant of [`build_dish_mesh`] — a circular brass tray with a raised
/// rim and a recessed floor. Uses the same -0.5..0.5 local extents as the
/// square dish so callers can drop it in without changing scale.
///
/// Geometry (from outside in): a flat bottom disc at y=-0.5, an outer
/// cylindrical side wall, an annular rim ring at y=+0.5, an inner
/// cylindrical wall dropping to the recess, and a recessed floor disc at
/// y=`RECESS_FLOOR`. UVs are placeholders ((0,0)/(1,1)) since the dish uses
/// the Plain material with no decal sampling.
pub fn build_round_dish_mesh() -> MeshCpu {
    const SEGS: usize = 32;
    // Alms-bowl proportions: deeper basin, slightly thicker rim, narrow
    // tapered base. The shop's sell-tray dispatch applies a per-instance
    // color (warm/cool tint based on focus/drag state) so the recessed
    // floor reads as "polished brass lighting up" when the player is
    // dragging an item toward it.
    const RIM_THICK: f32 = 0.072;
    const RECESS_FLOOR: f32 = -0.32;
    const OUTER_R: f32 = 0.5;
    const BASE_R: f32 = 0.28;
    const INNER_R: f32 = OUTER_R - RIM_THICK;
    const INNER_BASE_R: f32 = BASE_R - RIM_THICK;
    const RIM_TOP: f32 = 0.5;
    const BOTTOM: f32 = -0.5;

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let angle = |i: usize| (i as f32) / (SEGS as f32) * std::f32::consts::TAU;

    // Bottom disc (normal -Y) at the narrow base radius.
    {
        let center = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [0.0, BOTTOM, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        for i in 0..SEGS {
            let a = angle(i);
            vertices.push(Vertex3dTex {
                position: [BASE_R * a.cos(), BOTTOM, BASE_R * a.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for i in 0..SEGS {
            let a = center + 1 + i as u32;
            let b = center + 1 + ((i as u32 + 1) % SEGS as u32);
            // CCW from below (normal -Y): center, b, a
            indices.extend_from_slice(&[center, b, a]);
        }
    }

    // Outer tapered side wall (normal angled outward + slightly upward to
    // match the slope from BASE_R at the bottom to OUTER_R at the rim).
    {
        let dr = OUTER_R - BASE_R;
        let dy = RIM_TOP - BOTTOM;
        // Slope normal lies in the (radial, +Y) plane; tilt outward by the
        // ratio dy / hyp so the lighting catches the taper.
        let hyp = (dr * dr + dy * dy).sqrt().max(1e-6);
        let nr = dy / hyp; // radial component
        let ny = dr / hyp; // upward component
        for i in 0..SEGS {
            let a0 = angle(i);
            let a1 = angle(i + 1);
            let (cx0, cz0) = (a0.cos(), a0.sin());
            let (cx1, cz1) = (a1.cos(), a1.sin());
            let n0 = [nr * cx0, ny, nr * cz0];
            let n1 = [nr * cx1, ny, nr * cz1];
            let (bx0, bz0) = (BASE_R * cx0, BASE_R * cz0);
            let (bx1, bz1) = (BASE_R * cx1, BASE_R * cz1);
            let (tx0, tz0) = (OUTER_R * cx0, OUTER_R * cz0);
            let (tx1, tz1) = (OUTER_R * cx1, OUTER_R * cz1);
            let base = vertices.len() as u32;
            vertices.push(Vertex3dTex {
                position: [bx0, BOTTOM, bz0],
                normal: n0,
                uv: [0.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [bx1, BOTTOM, bz1],
                normal: n1,
                uv: [1.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx1, RIM_TOP, tz1],
                normal: n1,
                uv: [1.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx0, RIM_TOP, tz0],
                normal: n0,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    // Annular rim top (normal +Y) — flat ring between INNER_R and OUTER_R.
    for i in 0..SEGS {
        let a0 = angle(i);
        let a1 = angle(i + 1);
        let (ox0, oz0) = (OUTER_R * a0.cos(), OUTER_R * a0.sin());
        let (ox1, oz1) = (OUTER_R * a1.cos(), OUTER_R * a1.sin());
        let (ix0, iz0) = (INNER_R * a0.cos(), INNER_R * a0.sin());
        let (ix1, iz1) = (INNER_R * a1.cos(), INNER_R * a1.sin());
        let base = vertices.len() as u32;
        let n = [0.0, 1.0, 0.0];
        vertices.push(Vertex3dTex {
            position: [ix0, RIM_TOP, iz0],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox0, RIM_TOP, oz0],
            normal: n,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, RIM_TOP, oz1],
            normal: n,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, RIM_TOP, iz1],
            normal: n,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // Inner tapered wall (normal radially inward + slightly downward).
    {
        let dr = INNER_R - INNER_BASE_R;
        let dy = RIM_TOP - RECESS_FLOOR;
        let hyp = (dr * dr + dy * dy).sqrt().max(1e-6);
        let nr = -dy / hyp; // inward radial component (toward axis)
        let ny = -dr / hyp; // downward component
        for i in 0..SEGS {
            let a0 = angle(i);
            let a1 = angle(i + 1);
            let (cx0, cz0) = (a0.cos(), a0.sin());
            let (cx1, cz1) = (a1.cos(), a1.sin());
            let n0 = [nr * cx0, ny, nr * cz0];
            let n1 = [nr * cx1, ny, nr * cz1];
            let (bx0, bz0) = (INNER_BASE_R * cx0, INNER_BASE_R * cz0);
            let (bx1, bz1) = (INNER_BASE_R * cx1, INNER_BASE_R * cz1);
            let (tx0, tz0) = (INNER_R * cx0, INNER_R * cz0);
            let (tx1, tz1) = (INNER_R * cx1, INNER_R * cz1);
            let base = vertices.len() as u32;
            // CCW when viewed from inside the bowl (looking inward).
            vertices.push(Vertex3dTex {
                position: [bx1, RECESS_FLOOR, bz1],
                normal: n1,
                uv: [0.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [bx0, RECESS_FLOOR, bz0],
                normal: n0,
                uv: [1.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx0, RIM_TOP, tz0],
                normal: n0,
                uv: [1.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx1, RIM_TOP, tz1],
                normal: n1,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    // Recessed floor disc (normal +Y) at the narrow inner-base radius.
    {
        let center = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [0.0, RECESS_FLOOR, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5, 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        for i in 0..SEGS {
            let a = angle(i);
            vertices.push(Vertex3dTex {
                position: [INNER_BASE_R * a.cos(), RECESS_FLOOR, INNER_BASE_R * a.sin()],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for i in 0..SEGS {
            let a = center + 1 + i as u32;
            let b = center + 1 + ((i as u32 + 1) % SEGS as u32);
            // CCW from above (normal +Y): center, a, b
            indices.extend_from_slice(&[center, a, b]);
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Brass default. Shop's sell-tray dispatch overrides this
            // with the focus/drag-driven `sell_tray_color`, but the
            // brass tone shows through for any other caller.
            base_color: [0.78, 0.58, 0.28, 1.0],
            specular_strength: 0.88,
            specular_power: 104.0,
        },
    }
}

/// Hand-thrown porcelain dish — a rounded ceramic bowl with a continuous
/// curved profile, smooth-shaded side wall, rolled lip, recessed inner
/// cavity, and a small foot ring at the base. Local extents -0.5..+0.5
/// on each axis (Y-up); revolve around Y. Side normals are smoothly
/// interpolated along the profile so the dish reads as one continuous
/// thrown form rather than a stack of cylindrical bands.
///
/// Slight per-segment radial wobble (sub-mm at game scale) breaks the
/// silhouette away from machine-perfect circular so the dish reads as
/// handcrafted.
pub fn build_porcelain_dish_mesh() -> MeshCpu {
    // Angular tessellation. 48 around the perimeter is enough to keep
    // the silhouette smooth at typical viewing sizes without wobbling
    // pixels at close zoom.
    const SEGS: usize = 48;
    // Radial wobble amplitude (fraction of OUTER_R). Hand-thrown dishes
    // are rarely perfectly circular; ±0.4% reads as "made by hand" but
    // doesn't deform the silhouette into something visibly lumpy.
    const WOBBLE_AMP: f32 = 0.004;

    // ── Outer profile (foot ring → base curve → swelling wall → rolled
    //    rim). Each sample is (radius, y, n_radial, n_y) where the
    //    normal is the *outward* surface normal at that profile point.
    //    Normals are pre-computed analytically from the local profile
    //    tangent so shading reads as a continuous curved surface.
    // ──────────────────────────────────────────────────────────────
    let outer_profile: [(f32, f32, f32, f32); 9] = {
        // Profile points along the silhouette, before normals.
        // (r, y) only — y goes -0.5 (bottom of foot) to +0.5 (rim top).
        let pts: [(f32, f32); 9] = [
            (0.22, -0.50), // 0: outer foot ring base
            (0.24, -0.46), // 1: foot meets underside curve
            (0.30, -0.38), // 2: belly starts to curve outward
            (0.40, -0.22), // 3: lower belly
            (0.46, -0.05), // 4: widest belly
            (0.49, 0.15),  // 5: upper wall, slowing
            (0.50, 0.32),  // 6: just below rim
            (0.50, 0.46),  // 7: rim shoulder
            (0.49, 0.50),  // 8: rim top, rolled slightly inward
        ];
        let mut out = [(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); 9];
        for i in 0..pts.len() {
            // Tangent is the average of the segments adjacent to this
            // point (central difference). At the endpoints we use the
            // single available segment.
            let (pr, py) = pts[i];
            let (tr, ty) = if i == 0 {
                (pts[1].0 - pts[0].0, pts[1].1 - pts[0].1)
            } else if i == pts.len() - 1 {
                (pts[i].0 - pts[i - 1].0, pts[i].1 - pts[i - 1].1)
            } else {
                (pts[i + 1].0 - pts[i - 1].0, pts[i + 1].1 - pts[i - 1].1)
            };
            // Outward normal is perpendicular to the tangent, rotated 90°
            // clockwise (so positive radial slope → upward-outward normal).
            // Tangent (tr, ty) → normal (ty, -tr), then normalize.
            let mut nr = ty;
            let mut ny = -tr;
            let mag = (nr * nr + ny * ny).sqrt().max(1e-6);
            nr /= mag;
            ny /= mag;
            out[i] = (pr, py, nr, ny);
        }
        out
    };

    // ── Inner profile (rim lip → inner wall curve → recessed floor).
    //    Mirrors the outer profile but inset to give the dish wall its
    //    thickness, with a soft U-shaped basin floor.
    // ──────────────────────────────────────────────────────────────
    let inner_profile: [(f32, f32, f32, f32); 7] = {
        let pts: [(f32, f32); 7] = [
            (0.46, 0.50),  // 0: inner rim edge (just below outer rim top)
            (0.45, 0.42),  // 1: inner shoulder, curves inward
            (0.42, 0.25),  // 2: inner upper wall
            (0.36, 0.05),  // 3: inner middle wall
            (0.26, -0.18), // 4: lower inner wall
            (0.14, -0.30), // 5: basin shoulder
            (0.00, -0.34), // 6: basin floor centre
        ];
        let mut out = [(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); 7];
        for i in 0..pts.len() {
            let (pr, py) = pts[i];
            let (tr, ty) = if i == 0 {
                (pts[1].0 - pts[0].0, pts[1].1 - pts[0].1)
            } else if i == pts.len() - 1 {
                (pts[i].0 - pts[i - 1].0, pts[i].1 - pts[i - 1].1)
            } else {
                (pts[i + 1].0 - pts[i - 1].0, pts[i + 1].1 - pts[i - 1].1)
            };
            // Inward-facing normal: rotate the tangent the other way
            // (the inside of the bowl points toward the central axis).
            let mut nr = -ty;
            let mut ny = tr;
            let mag = (nr * nr + ny * ny).sqrt().max(1e-6);
            nr /= mag;
            ny /= mag;
            out[i] = (pr, py, nr, ny);
        }
        out
    };

    // Hash-driven radial wobble. Stable per-segment so neighboring
    // vertices share the same wobble offset and the silhouette stays
    // continuous — only the cross-section perimeter is non-circular.
    let wobble = |seg: usize| -> f32 {
        let s = seg as f32;
        let h = ((s * 12.9898).sin() * 43_758.547).fract();
        (h - 0.5) * 2.0 * WOBBLE_AMP
    };

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let angle = |i: usize| (i as f32) / (SEGS as f32) * std::f32::consts::TAU;

    // ── Outer wall: lathe the outer profile around Y. Each profile
    //    layer emits one ring of SEGS+1 vertices (last == first for UV
    //    continuity). Adjacent layers stitch into a quad strip.
    // ──────────────────────────────────────────────────────────────
    let outer_first = vertices.len() as u32;
    for &(r, y, nr, ny) in outer_profile.iter() {
        for i in 0..=SEGS {
            let a = angle(i % SEGS);
            let w = wobble(i % SEGS);
            let rr = r * (1.0 + w);
            let (cx, cz) = (a.cos(), a.sin());
            vertices.push(Vertex3dTex {
                position: [rr * cx, y, rr * cz],
                normal: [nr * cx, ny, nr * cz],
                uv: [(i as f32) / (SEGS as f32), (y + 0.5)],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }
    let row = (SEGS + 1) as u32;
    for layer in 0..(outer_profile.len() - 1) as u32 {
        for i in 0..SEGS as u32 {
            let i00 = outer_first + layer * row + i;
            let i01 = outer_first + layer * row + i + 1;
            let i10 = outer_first + (layer + 1) * row + i;
            let i11 = outer_first + (layer + 1) * row + i + 1;
            // CCW from outside (normal points outward).
            indices.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }

    // ── Foot ring underside (normal -Y) — closes the foot at the
    //    bottom. Triangle fan from the centre out to the foot perimeter.
    //    The inner basin opens out the *top* so this disc has no hole.
    // ──────────────────────────────────────────────────────────────
    {
        let bottom_y = outer_profile[0].1;
        let bottom_r = outer_profile[0].0;
        let center = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [0.0, bottom_y, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        for i in 0..SEGS {
            let a = angle(i);
            let w = wobble(i);
            let rr = bottom_r * (1.0 + w);
            vertices.push(Vertex3dTex {
                position: [rr * a.cos(), bottom_y, rr * a.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for i in 0..SEGS {
            let a = center + 1 + i as u32;
            let b = center + 1 + ((i as u32 + 1) % SEGS as u32);
            // CCW from below (normal -Y): center, b, a
            indices.extend_from_slice(&[center, b, a]);
        }
    }

    // ── Rim cap: a thin annular ring on top connecting the outer rim
    //    edge (outer_profile last point) to the inner rim edge (inner
    //    profile first point). Normal +Y so the rim catches an
    //    overhead candle highlight that visually "reads" as a polished
    //    glaze edge.
    // ──────────────────────────────────────────────────────────────
    {
        let outer_last = outer_profile[outer_profile.len() - 1];
        let inner_first = inner_profile[0];
        for i in 0..SEGS {
            let a0 = angle(i);
            let a1 = angle(i + 1);
            let w0 = wobble(i);
            let w1 = wobble((i + 1) % SEGS);
            let or0 = outer_last.0 * (1.0 + w0);
            let or1 = outer_last.0 * (1.0 + w1);
            let ir0 = inner_first.0 * (1.0 + w0);
            let ir1 = inner_first.0 * (1.0 + w1);
            let oy = outer_last.1;
            let iy = inner_first.1;
            let n = [0.0_f32, 1.0, 0.0];
            let base = vertices.len() as u32;
            vertices.push(Vertex3dTex {
                position: [or0 * a0.cos(), oy, or0 * a0.sin()],
                normal: n,
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [or1 * a1.cos(), oy, or1 * a1.sin()],
                normal: n,
                uv: [1.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [ir1 * a1.cos(), iy, ir1 * a1.sin()],
                normal: n,
                uv: [1.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [ir0 * a0.cos(), iy, ir0 * a0.sin()],
                normal: n,
                uv: [0.0, 1.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            // CCW from above.
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    // ── Inner wall + basin: lathe the inner profile (which already
    //    includes the floor-centre point at radius 0) around Y. The
    //    last "ring" collapses to the central axis so we emit a fan
    //    of degenerate triangles closing the basin floor.
    // ──────────────────────────────────────────────────────────────
    let inner_first = vertices.len() as u32;
    for &(r, y, nr, ny) in inner_profile.iter() {
        for i in 0..=SEGS {
            let a = angle(i % SEGS);
            let w = wobble(i % SEGS);
            let rr = r * (1.0 + w);
            let (cx, cz) = (a.cos(), a.sin());
            vertices.push(Vertex3dTex {
                position: [rr * cx, y, rr * cz],
                normal: [nr * cx, ny, nr * cz],
                uv: [(i as f32) / (SEGS as f32), 1.0 - (y + 0.5)],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }
    for layer in 0..(inner_profile.len() - 1) as u32 {
        for i in 0..SEGS as u32 {
            let i00 = inner_first + layer * row + i;
            let i01 = inner_first + layer * row + i + 1;
            let i10 = inner_first + (layer + 1) * row + i;
            let i11 = inner_first + (layer + 1) * row + i + 1;
            // CCW from inside (normal points inward toward axis).
            indices.extend_from_slice(&[i00, i01, i10, i01, i11, i10]);
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Porcelain,
            base_color: color::PARCHMENT,
            specular_strength: 0.7,
            specular_power: 128.0,
        },
    }
}

/// Build a small octagonal relic body for both in-world props and UI turntable
/// viewers. The broad front/back faces read well with generated object art
/// textures, while the beveled outline catches highlights more like a keepsake
/// medallion or carved curio than the old placeholder cube.
pub fn build_relic_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    fn push_quad(
        vertices: &mut Vec<Vertex3dTex>,
        indices: &mut Vec<u32>,
        normal: [f32; 3],
        corners: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
    ) {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal,
                uv: *uv,
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // Use enough radial segments that the shared fallback relic body reads as
    // a smooth medallion/keepsake in the collection turntable, rather than an
    // obvious octagon placeholder.
    const SIDES: usize = 24;
    const HALF_Y: f32 = 0.5;
    const R: f32 = 0.5;
    const UV_CENTER: [f32; 2] = [0.5, 0.5];

    let ring: Vec<[f32; 3]> = (0..SIDES)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / SIDES as f32 + std::f32::consts::FRAC_PI_8;
            [a.cos() * R, 0.0, a.sin() * R]
        })
        .collect();

    // Front cap (+Y)
    let front_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, HALF_Y, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: UV_CENTER,
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p[0], HALF_Y, p[2]],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5 + p[0], 0.5 + p[2]],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let a = front_center + 1 + i as u32;
        let b = front_center + 1 + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[front_center, a, b]);
    }

    // Back cap (-Y)
    let back_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, -HALF_Y, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: UV_CENTER,
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p[0], -HALF_Y, p[2]],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5 + p[0], 0.5 + p[2]],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let a = back_center + 1 + i as u32;
        let b = back_center + 1 + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[back_center, b, a]);
    }

    // Side wall quads.
    for i in 0..SIDES {
        let p0 = ring[i];
        let p1 = ring[(i + 1) % SIDES];
        let edge = glam::Vec3::new(p1[0] - p0[0], 0.0, p1[2] - p0[2]);
        let normal = glam::Vec3::new(edge.z, 0.0, -edge.x).normalize_or_zero();
        push_quad(
            &mut vertices,
            &mut indices,
            [normal.x, normal.y, normal.z],
            [
                [p0[0], -HALF_Y, p0[2]],
                [p1[0], -HALF_Y, p1[2]],
                [p1[0], HALF_Y, p1[2]],
                [p0[0], HALF_Y, p0[2]],
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        );
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.82, 0.74, 0.60, 1.0],
            specular_strength: 0.32,
            specular_power: 28.0,
        },
    }
}

#[inline]
fn triangle_signed_area_xz(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    let e1x = b[0] - a[0];
    let e1z = b[2] - a[2];
    let e2x = c[0] - a[0];
    let e2z = c[2] - a[2];
    0.5 * (e1x * e2z - e1z * e2x)
}

/// Sum signed areas of front-cap (+Y) triangles projected to XZ. Holes subtract
/// from outers so the magnitude matches physical silhouette area.
fn relic_front_cap_net_area_xz(vertices: &[Vertex3dTex], indices: &[u32]) -> f32 {
    const HALF_Y: f32 = 0.5;
    const EPS: f32 = 2e-3;
    let mut sum = 0.0_f32;
    for tri in indices.chunks_exact(3) {
        let v0 = &vertices[tri[0] as usize];
        let v1 = &vertices[tri[1] as usize];
        let v2 = &vertices[tri[2] as usize];
        if (v0.position[1] - HALF_Y).abs() > EPS {
            continue;
        }
        if (v1.position[1] - HALF_Y).abs() > EPS {
            continue;
        }
        if (v2.position[1] - HALF_Y).abs() > EPS {
            continue;
        }
        sum += triangle_signed_area_xz(&v0.position, &v1.position, &v2.position);
    }
    sum
}

/// Target front-cap area in XZ used to equalize on-screen footprint across relics
/// (disk of radius 0.5: same order as the legacy max-edge silhouette fit for a square).
const RELIC_CAP_REFERENCE_AREA_XZ: f32 = std::f32::consts::PI * 0.25;

fn scale_relic_mesh_vertex_positions(vertices: &mut [Vertex3dTex], scale: f32) {
    for v in vertices.iter_mut() {
        v.position[0] *= scale;
        v.position[1] *= scale;
        v.position[2] *= scale;
    }
}

/// Alpha cutoff for boss-icon extrusion (processed atlas cells use real transparency).
const BOSS_ICON_ALPHA_SOLID_THRESH: u8 = 24;

/// Build a relic mesh whose cap silhouette is extracted from the supplied
/// RGBA. Relic masks use **rec.709 luminance** ≥ 0.45 (many `*_mask.png` files
/// keep alpha fully opaque and encode the silhouette only in RGB).
///
/// Pipeline:
/// 1. Threshold the mask into a solid/empty bitmap.
/// 2. Marching squares over that bitmap produces one outer contour per island
///    plus one contour per interior hole.
/// 3. Contours are simplified (Douglas–Peucker, ~0.75 px tolerance) to cut
///    vertex count on long smooth edges without losing concavities.
/// 4. Lyon tessellates the resulting polygon-with-holes into triangles for the
///    front cap; the back cap mirrors those triangles with reversed winding,
///    and the side walls extrude every contour edge.
/// 5. A uniform scale adjusts all vertices so the front-cap footprint in XZ
///    matches [`RELIC_CAP_REFERENCE_AREA_XZ`] (equal apparent “pin” size across relics).
pub fn build_relic_mesh_from_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    source_label: &str,
) -> Option<MeshCpu> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    let w = width as usize;
    let h = height as usize;
    let solid = silhouette_solid_luma(rgba, w, h);
    build_extruded_pin_mesh_from_solid(&solid, width, height, source_label)
}

/// Boss archive / HUD 3D icon mesh — silhouette from **alpha** only so black-painted
/// art still extrudes correctly (luma would treat black pixels as empty).
pub fn build_boss_icon_mesh_from_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    source_label: &str,
) -> Option<MeshCpu> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    let w = width as usize;
    let h = height as usize;
    let solid = silhouette_solid_alpha(rgba, w, h, BOSS_ICON_ALPHA_SOLID_THRESH);
    build_extruded_pin_mesh_from_solid(&solid, width, height, source_label)
}

fn silhouette_solid_alpha(rgba: &[u8], w: usize, h: usize, alpha_thresh: u8) -> Vec<bool> {
    let mut solid = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            solid[y * w + x] = rgba[idx + 3] >= alpha_thresh;
        }
    }
    solid
}

fn silhouette_solid_luma(rgba: &[u8], w: usize, h: usize) -> Vec<bool> {
    let mut solid = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            let r = rgba[idx] as f32;
            let g = rgba[idx + 1] as f32;
            let b = rgba[idx + 2] as f32;
            solid[y * w + x] = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0 >= 0.45;
        }
    }
    solid
}

fn build_extruded_pin_mesh_from_solid(
    solid: &[bool],
    width: u32,
    height: u32,
    source_label: &str,
) -> Option<MeshCpu> {
    let w = width as usize;
    let h = height as usize;

    // Trace the silhouette as closed polygons in pixel-space. Marching squares
    // walks the 2×2 pixel grid and emits every boundary between solid and empty
    // cells; the segments are stitched into closed loops and classified as
    // outer contours or interior holes. Each loop captures concave edges that
    // the old radial sweep would have convex-hulled over.
    let contours = trace_silhouette_contours(solid, w as i32, h as i32);
    let contours = filter_border_noise_loops(contours, width as f32, height as f32, source_label);
    let polygons = group_contours_into_polygons(contours);
    if polygons.is_empty() {
        return None;
    }

    // Shared scale: normalise across the *union* of polygon bounding boxes so
    // multi-part silhouettes keep their relative sizing instead of each island
    // being blown up to fill ±0.5 independently.
    let (mut ux_min, mut uy_min) = (f32::INFINITY, f32::INFINITY);
    let (mut ux_max, mut uy_max) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for poly in &polygons {
        for &(x, y) in &poly.outer {
            ux_min = ux_min.min(x);
            uy_min = uy_min.min(y);
            ux_max = ux_max.max(x);
            uy_max = uy_max.max(y);
        }
    }
    let union_cx = 0.5 * (ux_min + ux_max);
    let union_cy = 0.5 * (uy_min + uy_max);
    let union_extent = ((ux_max - ux_min).max(uy_max - uy_min) * 0.5).max(1.0);

    let inv_w = 1.0 / width as f32;
    let inv_h = 1.0 / height as f32;
    const HALF_Y: f32 = 0.5;

    // Pixel-space → mesh-local: center on the union centroid and scale so the
    // wider of the two axes fills [-0.5, +0.5]. UVs come from raw pixel coords
    // so the albedo is sampled from where the art actually lives in the image.
    let to_local = |px_x: f32, px_y: f32| -> (f32, f32) {
        (
            (px_x - union_cx) * 0.5 / union_extent,
            (px_y - union_cy) * 0.5 / union_extent,
        )
    };
    let to_uv = |px_x: f32, px_y: f32| -> [f32; 2] { [px_x * inv_w, px_y * inv_h] };

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut emitted_any = false;

    for poly in &polygons {
        // Feed the polygon to lyon as an outer loop + one sub-path per hole.
        let mut builder = Path::builder();
        if !push_ring_to_lyon_path(&mut builder, &poly.outer) {
            continue;
        }
        for hole in &poly.holes {
            push_ring_to_lyon_path(&mut builder, hole);
        }
        let path = builder.build();

        let mut buffers: VertexBuffers<(f32, f32), u32> = VertexBuffers::new();
        let mut tess = FillTessellator::new();
        let options = FillOptions::tolerance(0.25).with_fill_rule(FillRule::NonZero);
        let result = tess.tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                let p = v.position();
                (p.x, p.y)
            }),
        );
        if result.is_err() || buffers.indices.len() < 3 {
            continue;
        }

        // Front cap: emit each tessellation vertex with normal +Y.
        let base_front = vertices.len() as u32;
        for &(x, y) in &buffers.vertices {
            let (lx, lz) = to_local(x, y);
            vertices.push(Vertex3dTex {
                position: [lx, HALF_Y, lz],
                normal: [0.0, 1.0, 0.0],
                uv: to_uv(x, y),
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for tri in buffers.indices.chunks_exact(3) {
            indices.extend_from_slice(&[
                base_front + tri[0],
                base_front + tri[1],
                base_front + tri[2],
            ]);
        }

        // Back cap: duplicate vertices with flipped normal and reversed winding
        // so both caps are front-facing from their respective sides.
        let base_back = vertices.len() as u32;
        for &(x, y) in &buffers.vertices {
            let (lx, lz) = to_local(x, y);
            vertices.push(Vertex3dTex {
                position: [lx, -HALF_Y, lz],
                normal: [0.0, -1.0, 0.0],
                uv: to_uv(x, y),
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        for tri in buffers.indices.chunks_exact(3) {
            indices.extend_from_slice(&[
                base_back + tri[0],
                base_back + tri[2],
                base_back + tri[1],
            ]);
        }

        // Side walls: extrude every contour edge (outer + holes). The
        // polygons come from marching squares which produces consistently
        // oriented closed loops, so consecutive points in each ring already
        // give us the wall edges.
        let mut rings: Vec<&Vec<(f32, f32)>> = Vec::with_capacity(1 + poly.holes.len());
        rings.push(&poly.outer);
        for hole in &poly.holes {
            rings.push(hole);
        }
        for ring in rings {
            let n = ring.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let (x0, y0) = ring[i];
                let (x1, y1) = ring[(i + 1) % n];
                let (lx0, lz0) = to_local(x0, y0);
                let (lx1, lz1) = to_local(x1, y1);
                let edge = glam::Vec3::new(lx1 - lx0, 0.0, lz1 - lz0);
                let normal = glam::Vec3::new(edge.z, 0.0, -edge.x).normalize_or_zero();
                let uv0 = to_uv(x0, y0);
                let uv1 = to_uv(x1, y1);
                let base = vertices.len() as u32;
                // Two bottom, two top (CCW when viewed from outside).
                vertices.push(Vertex3dTex {
                    position: [lx0, -HALF_Y, lz0],
                    normal: [normal.x, normal.y, normal.z],
                    uv: uv0,
                    tangent: Vertex3dTex::DEFAULT_TANGENT,
                    uv_emr: [0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                });
                vertices.push(Vertex3dTex {
                    position: [lx1, -HALF_Y, lz1],
                    normal: [normal.x, normal.y, normal.z],
                    uv: uv1,
                    tangent: Vertex3dTex::DEFAULT_TANGENT,
                    uv_emr: [0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                });
                vertices.push(Vertex3dTex {
                    position: [lx1, HALF_Y, lz1],
                    normal: [normal.x, normal.y, normal.z],
                    uv: uv1,
                    tangent: Vertex3dTex::DEFAULT_TANGENT,
                    uv_emr: [0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                });
                vertices.push(Vertex3dTex {
                    position: [lx0, HALF_Y, lz0],
                    normal: [normal.x, normal.y, normal.z],
                    uv: uv0,
                    tangent: Vertex3dTex::DEFAULT_TANGENT,
                    uv_emr: [0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                });
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }

        emitted_any = true;
    }

    if !emitted_any {
        return None;
    }

    // Equalize projected footprint: sparse silhouettes (rings, tall narrow art)
    // scale up so each relic's front cap has a similar XZ area in mesh space.
    let cap_net = relic_front_cap_net_area_xz(&vertices, &indices);
    let cap_mag = cap_net.abs().max(1e-10);
    let scale = (RELIC_CAP_REFERENCE_AREA_XZ / cap_mag)
        .sqrt()
        .clamp(0.35_f32, 3.5_f32);
    if (scale - 1.0).abs() > 1e-4 {
        scale_relic_mesh_vertex_positions(&mut vertices, scale);
    }

    Some(MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.82, 0.74, 0.60, 1.0],
            specular_strength: 0.32,
            specular_power: 28.0,
        },
    })
}

/// Drop marching-squares loops that ride the image border when at least one
/// interior outer loop exists.
///
/// Some authored masks contain faint anti-aliased edge noise that crosses the
/// luma threshold and creates a giant border-hugging contour. Extruding that
/// contour builds bogus side walls in 3D (often seen as white wire/triangle
/// artifacts in inspect mode). We keep border loops only when they are the only
/// outer silhouette available so full-bleed masks still work.
///
/// `source_label` identifies the mask in diagnostics (e.g. asset path). Tiny
/// drop counts are logged at debug level since a stray rim pixel or two is
/// usually sub-pixel anti-aliasing on an otherwise clean silhouette and not
/// worth pestering the developer about.
fn filter_border_noise_loops(
    loops: Vec<Vec<(f32, f32)>>,
    width: f32,
    height: f32,
    source_label: &str,
) -> Vec<Vec<(f32, f32)>> {
    if loops.is_empty() {
        return loops;
    }
    let x_max = width.max(1.0);
    let y_max = height.max(1.0);
    // Marching-squares loops sit on edge midpoints, so border-adjacent loops
    // often land at ~0.5 and ~(w-0.5)/(h-0.5), not exactly 0 and w/h.
    const EDGE_EPS: f32 = 1.0;
    let touches_border = |ring: &[(f32, f32)]| -> bool {
        let (xmin, ymin, xmax, ymax) = bbox(ring);
        xmin <= EDGE_EPS
            || ymin <= EDGE_EPS
            || xmax >= (x_max - EDGE_EPS)
            || ymax >= (y_max - EDGE_EPS)
    };
    let has_non_border_outer = loops
        .iter()
        .any(|ring| ring.len() >= 3 && signed_area_f32(ring) > 0.0 && !touches_border(ring));
    if !has_non_border_outer {
        return loops;
    }
    let total_loops = loops.len();
    let filtered: Vec<Vec<(f32, f32)>> = loops
        .into_iter()
        .filter(|ring| !touches_border(ring))
        .collect();
    let dropped = total_loops.saturating_sub(filtered.len());
    if dropped > 0 {
        // Heuristic: a handful of stray border loops is almost always sub-pixel
        // anti-aliasing on the rim and there is nothing the developer can do.
        // Hundreds of loops indicates a noisy mask edge that should be cleaned
        // up — keep that loud.
        const NOISY_LOOP_THRESHOLD: usize = 16;
        let w = width as u32;
        let h = height as u32;
        if dropped >= NOISY_LOOP_THRESHOLD {
            log::warn!(
                "relic silhouette `{source_label}` ({w}x{h}): dropped {dropped} border contour \
                 loop(s) as mask noise. Clean the mask: set the outermost border pixels to pure \
                 black (0,0,0) or trim the silhouette so it no longer touches the image edge.",
            );
        } else {
            log::debug!(
                "relic silhouette `{source_label}` ({w}x{h}): dropped {dropped} border contour \
                 loop(s) (likely sub-pixel rim anti-aliasing; not actionable)",
            );
        }
    }
    filtered
}

/// A closed polygon in pixel coords: one outer contour and zero or more holes.
struct SilhouettePolygon {
    outer: Vec<(f32, f32)>,
    holes: Vec<Vec<(f32, f32)>>,
}

/// Marching-squares contour tracing on a boolean grid.
///
/// The grid has `w × h` cells; we walk the `(w+1) × (h+1)` corner grid and for
/// every 2×2 window emit zero, one, or two line segments along cell edges,
/// depending on which of the 4 corners are solid. Segments are then stitched
/// into closed loops by chaining head-to-tail. Returned loops alternate in
/// winding: outer contours are CCW (positive signed area in image coords, Y
/// increasing downward means CCW-image is actually visually CW — the sign
/// depends on the axis convention, see `group_contours_into_polygons`).
fn trace_silhouette_contours(solid: &[bool], w: i32, h: i32) -> Vec<Vec<(f32, f32)>> {
    use rustc_hash::FxHashMap;

    // Cells are indexed by top-left pixel (cx, cy); the 4 corners used for
    // the marching-squares lookup are the pixel centers (treated as solid or
    // empty per `solid`). Standard 4-bit configuration:
    //   bit 0 = TL, bit 1 = TR, bit 2 = BR, bit 3 = BL.
    // Segments connect edge midpoints; each segment is oriented so solid is
    // to the right of the segment direction, which makes outer contours
    // clockwise in image coords (Y-down).
    let at = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= w || y >= h {
            false
        } else {
            solid[(y * w + x) as usize]
        }
    };

    // Segment endpoints are stored as integer keys at doubled resolution so
    // midpoints land on integers: a corner (cx, cy) → (2*cx, 2*cy); a top edge
    // midpoint of cell (cx, cy) → (2*cx+1, 2*cy). This keeps the stitching
    // hashmap fast and exact.
    type Key = (i32, i32);
    let mut adj: FxHashMap<Key, Vec<Key>> = FxHashMap::default();

    for cy in 0..h {
        for cx in 0..w {
            let tl = at(cx, cy);
            let tr = at(cx + 1, cy);
            let br = at(cx + 1, cy + 1);
            let bl = at(cx, cy + 1);
            let code = (tl as u8) | ((tr as u8) << 1) | ((br as u8) << 2) | ((bl as u8) << 3);

            // Edge midpoints (in doubled-integer coords):
            //   T = (2*cx+1, 2*cy)
            //   R = (2*cx+2, 2*cy+1)
            //   B = (2*cx+1, 2*cy+2)
            //   L = (2*cx,   2*cy+1)
            let t: Key = (2 * cx + 1, 2 * cy);
            let r: Key = (2 * cx + 2, 2 * cy + 1);
            let b: Key = (2 * cx + 1, 2 * cy + 2);
            let l: Key = (2 * cx, 2 * cy + 1);

            // Segments oriented so solid is on the RIGHT (yields CW outer in
            // image coords / CCW after a Y flip, but we treat all rings
            // uniformly via even-odd area classification later).
            //
            // Using the standard marching-squares table:
            let mut push = |a: Key, b: Key| {
                adj.entry(a).or_default().push(b);
            };
            match code {
                0 | 15 => {}
                1 => push(t, l),
                2 => push(r, t),
                3 => push(r, l),
                4 => push(b, r),
                5 => {
                    // Ambiguous saddle: treat center as solid → two segments
                    // that don't cross.
                    push(t, r);
                    push(b, l);
                }
                6 => push(b, t),
                7 => push(b, l),
                8 => push(l, b),
                9 => push(t, b),
                10 => {
                    push(r, b);
                    push(l, t);
                }
                11 => push(r, b),
                12 => push(l, r),
                13 => push(t, r),
                14 => push(l, t),
                _ => {}
            }
        }
    }

    // Stitch directed segments into closed loops.
    let mut loops: Vec<Vec<(f32, f32)>> = Vec::new();
    while let Some((&start, _)) = adj.iter().find(|(_, v)| !v.is_empty()) {
        let mut loop_pts: Vec<Key> = vec![start];
        let mut cur = start;
        while let Some(next) = adj.get_mut(&cur).and_then(|v| v.pop()) {
            if next == start {
                break;
            }
            loop_pts.push(next);
            cur = next;
            if loop_pts.len() > 1_000_000 {
                break; // safety valve
            }
        }
        if loop_pts.len() >= 3 {
            // Convert doubled-integer coords back to pixel-space floats (÷2).
            let mut pts: Vec<(f32, f32)> = loop_pts
                .into_iter()
                .map(|(x, y)| (x as f32 * 0.5, y as f32 * 0.5))
                .collect();
            simplify_polyline(&mut pts, 0.5);
            if pts.len() >= 3 {
                loops.push(pts);
            }
        }
    }
    loops
}

/// Simplify a closed ring by dropping vertices whose perpendicular distance
/// from the segment through their neighbors is under `tolerance` pixels.
/// Runs multiple passes until the ring stabilises. Cheaper and simpler than
/// Douglas–Peucker, and for axis-aligned marching-squares output (which is
/// mostly collinear runs broken by 90° corners) it converges in one pass.
fn simplify_polyline(pts: &mut Vec<(f32, f32)>, tolerance: f32) {
    if pts.len() < 4 {
        return;
    }
    let tol_sq = tolerance * tolerance;
    loop {
        let n = pts.len();
        if n < 4 {
            return;
        }
        let mut keep = vec![true; n];
        let mut removed = false;
        for i in 0..n {
            let prev = pts[(i + n - 1) % n];
            let cur = pts[i];
            let next = pts[(i + 1) % n];
            let dx = next.0 - prev.0;
            let dy = next.1 - prev.1;
            let seg_len_sq = dx * dx + dy * dy;
            let d_sq = if seg_len_sq < 1e-6 {
                let ex = cur.0 - prev.0;
                let ey = cur.1 - prev.1;
                ex * ex + ey * ey
            } else {
                let cross = (cur.1 - prev.1) * dx - (cur.0 - prev.0) * dy;
                (cross * cross) / seg_len_sq
            };
            if d_sq <= tol_sq {
                // Don't drop two consecutive points in the same pass — dropping
                // both neighbors of a kept vertex can distort the shape.
                let prev_kept = keep[(i + n - 1) % n];
                if prev_kept {
                    keep[i] = false;
                    removed = true;
                }
            }
        }
        if !removed {
            return;
        }
        let filtered: Vec<(f32, f32)> = pts
            .iter()
            .zip(keep.iter())
            .filter(|(_, k)| **k)
            .map(|(&p, _)| p)
            .collect();
        if filtered.len() < 3 || filtered.len() == pts.len() {
            return;
        }
        *pts = filtered;
    }
}

/// Group raw contour loops into (outer, holes) polygons based on ring winding
/// and bounding-box containment. A loop with positive signed area (image coords,
/// Y-down) is an outer contour emitted by marching squares around a solid
/// island; a negative-area loop is a hole around empty interior space. Each
/// hole is assigned to the smallest outer contour whose bounding box contains
/// it.
fn group_contours_into_polygons(loops: Vec<Vec<(f32, f32)>>) -> Vec<SilhouettePolygon> {
    let mut outers: Vec<SilhouettePolygon> = Vec::new();
    let mut holes: Vec<Vec<(f32, f32)>> = Vec::new();
    for lp in loops {
        if lp.len() < 3 {
            continue;
        }
        let area = signed_area_f32(&lp);
        if area.abs() < 0.25 {
            // Skip sub-pixel specks (well below one solid cell) so we don't
            // tessellate mask-authoring noise. A single solid pixel has a
            // ring area of 0.5 px², so this stays permissive enough to keep
            // isolated 1-px features.
            continue;
        }
        // Orientation in image coords (Y-down): marching-squares segments
        // with solid-on-right produce outer loops that are visually CW, which
        // the shoelace formula reports as *positive* signed area (because the
        // formula's sign is inverted relative to math-coord CCW in any Y-down
        // system). So: area > 0 → outer, area < 0 → hole.
        if area > 0.0 {
            outers.push(SilhouettePolygon {
                outer: lp,
                holes: Vec::new(),
            });
        } else {
            holes.push(lp);
        }
    }

    // Assign each hole to the smallest enclosing outer by bbox area.
    for hole in holes {
        let hbb = bbox(&hole);
        let mut best: Option<(usize, f32)> = None;
        for (i, o) in outers.iter().enumerate() {
            let obb = bbox(&o.outer);
            if obb.0 <= hbb.0 && obb.1 <= hbb.1 && obb.2 >= hbb.2 && obb.3 >= hbb.3 {
                let area = (obb.2 - obb.0) * (obb.3 - obb.1);
                if best.is_none_or(|(_, a)| area < a) {
                    best = Some((i, area));
                }
            }
        }
        if let Some((i, _)) = best {
            outers[i].holes.push(hole);
        }
    }

    outers
}

fn signed_area_f32(pts: &[(f32, f32)]) -> f32 {
    let n = pts.len();
    let mut a = 0.0_f32;
    for i in 0..n {
        let j = (i + 1) % n;
        a += (pts[i].0 * pts[j].1) - (pts[j].0 * pts[i].1);
    }
    a * 0.5
}

fn bbox(pts: &[(f32, f32)]) -> (f32, f32, f32, f32) {
    let (mut xmin, mut ymin) = (f32::INFINITY, f32::INFINITY);
    let (mut xmax, mut ymax) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in pts {
        xmin = xmin.min(x);
        ymin = ymin.min(y);
        xmax = xmax.max(x);
        ymax = ymax.max(y);
    }
    (xmin, ymin, xmax, ymax)
}

fn push_ring_to_lyon_path(builder: &mut lyon_path::path::Builder, ring: &[(f32, f32)]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    builder.begin(Point::new(ring[0].0, ring[0].1));
    for &(x, y) in &ring[1..] {
        builder.line_to(Point::new(x, y));
    }
    builder.end(true);
    true
}

#[cfg(test)]
mod silhouette_tests {
    use super::*;

    fn grid(w: i32, h: i32, filled: &[(i32, i32)]) -> Vec<bool> {
        let mut g = vec![false; (w * h) as usize];
        for &(x, y) in filled {
            g[(y * w + x) as usize] = true;
        }
        g
    }

    #[test]
    fn single_solid_cell_produces_one_outer_ring() {
        // 5×5 grid, center pixel solid — 1-px silhouettes survive DP simplification
        // only when the tolerance is fine enough; this test uses the real tolerance.
        let g = grid(5, 5, &[(2, 2)]);
        let loops = trace_silhouette_contours(&g, 5, 5);
        assert!(
            !loops.is_empty(),
            "expected at least one loop for a solid pixel"
        );
        let polys = group_contours_into_polygons(loops);
        assert_eq!(polys.len(), 1, "expected one polygon");
        assert!(polys[0].holes.is_empty());
    }

    #[test]
    fn two_disjoint_cells_produce_two_outer_rings() {
        // 5×3 grid with two isolated solid pixels.
        let g = grid(5, 3, &[(1, 1), (3, 1)]);
        let loops = trace_silhouette_contours(&g, 5, 3);
        let polys = group_contours_into_polygons(loops);
        assert_eq!(polys.len(), 2, "expected two polygons for two islands");
    }

    #[test]
    #[ignore = "hits real asset files; run explicitly"]
    fn real_relic_masks_build_reasonable_meshes() {
        use std::path::Path;
        let asset_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/textures/relics");
        for name in &[
            "joker_tile",
            "beggars_cup",
            "lapis_serpent",
            "eight_treasures",
            "glass_cannon",
            "cosmopolitan",
            "solitary_sage",
            "garden_keeper",
            "gold_idol",
            "green_luck",
            "honor_fury",
            "stone_lantern",
            "multiplier_master",
        ] {
            let p = asset_dir.join(format!("{name}_mask.png"));
            if !p.exists() {
                eprintln!("skip {name}: mask not found");
                continue;
            }
            let img = image::open(&p).unwrap().into_rgba8();
            let (w, h) = img.dimensions();
            let mesh = build_relic_mesh_from_rgba(img.as_raw(), w, h, name);
            match mesh {
                Some(m) => eprintln!(
                    "{name}: {}×{}  verts={}  tris={}",
                    w,
                    h,
                    m.vertices.len(),
                    m.indices.len() / 3,
                ),
                None => eprintln!("{name}: NONE"),
            }
        }
    }

    #[test]
    fn boss_icon_mesh_extrudes_from_alpha_not_luma() {
        let mut rgba = vec![0u8; 16 * 16 * 4];
        for y in 3..13 {
            for x in 3..13 {
                let i = (y * 16 + x) * 4;
                rgba[i] = 0;
                rgba[i + 1] = 0;
                rgba[i + 2] = 0;
                rgba[i + 3] = 255;
            }
        }
        let mesh = build_boss_icon_mesh_from_rgba(&rgba, 16, 16, "test:black_rgb_alpha")
            .expect("boss icon mesh should follow alpha");
        assert!(mesh.indices.len() >= 6 * 3);
        assert!(
            build_relic_mesh_from_rgba(&rgba, 16, 16, "test:black_rgb_alpha").is_none(),
            "luma-based relic path must not treat black RGB as solid"
        );
    }

    #[test]
    fn end_to_end_mesh_has_geometry() {
        // 16×16 mask with a solid 10×10 block centered. Build the full mesh
        // and check we actually got triangles — this catches winding bugs
        // where lyon tessellates an empty region.
        let mut rgba = vec![0u8; 16 * 16 * 4];
        for y in 3..13 {
            for x in 3..13 {
                let i = (y * 16 + x) * 4;
                rgba[i] = 255;
                rgba[i + 1] = 255;
                rgba[i + 2] = 255;
                rgba[i + 3] = 255;
            }
        }
        let mesh = build_relic_mesh_from_rgba(&rgba, 16, 16, "test:solid_block")
            .expect("mesh should build for solid block");
        assert!(
            mesh.indices.len() >= 3,
            "mesh must have at least one triangle"
        );
        assert!(!mesh.vertices.is_empty(), "mesh must have vertices");
        // With two caps + wall quads around a roughly-rectangular ring we
        // should see substantially more than a bare fan.
        assert!(
            mesh.indices.len() >= 6 * 3,
            "expected multiple triangles (got {} indices)",
            mesh.indices.len(),
        );
    }

    #[test]
    fn ring_with_hole_yields_outer_plus_hole() {
        // 5×5 grid: solid 3×3 square centered, with a single-pixel hole in the
        // middle → one outer contour + one hole contour.
        let mut filled = Vec::new();
        for y in 1..4 {
            for x in 1..4 {
                if !(x == 2 && y == 2) {
                    filled.push((x, y));
                }
            }
        }
        let g = grid(5, 5, &filled);
        let loops = trace_silhouette_contours(&g, 5, 5);
        let polys = group_contours_into_polygons(loops);
        assert_eq!(polys.len(), 1, "expected one polygon");
        assert_eq!(polys[0].holes.len(), 1, "expected one hole");
    }

    #[test]
    fn border_noise_loop_is_filtered_when_interior_outer_exists() {
        let border_outer = vec![(0.0, 0.0), (16.0, 0.0), (16.0, 16.0), (0.0, 16.0)];
        let interior_outer = vec![(4.0, 4.0), (12.0, 4.0), (12.0, 12.0), (4.0, 12.0)];
        let loops = vec![border_outer, interior_outer.clone()];
        let filtered = filter_border_noise_loops(loops, 16.0, 16.0, "test");
        assert_eq!(filtered.len(), 1, "border loop should be dropped");
        assert_eq!(filtered[0], interior_outer);
    }

    #[test]
    fn border_loop_kept_when_it_is_only_outer() {
        let border_outer = vec![(0.0, 0.0), (16.0, 0.0), (16.0, 16.0), (0.0, 16.0)];
        let filtered = filter_border_noise_loops(vec![border_outer.clone()], 16.0, 16.0, "test");
        assert_eq!(filtered.len(), 1, "single full-bleed loop should be kept");
        assert_eq!(filtered[0], border_outer);
    }
}
