//! Debug overlay for inferred focus-nav geometry (rows, groups, edges).

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

use super::scope::FocusScope;
use super::session::FocusNavState;
use super::{FocusDir, FocusMemory, rect_center};

const GROUP_COLORS: [[f32; 4]; 6] = [
    [0.35, 0.75, 1.0, 0.55],
    [0.45, 1.0, 0.55, 0.55],
    [1.0, 0.85, 0.35, 0.55],
    [1.0, 0.45, 0.75, 0.55],
    [0.75, 0.55, 1.0, 0.55],
    [1.0, 0.55, 0.35, 0.55],
];

/// Captured focus graph geometry for one frame (type-erased labels).
#[derive(Clone, Debug, Default)]
pub struct FocusNavDebugSnapshot {
    pub nodes: Vec<FocusNavDebugNode>,
    pub rows: Vec<Vec<usize>>,
    pub groups: Vec<u32>,
    pub edges: Vec<(usize, FocusDir, usize)>,
    pub current: Option<usize>,
    pub desired_x: Option<f32>,
    pub desired_y: Option<f32>,
    pub scope_filter: Option<FocusScope>,
}

#[derive(Clone, Debug)]
pub struct FocusNavDebugNode {
    pub rect: [f32; 4],
    pub scope: FocusScope,
    pub label: String,
}

/// Build a one-shot snapshot from explicit rects (and optional edges) without
/// mutating a live [`FocusNavState`].
pub fn debug_snapshot_from_candidates<T: Copy + PartialEq>(
    candidates: &[(T, [f32; 4])],
    edges: &[(T, FocusDir, T)],
    current: Option<T>,
    memory: &FocusMemory,
    label: impl Fn(T) -> String,
) -> FocusNavDebugSnapshot {
    let mut nav = FocusNavState::new();
    nav.load_candidates(candidates, edges);
    let mut snap = nav.debug_snapshot(current, label);
    snap.desired_x = memory.desired_x;
    snap.desired_y = memory.desired_y;
    snap
}

pub fn publish_debug_snapshot(
    enabled: bool,
    out: Option<&mut Option<FocusNavDebugSnapshot>>,
    snapshot: FocusNavDebugSnapshot,
) {
    if enabled && let Some(slot) = out {
        *slot = Some(snapshot);
    }
}

pub fn publish_focus_nav_snapshot<T: Copy + PartialEq>(
    enabled: bool,
    out: Option<&mut Option<FocusNavDebugSnapshot>>,
    nav: &FocusNavState<T>,
    current: Option<T>,
    label: impl Fn(T) -> String,
) {
    if !enabled {
        return;
    }
    if let Some(slot) = out {
        *slot = Some(nav.debug_snapshot(current, label));
    }
}

pub fn publish_focus_nav_graph<T: Copy + PartialEq>(
    enabled: bool,
    out: Option<&mut Option<FocusNavDebugSnapshot>>,
    candidates: &[(T, [f32; 4])],
    edges: &[(T, FocusDir, T)],
    current: Option<T>,
    memory: &FocusMemory,
    label: impl Fn(T) -> String,
) {
    if !enabled {
        return;
    }
    if let Some(slot) = out {
        *slot = Some(debug_snapshot_from_candidates(
            candidates, edges, current, memory, label,
        ));
    }
}

/// Draw row bands, node rects, group tint, explicit edges, and axis memory.
pub fn push_focus_nav_debug_overlay(
    snap: &FocusNavDebugSnapshot,
    window_w: f32,
    window_h: f32,
    scale: f32,
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    if snap.nodes.is_empty() {
        return;
    }

    let row_band = (2.0 * scale).max(1.0);
    for row in &snap.rows {
        if row.is_empty() {
            continue;
        }
        let mut y0 = f32::INFINITY;
        let mut y1 = f32::NEG_INFINITY;
        for &ni in row {
            let r = snap.nodes[ni].rect;
            y0 = y0.min(r[1]);
            y1 = y1.max(r[1] + r[3]);
        }
        quads.push(GpuInstance {
            rect: [0.0, y0 - row_band * 0.5, window_w, row_band],
            color: [0.25, 0.55, 0.95, 0.22],
            user: 0,
        });
        quads.push(GpuInstance {
            rect: [0.0, y1 - row_band * 0.5, window_w, row_band],
            color: [0.25, 0.55, 0.95, 0.22],
            user: 0,
        });
    }

    for (i, node) in snap.nodes.iter().enumerate() {
        let [x, y, w, h] = node.rect;
        let group = snap.groups.get(i).copied().unwrap_or(0);
        let tint = GROUP_COLORS[group as usize % GROUP_COLORS.len()];
        quads.push(GpuInstance {
            rect: [x, y, w, h],
            color: tint,
            user: 0,
        });
        let border = (1.5 * scale).max(1.0);
        let edge = match node.scope {
            FocusScope::Scene => [0.55, 0.55, 0.55, 0.85],
            FocusScope::Modal => [1.0, 0.82, 0.25, 0.95],
            FocusScope::Overlay => [0.85, 0.45, 1.0, 0.95],
        };
        for b in [
            [x, y, w, border],
            [x, y + h - border, w, border],
            [x, y, border, h],
            [x + w - border, y, border, h],
        ] {
            quads.push(GpuInstance {
                rect: b,
                color: edge,
                user: 0,
            });
        }
        if !node.label.is_empty() {
            let fs = (10.0 * scale).max(8.0);
            labels.push(TextLabel {
                rect: [x + 2.0, y + 2.0, (w - 4.0).max(20.0), fs + 4.0],
                text: node.label.clone(),
                color: color::PARCHMENT,
                font_px: Some(fs),
                align: TextAlign::Left,
                ..Default::default()
            });
        }
    }

    if let Some(i) = snap.current {
        if let Some(node) = snap.nodes.get(i) {
            let pad = (3.0 * scale).max(2.0);
            let r = node.rect;
            let ring = color::RELIC_GOLD;
            let bt = (3.0 * scale).max(2.0);
            for b in [
                [r[0] - pad, r[1] - pad, r[2] + pad * 2.0, bt],
                [r[0] - pad, r[1] + r[3] + pad - bt, r[2] + pad * 2.0, bt],
                [r[0] - pad, r[1] - pad, bt, r[3] + pad * 2.0],
                [r[0] + r[2] + pad - bt, r[1] - pad, bt, r[3] + pad * 2.0],
            ] {
                quads.push(GpuInstance {
                    rect: b,
                    color: ring,
                    user: 0,
                });
            }
        }
    }

    let mem_line = (1.0 * scale).max(1.0);
    if let Some(x) = snap.desired_x {
        quads.push(GpuInstance {
            rect: [x - mem_line * 0.5, 0.0, mem_line, window_h],
            color: [1.0, 0.35, 0.35, 0.65],
            user: 0,
        });
    }
    if let Some(y) = snap.desired_y {
        quads.push(GpuInstance {
            rect: [0.0, y - mem_line * 0.5, window_w, mem_line],
            color: [0.35, 1.0, 0.45, 0.65],
            user: 0,
        });
    }

    let arrow = (10.0 * scale).max(6.0);
    for &(from, dir, to) in &snap.edges {
        let Some(a) = snap.nodes.get(from) else {
            continue;
        };
        let Some(b) = snap.nodes.get(to) else {
            continue;
        };
        let (ax, ay) = rect_center(a.rect);
        let (bx, by) = rect_center(b.rect);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let ux = dx / len;
        let uy = dy / len;
        let mx = ax + ux * len * 0.45;
        let my = ay + uy * len * 0.45;
        quads.push(GpuInstance {
            rect: [mx - arrow * 0.5, my - arrow * 0.5, arrow, arrow],
            color: [1.0, 0.95, 0.4, 0.9],
            user: 0,
        });
        let _ = dir;
    }
}
