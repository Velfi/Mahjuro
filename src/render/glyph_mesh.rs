//! 3D extruded font glyphs for floating score popups.
//!
//! Uses `ttf-parser` to extract Cormorant Garamond outlines and `earcutr`
//! to triangulate the front/back caps. Side walls are built by extruding
//! each edge of the flattened contour.
//!
//! Supported characters: any glyph present in the font. Missing glyphs
//! are silently skipped.
//!
//! Each label mesh is recentred so the rendered string sits around the
//! world origin with a height of ~1.0 unit.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Half-extent of a glyph along the extrusion (depth) axis.
const DEPTH: f32 = 0.10;

/// Number of line segments per quarter-turn when flattening bezier curves.
const CURVE_SUBDIVISIONS: usize = 4;

// ---------------------------------------------------------------------------
// Outline collection via ttf-parser
// ---------------------------------------------------------------------------

struct OutlineCollector {
    contours: Vec<Vec<(f32, f32)>>,
    current: Vec<(f32, f32)>,
}

impl OutlineCollector {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            current: Vec::new(),
        }
    }
}

impl ttf_parser::OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.current.len() > 2 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
        self.current.push((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push((x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (px, py) = *self.current.last().unwrap_or(&(0.0, 0.0));
        for i in 1..=CURVE_SUBDIVISIONS {
            let t = i as f32 / CURVE_SUBDIVISIONS as f32;
            let mt = 1.0 - t;
            let qx = mt * mt * px + 2.0 * mt * t * x1 + t * t * x;
            let qy = mt * mt * py + 2.0 * mt * t * y1 + t * t * y;
            self.current.push((qx, qy));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (px, py) = *self.current.last().unwrap_or(&(0.0, 0.0));
        for i in 1..=CURVE_SUBDIVISIONS {
            let t = i as f32 / CURVE_SUBDIVISIONS as f32;
            let mt = 1.0 - t;
            let cx =
                mt * mt * mt * px + 3.0 * mt * mt * t * x1 + 3.0 * mt * t * t * x2 + t * t * t * x;
            let cy =
                mt * mt * mt * py + 3.0 * mt * mt * t * y1 + 3.0 * mt * t * t * y2 + t * t * t * y;
            self.current.push((cx, cy));
        }
    }

    fn close(&mut self) {
        if self.current.len() > 2 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Font loading (cached)
// ---------------------------------------------------------------------------

fn load_font_data() -> Option<&'static [u8]> {
    static CACHE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    CACHE
        .get_or_init(|| crate::render::decal::load_ui_font_bytes())
        .as_deref()
}

// ---------------------------------------------------------------------------
// Triangulation helpers
// ---------------------------------------------------------------------------

/// Build front and back cap triangles plus side walls for a set of contours.
fn extrude_contours(
    contours: &[Vec<(f32, f32)>],
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
) {
    // Flatten all contours into earcutr format: a flat vec of [x, y, x, y, ...]
    // with a `hole_indices` vec marking where each hole contour starts.
    let mut coords: Vec<f64> = Vec::new();
    let mut hole_indices: Vec<usize> = Vec::new();

    // The first contour is the outer boundary; subsequent contours are holes
    // (inner cutouts like the counter of '0', '8', etc.).
    //
    // ttf-parser emits contours in the order the font stores them — for most
    // TrueType fonts the outer contour comes first and inner contours follow,
    // but there's no hard guarantee. We use signed area to sort: the contour
    // with the largest absolute area is the outer boundary, and the rest are
    // holes.
    if contours.is_empty() {
        return;
    }

    // Compute signed area for each contour (positive = CCW, negative = CW).
    let signed_area = |pts: &[(f32, f32)]| -> f64 {
        let n = pts.len();
        let mut a = 0.0_f64;
        for i in 0..n {
            let j = (i + 1) % n;
            a += (pts[j].0 as f64 - pts[i].0 as f64) * (pts[j].1 as f64 + pts[i].1 as f64);
        }
        a * 0.5
    };

    let mut areas: Vec<(usize, f64)> = contours
        .iter()
        .enumerate()
        .map(|(i, c)| (i, signed_area(c)))
        .collect();
    // Sort by descending absolute area — largest is outer.
    areas.sort_by(|a, b| {
        b.1.abs()
            .partial_cmp(&a.1.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let outer_idx = areas[0].0;
    let outer_area = areas[0].1;

    // Outer contour must be CCW for earcutr. If it's CW (negative area), we
    // need to reverse it. Holes must be CW (opposite winding).
    let maybe_reverse = |pts: &[(f32, f32)], want_ccw: bool, area: f64| -> Vec<(f32, f32)> {
        let is_ccw = area > 0.0;
        if is_ccw == want_ccw {
            pts.to_vec()
        } else {
            pts.iter().copied().rev().collect()
        }
    };

    let outer = maybe_reverse(&contours[outer_idx], true, outer_area);
    for &(x, y) in &outer {
        coords.push(x as f64);
        coords.push(y as f64);
    }

    let hole_contours: Vec<Vec<(f32, f32)>> = areas[1..]
        .iter()
        .map(|&(i, a)| maybe_reverse(&contours[i], false, a))
        .collect();

    for hole in &hole_contours {
        hole_indices.push(coords.len() / 2);
        for &(x, y) in hole {
            coords.push(x as f64);
            coords.push(y as f64);
        }
    }

    // Triangulate
    let tri_indices = match earcutr::earcut(&coords, &hole_indices, 2) {
        Ok(idx) => idx,
        Err(_) => return, // triangulation failed — skip this glyph
    };

    let all_pts: Vec<(f32, f32)> = coords
        .chunks_exact(2)
        .map(|c| (c[0] as f32, c[1] as f32))
        .collect();

    // Front cap (z = +DEPTH), normal pointing +Z
    let base = vertices.len() as u32;
    for &(x, y) in &all_pts {
        vertices.push(Vertex3dTex {
            position: [x, y, DEPTH],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        });
    }
    for &i in &tri_indices {
        indices.push(base + i as u32);
    }

    // Back cap (z = -DEPTH), normal pointing -Z, reverse winding
    let base2 = vertices.len() as u32;
    for &(x, y) in &all_pts {
        vertices.push(Vertex3dTex {
            position: [x, y, -DEPTH],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
        });
    }
    for tri in tri_indices.chunks(3) {
        indices.push(base2 + tri[0] as u32);
        indices.push(base2 + tri[2] as u32);
        indices.push(base2 + tri[1] as u32);
    }

    // Side walls — extrude each contour edge
    let mut build_walls = |pts: &[(f32, f32)]| {
        let n = pts.len();
        for i in 0..n {
            let j = (i + 1) % n;
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[j];
            // Edge normal (pointing outward for CCW winding)
            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = (dx * dx + dy * dy).sqrt().max(1e-8);
            let nx = dy / len;
            let ny = -dx / len;
            let normal = [nx, ny, 0.0];

            let base_w = vertices.len() as u32;
            // Quad: two triangles
            vertices.push(Vertex3dTex {
                position: [x0, y0, DEPTH],
                normal,
                uv: [0.0, 0.0],
            });
            vertices.push(Vertex3dTex {
                position: [x1, y1, DEPTH],
                normal,
                uv: [1.0, 0.0],
            });
            vertices.push(Vertex3dTex {
                position: [x1, y1, -DEPTH],
                normal,
                uv: [1.0, 1.0],
            });
            vertices.push(Vertex3dTex {
                position: [x0, y0, -DEPTH],
                normal,
                uv: [0.0, 1.0],
            });
            indices.extend_from_slice(&[
                base_w,
                base_w + 1,
                base_w + 2,
                base_w,
                base_w + 2,
                base_w + 3,
            ]);
        }
    };

    build_walls(&outer);
    for hole in &hole_contours {
        build_walls(hole);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Builds and caches `MeshCpu`s for label strings.
///
/// All labels are composed from Cormorant Garamond outlines, so any
/// character the font supports can appear. The per-string cache is bounded
/// by the number of distinct popups the gameplay scene actually emits in a
/// session — typically a few dozen.
pub struct GlyphMeshCache {
    cache: HashMap<String, MeshCpu>,
}

impl GlyphMeshCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Get (and lazily build) the mesh for `label`. Returns `None` only
    /// when the label contains no renderable characters.
    pub fn mesh_for(&mut self, label: &str) -> Option<&MeshCpu> {
        if !self.cache.contains_key(label) {
            let mesh = build_label_mesh(label)?;
            self.cache.insert(label.to_string(), mesh);
        }
        self.cache.get(label)
    }
}

impl Default for GlyphMeshCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a complete label mesh by walking the input string left-to-right,
/// extracting each character's font outline, extruding it, then recentring
/// the whole thing around the origin so the renderer can place it via a
/// translation alone.
fn build_label_mesh(label: &str) -> Option<MeshCpu> {
    let font_data = load_font_data()?;
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;

    let units_per_em = face.units_per_em() as f32;
    let scale = 1.0 / units_per_em; // normalise to ~1.0 height

    let chars: Vec<char> = label.chars().collect();
    if chars.is_empty() {
        return None;
    }

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut cursor_x = 0.0_f32;
    let mut emitted = false;

    for &ch in &chars {
        let glyph_id = face.glyph_index(ch)?;
        let advance = face
            .glyph_hor_advance(glyph_id)
            .unwrap_or(units_per_em as u16) as f32
            * scale;

        let mut collector = OutlineCollector::new();
        if face.outline_glyph(glyph_id, &mut collector).is_none() {
            // No outline (e.g. space) — just advance the cursor.
            cursor_x += advance;
            continue;
        }
        // Flush any trailing open contour.
        if collector.current.len() > 2 {
            collector.contours.push(collector.current);
        }
        if collector.contours.is_empty() {
            cursor_x += advance;
            continue;
        }

        // Scale and translate contour points.
        let offset_x = cursor_x;
        let contours: Vec<Vec<(f32, f32)>> = collector
            .contours
            .iter()
            .map(|c| {
                c.iter()
                    .map(|&(x, y)| (x * scale + offset_x, -y * scale))
                    .collect()
            })
            .collect();

        extrude_contours(&contours, &mut vertices, &mut indices);
        emitted = true;
        cursor_x += advance;
    }

    if !emitted {
        return None;
    }

    // Recentre the mesh around the origin.
    let half_w = cursor_x * 0.5;
    // Find vertical bounds to centre vertically too.
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for v in &vertices {
        min_y = min_y.min(v.position[1]);
        max_y = max_y.max(v.position[1]);
    }
    let mid_y = (min_y + max_y) * 0.5;
    for v in &mut vertices {
        v.position[0] -= half_w;
        v.position[1] -= mid_y;
    }

    Some(MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [1.0, 0.92, 0.55, 1.0], // overridden per-instance
            specular_strength: 0.55,
            specular_power: 64.0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_meshes_for_score_labels() {
        let mut cache = GlyphMeshCache::new();
        for label in ["+50", "+100", "=12500"] {
            let mesh = cache.mesh_for(label);
            assert!(mesh.is_some(), "mesh_for({label:?}) returned None");
            let mesh = mesh.unwrap();
            assert!(
                !mesh.vertices.is_empty(),
                "mesh_for({label:?}) produced 0 vertices"
            );
        }
    }

    #[test]
    fn empty_string_returns_none() {
        let mut cache = GlyphMeshCache::new();
        assert!(cache.mesh_for("").is_none());
    }
}
