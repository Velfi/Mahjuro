//! Generic spatial-navigation helpers shared by scenes that drive a 2D
//! "focus rect graph" — a list of `(target, screen_rect)` tuples that the
//! cursor + controller / keyboard agree on as the navigable surface.
//!
//! Directional navigation auto-infers rows, columns, and loose groups from
//! rect geometry (`graph` module). Scenes usually only register rects via
//! [`FocusNavState::add`]; explicit edges and [`FocusScope`] filters are
//! optional escape hatches.
//!
//! ## Typical scene usage
//!
//! ```ignore
//! focus_nav.begin_frame();
//! focus_nav.add(target, rect);
//! focus_nav.end_frame();
//! let next = focus_nav.pick(current, FocusDir::Right);
//! ```
//!
mod debug;
mod flat_scroll;
mod graph;
mod rect_focus;
mod scope;
mod session;

pub use debug::{
    FocusNavDebugSnapshot, debug_snapshot_from_candidates, push_focus_nav_debug_overlay,
};
pub use flat_scroll::clamp_index_into_viewport;
pub use graph::FocusMemory;
pub use rect_focus::RectFocusSession;
pub use scope::FocusScope;
pub use session::FocusNavState;

use crate::render::theme::color;
use crate::render::wgpu_renderer::GpuInstance;

/// Direction for spatial navigation between focus targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[inline]
pub fn clamp_rect_to_viewport(rect: [f32; 4], window_w: f32, window_h: f32) -> Option<[f32; 4]> {
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

/// Hit-test registered nodes for `(cx, cy)`, preferring the smallest rect.
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
    if rw <= bt * 2.0 || rh <= bt * 2.0 {
        quads.push(GpuInstance {
            rect: clamped,
            color: color::RELIC_GOLD,
            user: 0,
        });
        return;
    }

    let ring = color::RELIC_GOLD;
    let borders = [
        [rx, ry, rw, bt],
        [rx, ry + rh - bt, rw, bt],
        [rx, ry, bt, rh],
        [rx + rw - bt, ry, bt, rh],
    ];
    for border in borders {
        quads.push(GpuInstance {
            rect: border,
            color: ring,
            user: 0,
        });
    }
}
