//! 3D extruded number glyphs for floating score popups.
//!
//! Originally tried `meshtext` to extrude the project's UI font (Cormorant
//! Garamond) but that font's variable-axis outlines have overlapping
//! subpaths that crash the constrained-Delaunay triangulator on every "+",
//! "x", and several digits. Rather than bundle a second font just for the
//! popups, this module builds glyphs procedurally from a small set of
//! axis-aligned bone segments — a 7-segment-display style that reads as a
//! "score readout" and ties visually back to the carved-bone aesthetic of
//! the cascade tokens.
//!
//! Supported characters: `0..=9`, `+`, `-`, `=`, `.`, `x` (rendered as a
//! horizontal+vertical cross — the colour distinguishes mult from chips,
//! so the visual ambiguity with `+` is harmless). Anything else is silently
//! skipped.
//!
//! Each character occupies a normalised 0.6 × 1.0 box. Whole label meshes
//! are recentred so the rendered string sits around the world origin with
//! a height of ≈1.0 unit, matching the height the renderer expects.

use std::collections::HashMap;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Per-character cell width (the 0.6 figure makes digits taller than wide,
/// like real numerals).
const CHAR_WIDTH: f32 = 0.62;
/// Horizontal gap between adjacent characters (added to `CHAR_WIDTH` for
/// the per-glyph advance).
const CHAR_KERN: f32 = 0.10;
/// Half-thickness of a single segment in normalised units.
const T: f32 = 0.075;
/// Half-extent of a glyph along the extrusion (depth) axis.
const DEPTH: f32 = 0.18;

/// One axis-aligned segment in a glyph's local coordinates. Coordinates are
/// in [-CHAR_WIDTH/2, +CHAR_WIDTH/2] × [-0.5, +0.5].
#[derive(Clone, Copy, Debug)]
struct Seg {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

/// Standard 7-segment positions, lifted out so each digit/symbol below is
/// just a list of references.
mod segs {
    use super::{Seg, T};

    /// Inner half-width that horizontal bars span. Leaves room for vertical
    /// bars at each end.
    const HX: f32 = 0.62 / 2.0 - 2.0 * T;

    /// Top horizontal (segment a in 7-seg parlance).
    pub const A: Seg = Seg { x0: -HX, x1: HX, y0: 0.5 - 2.0 * T, y1: 0.5 };
    /// Middle horizontal (segment g).
    pub const G: Seg = Seg { x0: -HX, x1: HX, y0: -T, y1: T };
    /// Bottom horizontal (segment d).
    pub const D: Seg = Seg { x0: -HX, x1: HX, y0: -0.5, y1: -0.5 + 2.0 * T };

    /// Top-right vertical (segment b).
    pub const B: Seg = Seg { x0: HX, x1: HX + 2.0 * T, y0: T, y1: 0.5 - 2.0 * T };
    /// Bottom-right vertical (segment c).
    pub const C: Seg = Seg { x0: HX, x1: HX + 2.0 * T, y0: -0.5 + 2.0 * T, y1: -T };
    /// Top-left vertical (segment f).
    pub const F: Seg = Seg { x0: -HX - 2.0 * T, x1: -HX, y0: T, y1: 0.5 - 2.0 * T };
    /// Bottom-left vertical (segment e).
    pub const E: Seg = Seg { x0: -HX - 2.0 * T, x1: -HX, y0: -0.5 + 2.0 * T, y1: -T };

    /// Decimal point: small square anchored at the bottom-right.
    pub const DOT: Seg = Seg { x0: HX, x1: HX + 2.0 * T, y0: -0.5, y1: -0.5 + 2.0 * T };

    /// "+" / "x" vertical bar centred about the origin.
    pub const PLUS_V: Seg = Seg { x0: -T, x1: T, y0: -0.32, y1: 0.32 };
}

fn segments_for(c: char) -> &'static [Seg] {
    use segs::*;
    match c {
        '0' => &[A, B, C, D, E, F],
        '1' => &[B, C],
        '2' => &[A, B, G, E, D],
        '3' => &[A, B, C, D, G],
        '4' => &[F, G, B, C],
        '5' => &[A, F, G, C, D],
        '6' => &[A, F, G, E, C, D],
        '7' => &[A, B, C],
        '8' => &[A, B, C, D, E, F, G],
        '9' => &[A, B, C, D, F, G],
        '+' => &[G, PLUS_V],
        // Multiplication "x": same shape as "+". The popup colour (warm
        // crimson for mult) keeps the readout legible despite the visual
        // collision with chip popups, and the alternative (a true diagonal
        // cross) would need rotated geometry that doesn't compose with the
        // axis-aligned `push_box` builder.
        'x' | 'X' | '×' => &[G, PLUS_V],
        '-' => &[G],
        '=' => &[A, D],
        '.' => &[DOT],
        _ => &[],
    }
}

/// Builds and caches `MeshCpu`s for label strings.
///
/// All labels are composed from a small fixed character set, so the
/// per-string cache is bounded by the number of distinct popups the
/// gameplay scene actually emits in a session — typically a few dozen.
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
/// emitting each character's segments as extruded boxes, then recentring
/// the whole thing around the origin so the renderer can place it via a
/// translation alone.
fn build_label_mesh(label: &str) -> Option<MeshCpu> {
    let chars: Vec<char> = label.chars().collect();
    if chars.is_empty() {
        return None;
    }

    // Two passes: first compute total advance so we can centre the layout,
    // then emit segments at offset positions.
    let advance: f32 = (CHAR_WIDTH + CHAR_KERN) * chars.len() as f32 - CHAR_KERN;
    let start_x = -advance * 0.5;

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut emitted_anything = false;

    for (i, c) in chars.iter().enumerate() {
        let cx = start_x + (CHAR_WIDTH + CHAR_KERN) * i as f32 + CHAR_WIDTH * 0.5;
        for s in segments_for(*c) {
            push_box(
                &mut vertices,
                &mut indices,
                cx + s.x0,
                cx + s.x1,
                s.y0,
                s.y1,
                -DEPTH,
                DEPTH,
            );
            emitted_anything = true;
        }
    }

    if !emitted_anything {
        return None;
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
        for label in ["+50", "+100", "+2x", "=12500", "+0.6x"] {
            let mesh = cache.mesh_for(label);
            assert!(mesh.is_some(), "mesh_for({label:?}) returned None");
            let mesh = mesh.unwrap();
            assert!(
                !mesh.vertices.is_empty(),
                "mesh_for({label:?}) produced 0 vertices"
            );
            // Procedural glyphs always emit triangles in multiples of 6
            // per box (12 per box, 6 per face × 2 faces actually — no, 2
            // tris per face × 6 faces = 12 indices per box... wait, 6
            // indices per face × 6 faces = 36 indices per box).
            assert_eq!(mesh.indices.len() % 6, 0);
        }
    }

    #[test]
    fn unsupported_chars_are_ignored() {
        let mut cache = GlyphMeshCache::new();
        // "Q" is unsupported; "5" is. The mesh should still build with
        // just the "5" segments and not panic.
        let mesh = cache.mesh_for("Q5").expect("Q5 should still produce a mesh");
        assert!(!mesh.vertices.is_empty());
    }

    #[test]
    fn empty_string_returns_none() {
        let mut cache = GlyphMeshCache::new();
        assert!(cache.mesh_for("").is_none());
    }
}
