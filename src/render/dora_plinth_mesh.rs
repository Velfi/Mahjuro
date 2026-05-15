//! Procedural mesh for the gameplay scene's ornate brass dora indicator
//! plinth.
//!
//! Roofless — the offering platform on top is meant to display 1–2 dora
//! indicator tiles (drawn separately via `ShowcaseTilePlacement`), so the
//! platform must be visible from the front and from above. The silhouette
//! is a tiered brass pedestal: stepped base, slim central column, broad
//! offering platform with an inset rim.
//!
//! Built in normalized local space spanning -0.5..+0.5 on each axis so
//! per-instance scale can size it. Y is "up" in mesh-local space; the
//! renderer rotates this into world Z-up via
//! `mesh_y_thickness_along_local_y_to_z_up()`.
//!
//! ```text
//!   +Y up
//!     ┌────────┐                  ← offering platform (wide, low)
//!     ├────────┤                  ← inset rim collar
//!       │ ░░ │                    ← central pillar (slim column)
//!       │ ░░ │
//!       │ ░░ │
//!     ┌─┴────┴─┐                  ← upper plinth step
//!   ┌─┴────────┴─┐                ← lower plinth slab (widest)
//! ```

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::theme::color;
use crate::render::tile_glb::Vertex3dTex;

/// Build the dora plinth mesh in local space (-0.5..+0.5 on each axis).
pub fn build_dora_plinth_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // ── 1. Lower plinth slab — widest box at the bottom, low profile.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.50, 0.50, -0.50, -0.40, -0.40, 0.40),
    );
    // Upper plinth step — slightly inset, taller, gives a tiered silhouette
    // so the base reads as masonry rather than a single slab.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.42, 0.42, -0.40, -0.30, -0.34, 0.34),
    );

    // ── 2. Central pillar — slim column rising to the platform height.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.14, 0.14, -0.30, 0.18, -0.14, 0.14),
    );

    // ── 3. Inset rim collar — slightly wider than the pillar, just below
    // the platform. Reads as a decorative capital where the column meets
    // the platform.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.20, 0.20, 0.18, 0.24, -0.20, 0.20),
    );

    // ── 4. Offering platform — wide thin slab on top. Sized to comfortably
    // hold a single 30mm tile (or two side-by-side at narrower spacing) at
    // the renderer's chosen tile width when `extents.x` matches a
    // tile-friendly screen width.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, 0.24, 0.32, -0.30, 0.30),
    );
    // Platform lip — a thin raised rim around the perimeter so the tile
    // visually sits *in* the platform, not just on top of it. Built as
    // four thin walls (front / back / left / right) leaving the center
    // open for the indicator tile face.
    let lip_y0 = 0.32;
    let lip_y1 = 0.36;
    // Front lip (-Z side)
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, lip_y0, lip_y1, -0.30, -0.26),
    );
    // Back lip (+Z side)
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, lip_y0, lip_y1, 0.26, 0.30),
    );
    // Left lip (-X side)
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, -0.42, lip_y0, lip_y1, -0.26, 0.26),
    );
    // Right lip (+X side)
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(0.42, 0.46, lip_y0, lip_y1, -0.26, 0.26),
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            // Warm brass — the per-instance color overrides this and tints
            // the metallic specular lobe. Default here is the canonical
            // `BRASS` token, slightly lifted toward `GOLD` by the metal
            // shader's Fresnel highlights.
            base_color: color::GOLD,
            specular_strength: 0.85,
            specular_power: 64.0,
        },
    }
}
