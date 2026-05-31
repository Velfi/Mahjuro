//! Shared image → cap-local → UV mapping for silhouette-extruded lit meshes.
//!
//! See `docs/agents/cap-mesh-coordinates.md` for the full convention table
//! (relic pin ±Y cap vs talisman ±Z cap, shader normal signs, placement).

use crate::tile_glb::Vertex3dTex;

/// Target front-cap area used to equalize on-screen footprint across pins/pendants.
pub const CAP_REFERENCE_AREA: f32 = std::f32::consts::PI * 0.25;

/// Which local axis is thin; the cap lies in the plane spanned by the other two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapExtrudeKind {
    /// Relic / ordeal icon pin: cap normal **+Y**, silhouette **XZ**, thickness **±Y**.
    /// Image row 0 → smaller local **Z** (no vertical flip).
    RelicPinY,
    /// Talisman pendant: cap normal **+Z**, silhouette **XY**, thickness **±Z**.
    /// Image row 0 → larger local **+Y** (vertical flip in [`pixel_to_cap_local`]).
    TalismanZ,
}

impl CapExtrudeKind {
    /// When true, cap “display up” uses `(union_cy - py)` instead of `(py - union_cy)`.
    #[inline]
    pub const fn flip_image_y_to_cap_vertical(self) -> bool {
        matches!(self, CapExtrudeKind::TalismanZ)
    }

    /// CPU height-gradient normal for a flat **front** cap (+Y or +Z).
    ///
    /// Relic (+Y cap): `(-dhdu, 1, -dhdv)` — matches WGSL chitin branch for Y caps.
    /// Talisman (+Z cap): `(-dhdu, +dhdv, 1)` — **+dhdv** because texture v grows toward −local Y.
    #[inline]
    pub fn height_normal_from_grad_front(self, dhdu: f32, dhdv: f32) -> [f32; 3] {
        match self {
            CapExtrudeKind::RelicPinY => [-dhdu, 1.0, -dhdv],
            CapExtrudeKind::TalismanZ => [-dhdu, dhdv, 1.0],
        }
    }
}

/// Union bbox + image size for silhouette extrusion (pixel space, y down).
#[derive(Clone, Copy, Debug)]
pub struct SilhouetteUnionBounds {
    pub union_cx: f32,
    pub union_cy: f32,
    pub union_extent: f32,
    pub inv_w: f32,
    pub inv_h: f32,
}

impl SilhouetteUnionBounds {
    pub fn from_bbox(ux_min: f32, uy_min: f32, ux_max: f32, uy_max: f32, width: u32, height: u32) -> Self {
        Self {
            union_cx: 0.5 * (ux_min + ux_max),
            union_cy: 0.5 * (uy_min + uy_max),
            union_extent: ((ux_max - ux_min).max(uy_max - uy_min) * 0.5).max(1.0),
            inv_w: 1.0 / width.max(1) as f32,
            inv_h: 1.0 / height.max(1) as f32,
        }
    }
}

/// Albedo / heightmap UV from raw pixel coords. **Never flip V** — GPU row 0 is v=0 (image top).
#[inline]
pub fn pixel_to_albedo_uv(px_x: f32, px_y: f32, bounds: SilhouetteUnionBounds) -> [f32; 2] {
    [px_x * bounds.inv_w, px_y * bounds.inv_h]
}

/// Cap-plane local coords (centered, ±0.5 extent on the wider bbox axis).
#[inline]
pub fn pixel_to_cap_local(
    px_x: f32,
    px_y: f32,
    bounds: SilhouetteUnionBounds,
    kind: CapExtrudeKind,
) -> (f32, f32) {
    let lx = (px_x - bounds.union_cx) * 0.5 / bounds.union_extent;
    let ly_raw = (px_y - bounds.union_cy) * 0.5 / bounds.union_extent;
    let ly = if kind.flip_image_y_to_cap_vertical() {
        -ly_raw
    } else {
        ly_raw
    };
    (lx, ly)
}

#[inline]
pub fn cap_local_and_uv_from_pixel(
    px_x: f32,
    px_y: f32,
    bounds: SilhouetteUnionBounds,
    kind: CapExtrudeKind,
) -> ((f32, f32), [f32; 2]) {
    (
        pixel_to_cap_local(px_x, px_y, bounds, kind),
        pixel_to_albedo_uv(px_x, px_y, bounds),
    )
}

/// Parametric cap UV when coords already live in ~**[-0.5, +0.5]** on the cap plane.
/// +cap_vertical → low v (texture / image top). Used by pack mesh, talisman, ribbon, ofuda +Z face.
#[inline]
pub fn parametric_cap_uv(cap_x: f32, cap_vertical: f32) -> [f32; 2] {
    [cap_x + 0.5, 0.5 - cap_vertical]
}

/// Same as [`parametric_cap_uv`] but mirrors U for a +Y back cap (pack card).
#[inline]
pub fn parametric_cap_uv_mirror_u(cap_x: f32, cap_vertical: f32) -> [f32; 2] {
    let uv = parametric_cap_uv(cap_x, cap_vertical);
    [1.0 - uv[0], uv[1]]
}

/// +Y-facing cap: planar UV from local X/Z in ~**[-0.5, +0.5]**. **+Z → high v**.
/// Coin, mirror, bone yaku tablet; heightmaps projected from above (not PNG row order).
#[inline]
pub fn planar_y_cap_uv_xz(cap_x: f32, cap_z: f32) -> [f32; 2] {
    [cap_x + 0.5, cap_z + 0.5]
}

/// +Y cap with independent half-extents (non-square tray footprint).
#[inline]
pub fn planar_y_cap_uv_xz_extents(cap_x: f32, cap_z: f32, half_w: f32, half_d: f32) -> [f32; 2] {
    [
        cap_x / (2.0 * half_w) + 0.5,
        cap_z / (2.0 * half_d) + 0.5,
    ]
}

/// Sample grayscale height from RGBA at normalized albedo UV (v=0 = image top).
pub fn sample_height_r_luma(rgba: &[u8], width: u32, height: u32, u: f32, v: f32) -> f32 {
    let w = width.max(1);
    let h = height.max(1);
    let x = (u * w as f32).floor().clamp(0.0, w as f32 - 1.0) as u32;
    let y = (v * h as f32).floor().clamp(0.0, h as f32 - 1.0) as u32;
    let i = ((y * w + x) * 4) as usize;
    rgba.get(i).copied().unwrap_or(128) as f32 / 255.0
}

/// Signed area of a closed ring in cap-local coords (shoelace).
pub fn cap_ring_signed_area(cap_pts: &[(f32, f32)]) -> f32 {
    let n = cap_pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0_f32;
    for i in 0..n {
        let j = (i + 1) % n;
        a += cap_pts[i].0 * cap_pts[j].1 - cap_pts[j].0 * cap_pts[i].1;
    }
    a * 0.5
}

/// Outward wall normal for a silhouette edge. `ring_signed_area` is the shoelace
/// area of the full ring in cap-local coords (outer vs hole have opposite sign).
#[inline]
pub fn outward_wall_normal_from_cap_edge(
    edge_a: f32,
    edge_b: f32,
    kind: CapExtrudeKind,
    ring_signed_area: f32,
) -> [f32; 3] {
    let mut n = wall_normal_from_cap_edge(edge_a, edge_b, kind);
    let flip = match kind {
        // Image-Y flip: outers are CW in pixels → CCW in cap-local (negative area).
        CapExtrudeKind::TalismanZ => ring_signed_area > 0.0,
        // No flip: outers positive, holes negative.
        CapExtrudeKind::RelicPinY => ring_signed_area < 0.0,
    };
    if flip {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

/// Outward wall normal from a silhouette edge by probing which side of the edge
/// is solid in the source bitmap (robust for outers, holes, and nested rings).
pub fn outward_wall_normal_for_silhouette_edge(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    solid: &[bool],
    grid_w: i32,
    grid_h: i32,
    bounds: SilhouetteUnionBounds,
    kind: CapExtrudeKind,
) -> [f32; 3] {
    let (lx0, ly0) = pixel_to_cap_local(x0, y0, bounds, kind);
    let (lx1, ly1) = pixel_to_cap_local(x1, y1, bounds, kind);
    let mut n = wall_normal_from_cap_edge(lx1 - lx0, ly1 - ly0, kind);

    let mx = 0.5 * (x0 + x1);
    let my = 0.5 * (y0 + y1);
    let edx = x1 - x0;
    let edy = y1 - y0;
    let elen = (edx * edx + edy * edy).sqrt().max(1e-6);
    let probe = 0.55;
    let nx_px = -edy / elen * probe;
    let ny_px = edx / elen * probe;

    let left = sample_solid_at(solid, grid_w, grid_h, mx + nx_px, my + ny_px);
    let right = sample_solid_at(solid, grid_w, grid_h, mx - nx_px, my - ny_px);
    let (cm_x, cm_y) = pixel_to_cap_local(mx, my, bounds, kind);
    let empty_dir = if left && !right {
        let (r_x, r_y) = pixel_to_cap_local(mx - nx_px, my - ny_px, bounds, kind);
        glam::Vec3::new(r_x - cm_x, r_y - cm_y, 0.0)
    } else if right && !left {
        let (l_x, l_y) = pixel_to_cap_local(mx + nx_px, my + ny_px, bounds, kind);
        glam::Vec3::new(l_x - cm_x, l_y - cm_y, 0.0)
    } else {
        return n;
    };
    if empty_dir.length_squared() < 1e-12 {
        return n;
    }
    if glam::Vec3::from(n).dot(empty_dir.normalize()) < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

#[inline]
fn sample_solid_at(solid: &[bool], w: i32, h: i32, px: f32, py: f32) -> bool {
    let x = px.floor() as i32;
    let y = py.floor() as i32;
    if x < 0 || y < 0 || x >= w || y >= h {
        false
    } else {
        solid[(y * w + x) as usize]
    }
}

fn tri_face_normal_dot(v: [&Vertex3dTex; 3], want: glam::Vec3) -> f32 {
    let p0 = glam::Vec3::from(v[0].position);
    let p1 = glam::Vec3::from(v[1].position);
    let p2 = glam::Vec3::from(v[2].position);
    let f = (p1 - p0).cross(p2 - p0);
    if f.length_squared() < 1e-12 {
        return 0.0;
    }
    f.normalize().dot(want)
}

/// Triangle indices for a side-wall quad, oriented so face normals agree with
/// the quad's assigned vertex normal. Vert layout: 0=bot0, 1=top0, 2=top1, 3=bot1.
pub fn side_wall_quad_indices_oriented(base: u32, vertices: &[Vertex3dTex]) -> [u32; 6] {
    let v = |i: u32| &vertices[i as usize];
    let want = glam::Vec3::from(v(base).normal);

    let a: [[u32; 3]; 2] = [
        [base, base + 1, base + 2],
        [base + 3, base + 2, base],
    ];
    let b: [[u32; 3]; 2] = [
        [base + 1, base + 3, base],
        [base + 2, base + 3, base],
    ];
    let score = |tris: &[[u32; 3]; 2]| -> f32 {
        tris.iter()
            .map(|tri| tri_face_normal_dot([v(tri[0]), v(tri[1]), v(tri[2])], want))
            .fold(f32::INFINITY, f32::min)
    };

    if score(&a) >= score(&b) {
        [base, base + 1, base + 2, base + 3, base + 2, base]
    } else {
        [base + 1, base + 3, base, base + 2, base + 3, base]
    }
}

/// Triangle indices for a side-wall quad. Vert layout:
/// 0=bot0, 1=top0, 2=top1, 3=bot1 (same XY as edge endpoints).
#[inline]
pub fn side_wall_quad_indices(base: u32, kind: CapExtrudeKind, ring_signed_area: f32) -> [u32; 6] {
    let outer_winding = match kind {
        CapExtrudeKind::TalismanZ => ring_signed_area <= 0.0,
        CapExtrudeKind::RelicPinY => ring_signed_area >= 0.0,
    };
    if outer_winding {
        [base + 1, base + 3, base, base + 2, base + 3, base]
    } else {
        [base + 3, base + 2, base + 1, base + 3, base + 0, base + 1]
    }
}

/// Outward wall normal from an edge in the cap plane (silhouette extrusion).
#[inline]
pub fn wall_normal_from_cap_edge(edge_a: f32, edge_b: f32, kind: CapExtrudeKind) -> [f32; 3] {
    match kind {
        CapExtrudeKind::RelicPinY => {
            let n = glam::Vec3::new(edge_b, 0.0, -edge_a).normalize_or_zero();
            [n.x, n.y, n.z]
        }
        CapExtrudeKind::TalismanZ => {
            let n = glam::Vec3::new(-edge_b, edge_a, 0.0).normalize_or_zero();
            [n.x, n.y, n.z]
        }
    }
}

pub fn scale_mesh_xy(vertices: &mut [Vertex3dTex], scale: f32) {
    for v in vertices.iter_mut() {
        v.position[0] *= scale;
        v.position[1] *= scale;
    }
}

pub fn scale_mesh_uniform(vertices: &mut [Vertex3dTex], scale: f32) {
    for v in vertices.iter_mut() {
        v.position[0] *= scale;
        v.position[1] *= scale;
        v.position[2] *= scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_uv_does_not_flip_v() {
        let b = SilhouetteUnionBounds::from_bbox(0.0, 0.0, 32.0, 32.0, 32, 32);
        let top = pixel_to_albedo_uv(16.0, 0.0, b);
        let bottom = pixel_to_albedo_uv(16.0, 31.0, b);
        assert!(top[1] < bottom[1]);
    }

    #[test]
    fn talisman_flip_maps_image_top_to_plus_cap_vertical() {
        let b = SilhouetteUnionBounds::from_bbox(0.0, 0.0, 32.0, 32.0, 32, 32);
        let top = pixel_to_cap_local(16.0, 0.0, b, CapExtrudeKind::TalismanZ);
        let bottom = pixel_to_cap_local(16.0, 31.0, b, CapExtrudeKind::TalismanZ);
        assert!(top.1 > bottom.1, "image top should be +cap vertical");
        let uv_top = pixel_to_albedo_uv(16.0, 0.0, b);
        assert!(uv_top[1] < 0.05);
    }

    #[test]
    fn relic_pin_maps_image_top_to_low_cap_depth() {
        let b = SilhouetteUnionBounds::from_bbox(0.0, 0.0, 32.0, 32.0, 32, 32);
        let top = pixel_to_cap_local(16.0, 0.0, b, CapExtrudeKind::RelicPinY);
        let bottom = pixel_to_cap_local(16.0, 31.0, b, CapExtrudeKind::RelicPinY);
        assert!(top.1 < bottom.1, "image top should be smaller local Z");
    }

    #[test]
    fn height_normal_signs_match_shader() {
        let dhdu = 0.1_f32;
        let dhdv = 0.2_f32;
        let relic = CapExtrudeKind::RelicPinY.height_normal_from_grad_front(dhdu, dhdv);
        assert_eq!(relic, [-0.1, 1.0, -0.2]);
        let tal = CapExtrudeKind::TalismanZ.height_normal_from_grad_front(dhdu, dhdv);
        assert_eq!(tal, [-0.1, 0.2, 1.0]);
    }

    #[test]
    fn parametric_cap_uv_matches_legacy_talisman_formula() {
        const R: f32 = 0.5;
        let x = R * 0.4;
        let y = R * 0.92;
        let legacy = [x / R * 0.5 + 0.5, 0.5 - y / R * 0.5];
        let unified = parametric_cap_uv(x / (2.0 * R), y / (2.0 * R));
        assert!((legacy[0] - unified[0]).abs() < 1e-6);
        assert!((legacy[1] - unified[1]).abs() < 1e-6);
    }

    #[test]
    fn planar_y_cap_plus_z_is_high_v() {
        let uv = planar_y_cap_uv_xz(0.0, 0.5);
        assert!((uv[1] - 1.0).abs() < 1e-6);
        let top = parametric_cap_uv(0.0, 0.5);
        assert!(top[1] < 0.01);
    }

    #[test]
    fn planar_extents_matches_unit_square_at_half_extent() {
        let a = planar_y_cap_uv_xz(0.25, -0.25);
        let b = planar_y_cap_uv_xz_extents(0.25, -0.25, 0.5, 0.5);
        assert!((a[0] - b[0]).abs() < 1e-6);
        assert!((a[1] - b[1]).abs() < 1e-6);
    }

    #[test]
    fn outward_normal_flips_for_hole_winding() {
        let outer = outward_wall_normal_from_cap_edge(1.0, 0.0, CapExtrudeKind::TalismanZ, -1.0);
        let hole = outward_wall_normal_from_cap_edge(1.0, 0.0, CapExtrudeKind::TalismanZ, 1.0);
        assert!((outer[0] + hole[0]).abs() < 1e-6);
        assert!((outer[1] + hole[1]).abs() < 1e-6);
    }

    #[test]
    fn mirror_u_flips_horizontal_only() {
        let front = parametric_cap_uv(0.2, 0.3);
        let back = parametric_cap_uv_mirror_u(0.2, 0.3);
        assert!((front[1] - back[1]).abs() < 1e-6);
        assert!((front[0] + back[0] - 1.0).abs() < 1e-6);
    }
}
