/// Rectangle intersection helpers for scroll/view clipping.
///
/// Rect format is `[x, y, w, h]` in screen pixels.
#[inline]
pub fn intersect_rect(rect: [f32; 4], clip: [f32; 4]) -> Option<[f32; 4]> {
    let [rx, ry, rw, rh] = rect;
    let [cx, cy, cw, ch] = clip;
    if !(rw > 0.0 && rh > 0.0 && cw > 0.0 && ch > 0.0) {
        return None;
    }
    let x0 = rx.max(cx);
    let y0 = ry.max(cy);
    let x1 = (rx + rw).min(cx + cw);
    let y1 = (ry + rh).min(cy + ch);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

#[cfg(test)]
mod tests {
    use super::intersect_rect;

    #[test]
    fn intersect_rect_clips_partial_overlap() {
        assert_eq!(
            intersect_rect([10.0, 10.0, 20.0, 20.0], [0.0, 0.0, 20.0, 20.0]),
            Some([10.0, 10.0, 10.0, 10.0])
        );
    }

    #[test]
    fn intersect_rect_returns_none_when_disjoint() {
        assert_eq!(
            intersect_rect([50.0, 50.0, 10.0, 10.0], [0.0, 0.0, 20.0, 20.0]),
            None
        );
    }
}
