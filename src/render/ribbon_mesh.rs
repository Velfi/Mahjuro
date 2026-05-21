//! Procedural mesh for a hanging silk ribbon, used by the shop scene.
//!
//! The mesh is a thin vertical strip subdivided into ~12 segments along its
//! length so the lit-mesh shader's per-fragment lighting reads as a smooth
//! gradient down the ribbon. Local extents:
//!
//! - x ∈ [-0.5, 0.5] — width
//! - y ∈ [-0.5, 0.5] — length; origin at centroid (finial toward +Y)
//! - z ≈ ±0.05      — slight thickness so a back-face exists
//!
//! UVs run 0→1 along y for future texturing/animation.

use crate::render::draw_cmd::{Object3d, Object3dEuler, Object3dKind};
use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const SEGMENTS: usize = 12;
const HALF_THICKNESS: f32 = 0.05;

/// Length-to-width aspect of a zodiac ribbon, matching the source textures
/// at `assets/textures/zodiacs/zodiac_*.png` (1024 × 3072 px → 1:3). Every
/// ribbon `Object3d` in the game derives its width from its length via this
/// constant so the texture renders without stretching.
pub const RIBBON_LENGTH_OVER_WIDTH: f32 = 3.0;

/// Ribbon depth (front-to-back thickness) as a fraction of width — the silk
/// reads as a thin slab with a sliver of side strip visible at glancing
/// angles. Matches the `±HALF_THICKNESS` extent of [`build_ribbon_mesh`]
/// after the standard `extents = (width, length, depth)` scale, so the
/// modeled side strip stays visually consistent with the painted silk.
pub const RIBBON_DEPTH_OVER_WIDTH: f32 = 0.15;

/// Global size multiplier for every zodiac ribbon placement (shop, dish HUD,
/// collection, showcase). Applied in [`zodiac_ribbon_object3d`]; use
/// [`ribbon_display_length`] when positioning from a logical length.
pub const RIBBON_WORLD_SCALE: f32 = 2.0;

/// Logical ribbon length → world length after [`RIBBON_WORLD_SCALE`].
#[inline]
pub fn ribbon_display_length(length: f32) -> f32 {
    length * RIBBON_WORLD_SCALE
}

/// Inputs for the canonical zodiac-ribbon [`Object3d`] constructor.
///
/// Width and depth are derived from `length` so every ribbon placement uses
/// the same aspect (and the same texture mapping) regardless of the scene
/// it's drawn in. Use [`zodiac_ribbon_object3d`] to build the placement.
#[derive(Clone, Debug)]
pub struct ZodiacRibbonSpec {
    /// Centroid position passed straight through to `Object3d::pos`.
    pub pos: [f32; 3],
    /// Drives the rendered ribbon size: width = `length /
    /// RIBBON_LENGTH_OVER_WIDTH`, depth = `width * RIBBON_DEPTH_OVER_WIDTH`.
    pub length: f32,
    pub rotation: Object3dEuler,
    pub color: [f32; 4],
    pub kind: Option<crate::core::zodiac::ZodiacKind>,
    pub hover_target: f32,
    pub anim_id: u64,
    /// Extra placement rotation (degrees) composed onto `rotation` at build time.
    pub placement_rot_deg: [f32; 3],
}

/// Build the standard zodiac-ribbon [`Object3d`] from `spec`. Width and
/// depth are derived from `spec.length` via [`RIBBON_LENGTH_OVER_WIDTH`] and
/// [`RIBBON_DEPTH_OVER_WIDTH`]; this is the only place ribbon proportions
/// should be set so all scenes stay matched to the source texture aspect.
pub fn zodiac_ribbon_object3d(spec: ZodiacRibbonSpec) -> Object3d {
    let length = ribbon_display_length(spec.length);
    let width = length / RIBBON_LENGTH_OVER_WIDTH;
    let depth = width * RIBBON_DEPTH_OVER_WIDTH;
    Object3d {
        pos: spec.pos,
        extents: [width, length, depth],
        rotation: crate::render::table_transform::compose_rotation_euler(
            crate::render::table_transform::rot_euler_xyz_rad(
                spec.rotation[0],
                spec.rotation[1],
                spec.rotation[2],
            ),
            spec.placement_rot_deg,
        ),
        color: spec.color,
        kind: Object3dKind::ZodiacRibbon { kind: spec.kind },
        hover_target: spec.hover_target,
        anim_id: spec.anim_id,
    }
}

/// Largest ribbon length whose 3:1 footprint (width = length / 3) fits
/// inside the given screen-rect envelope `(rect_w, rect_h)`. Used by the
/// shop inspect path so ribbons stay 3:1 even when the slot rect's aspect
/// would otherwise stretch them.
#[inline]
pub fn ribbon_length_fitting_rect(rect_w: f32, rect_h: f32) -> f32 {
    rect_h.min(rect_w * RIBBON_LENGTH_OVER_WIDTH).max(0.0)
}

/// Build a hanging-ribbon mesh. Front face (toward +Z) has many segments
/// for smooth lighting; back face is a single quad. Two side strips close
/// the seam so the ribbon has a tiny visible edge.
pub fn build_ribbon_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Helper: push a quad as two triangles given four corner vertices
    // already appended to `vertices`, returning the next base index.
    let push_quad = |indices: &mut Vec<u32>, base: u32| {
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // ── Front face (+Z normal), subdivided into SEGMENTS quads top→bottom.
    let front_normal = [0.0, 0.0, 1.0];
    for s in 0..SEGMENTS {
        let v0 = s as f32 / SEGMENTS as f32;
        let v1 = (s + 1) as f32 / SEGMENTS as f32;
        let y0 = 0.5 - v0; // y top of this segment
        let y1 = 0.5 - v1; // y bottom of this segment
        let base = vertices.len() as u32;
        // Order: top-left, top-right, bottom-right, bottom-left.
        vertices.push(Vertex3dTex {
            position: [-0.5, y0, HALF_THICKNESS],
            normal: front_normal,
            uv: [0.0, v0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [0.5, y0, HALF_THICKNESS],
            normal: front_normal,
            uv: [1.0, v0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [0.5, y1, HALF_THICKNESS],
            normal: front_normal,
            uv: [1.0, v1],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [-0.5, y1, HALF_THICKNESS],
            normal: front_normal,
            uv: [0.0, v1],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        push_quad(&mut indices, base);
    }

    // ── Back face (-Z normal), single quad.
    let back_normal = [0.0, 0.0, -1.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.5, 0.5, -HALF_THICKNESS],
        normal: back_normal,
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.5, -HALF_THICKNESS],
        normal: back_normal,
        uv: [1.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -0.5, -HALF_THICKNESS],
        normal: back_normal,
        uv: [1.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -0.5, -HALF_THICKNESS],
        normal: back_normal,
        uv: [0.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    push_quad(&mut indices, base);

    // ── Left edge strip (-X normal).
    let left_normal = [-1.0, 0.0, 0.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.5, -HALF_THICKNESS],
        normal: left_normal,
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.5, HALF_THICKNESS],
        normal: left_normal,
        uv: [1.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -0.5, HALF_THICKNESS],
        normal: left_normal,
        uv: [1.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -0.5, -HALF_THICKNESS],
        normal: left_normal,
        uv: [0.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    push_quad(&mut indices, base);

    // ── Right edge strip (+X normal).
    let right_normal = [1.0, 0.0, 0.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.5, 0.5, HALF_THICKNESS],
        normal: right_normal,
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, 0.5, -HALF_THICKNESS],
        normal: right_normal,
        uv: [1.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -0.5, -HALF_THICKNESS],
        normal: right_normal,
        uv: [1.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -0.5, HALF_THICKNESS],
        normal: right_normal,
        uv: [0.0, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    push_quad(&mut indices, base);

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Default base color is overridden per-instance from the
            // ZodiacRibbonPlacement; this fallback is a soft cream.
            base_color: [0.92, 0.86, 0.72, 1.0],
            specular_strength: 0.25,
            specular_power: 16.0,
        },
    }
}
