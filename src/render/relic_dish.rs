//! Procedural meshes for the on-table relic dish + the placeholder relic boxes
//! that sit inside it. Both are simple axis-aligned boxes built once at startup
//! and instanced via [`crate::render::lit_mesh::LitMeshInstance`].
//!
//! The dish is a flat low box centered at local (0,0,0); the relic placeholder
//! is a unit cube spanning -0.5..+0.5 on each axis so a per-instance scale can
//! turn it into rectangular prisms of varying sizes.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Unit cube (-0.5..+0.5) for **tile booster packs**, oriented as an
/// upright card: broad faces on ±Y (front/back, toward/away from camera),
/// width on ±X, height on ±Z. Callers scale via `Object3d.extents` with
/// `[width, thickness, height]` where `width = height * aspect_w_over_h`.
///
/// UVs on the -Y (camera-facing) face map the full texture: U along +X,
/// V along -Z, so the image reads upright with (0,0) at the top-left.
/// The +Y back mirrors the front. Thin edges (±X, ±Z) sample the texture
/// corner (0,0) — they're a few pixels on screen, so a solid sliver reads
/// cleanly.
pub fn build_pack_mesh() -> MeshCpu {
    // Per face: normal, 4 corners (CCW from outside), 4 UVs.
    let faces: &[([f32; 3], [[f32; 3]; 4], [[f32; 2]; 4])] = &[
        // -Y (front, toward camera) — full art, upright.
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, -0.5], // bot-left
                [0.5, -0.5, -0.5],  // bot-right
                [0.5, -0.5, 0.5],   // top-right
                [-0.5, -0.5, 0.5],  // top-left
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        ),
        // +Y (back) — mirrored front art.
        (
            [0.0, 1.0, 0.0],
            [
                [0.5, 0.5, -0.5],  // bot-left (as seen from behind)
                [-0.5, 0.5, -0.5], // bot-right
                [-0.5, 0.5, 0.5],  // top-right
                [0.5, 0.5, 0.5],   // top-left
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        ),
        // +X (right edge) — thin strip.
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [0.5, -0.5, 0.5],
            ],
            [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
        ),
        // -X (left edge) — thin strip.
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
        ),
        // +Z (top edge) — thin strip.
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
        ),
        // -Z (bottom edge) — thin strip.
        (
            [0.0, 0.0, -1.0],
            [
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
            ],
            [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
        ),
    ];

    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(24);
    let mut indices: Vec<u32> = Vec::with_capacity(36);

    for (normal, corners, uvs) in faces {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: *normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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

    push_box(&mut vertices, &mut indices, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5);

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
    vertices[8].uv  = [0.0, 0.0];
    vertices[9].uv  = [0.0, 1.0];
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
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        // Darker brass tint — matches the Midnight Gold palette and reads
        // as a recessed metal tray under candlelight.
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
    // Thinner rim (was 0.10) so the lip reads delicate, wider footprint at
    // the top and a noticeably narrower base for a tapered bowl silhouette.
    const RIM_THICK: f32 = 0.04;
    const RECESS_FLOOR: f32 = -0.10;
    const OUTER_R: f32 = 0.5;
    const BASE_R: f32 = 0.32;
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
        });
        for i in 0..SEGS {
            let a = angle(i);
            vertices.push(Vertex3dTex {
                position: [BASE_R * a.cos(), BOTTOM, BASE_R * a.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.0, 0.0],
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
            });
            vertices.push(Vertex3dTex {
                position: [bx1, BOTTOM, bz1],
                normal: n1,
                uv: [1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx1, RIM_TOP, tz1],
                normal: n1,
                uv: [1.0, 0.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx0, RIM_TOP, tz0],
                normal: n0,
                uv: [0.0, 0.0],
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
        });
        vertices.push(Vertex3dTex {
            position: [ox0, RIM_TOP, oz0],
            normal: n,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, RIM_TOP, oz1],
            normal: n,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, RIM_TOP, iz1],
            normal: n,
            uv: [0.0, 1.0],
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
            });
            vertices.push(Vertex3dTex {
                position: [bx0, RECESS_FLOOR, bz0],
                normal: n0,
                uv: [1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx0, RIM_TOP, tz0],
                normal: n0,
                uv: [1.0, 0.0],
            });
            vertices.push(Vertex3dTex {
                position: [tx1, RIM_TOP, tz1],
                normal: n1,
                uv: [0.0, 0.0],
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
        });
        for i in 0..SEGS {
            let a = angle(i);
            vertices.push(Vertex3dTex {
                position: [INNER_BASE_R * a.cos(), RECESS_FLOOR, INNER_BASE_R * a.sin()],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
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
            base_color: [0.32, 0.22, 0.10, 1.0],
            specular_strength: 0.55,
            specular_power: 48.0,
        },
    }
}

/// Folded "tent card" mesh: two flat quads meeting at a top crease along the
/// +Z axis, sitting on the XZ plane. Local extents -0.5..0.5 on X, 0..0.5 on
/// Y (bottom edges rest at y=0), -0.5..0.5 on Z. Each face has UV (0,0) at
/// the bottom-left so a portrait-oriented decal reads upright on each side.
///
/// Triangles are emitted for both faces of each side (4 triangles total) so
/// the card reads from any angle without backface-culling concerns.
pub fn build_tent_card_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Side A: from (-x, 0, +z) at the bottom-front to the crease at (0, +y, 0).
    // Slanted plane facing roughly -X / +Z.
    // Side B: mirror facing +X / -Z.
    let half = 0.5_f32;
    let top = 0.5_f32;

    // Compute side-A normal: plane spanned by (bottom edge along Z) and
    // (slope from bottom-front to crease). Bottom edge: (-half,0,-half) →
    // (-half,0,+half). Slope at the front: (-half,0,+half) → (0,+top,0).
    // Cross product gives outward normal.
    let na = {
        let e1 = [0.0, 0.0, 1.0];
        let e2 = [half, top, -half];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        [n[0] / len, n[1] / len, n[2] / len]
    };
    let nb = [-na[0], na[1], -na[2]];

    // Side A: outward face (front-left).
    {
        let base = vertices.len() as u32;
        let n = na;
        // Bottom-back, bottom-front, top-front (crease back), top-back (crease front)
        // Quad corners CCW viewed along -na:
        //   bot-left  = (-half, 0, -half)
        //   bot-right = (-half, 0,  half)
        //   top-right = (0, top,  half)
        //   top-left  = (0, top, -half)
        let corners = [
            ([-half, 0.0, -half], [0.0, 1.0]),
            ([-half, 0.0, half], [1.0, 1.0]),
            ([0.0, top, half], [1.0, 0.0]),
            ([0.0, top, -half], [0.0, 0.0]),
        ];
        for (p, uv) in corners.iter() {
            vertices.push(Vertex3dTex {
                position: *p,
                normal: n,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        // Backside of side A (so the quad reads from the inner fold too).
        let base2 = vertices.len() as u32;
        let n_back = [-n[0], -n[1], -n[2]];
        for (p, uv) in corners.iter() {
            vertices.push(Vertex3dTex {
                position: *p,
                normal: n_back,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[
            base2,
            base2 + 2,
            base2 + 1,
            base2,
            base2 + 3,
            base2 + 2,
        ]);
    }

    // Side B: mirror of side A (front-right).
    {
        let base = vertices.len() as u32;
        let n = nb;
        let corners = [
            ([half, 0.0, half], [0.0, 1.0]),
            ([half, 0.0, -half], [1.0, 1.0]),
            ([0.0, top, -half], [1.0, 0.0]),
            ([0.0, top, half], [0.0, 0.0]),
        ];
        for (p, uv) in corners.iter() {
            vertices.push(Vertex3dTex {
                position: *p,
                normal: n,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        let base2 = vertices.len() as u32;
        let n_back = [-n[0], -n[1], -n[2]];
        for (p, uv) in corners.iter() {
            vertices.push(Vertex3dTex {
                position: *p,
                normal: n_back,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[
            base2,
            base2 + 2,
            base2 + 1,
            base2,
            base2 + 3,
            base2 + 2,
        ]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.96, 0.93, 0.84, 1.0],
            specular_strength: 0.10,
            specular_power: 8.0,
        },
    }
}

/// Build a book mesh: a rectangular box with a rounded spine on the −X edge
/// and a narrow page-block inset visible on the other three open edges
/// (+X, +Z, −Z). The mesh spans −0.5..+0.5 on each axis so callers can
/// scale it into the desired proportions (width × height × depth).
///
/// The spine is approximated with 6 arc segments to give it a convincing
/// curved look under the lit-mesh shader. The page inset is a thinner
/// inner box offset from the cover by `PAGE_INSET` on three sides,
/// recessed by `COVER_THICK` in Y, giving the silhouette of paper pages
/// peeking out between the covers.
pub fn build_book_mesh() -> MeshCpu {
    // How far the spine bulges past the cover on the −X side.
    const SPINE_BULGE: f32 = 0.08;
    // Number of arc segments in the spine curve.
    const SPINE_SEGS: usize = 6;
    // Cover thickness (how far pages are recessed from the cover surface).
    const COVER_THICK: f32 = 0.06;
    // How far pages are inset from the cover edge on +X / +Z / −Z.
    const PAGE_INSET: f32 = 0.02;

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Helper: push a quad (4 verts, 6 indices) with a flat normal.
    let mut push_quad = |normal: [f32; 3], corners: [[f32; 3]; 4]| {
        let base = vertices.len() as u32;
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (c, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *c,
                normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // ── Cover box (outer shell) ───────────────────────────────────────
    // +X face (open edge of pages)
    push_quad(
        [1.0, 0.0, 0.0],
        [
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            [0.5, -0.5, 0.5],
        ],
    );
    // +Y face (front cover)
    push_quad(
        [0.0, 1.0, 0.0],
        [
            [-0.5, 0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 0.5, -0.5],
        ],
    );
    // −Y face (back cover)
    push_quad(
        [0.0, -1.0, 0.0],
        [
            [-0.5, -0.5, 0.5],
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
        ],
    );
    // +Z face (top edge)
    push_quad(
        [0.0, 0.0, 1.0],
        [
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ],
    );
    // −Z face (bottom edge)
    push_quad(
        [0.0, 0.0, -1.0],
        [
            [0.5, -0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
        ],
    );

    // ── Spine (rounded −X edge) ───────────────────────────────────────
    // Arc from (−0.5, −0.5) to (−0.5, +0.5) in the XY plane, bulging
    // out to x = −0.5 − SPINE_BULGE at the midpoint. Each segment is a
    // quad strip running along z (−0.5..+0.5).
    {
        let cx = -0.5_f32; // arc center x (flat cover edge)
        let cy = 0.0_f32; // arc center y
        let ry = 0.5_f32; // half-height of the cover
        let rx = SPINE_BULGE; // how far the spine pokes out
        for seg in 0..SPINE_SEGS {
            let t0 = seg as f32 / SPINE_SEGS as f32;
            let t1 = (seg + 1) as f32 / SPINE_SEGS as f32;
            // Angles from π/2 (top, +Y) to −π/2 (bottom, −Y), going
            // through π (leftward bulge).
            let a0 = std::f32::consts::FRAC_PI_2 - t0 * std::f32::consts::PI;
            let a1 = std::f32::consts::FRAC_PI_2 - t1 * std::f32::consts::PI;
            let x0 = cx - rx * a0.cos().abs();
            let y0 = cy + ry * a0.sin();
            let x1 = cx - rx * a1.cos().abs();
            let y1 = cy + ry * a1.sin();
            // Normal points outward from the arc center.
            let nx0 = -a0.cos();
            let ny0 = a0.sin();
            let nx1 = -a1.cos();
            let ny1 = a1.sin();
            let nmx = (nx0 + nx1) * 0.5;
            let nmy = (ny0 + ny1) * 0.5;
            let len = (nmx * nmx + nmy * nmy).sqrt().max(0.001);
            let normal = [nmx / len, nmy / len, 0.0];
            push_quad(
                normal,
                [[x0, y0, -0.5], [x1, y1, -0.5], [x1, y1, 0.5], [x0, y0, 0.5]],
            );
        }
    }

    // ── Page block (inner lighter box on the open edge) ───────────────
    // Visible on +X, +Z, −Z as a recessed cream-colored band. The page
    // block sits between the two covers (y = ±(0.5 − COVER_THICK)) and
    // is inset from the cover edges.
    let py = 0.5 - COVER_THICK; // page half-thickness in Y
    let pz = 0.5 - PAGE_INSET; // page half-depth in Z
    let px_inner = -0.5 + PAGE_INSET; // page start on −X side
    let px_outer = 0.5 - PAGE_INSET; // page end on +X side
    // +X face of pages (the fore-edge)
    push_quad(
        [1.0, 0.0, 0.0],
        [
            [px_outer, -py, -pz],
            [px_outer, py, -pz],
            [px_outer, py, pz],
            [px_outer, -py, pz],
        ],
    );
    // +Z face of pages (top edge pages)
    push_quad(
        [0.0, 0.0, 1.0],
        [
            [px_inner, -py, pz],
            [px_outer, -py, pz],
            [px_outer, py, pz],
            [px_inner, py, pz],
        ],
    );
    // −Z face of pages (bottom edge pages)
    push_quad(
        [0.0, 0.0, -1.0],
        [
            [px_outer, -py, -pz],
            [px_inner, -py, -pz],
            [px_inner, py, -pz],
            [px_outer, py, -pz],
        ],
    );
    // +Y face of pages (front page surface, recessed below front cover)
    push_quad(
        [0.0, 1.0, 0.0],
        [
            [px_inner, py, -pz],
            [px_inner, py, pz],
            [px_outer, py, pz],
            [px_outer, py, -pz],
        ],
    );
    // −Y face of pages (back page surface, recessed below back cover)
    push_quad(
        [0.0, -1.0, 0.0],
        [
            [px_inner, -py, pz],
            [px_inner, -py, -pz],
            [px_outer, -py, -pz],
            [px_outer, -py, pz],
        ],
    );

    MeshCpu {
        vertices,
        indices,
        // Deep oxblood / maroon cover — reads as aged leather under warm
        // candlelight. The page block will be tinted at render time via
        // a per-instance material override (cream/ivory).
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.30, 0.12, 0.08, 1.0],
            specular_strength: 0.20,
            specular_power: 16.0,
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
            let a = std::f32::consts::TAU * i as f32 / SIDES as f32
                + std::f32::consts::FRAC_PI_8;
            [a.cos() * R, 0.0, a.sin() * R]
        })
        .collect();

    // Front cap (+Y)
    let front_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, HALF_Y, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: UV_CENTER,
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p[0], HALF_Y, p[2]],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5 + p[0], 0.5 + p[2]],
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
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p[0], -HALF_Y, p[2]],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5 + p[0], 0.5 + p[2]],
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

/// Build a relic mesh whose cap silhouette is extracted from the supplied
/// RGBA. Normally we use the **alpha** channel (threshold 24). Many offline
/// `*_mask.png` files keep **alpha fully opaque** and encode the silhouette only
/// in RGB (white on black); when alpha has almost no variation near 255, we
/// treat pixels as solid using **rec.709 luminance** ≥ 0.45 instead.
///
/// This is intentionally lightweight: radial samples into a convex-ish
/// polygon — enough for distinct relic silhouettes without full contour tracing.
pub fn build_relic_mesh_from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<MeshCpu> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }

    let px = (width as usize) * (height as usize);
    let mut a_min = 255u8;
    let mut a_max = 0u8;
    for i in 0..px {
        let a = rgba[i * 4 + 3];
        a_min = a_min.min(a);
        a_max = a_max.max(a);
    }
    // Opaque-alpha mask images (no transparency in A): carve from brightness.
    let use_luma = a_max.saturating_sub(a_min) <= 4 && a_min >= 240;

    let solid_at = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return false;
        }
        let idx = ((y as u32 * width + x as u32) * 4) as usize;
        if use_luma {
            let r = rgba[idx] as f32;
            let g = rgba[idx + 1] as f32;
            let b = rgba[idx + 2] as f32;
            let l = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
            l >= 0.45
        } else {
            rgba[idx + 3] >= 24
        }
    };

    let mut min_x = width as i32;
    let mut min_y = height as i32;
    let mut max_x = -1_i32;
    let mut max_y = -1_i32;
    let mut sum_x = 0.0_f32;
    let mut sum_y = 0.0_f32;
    let mut count = 0.0_f32;

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            if solid_at(x, y) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                sum_x += x as f32 + 0.5;
                sum_y += y as f32 + 0.5;
                count += 1.0;
            }
        }
    }

    if count < 16.0 || max_x < min_x || max_y < min_y {
        return None;
    }

    let cx = sum_x / count;
    let cy = sum_y / count;
    let bbox_w = (max_x - min_x + 1).max(1) as f32;
    let bbox_h = (max_y - min_y + 1).max(1) as f32;
    let max_dim = bbox_w.max(bbox_h).max(1.0);

    #[derive(Clone, Copy)]
    struct RingPoint {
        pos: [f32; 2],
        uv: [f32; 2],
    }

    let mut points: Vec<RingPoint> = Vec::new();
    let max_r = ((width * width + height * height) as f32).sqrt();
    const SAMPLE_ANGLES: usize = 48;

    for i in 0..SAMPLE_ANGLES {
        let a = std::f32::consts::TAU * i as f32 / SAMPLE_ANGLES as f32;
        let dir = glam::Vec2::new(a.cos(), a.sin());
        let mut hit: Option<glam::Vec2> = None;
        let mut r = 0.0_f32;
        while r <= max_r {
            let sx = (cx + dir.x * r).round() as i32;
            let sy = (cy + dir.y * r).round() as i32;
            if solid_at(sx, sy) {
                hit = Some(glam::Vec2::new(sx as f32 + 0.5, sy as f32 + 0.5));
            } else if hit.is_some() && r > 2.0 {
                break;
            }
            r += 1.0;
        }
        if let Some(p) = hit {
            let nx = ((p.x - cx) / max_dim) * 1.8;
            let nz = ((p.y - cy) / max_dim) * 1.8;
            let px = nx.clamp(-0.5, 0.5);
            let pz = nz.clamp(-0.5, 0.5);
            // Derive UV from the normalised 3-D position so that linear
            // interpolation across fan triangles stays consistent with the
            // geometry (no pincushion / pinwheel distortion).
            points.push(RingPoint {
                pos: [px, pz],
                uv: [px + 0.5, pz + 0.5],
            });
        }
    }

    if points.len() < 6 {
        return None;
    }

    // Merge near-duplicate points produced by adjacent radial samples.
    let mut ring: Vec<RingPoint> = Vec::new();
    for p in points {
        let keep = ring.last().map(|q| {
            let dx = q.pos[0] - p.pos[0];
            let dy = q.pos[1] - p.pos[1];
            dx * dx + dy * dy > 0.0004
        }).unwrap_or(true);
        if keep {
            ring.push(p);
        }
    }
    if ring.len() >= 2 {
        let first = ring[0].pos;
        let last = ring[ring.len() - 1].pos;
        let dx = first[0] - last[0];
        let dy = first[1] - last[1];
        if dx * dx + dy * dy <= 0.0004 {
            ring.pop();
        }
    }
    if ring.len() < 6 {
        return None;
    }

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
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    const HALF_Y: f32 = 0.5;
    let front_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, HALF_Y, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 0.5],
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p.pos[0], HALF_Y, p.pos[1]],
            normal: [0.0, 1.0, 0.0],
            uv: p.uv,
        });
    }
    for i in 0..ring.len() {
        let a = front_center + 1 + i as u32;
        let b = front_center + 1 + ((i + 1) % ring.len()) as u32;
        indices.extend_from_slice(&[front_center, a, b]);
    }

    let back_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, -HALF_Y, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
    });
    for p in &ring {
        vertices.push(Vertex3dTex {
            position: [p.pos[0], -HALF_Y, p.pos[1]],
            normal: [0.0, -1.0, 0.0],
            uv: p.uv,
        });
    }
    for i in 0..ring.len() {
        let a = back_center + 1 + i as u32;
        let b = back_center + 1 + ((i + 1) % ring.len()) as u32;
        indices.extend_from_slice(&[back_center, b, a]);
    }

    for i in 0..ring.len() {
        let p0 = ring[i];
        let p1 = ring[(i + 1) % ring.len()];
        let edge = glam::Vec3::new(p1.pos[0] - p0.pos[0], 0.0, p1.pos[1] - p0.pos[1]);
        let normal = glam::Vec3::new(edge.z, 0.0, -edge.x).normalize_or_zero();
        let side_uv = [
            ((p0.uv[0] + p1.uv[0]) * 0.5).clamp(0.0, 1.0),
            ((p0.uv[1] + p1.uv[1]) * 0.5).clamp(0.0, 1.0),
        ];
        push_quad(
            &mut vertices,
            &mut indices,
            [normal.x, normal.y, normal.z],
            [
                [p0.pos[0], -HALF_Y, p0.pos[1]],
                [p1.pos[0], -HALF_Y, p1.pos[1]],
                [p1.pos[0], HALF_Y, p1.pos[1]],
                [p0.pos[0], HALF_Y, p0.pos[1]],
            ],
            [side_uv, side_uv, side_uv, side_uv],
        );
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
