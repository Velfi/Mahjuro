//! Procedural meshes for the shop counter's journal — leather-bound book.
//!
//! The journal renders as **two meshes** that share a base model matrix:
//!
//!   * [`build_book_body_mesh`] — back cover, page block, page-content
//!     surface, spine band with cord ridges. The static "rest of the
//!     book." Drawn at the book's base transform.
//!
//!   * [`build_book_cover_mesh`] — front cover (carrying the gilded
//!     "Journal" decal) plus the silk ribbon bookmark glued to it.
//!     Drawn with an extra **hinge rotation** around the spine axis
//!     (local X = -0.5) so the cover swings open as `open_amount` goes
//!     0 → 1. At 0 it sits flush over the page-content surface; at 1
//!     it has rotated ~170° to lay flat on the camera-right, exposing
//!     the page surface to the camera.
//!
//! Splitting cover from body lets the renderer animate the cover as a
//! rigid body via `model = base * hinge_rot(open_amount)` without any
//! vertex-shader changes.
//!
//! Coordinate convention after the shop's `cam_rot`:
//!   * local +Y → camera-facing
//!   * local -Y → away from camera
//!   * local +X → camera-right (fore-edge)
//!   * local -X → camera-left  (spine — hinge axis)
//!   * local -Z → screen-up
//!   * local +Z → screen-down
//!
//! UV.x sentinels read by the leather shader's `is_leather` branch:
//!   * `< 1.5`             : leather body (default)
//!   * `≈ PAGE_UV_X`       : page-stack edge (cream stratified paper)
//!   * `≈ RIBBON_UV_X`     : silk ribbon bookmark
//!   * `≥ PAGE_CONTENT_UV_X`: page-content surface, sample journal
//!                            target at (uv.x − 4.0, uv.y).

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

const COVER_THICK: f32 = 0.06;
const SPINE_PROUD: f32 = 0.05;
const PAGE_INSET: f32 = 0.025;
const PAGE_UV_X: f32 = 2.0;
const RIBBON_UV_X: f32 = 3.0;
const PAGE_CONTENT_UV_X: f32 = 4.0;
const BM_W: f32 = 0.10;
const BM_THICK: f32 = 0.014;
const BM_DROP: f32 = 0.42;
const BM_X_OFFSET: f32 = 0.18;

/// Y-bounds of the front cover in unit-cube space. Public so the
/// renderer can position the cover's hinge rotation matrix correctly.
pub const FRONT_COVER_Y_LO: f32 = 0.5 - COVER_THICK;
pub const FRONT_COVER_Y_HI: f32 = 0.5;
/// X-position of the spine — the cover's hinge axis.
pub const SPINE_X: f32 = -0.5;

/// Build the static body mesh (everything except the front cover + ribbon).
pub fn build_book_body_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let y_front_lo = FRONT_COVER_Y_LO;
    let y_front_hi = FRONT_COVER_Y_HI;
    let y_back_lo = -0.5;
    let y_back_hi = -0.5 + COVER_THICK;
    let y_page_lo = y_back_hi;
    let y_page_hi = y_front_lo;

    // ── Back cover ──────────────────────────────────────────────────────
    let back_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, y_back_lo, y_back_hi, -0.5, 0.5),
    );
    for v in vertices.iter_mut().skip(back_base) {
        v.uv = [0.0, 0.0];
    }

    // ── Page-content surface ────────────────────────────────────────────
    // Flat +Y-facing quad sitting in front of the cover's outer face
    // by a comfortable margin — that way the page never gets occluded
    // by the cover's slab geometry as the cover swings open. The
    // shader discards page fragments while `open_amount < 0.5` (carried
    // in `mesh.base_color.a`) so the front cover still gets to be the
    // visible surface in the closed state.
    let page_surface_y = y_front_hi + 0.04;
    let psx0 = -0.5 + 0.02;
    let psx1 = 0.5 - 0.02;
    let psz0 = -0.5 + 0.02;
    let psz1 = 0.5 - 0.02;
    let page_surface_base = vertices.len() as u32;
    let page_surface_corners = [
        ([psx0, page_surface_y, psz0], [0.0, 0.0]),
        ([psx0, page_surface_y, psz1], [0.0, 1.0]),
        ([psx1, page_surface_y, psz1], [1.0, 1.0]),
        ([psx1, page_surface_y, psz0], [1.0, 0.0]),
    ];
    for (pos, uv) in &page_surface_corners {
        vertices.push(Vertex3dTex {
            position: *pos,
            normal: [0.0, 1.0, 0.0],
            uv: [PAGE_CONTENT_UV_X + uv[0], uv[1]],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    indices.extend_from_slice(&[
        page_surface_base,
        page_surface_base + 1,
        page_surface_base + 2,
        page_surface_base,
        page_surface_base + 2,
        page_surface_base + 3,
    ]);

    // ── Page block (bound stack of pages) ───────────────────────────────
    let pbx0 = -0.5;
    let pbx1 = 0.5 - PAGE_INSET;
    let pbz0 = -0.5 + PAGE_INSET;
    let pbz1 = 0.5 - PAGE_INSET;
    let page_block_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(pbx0, pbx1, y_page_lo, y_page_hi, pbz0, pbz1),
    );
    for v in vertices.iter_mut().skip(page_block_base) {
        v.uv = [0.0, 0.0];
    }
    for i in 0..4 {
        vertices[page_block_base + i].uv = [PAGE_UV_X, i as f32 * 0.25];
    }
    for i in 0..4 {
        vertices[page_block_base + 20 + i].uv = [PAGE_UV_X, i as f32 * 0.25];
    }

    // ── Spine band ──────────────────────────────────────────────────────
    let spine_x0 = -0.5 - SPINE_PROUD;
    let spine_x1 = -0.5 + 0.04;
    let spine_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(spine_x0, spine_x1, y_back_lo, FRONT_COVER_Y_HI, -0.5, 0.5),
    );
    for v in vertices.iter_mut().skip(spine_base) {
        v.uv = [0.0, 0.0];
    }

    // ── Spine cord-hub ridges ──────────────────────────────────────────
    for hub_z in [-0.18_f32, 0.18_f32] {
        let hub_base = vertices.len();
        push_box(
            &mut vertices,
            &mut indices,
            Aabb::new(
                spine_x0 - 0.012,
                spine_x1,
                y_back_lo,
                FRONT_COVER_Y_HI,
                hub_z - 0.025,
                hub_z + 0.025,
            ),
        );
        for v in vertices.iter_mut().skip(hub_base) {
            v.uv = [0.0, 0.0];
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: book_material(),
    }
}

/// Build the front-cover mesh (drawn separately so the renderer can
/// hinge-rotate it). Includes the silk ribbon glued to its +Y face.
pub fn build_book_cover_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let y_front_lo = FRONT_COVER_Y_LO;
    let y_front_hi = FRONT_COVER_Y_HI;

    // ── Front cover ─────────────────────────────────────────────────────
    let front_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, y_front_lo, y_front_hi, -0.5, 0.5),
    );
    // Standard wood-tablet decal layout on +Y; zero-uv on other faces.
    for i in
        (front_base..front_base + 24).filter(|i| !(front_base + 8..front_base + 12).contains(i))
    {
        vertices[i].uv = [0.0, 0.0];
    }
    vertices[front_base + 8].uv = [0.0, 0.0];
    vertices[front_base + 9].uv = [0.0, 1.0];
    vertices[front_base + 10].uv = [1.0, 1.0];
    vertices[front_base + 11].uv = [1.0, 0.0];

    // ── Bookmark — silk ribbon glued to the +Y cover face ──────────────
    let bm_x0 = -0.5 + BM_X_OFFSET;
    let bm_x1 = bm_x0 + BM_W;

    // Loop: thin band poking up between page block top and cover.
    let loop_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            bm_x0,
            bm_x1,
            y_front_hi - 0.005,
            y_front_hi + BM_THICK,
            -0.5 - 0.02,
            -0.5 + 0.04,
        ),
    );
    for v in vertices.iter_mut().skip(loop_base) {
        v.uv = [RIBBON_UV_X, 0.0];
    }

    // Drape: long strip down the cover face.
    let drape_z0 = -0.5 + 0.04;
    let drape_z1 = drape_z0 + BM_DROP;
    let drape_y0 = y_front_hi - 0.002;
    let drape_y1 = y_front_hi + BM_THICK;
    let drape_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(bm_x0, bm_x1, drape_y0, drape_y1, drape_z0, drape_z1),
    );
    for v in vertices.iter_mut().skip(drape_base) {
        v.uv = [RIBBON_UV_X, 0.0];
    }

    // Tassel.
    let tassel_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            bm_x0 - 0.012,
            bm_x1 + 0.012,
            drape_y0 - 0.006,
            drape_y1 + 0.008,
            drape_z1,
            drape_z1 + 0.05,
        ),
    );
    for v in vertices.iter_mut().skip(tassel_base) {
        v.uv = [RIBBON_UV_X, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: book_material(),
    }
}

fn book_material() -> MaterialParams {
    MaterialParams {
        kind: MaterialKind::Leather,
        base_color: [0.56, 0.20, 0.12, 1.0],
        specular_strength: 0.28,
        specular_power: 22.0,
    }
}
