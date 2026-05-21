//! Procedural mesh for the gameplay scene's ornate brass plinth.
//!
//! Roofless — the offering platform on top is meant to display upright tile
//! faces / icons (drawn separately), so the platform must be visible from the
//! front and from above. The silhouette is a tiered brass pedestal: stepped
//! base, slim central column, broad offering platform with an inset rim.
//!
//! Built in normalized local space spanning -0.5..+0.5 on each axis so
//! per-instance scale can size it. Y is "up" in mesh-local space; the
//! renderer rotates this into world Z-up via
//! `mesh_y_thickness_along_local_y_to_z_up()`.

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::theme::color;
use crate::render::tile_glb::Vertex3dTex;

/// Build the gameplay plinth mesh in local space (-0.5..+0.5 on each axis).
pub fn build_plinth_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.50, 0.50, -0.50, -0.40, -0.40, 0.40),
    );
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.42, 0.42, -0.40, -0.30, -0.34, 0.34),
    );

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.14, 0.14, -0.30, 0.18, -0.14, 0.14),
    );

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.20, 0.20, 0.18, 0.24, -0.20, 0.20),
    );

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, 0.24, 0.32, -0.30, 0.30),
    );

    let lip_y0 = 0.32;
    let lip_y1 = 0.36;
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, lip_y0, lip_y1, -0.30, -0.26),
    );
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, 0.46, lip_y0, lip_y1, 0.26, 0.30),
    );
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.46, -0.42, lip_y0, lip_y1, -0.26, 0.26),
    );
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
            base_color: color::GOLD,
            specular_strength: 0.85,
            specular_power: 64.0,
        },
    }
}
