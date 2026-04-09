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
pub fn push_focus_ring(rect: [f32; 4], scale: f32, quads: &mut Vec<GpuInstance>) {
    let bt = (3.0 * scale).max(2.0);
    let pad = (4.0 * scale).max(3.0);
    let rx = rect[0] - pad;
    let ry = rect[1] - pad;
    let rw = rect[2] + pad * 2.0;
    let rh = rect[3] + pad * 2.0;
    let ring = [0.95, 0.78, 0.32, 1.0];
    quads.push(GpuInstance {
        rect: [rx, ry, rw, bt],
        color: ring,
    });
    quads.push(GpuInstance {
        rect: [rx, ry + rh - bt, rw, bt],
        color: ring,
    });
    quads.push(GpuInstance {
        rect: [rx, ry, bt, rh],
        color: ring,
    });
    quads.push(GpuInstance {
        rect: [rx + rw - bt, ry, bt, rh],
        color: ring,
    });
}
