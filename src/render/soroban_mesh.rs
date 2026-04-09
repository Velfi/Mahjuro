//! Procedural mesh for the live meld pill — a soroban-styled readout that
//! sits above the player's tile selection and reports the meld currently
//! being formed (name + chips × mult).
//!
//! The pill is a long horizontal lacquered-wood frame with two parallel
//! brass rails (chip rail on top, mult rail on the bottom) carrying sliding
//! beads, an inset bone name-tag on the left end, and a row of small yaku
//! candidacy pip indents on the right end. Beads must slide independently
//! per scoring tick, so each bead is its own instance — this module exports
//! one `build_*_mesh` per material/role and a set of layout constants the
//! renderer reads to place rods, beads, and decals in frame-local space.
//!
//! All meshes are authored in normalized local space (`-0.5..+0.5` on each
//! axis for the frame; rods/beads use small extents around the origin and
//! the renderer translates them into frame-local space using the constants
//! below before applying the per-instance world matrix).

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

// ---------------------------------------------------------------------------
// Frame-local layout constants. The renderer composes a soroban from one
// frame instance + two rod instances + N bead instances, all parented to
// the same world transform. These constants describe where each piece sits
// inside the frame's normalized -0.5..+0.5 local space.
// ---------------------------------------------------------------------------

/// Half-thickness of the wood frame slab (local Z).
pub const FRAME_HALF_Z: f32 = 0.06;
/// Half-height of the frame body (local Y). The pill is ~3.5:1 aspect.
pub const FRAME_HALF_Y: f32 = 0.14;

/// Frame-face cartouche region. The meld name is no longer painted onto
/// the frame's wood face — it's carried by a *separate* bone-cartouche
/// mesh hanging above the pill (built by `build_soroban_cartouche_mesh`),
/// so the front face is freed up for the chips × mult numerals. These
/// constants are kept for backwards compatibility with the old single-
/// instance layout but are no longer referenced by the renderer.
#[allow(dead_code)]
pub const TAG_X_MIN: f32 = -0.48;
#[allow(dead_code)]
pub const TAG_X_MAX: f32 = -0.26;
#[allow(dead_code)]
pub const TAG_HALF_Y: f32 = 0.10;
#[allow(dead_code)]
pub const TAG_Z_FRONT: f32 = FRAME_HALF_Z + 0.005;

/// Rails span the full available pill width now that the inset name tag
/// has moved to a separate cartouche mesh. The chips × mult numerals
/// painted onto the frame face occupy the central band; the rails sit
/// just inside the frame's left/right end caps.
pub const RAIL_X_MIN: f32 = -0.42;
pub const RAIL_X_MAX: f32 = 0.42;
/// Chip rail (upper) and mult rail (lower) Y positions in frame-local space.
pub const CHIP_RAIL_Y: f32 = 0.055;
pub const MULT_RAIL_Y: f32 = -0.055;
/// Rails sit in front of the frame body so the beads are visible.
pub const RAIL_Z: f32 = FRAME_HALF_Z + 0.025;

pub const CHIP_BEAD_COUNT: usize = 10;
pub const MULT_BEAD_COUNT: usize = 5;

/// Right-end yaku candidacy pip cluster: small recessed dots near the right cap.
pub const PIP_X: f32 = 0.455;
pub const PIP_COUNT: usize = 5;
pub const PIP_Y_SPACING: f32 = 0.045;

// ── Cartouche (separate mesh, hangs above the pill) ────────────────────
//
// The cartouche is a small bone slab carrying the meld name as a decal,
// hanging off the top edge of the pill via two short brass attachment
// nubs. It's authored as its own `MeshCpu` (separate vertex/index
// buffers, separate material) so it can use the bone-coloured Plain
// material and a different decal texture without fighting the wood
// frame for material slots.
//
// Local space: spans `-0.5..+0.5` on every axis like the other
// procedural meshes; the renderer applies a per-instance scale to size
// it in world units. Aspect is roughly 2.4:1 wide so a single short
// meld-name string ("Pung", "Triplet", "Hand") fills the inset.

/// Half-thickness of the cartouche bone slab.
pub const CARTOUCHE_HALF_Z: f32 = 0.07;
/// Half-height of the cartouche slab body (excluding the brass hanger nubs).
pub const CARTOUCHE_HALF_Y: f32 = 0.32;
/// Brass hanger nub width (measured along local X).
pub const CARTOUCHE_NUB_W: f32 = 0.10;
/// How far the brass nubs poke *below* the cartouche bottom edge — this
/// gap visually attaches the cartouche to the pill's top edge.
pub const CARTOUCHE_NUB_H: f32 = 0.18;

/// Cartouche position in pill-frame-local coordinates. The renderer
/// reads these to anchor the cartouche relative to the pill's anchor
/// matrix so it tracks the pill's rotation/translation but uses its own
/// (bone-friendly) scale.
///
/// `CARTOUCHE_OFFSET_Y` is in *frame-local* units (`0.5` = the top edge
/// of the pill's `-0.5..+0.5` Y range), so the cartouche sits a bit
/// above the top edge with the brass nubs visually crossing the gap.
pub const CARTOUCHE_OFFSET_Y: f32 = 0.95;
/// Cartouche front-face offset in frame-local Z, so it sits proud of
/// the pill's front face the same way the rails do.
pub const CARTOUCHE_OFFSET_Z: f32 = FRAME_HALF_Z + 0.02;

/// Compute the world-frame-local X position of the `i`-th chip bead at a
/// given fill amount in `0.0..=1.0`. Beads cluster to the left when empty
/// and slide right as `fill` rises; this matches a real soroban where
/// counted beads are pushed toward the active end of the rod.
pub fn chip_bead_x(i: usize, fill: f32) -> f32 {
    bead_x_along_rail(i, CHIP_BEAD_COUNT, fill)
}

pub fn mult_bead_x(i: usize, fill: f32) -> f32 {
    bead_x_along_rail(i, MULT_BEAD_COUNT, fill)
}

fn bead_x_along_rail(i: usize, count: usize, fill: f32) -> f32 {
    let span = RAIL_X_MAX - RAIL_X_MIN;
    let bead_pitch = span / (count as f32 + 1.0);
    let lit = (fill.clamp(0.0, 1.0) * count as f32).round() as usize;
    // Lit beads pack flush against the right end of the rod; unlit beads
    // pack flush against the left. The boundary slides with `fill`.
    if i < count - lit {
        RAIL_X_MIN + bead_pitch * (i as f32 + 0.6)
    } else {
        let from_right = count - 1 - i;
        RAIL_X_MAX - bead_pitch * (from_right as f32 + 0.6)
    }
}

// ---------------------------------------------------------------------------
// Mesh builders. Each returns a self-contained MeshCpu so the renderer can
// build one LitMeshGpu per role and instance them as needed.
// ---------------------------------------------------------------------------

/// Build the soroban frame: lacquered-wood body + a row of small pip
/// indents on the right end cap. The meld name now lives on a *separate*
/// cartouche mesh hanging above the pill (see `build_soroban_cartouche_mesh`),
/// so the front face is freed up for the chips × mult numerals painted in
/// by `rasterize_soroban_decal`.
pub fn build_soroban_frame_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Main lacquered-wood body.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -FRAME_HALF_Y,
        FRAME_HALF_Y,
        -FRAME_HALF_Z,
        FRAME_HALF_Z,
    );

    // Reorient the +Z front-face UVs so the engraved decal (meld name on
    // the left tag, pip glow on the right cap, debug rail markings) reads
    // landscape across the long axis. push_box emits faces in the order
    // +X, -X, +Y, -Y, +Z, -Z so the +Z face occupies vertices 16..20 with
    // corner order (x0,y0), (x1,y0), (x1,y1), (x0,y1).
    vertices[16].uv = [0.0, 1.0];
    vertices[17].uv = [1.0, 1.0];
    vertices[18].uv = [1.0, 0.0];
    vertices[19].uv = [0.0, 0.0];
    // Every other body face samples the transparent (0,0) corner so the
    // decal stays pinned to the visible front face. lit_mesh composites
    // decals as `mix(albedo, tex.rgb, tex.a)`, so alpha=0 is a no-op.
    for i in (0..16).chain(20..24) {
        vertices[i].uv = [0.0, 0.0];
    }

    // (The inset bone tag panel that used to live here has been promoted
    // to a separate mesh — `build_soroban_cartouche_mesh` — that hangs
    // above the pill, so the front face is free for the chips × mult
    // numerals.)

    // Right end cap pip cluster — five small recessed dots stacked
    // vertically. Each pip is a tiny inset box; the decal lights them up
    // when the current selection feeds a yaku candidate.
    for k in 0..PIP_COUNT {
        let pip_base = vertices.len();
        let cy = (k as f32 - (PIP_COUNT as f32 - 1.0) * 0.5) * PIP_Y_SPACING;
        push_box(
            &mut vertices,
            &mut indices,
            PIP_X - 0.012,
            PIP_X + 0.012,
            cy - 0.012,
            cy + 0.012,
            FRAME_HALF_Z - 0.008,
            FRAME_HALF_Z + 0.002,
        );
        for i in pip_base..vertices.len() {
            vertices[i].uv = [0.0, 0.0];
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            // Flat variant: the soroban frame is a thin slab like the
            // hanging plaque, so the table-tuned wood vertex displacement
            // would punch through it. Use the displacement-free branch.
            kind: MaterialKind::LacqueredWoodFlat,
            base_color: [1.0, 1.0, 1.0, 1.0],
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}

/// Build the cartouche: a small bone slab carrying the meld name as a
/// per-instance decal, plus two short brass attachment nubs at the bottom
/// edge that visually "hang" it off the top of the soroban pill (mirror
/// of the way the score plaque hangs from chain nubs at its top edge).
///
/// Author space spans `-0.5..+0.5` on every axis. The renderer applies a
/// per-instance scale derived from the pill's height so the cartouche
/// reads as a small nameplate hanging above the pill rather than a
/// second pill in its own right.
///
/// Single material: the bone-coloured `Plain` branch — the cartouche is
/// a small object and the procedural lacquered-wood material would read
/// as visual noise at this scale. The decal is a transparent overlay
/// composited as `mix(albedo, tex.rgb, tex.a)` exactly the same as the
/// plaque + frame paths, so an empty meld name produces a blank
/// cartouche with the bone material showing through.
pub fn build_soroban_cartouche_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Bone slab body.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -CARTOUCHE_HALF_Y,
        CARTOUCHE_HALF_Y,
        -CARTOUCHE_HALF_Z,
        CARTOUCHE_HALF_Z,
    );

    // The +Z front face occupies vertices 16..20 — orient its UVs so the
    // engraved meld name reads landscape across the long axis with the
    // top of the texture sitting at the top of the visible face.
    vertices[16].uv = [0.0, 1.0];
    vertices[17].uv = [1.0, 1.0];
    vertices[18].uv = [1.0, 0.0];
    vertices[19].uv = [0.0, 0.0];
    // All other body faces sample the transparent corner so the engraved
    // meld name only appears on the front face. The bone material on
    // the back/edges stays untouched.
    for i in (0..16).chain(20..24) {
        vertices[i].uv = [0.0, 0.0];
    }

    // Two brass attachment nubs at the bottom edge. They poke *down*
    // out of the bottom of the cartouche slab so the renderer can
    // visually bridge the small gap between the cartouche bottom and
    // the pill top — same idea as the score plaque's chain nubs at the
    // top corners, just upside down because the cartouche hangs from
    // the bottom instead of from the top.
    let nub_inset = 0.18;
    let left_nub_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        -0.5 + nub_inset,
        -0.5 + nub_inset + CARTOUCHE_NUB_W,
        -CARTOUCHE_HALF_Y - CARTOUCHE_NUB_H,
        -CARTOUCHE_HALF_Y,
        -CARTOUCHE_HALF_Z * 0.5,
        CARTOUCHE_HALF_Z * 0.5,
    );
    let right_nub_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        0.5 - nub_inset - CARTOUCHE_NUB_W,
        0.5 - nub_inset,
        -CARTOUCHE_HALF_Y - CARTOUCHE_NUB_H,
        -CARTOUCHE_HALF_Y,
        -CARTOUCHE_HALF_Z * 0.5,
        CARTOUCHE_HALF_Z * 0.5,
    );
    // Nub vertices sample the transparent decal corner so the meld name
    // engraving doesn't bleed onto them. The brass material is uniform
    // (the renderer sets it via the per-instance base color override).
    for i in left_nub_base..vertices.len() {
        vertices[i].uv = [0.0, 0.0];
    }
    let _ = right_nub_base;

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            // Plain branch — the renderer overrides base_color per
            // instance so the bone slab + brass nubs share one mesh /
            // one material slot but read as different surfaces via the
            // engraved decal compositing.
            kind: MaterialKind::Plain,
            // Warm bone cream — same hue as the old inset tag.
            base_color: [0.92, 0.88, 0.78, 1.0],
            specular_strength: 0.20,
            specular_power: 48.0,
        },
    }
}

/// Build a single brass rod mesh. The renderer instances this twice (chip
/// rail + mult rail) and translates each copy to its rail Y in frame-local
/// space. Authored in local space spanning the full rail X range so a
/// per-instance translate (no scale) places it correctly.
pub fn build_soroban_rod_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // A thin square-section bar approximating a turned brass rod. Square
    // section keeps the tri count at the price of looking flat from a
    // top-down camera; the gameplay camera is angled enough that this
    // reads as a rod.
    let half_section = 0.008;
    push_box(
        &mut vertices,
        &mut indices,
        RAIL_X_MIN - 0.02,
        RAIL_X_MAX + 0.02,
        -half_section,
        half_section,
        -half_section,
        half_section,
    );
    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            // Warm brass tint, matches the chain nubs on plaque.
            base_color: [0.95, 0.78, 0.34, 1.0],
            specular_strength: 0.85,
            specular_power: 128.0,
        },
    }
}

/// Build a single chip-rail bead: a flattened oblate disc in bone white.
/// The renderer instances this `CHIP_BEAD_COUNT` times and slides each one
/// along the chip rail per `chip_bead_x()`.
pub fn build_soroban_chip_bead_mesh() -> MeshCpu {
    build_bead_mesh([0.93, 0.90, 0.82, 1.0], 0.20, 0.45)
}

/// Build a single mult-rail bead: larger and red-lacquered. The mult rail
/// uses fewer, heavier beads — each one represents a bigger value and
/// animates with a slower, more ceremonial curve.
pub fn build_soroban_mult_bead_mesh() -> MeshCpu {
    build_bead_mesh([0.78, 0.18, 0.18, 1.0], 0.25, 0.55)
}

/// Shared bead builder. Beads are oblate (squished on Z so they read as
/// discs threaded on the rod), with a small gold pip implied via the decal
/// texture rather than separate geometry.
fn build_bead_mesh(color: [f32; 4], specular_strength: f32, _gold_pip_hint: f32) -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Half-extents for the oblate bead. Wider on Y/Z (the visible faces),
    // narrow on X (the threading axis). Rendered as a lo-poly faceted
    // disc — the gameplay camera distance hides the facets.
    let hx = 0.012;
    let hy = 0.022;
    let hz = 0.022;

    // Center cube body...
    push_box(&mut vertices, &mut indices, -hx, hx, -hy, hy, -hz, hz);
    // ...plus four chamfer slabs that round off the silhouette into
    // something disc-like without paying for a real sphere tessellation.
    let cham = 0.006;
    // top chamfer
    push_box(
        &mut vertices,
        &mut indices,
        -hx + cham,
        hx - cham,
        hy,
        hy + cham,
        -hz + cham,
        hz - cham,
    );
    // bottom chamfer
    push_box(
        &mut vertices,
        &mut indices,
        -hx + cham,
        hx - cham,
        -hy - cham,
        -hy,
        -hz + cham,
        hz - cham,
    );
    // front chamfer
    push_box(
        &mut vertices,
        &mut indices,
        -hx + cham,
        hx - cham,
        -hy + cham,
        hy - cham,
        hz,
        hz + cham,
    );
    // back chamfer
    push_box(
        &mut vertices,
        &mut indices,
        -hx + cham,
        hx - cham,
        -hy + cham,
        hy - cham,
        -hz - cham,
        -hz,
    );

    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            // Plain branch — the bead surface is a solid lacquered colour
            // shaded by the standard diffuse+specular path. The gold pip
            // is painted in by the decal texture when present.
            kind: MaterialKind::Plain,
            base_color: color,
            specular_strength,
            specular_power: 72.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bead_layout_packs_lit_beads_to_the_right() {
        // With fill = 0 every bead sits in the unlit cluster, packed flush
        // against the left end of the rail in monotonic order.
        let xs_empty: Vec<f32> = (0..CHIP_BEAD_COUNT).map(|i| chip_bead_x(i, 0.0)).collect();
        assert!(xs_empty.windows(2).all(|w| w[0] < w[1]));
        assert!(xs_empty[0] >= RAIL_X_MIN);
        assert!(*xs_empty.last().unwrap() <= RAIL_X_MAX);

        // With fill = 1 every bead sits in the lit cluster, packed flush
        // against the right end.
        let xs_full: Vec<f32> = (0..CHIP_BEAD_COUNT).map(|i| chip_bead_x(i, 1.0)).collect();
        assert!(xs_full.windows(2).all(|w| w[0] < w[1]));
        assert!(*xs_full.last().unwrap() <= RAIL_X_MAX);
        // Full state must place beads strictly to the right of empty state.
        assert!(xs_full[0] > xs_empty[0]);

        // Half full: exactly half the beads moved off their empty positions
        // and into the lit cluster on the right.
        let xs_half: Vec<f32> = (0..CHIP_BEAD_COUNT).map(|i| chip_bead_x(i, 0.5)).collect();
        let moved = xs_half
            .iter()
            .zip(xs_empty.iter())
            .filter(|(h, e)| (*h - *e).abs() > 1e-4)
            .count();
        assert_eq!(moved, CHIP_BEAD_COUNT / 2);
    }

    #[test]
    fn frame_mesh_builds_without_panic() {
        let m = build_soroban_frame_mesh();
        assert!(!m.vertices.is_empty());
        assert!(!m.indices.is_empty());
        assert_eq!(m.indices.len() % 3, 0);
    }

    #[test]
    fn rod_and_bead_meshes_build() {
        for m in [
            build_soroban_rod_mesh(),
            build_soroban_chip_bead_mesh(),
            build_soroban_mult_bead_mesh(),
        ] {
            assert!(!m.vertices.is_empty());
            assert_eq!(m.indices.len() % 3, 0);
        }
    }
}
