//! 3D extruded font glyphs for floating score popups.
//!
//! Uses `ttf-parser` to extract Instrument Serif outlines and
//! `lyon_tessellation` to triangulate the front/back caps with a nonzero
//! fill rule (the rule TrueType outlines are authored against). Side walls
//! and the top bevel are derived from the boundary edges of the fill
//! tessellation — edges adjacent to exactly one triangle — so self-
//! intersecting outlines (e.g. `8`) and unions of overlapping sub-paths
//! (e.g. the two crossing rectangles of `+`) all extrude correctly.
//!
//! Supported characters: any glyph present in the font. Missing glyphs
//! are silently skipped.
//!
//! Each label mesh is recentred so the rendered string sits around the
//! world origin with a height of ~1.0 unit.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;

use lyon_path::Path;
use lyon_path::math::Point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Half-extent of a glyph along the extrusion (depth) axis.
const DEPTH: f32 = 0.032;
/// Inset distance for the polished top bevel.
const BEVEL_INSET: f32 = 0.020;
/// How far the beveled shoulder drops from the top face.
const BEVEL_DEPTH: f32 = 0.012;

/// Flatness tolerance for curve approximation (in normalised em units).
/// Smaller = smoother glyphs, more triangles.
const FLATTEN_TOLERANCE: f32 = 0.0015;

// ---------------------------------------------------------------------------
// Outline collection via ttf-parser → lyon path
// ---------------------------------------------------------------------------

struct LyonPathBuilder {
    builder: lyon_path::path::Builder,
    has_subpath: bool,
    first: Option<(f32, f32)>,
}

impl LyonPathBuilder {
    fn new() -> Self {
        Self {
            builder: Path::builder(),
            has_subpath: false,
            first: None,
        }
    }

    fn finish_open_subpath(&mut self) {
        if self.has_subpath {
            self.builder.end(false);
            self.has_subpath = false;
            self.first = None;
        }
    }
}

impl ttf_parser::OutlineBuilder for LyonPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_open_subpath();
        self.builder.begin(Point::new(x, y));
        self.has_subpath = true;
        self.first = Some((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(Point::new(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder
            .quadratic_bezier_to(Point::new(x1, y1), Point::new(x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder
            .cubic_bezier_to(Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y));
    }

    fn close(&mut self) {
        if self.has_subpath {
            self.builder.end(true);
            self.has_subpath = false;
            self.first = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Font loading (cached)
// ---------------------------------------------------------------------------

fn load_font_data() -> Option<&'static [u8]> {
    static CACHE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    CACHE
        .get_or_init(crate::render::decal::load_ui_font_bytes)
        .as_deref()
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

fn vec2_normalize(v: (f32, f32)) -> (f32, f32) {
    let len = (v.0 * v.0 + v.1 * v.1).sqrt().max(1e-6);
    (v.0 / len, v.1 / len)
}

fn signed_area(pts: &[(f32, f32)]) -> f64 {
    let n = pts.len();
    let mut a = 0.0_f64;
    for i in 0..n {
        let j = (i + 1) % n;
        a += (pts[j].0 as f64 - pts[i].0 as f64) * (pts[j].1 as f64 + pts[i].1 as f64);
    }
    a * 0.5
}

/// Inset a closed polyline inward by `inset` along the vertex bisector.
///
/// Caller passes the boundary loop's winding via `ccw` (true when the loop
/// is visually CCW in the current coordinate space — i.e. the interior lies
/// to the right of each edge in Y-down space, or to the left in Y-up).
fn inset_contour(pts: &[(f32, f32)], inset: f32, ccw: bool) -> Vec<(f32, f32)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let inward = |dx: f32, dy: f32| {
        if ccw {
            vec2_normalize((dy, -dx))
        } else {
            vec2_normalize((-dy, dx))
        }
    };

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        let prev_edge = (cur.0 - prev.0, cur.1 - prev.1);
        let next_edge = (next.0 - cur.0, next.1 - cur.1);
        let prev_len = (prev_edge.0 * prev_edge.0 + prev_edge.1 * prev_edge.1)
            .sqrt()
            .max(1e-6);
        let next_len = (next_edge.0 * next_edge.0 + next_edge.1 * next_edge.1)
            .sqrt()
            .max(1e-6);
        let in_prev = inward(prev_edge.0, prev_edge.1);
        let in_next = inward(next_edge.0, next_edge.1);
        let bis_raw = (in_prev.0 + in_next.0, in_prev.1 + in_next.1);
        let bis = if bis_raw.0.abs() < 1e-5 && bis_raw.1.abs() < 1e-5 {
            in_prev
        } else {
            vec2_normalize(bis_raw)
        };
        let denom = (bis.0 * in_prev.0 + bis.1 * in_prev.1).abs().max(1e-4);
        let max_d = prev_len.min(next_len) * 0.45;
        let d = (inset / denom).min(max_d);
        out.push((cur.0 + bis.0 * d, cur.1 + bis.1 * d));
    }
    out
}

// ---------------------------------------------------------------------------
// Cap tessellation + boundary loop extraction
// ---------------------------------------------------------------------------

/// Triangulated cap and the boundary loops that define its silhouette.
///
/// The boundary loops are reconstructed from the tessellation: an edge that
/// belongs to exactly one triangle is a boundary edge, and these edges chain
/// into closed loops. This works correctly for self-intersecting or
/// overlapping sub-paths because lyon resolves the fill rule before we ever
/// see the geometry.
struct GlyphCap {
    positions: Vec<(f32, f32)>,
    triangles: Vec<[u32; 3]>,
    /// Closed boundary loops, each a sequence of vertex indices into
    /// `positions`. Loop winding follows the tessellator's output (CCW in
    /// Y-up; after our Y-flip, CW visually, i.e. `signed_area` < 0).
    loops: Vec<Vec<u32>>,
}

fn tessellate_glyph(path: &Path) -> Option<GlyphCap> {
    let mut buffers: VertexBuffers<(f32, f32), u32> = VertexBuffers::new();
    let mut tess = FillTessellator::new();
    let options = FillOptions::tolerance(FLATTEN_TOLERANCE).with_fill_rule(FillRule::NonZero);
    let result = tess.tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
            let p = v.position();
            (p.x, p.y)
        }),
    );
    if result.is_err() || buffers.indices.is_empty() {
        return None;
    }

    let triangles: Vec<[u32; 3]> = buffers
        .indices
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();

    let loops = extract_boundary_loops(&triangles);

    Some(GlyphCap {
        positions: buffers.vertices,
        triangles,
        loops,
    })
}

/// Build closed boundary loops from a triangulation. A boundary edge is one
/// used by exactly one triangle; boundary edges chain head-to-tail into
/// loops because the triangulation is a manifold with boundary.
fn extract_boundary_loops(triangles: &[[u32; 3]]) -> Vec<Vec<u32>> {
    // Count how many times each directed edge appears. In a consistently
    // oriented manifold each interior edge appears once forward and once
    // reversed; boundary edges appear only in one direction.
    let mut edge_count: FxHashMap<(u32, u32), i32> = FxHashMap::default();
    for tri in triangles {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let key = if a < b { (a, b) } else { (b, a) };
            let sign = if a < b { 1 } else { -1 };
            *edge_count.entry(key).or_insert(0) += sign;
        }
    }

    // Collect boundary edges in their original (oriented) direction.
    let mut boundary: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for tri in triangles {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let key = if a < b { (a, b) } else { (b, a) };
            // A boundary edge appears exactly once across all triangles.
            // Check by seeing whether the edge's reverse also appears —
            // if edge_count is ±1 it's on the boundary (unmatched).
            if edge_count.get(&key).copied().unwrap_or(0).abs() == 1 {
                boundary.entry(a).or_default().push(b);
            }
        }
    }

    let mut loops: Vec<Vec<u32>> = Vec::new();
    while let Some((&start, _)) = boundary.iter().find(|(_, v)| !v.is_empty()) {
        let mut loop_pts = vec![start];
        let mut cur = start;
        while let Some(next) = boundary.get_mut(&cur).and_then(|v| v.pop()) {
            if next == start {
                break;
            }
            loop_pts.push(next);
            cur = next;
            if loop_pts.len() > 100_000 {
                break; // safety valve
            }
        }
        if loop_pts.len() >= 3 {
            loops.push(loop_pts);
        }
    }

    loops
}

// ---------------------------------------------------------------------------
// Extrusion
// ---------------------------------------------------------------------------

fn extrude_cap(cap: &GlyphCap, vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    // Build the inset top cap: offset each boundary loop inward and
    // re-tessellate the inset polygon.
    let mut inset_path = Path::builder();
    for lp in &cap.loops {
        let pts: Vec<(f32, f32)> = lp.iter().map(|&i| cap.positions[i as usize]).collect();
        // `signed_area > 0` is CCW in the standard (Y-up) sense. Our glyph
        // coords are Y-down (we flipped Y earlier), so the trapezoid variant
        // used by `signed_area` gives negative values for visually-CW loops.
        // Lyon emits a consistently oriented manifold so each loop has a
        // well-defined winding; pass it straight to `inset_contour`.
        let area = signed_area(&pts);
        let ccw = area > 0.0;
        let inset = inset_contour(&pts, BEVEL_INSET, ccw);
        if inset.len() < 3 {
            continue;
        }
        inset_path.begin(Point::new(inset[0].0, inset[0].1));
        for &(x, y) in &inset[1..] {
            inset_path.line_to(Point::new(x, y));
        }
        inset_path.end(true);
    }
    let inset_path = inset_path.build();

    let inset_cap = match tessellate_glyph(&inset_path) {
        Some(c) => c,
        None => return,
    };

    // Top cap (inset), normal +Z.
    let base_top = vertices.len() as u32;
    for &(x, y) in &inset_cap.positions {
        vertices.push(Vertex3dTex {
            position: [x, y, DEPTH],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for tri in &inset_cap.triangles {
        indices.push(base_top + tri[0]);
        indices.push(base_top + tri[1]);
        indices.push(base_top + tri[2]);
    }

    // Back cap (full outline), normal -Z, reverse winding.
    let base_back = vertices.len() as u32;
    for &(x, y) in &cap.positions {
        vertices.push(Vertex3dTex {
            position: [x, y, -DEPTH],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for tri in &cap.triangles {
        indices.push(base_back + tri[0]);
        indices.push(base_back + tri[2]);
        indices.push(base_back + tri[1]);
    }

    // Bevel + side walls built per boundary loop.
    for lp in &cap.loops {
        let outer: Vec<(f32, f32)> = lp.iter().map(|&i| cap.positions[i as usize]).collect();
        let area = signed_area(&outer);
        let ccw = area > 0.0;
        let inner = inset_contour(&outer, BEVEL_INSET, ccw);
        if inner.len() != outer.len() {
            continue;
        }
        build_bevel(&outer, &inner, vertices, indices);
        build_walls(&outer, vertices, indices);
    }
}

fn build_bevel(
    outer: &[(f32, f32)],
    inner: &[(f32, f32)],
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
) {
    let n = outer.len().min(inner.len());
    for i in 0..n {
        let j = (i + 1) % n;
        let p0 = outer[i];
        let p1 = outer[j];
        let q0 = inner[i];
        let q1 = inner[j];
        let e1 = [p1.0 - p0.0, p1.1 - p0.1, 0.0];
        let e2 = [q0.0 - p0.0, q0.1 - p0.1, -BEVEL_DEPTH];
        let normal = {
            let nx = e1[1] * e2[2] - e1[2] * e2[1];
            let ny = e1[2] * e2[0] - e1[0] * e2[2];
            let nz = e1[0] * e2[1] - e1[1] * e2[0];
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            [nx / len, ny / len, nz / len]
        };
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [p0.0, p0.1, DEPTH],
            normal,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [p1.0, p1.1, DEPTH],
            normal,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [q1.0, q1.1, DEPTH - BEVEL_DEPTH],
            normal,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [q0.0, q0.1, DEPTH - BEVEL_DEPTH],
            normal,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3,
            base,
            base + 2,
            base + 1,
            base,
            base + 3,
            base + 2,
        ]);
    }
}

fn build_walls(pts: &[(f32, f32)], vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>) {
    let n = pts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[j];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1e-8);
        let nx = dy / len;
        let ny = -dx / len;
        let normal = [nx, ny, 0.0];

        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [x0, y0, DEPTH - BEVEL_DEPTH],
            normal,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, DEPTH - BEVEL_DEPTH],
            normal,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, -DEPTH],
            normal,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, y0, -DEPTH],
            normal,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Builds and caches `MeshCpu`s for label strings.
///
/// All labels are composed from Instrument Serif outlines, so any
/// character the font supports can appear. The per-string cache is bounded
/// by the number of distinct popups the gameplay scene actually emits in a
/// session — typically a few dozen.
pub struct GlyphMeshCache {
    cache: FxHashMap<String, MeshCpu>,
}

impl GlyphMeshCache {
    pub fn new() -> Self {
        Self {
            cache: FxHashMap::default(),
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

        let mut collector = LyonPathBuilder::new();
        if face.outline_glyph(glyph_id, &mut collector).is_none() {
            // No outline (e.g. space) — just advance the cursor.
            cursor_x += advance;
            continue;
        }
        collector.finish_open_subpath();
        let raw_path = collector.builder.build();

        // Transform path into label space: scale to em-normalised, flip Y,
        // and offset by the horizontal cursor.
        let offset_x = cursor_x;
        let mut shaped = Path::builder();
        let mut sub_started = false;
        for event in raw_path.iter() {
            use lyon_path::Event;
            let tx = |p: Point| Point::new(p.x * scale + offset_x, -p.y * scale);
            match event {
                Event::Begin { at } => {
                    shaped.begin(tx(at));
                    sub_started = true;
                }
                Event::Line { to, .. } => {
                    shaped.line_to(tx(to));
                }
                Event::Quadratic { ctrl, to, .. } => {
                    shaped.quadratic_bezier_to(tx(ctrl), tx(to));
                }
                Event::Cubic {
                    ctrl1, ctrl2, to, ..
                } => {
                    shaped.cubic_bezier_to(tx(ctrl1), tx(ctrl2), tx(to));
                }
                Event::End { close, .. } => {
                    if sub_started {
                        shaped.end(close);
                        sub_started = false;
                    }
                }
            }
        }
        let shaped = shaped.build();

        if let Some(cap) = tessellate_glyph(&shaped) {
            extrude_cap(&cap, &mut vertices, &mut indices);
            emitted = true;
        }
        cursor_x += advance;
    }

    if !emitted {
        return None;
    }

    // Recentre the mesh around the origin.
    let half_w = cursor_x * 0.5;
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

    #[test]
    fn problem_glyphs_produce_nonempty_meshes() {
        let mut cache = GlyphMeshCache::new();
        for label in ["w", "+", "8", "www", "+++", "888"] {
            let mesh = cache.mesh_for(label);
            assert!(mesh.is_some(), "mesh_for({label:?}) returned None");
            let mesh = mesh.unwrap();
            assert!(
                mesh.vertices.len() >= 12,
                "mesh_for({label:?}) produced suspiciously few vertices: {}",
                mesh.vertices.len()
            );
            assert!(
                mesh.indices.len().is_multiple_of(3),
                "mesh_for({label:?}) index count not divisible by 3"
            );
        }
    }
}
