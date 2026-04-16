//! Generic spatial-navigation helpers shared by scenes that drive a 2D
//! "focus rect graph" — a list of `(target, screen_rect)` tuples that the
//! cursor + controller / keyboard agree on as the navigable surface.
//!
//! These functions are deliberately generic over the focus-target type so
//! each scene can use its own enum (gameplay's `FocusTarget`, shop's
//! `ShopFocus`, etc.) without coupling to a single shared enum. The shape
//! of the per-frame "rect graph" is identical across scenes — only the
//! tag types differ — so the picker math, the cursor hit-test, and the
//! brass focus-ring drawing all live here.

use crate::render::wgpu_renderer::GpuInstance;

/// Direction for spatial navigation between focus targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Up,
    Down,
    Left,
    Right,
}

#[inline]
pub fn rect_contains(rect: [f32; 4], cx: f32, cy: f32) -> bool {
    cx >= rect[0] && cx <= rect[0] + rect[2] && cy >= rect[1] && cy <= rect[1] + rect[3]
}

/// Center of an `[x, y, w, h]` rect.
#[inline]
pub fn rect_center(r: [f32; 4]) -> (f32, f32) {
    (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5)
}

/// Intersect `rect` with `[0, 0, window_w, window_h]`. Returns `None`
/// when the rect is entirely outside the viewport.
///
/// Use for UI elements whose natural AABB (e.g. a projected 3D-mesh rect,
/// a tooltip, a centered text label) can extend off-screen. Text inside
/// the returned rect stays centered within the *visible* portion of the
/// element rather than disappearing off the edge of the window.
#[inline]
pub fn clamp_rect_to_viewport(
    rect: [f32; 4],
    window_w: f32,
    window_h: f32,
) -> Option<[f32; 4]> {
    let x0 = rect[0].max(0.0);
    let y0 = rect[1].max(0.0);
    let x1 = (rect[0] + rect[2]).min(window_w);
    let y1 = (rect[1] + rect[3]).min(window_h);
    if x1 > x0 && y1 > y0 {
        Some([x0, y0, x1 - x0, y1 - y0])
    } else {
        None
    }
}

/// Pick the spatially-nearest focus target in `dir` from `current`.
/// Filters candidates whose center is in the requested half-plane and
/// not too far off-axis ("L-shaped" jumps), then ranks by
/// `axial + 2 * perp`. Returns `None` if no candidate qualifies — the
/// caller can fall back to wrapping or staying put.
pub fn pick_neighbor<T: Copy + PartialEq>(
    current: [f32; 4],
    dir: FocusDir,
    candidates: &[(T, [f32; 4])],
) -> Option<T> {
    let (ccx, ccy) = rect_center(current);
    let mut best: Option<(T, f32, f32)> = None;
    for &(target, rect) in candidates {
        // Skip the current rect itself (matched by exact value).
        if rect == current {
            continue;
        }
        let (tcx, tcy) = rect_center(rect);
        let dx = tcx - ccx;
        let dy = tcy - ccy;
        let (axial, perp, in_dir) = match dir {
            FocusDir::Right => (dx, dy.abs(), dx > 0.0),
            FocusDir::Left => (-dx, dy.abs(), dx < 0.0),
            FocusDir::Down => (dy, dx.abs(), dy > 0.0),
            FocusDir::Up => (-dy, dx.abs(), dy < 0.0),
        };
        if !in_dir {
            continue;
        }
        // Reject jumps where the off-axis component dominates — those feel
        // like teleports rather than directional moves.
        if axial < perp * 0.3 {
            continue;
        }
        let score = axial + 2.0 * perp;
        let manhattan = dx.abs() + dy.abs();
        let is_better = match best {
            None => true,
            Some((_, bs, bm)) => score < bs || (score == bs && manhattan < bm),
        };
        if is_better {
            best = Some((target, score, manhattan));
        }
    }
    best.map(|(t, _, _)| t)
}

/// Hit-test the focus rect graph for `(cx, cy)`. Returns the target
/// whose rect contains the cursor, preferring the smallest rect when
/// multiple overlap (so a Consumable slot wins over the Score panel
/// parent it sits near). Used by cursor mode to keep the scene's focus
/// state in sync with the cursor.
pub fn focus_target_at_cursor<T: Copy>(
    candidates: &[(T, [f32; 4])],
    cx: f32,
    cy: f32,
) -> Option<T> {
    let mut best: Option<(T, f32)> = None;
    for &(target, rect) in candidates {
        if !rect_contains(rect, cx, cy) {
            continue;
        }
        let area = rect[2] * rect[3];
        let is_better = match best {
            None => true,
            Some((_, ba)) => area < ba,
        };
        if is_better {
            best = Some((target, area));
        }
    }
    best.map(|(t, _)| t)
}

/// Push a brass focus ring (four border quads) around `rect` into
/// `quads`. Centralizes the highlight pattern that used to live per-zone
/// in scene-specific button-bar / inventory code.
///
/// The padded outer rect is clamped to `[0, 0, window_w, window_h]` as
/// a whole before the four border quads are emitted, so an anchor that
/// extends past a screen edge produces a complete rectangular frame
/// flush against the viewport edge rather than a partial ring that
/// trails off-screen.
pub fn push_focus_ring(
    rect: [f32; 4],
    scale: f32,
    window_w: f32,
    window_h: f32,
    quads: &mut Vec<GpuInstance>,
) {
    let bt = (3.0 * scale).max(2.0);
    let pad = (4.0 * scale).max(3.0);
    let outer = [
        rect[0] - pad,
        rect[1] - pad,
        rect[2] + pad * 2.0,
        rect[3] + pad * 2.0,
    ];
    let Some(clamped) = clamp_rect_to_viewport(outer, window_w, window_h) else {
        return;
    };
    let [rx, ry, rw, rh] = clamped;
    // Degenerate rects thinner than the border thickness can't host a
    // four-sided frame without overlapping — fall back to a single fill
    // so the focus indicator still reads as present.
    if rw <= bt * 2.0 || rh <= bt * 2.0 {
        quads.push(GpuInstance {
            rect: clamped,
            color: [0.95, 0.78, 0.32, 1.0],
        });
        return;
    }

    let ring = [0.95, 0.78, 0.32, 1.0];
    let borders = [
        [rx, ry, rw, bt],           // top
        [rx, ry + rh - bt, rw, bt], // bottom
        [rx, ry, bt, rh],           // left
        [rx + rw - bt, ry, bt, rh], // right
    ];
    for border in borders {
        quads.push(GpuInstance {
            rect: border,
            color: ring,
        });
    }
}
