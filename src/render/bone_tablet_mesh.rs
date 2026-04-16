//! Procedural mesh for a small carved bone tablet — used by the gameplay
//! scene's yaku selectors and (later) cascade score tokens.
//!
//! The tablet is shaped like a flattened mahjong tile: a rectangular block
//! with a slight top bevel that catches candlelight. Local space spans
//! `-0.5..+0.5` on each axis; per-instance scale supplies the actual size.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_quad};
use crate::render::tile_glb::Vertex3dTex;

/// Bevel inset as a fraction of the full extent. Chamfers the top and
/// bottom perimeter so the tablet reads as a rounded rectangle from the
/// front without costing the decal any usable face area.
const BEVEL: f32 = 0.06;

/// Build the bone tablet mesh: a rectangular block with chamfered top
/// and bottom edges for a porcelain rounded-rectangle silhouette.
///
/// The engraved-label decal lives on the (slightly inset) +Y top face.
/// UVs follow `+u = +X`, `+v = +Z` so multi-letter yaku names read
/// upright along the tablet's long screen-X axis.
pub fn build_bone_tablet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Full extents and inset extents for the beveled top/bottom rings.
    let x0 = -0.5;
    let x1 = 0.5;
    let y0 = -0.5;
    let y1 = 0.5;
    let z0 = -0.5;
    let z1 = 0.5;
    let ix0 = x0 + BEVEL;
    let ix1 = x1 - BEVEL;
    let iz0 = z0 + BEVEL;
    let iz1 = z1 - BEVEL;
    let iy_top = y1 - BEVEL * 0.5;
    let iy_bot = y0 + BEVEL * 0.5;

    // ── Side faces (shrunk vertically to leave room for the bevels) ──
    //
    // Each side now spans y ∈ [iy_bot, iy_top] instead of [y0, y1] so it
    // meets the chamfered ring cleanly.

    // +X side
    push_quad(
        &mut vertices,
        &mut indices,
        [x1, iy_bot, z0],
        [x1, iy_top, z0],
        [x1, iy_top, z1],
        [x1, iy_bot, z1],
        [1.0, 0.0, 0.0],
    );
    // -X side
    push_quad(
        &mut vertices,
        &mut indices,
        [x0, iy_bot, z1],
        [x0, iy_top, z1],
        [x0, iy_top, z0],
        [x0, iy_bot, z0],
        [-1.0, 0.0, 0.0],
    );
    // +Z side
    push_quad(
        &mut vertices,
        &mut indices,
        [x0, iy_bot, z1],
        [x1, iy_bot, z1],
        [x1, iy_top, z1],
        [x0, iy_top, z1],
        [0.0, 0.0, 1.0],
    );
    // -Z side
    push_quad(
        &mut vertices,
        &mut indices,
        [x1, iy_bot, z0],
        [x0, iy_bot, z0],
        [x0, iy_top, z0],
        [x1, iy_top, z0],
        [0.0, 0.0, -1.0],
    );

    // ── Top cap (+Y), inset by BEVEL on all four sides ──
    //
    // UVs are mapped so +u = +X and +v = +Z, corners wound to match
    // push_box's CCW order when viewed from +Y. The decal rasterisation
    // lands on this (now slightly smaller) face.
    let base_top = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [ix0, y1, iz0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [ix0, y1, iz1],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [ix1, y1, iz1],
        normal: [0.0, 1.0, 0.0],
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [ix1, y1, iz0],
        normal: [0.0, 1.0, 0.0],
        uv: [1.0, 0.0],
    });
    indices.extend_from_slice(&[
        base_top,
        base_top + 1,
        base_top + 2,
        base_top,
        base_top + 2,
        base_top + 3,
    ]);

    // Top bevels — four angled quads connecting each side's top edge to
    // the inset top face. Each bevel normal averages the side normal and
    // +Y so it catches a soft rim highlight.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    // +X top bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [x1, iy_top, z0],
        [ix1, y1, iz0],
        [ix1, y1, iz1],
        [x1, iy_top, z1],
        [s, s, 0.0],
    );
    // -X top bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [x0, iy_top, z1],
        [ix0, y1, iz1],
        [ix0, y1, iz0],
        [x0, iy_top, z0],
        [-s, s, 0.0],
    );
    // +Z top bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [x0, iy_top, z1],
        [x1, iy_top, z1],
        [ix1, y1, iz1],
        [ix0, y1, iz1],
        [0.0, s, s],
    );
    // -Z top bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [x1, iy_top, z0],
        [x0, iy_top, z0],
        [ix0, y1, iz0],
        [ix1, y1, iz0],
        [0.0, s, -s],
    );

    // ── Bottom cap (-Y), inset. UVs zeroed — bottom never shows a decal.
    push_quad(
        &mut vertices,
        &mut indices,
        [ix0, y0, iz1],
        [ix0, y0, iz0],
        [ix1, y0, iz0],
        [ix1, y0, iz1],
        [0.0, -1.0, 0.0],
    );

    // Bottom bevels.
    // +X bottom bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [ix1, y0, iz0],
        [x1, iy_bot, z0],
        [x1, iy_bot, z1],
        [ix1, y0, iz1],
        [s, -s, 0.0],
    );
    // -X bottom bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [ix0, y0, iz1],
        [x0, iy_bot, z1],
        [x0, iy_bot, z0],
        [ix0, y0, iz0],
        [-s, -s, 0.0],
    );
    // +Z bottom bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [ix0, y0, iz1],
        [ix1, y0, iz1],
        [x1, iy_bot, z1],
        [x0, iy_bot, z1],
        [0.0, -s, s],
    );
    // -Z bottom bevel
    push_quad(
        &mut vertices,
        &mut indices,
        [ix1, y0, iz0],
        [ix0, y0, iz0],
        [x0, iy_bot, z0],
        [x1, iy_bot, z0],
        [0.0, -s, -s],
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Porcelain: bright cool-white base with a sharp specular
            // highlight. Per-instance color overrides for active state.
            base_color: [0.97, 0.96, 0.94, 1.0],
            specular_strength: 0.65,
            specular_power: 96.0,
        },
    }
}
