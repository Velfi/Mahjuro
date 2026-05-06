//! Rasterize embedded Kenney prompt SVGs ([`crate::asset_path`]) to straight-alpha RGBA8.

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// Longest edge in pixels — icons are scaled in screen space; keep texture sharp on HiDPI.
const MAX_EDGE_PX: u32 = 128;

#[inline]
fn unpremultiply_rgba(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        if a <= 1e-6 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        let inv = 1.0 / a;
        px[0] = ((px[0] as f32 * inv).round().clamp(0.0, 255.0)) as u8;
        px[1] = ((px[1] as f32 * inv).round().clamp(0.0, 255.0)) as u8;
        px[2] = ((px[2] as f32 * inv).round().clamp(0.0, 255.0)) as u8;
    }
}

/// Returns `(rgba8, width, height)` or `None` if the asset is missing or invalid.
pub fn rasterize_embedded_svg_rgba(asset_rel_path: &str) -> Option<(Vec<u8>, u32, u32)> {
    let file = crate::asset_path::get(asset_rel_path)?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(file.data.as_ref(), &opt).ok()?;
    let sz = tree.size();
    let w = sz.width();
    let h = sz.height();
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let max_dim = w.max(h);
    let scale = MAX_EDGE_PX as f32 / max_dim;
    let out_w = (w * scale).ceil().max(1.0) as u32;
    let out_h = (h * scale).ceil().max(1.0) as u32;
    let mut pixmap = Pixmap::new(out_w, out_h)?;
    let ts = Transform::from_scale(scale, scale);
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    let mut rgba = pixmap.data().to_vec();
    unpremultiply_rgba(&mut rgba);
    Some((rgba, out_w, out_h))
}
