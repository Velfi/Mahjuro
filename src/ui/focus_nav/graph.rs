//! Auto-inferred focus navigation graph.
//!
//! Scenes register `(target, rect)` pairs each frame; this module clusters them
//! into rows/columns, resolves directional moves along inferred structure first,
//! and falls back to tiered beam search. Explicit edges are optional overrides.

use std::collections::HashMap;

use super::scope::FocusScope;
use super::{FocusDir, rect_center};

const OVERLAP_RATIO_MIN: f32 = 0.35;
const MIN_FOCUSABLE_SIZE: f32 = 4.0;
/// Primary-axis distance must be at least this multiple of perpendicular offset
/// for a non-aligned beam candidate (≈45° cone when 1.0).
const CARDINAL_CONE_RATIO: f32 = 1.0;

/// Axis memory preserved across moves so uneven grids feel predictable.
#[derive(Clone, Copy, Debug, Default)]
pub struct FocusMemory {
    pub(crate) desired_x: Option<f32>,
    pub(crate) desired_y: Option<f32>,
}

impl FocusMemory {
    pub fn record_move(&mut self, from: [f32; 4], dir: FocusDir) {
        let (cx, cy) = rect_center(from);
        match dir {
            FocusDir::Up | FocusDir::Down => self.desired_x = Some(cx),
            FocusDir::Left | FocusDir::Right => {
                self.desired_y = Some(cy);
                // Keep the vertical column aligned with wherever L/R landed
                // (e.g. Play on the right → Up returns to tiles 10/11, not a
                // stale column from an earlier hand tile).
                self.desired_x = Some(cx);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Node<T> {
    target: T,
    rect: [f32; 4],
    enabled: bool,
    focusable: bool,
    scope: FocusScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NavScore {
    tier: u8,
    group_penalty: u32,
    /// Perpendicular offset from the intended axis — lower is more cardinal.
    secondary: u32,
    /// Distance along the intended axis.
    forward: u32,
    manhattan: u32,
}

fn f32_to_sort_key(v: f32) -> u32 {
    v.to_bits()
}

/// Per-frame focus graph with optional explicit edges.
pub struct FocusNav<T: Copy + PartialEq> {
    nodes: Vec<Node<T>>,
    edges: Vec<(T, FocusDir, T)>,
    scope_filter: Option<FocusScope>,
    layout: Option<InferredLayout>,
}

struct InferredLayout {
    active: Vec<usize>,
    rows: Vec<Vec<usize>>,
    groups: Vec<u32>,
    median_h: f32,
}

impl<T: Copy + PartialEq> Default for FocusNav<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + PartialEq> FocusNav<T> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            scope_filter: None,
            layout: None,
        }
    }

    pub fn begin_frame(&mut self) {
        self.nodes.clear();
        self.layout = None;
    }

    pub fn end_frame(&mut self) {
        self.ensure_layout();
    }

    pub fn set_scope_filter(&mut self, scope: Option<FocusScope>) {
        if self.scope_filter != scope {
            self.scope_filter = scope;
            self.layout = None;
        }
    }

    pub fn clear_edges(&mut self) {
        self.edges.clear();
    }

    pub fn add(&mut self, target: T, rect: [f32; 4]) {
        self.add_scoped(target, rect, FocusScope::Scene);
    }

    pub fn add_scoped(&mut self, target: T, rect: [f32; 4], scope: FocusScope) {
        self.nodes.push(Node {
            target,
            rect,
            enabled: true,
            focusable: true,
            scope,
        });
        self.layout = None;
    }

    pub fn edge(&mut self, from: T, dir: FocusDir, to: T) {
        if let Some(slot) = self
            .edges
            .iter_mut()
            .find(|(f, d, _)| *f == from && *d == dir)
        {
            slot.2 = to;
        } else {
            self.edges.push((from, dir, to));
        }
    }

    pub fn pick(&mut self, current: T, dir: FocusDir, memory: &mut FocusMemory) -> Option<T> {
        self.ensure_layout();
        pick_neighbor_default(self, current, dir, memory)
    }

    pub fn debug_snapshot(
        &self,
        current: Option<T>,
        label: impl Fn(T) -> String,
    ) -> super::debug::FocusNavDebugSnapshot {
        let layout = match self.layout.as_ref() {
            Some(l) => l,
            None => {
                return super::debug::FocusNavDebugSnapshot {
                    scope_filter: self.scope_filter,
                    ..Default::default()
                };
            }
        };

        let mut nodes = Vec::with_capacity(layout.active.len());
        let mut index_map = vec![None; self.nodes.len()];
        for (out_i, &node_i) in layout.active.iter().enumerate() {
            index_map[node_i] = Some(out_i);
            let n = &self.nodes[node_i];
            nodes.push(super::debug::FocusNavDebugNode {
                rect: n.rect,
                scope: n.scope,
                label: label(n.target),
            });
        }

        let remap = |active_idx: usize| -> usize {
            let node_i = layout.active[active_idx];
            index_map[node_i].unwrap_or(active_idx)
        };

        let rows = layout
            .rows
            .iter()
            .map(|row| row.iter().map(|&i| remap(i)).collect())
            .collect();
        let groups = layout.groups.clone();

        let mut edges = Vec::new();
        for &(from, dir, to) in &self.edges {
            let Some(from_i) = self.nodes.iter().position(|n| n.target == from) else {
                continue;
            };
            let Some(to_i) = self.nodes.iter().position(|n| n.target == to) else {
                continue;
            };
            if let (Some(f), Some(t)) = (index_map[from_i], index_map[to_i]) {
                edges.push((f, dir, t));
            }
        }

        let current = current.and_then(|t| {
            self.nodes
                .iter()
                .position(|n| n.target == t)
                .and_then(|i| index_map[i])
        });

        super::debug::FocusNavDebugSnapshot {
            nodes,
            rows,
            groups,
            edges,
            current,
            desired_x: None,
            desired_y: None,
            scope_filter: self.scope_filter,
        }
    }

    fn ensure_layout(&mut self) {
        if self.layout.is_none() {
            self.layout = Some(infer_layout(&self.nodes, self.scope_filter));
        }
    }

    fn node_rect(&self, target: T) -> Option<[f32; 4]> {
        self.nodes
            .iter()
            .find(|n| n.target == target)
            .map(|n| n.rect)
    }

    fn explicit_edge(&self, from: T, dir: FocusDir) -> Option<T> {
        self.edges
            .iter()
            .find(|(f, d, _)| *f == from && *d == dir)
            .map(|(_, _, to)| *to)
    }

    fn active_index(&self, target: T) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        layout
            .active
            .iter()
            .position(|&node_i| self.nodes[node_i].target == target)
    }

    fn layout(&self) -> Option<&InferredLayout> {
        self.layout.as_ref()
    }

    fn target_at(&self, node_index: usize) -> T {
        self.nodes[node_index].target
    }

    fn rect_at_node(&self, node_index: usize) -> [f32; 4] {
        self.nodes[node_index].rect
    }
}

fn record_pick_arrival<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    to: T,
    dir: FocusDir,
    memory: &mut FocusMemory,
) {
    if let Some(rect) = nav.node_rect(to) {
        memory.record_move(rect, dir);
    }
}

fn pick_neighbor_default<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    current: T,
    dir: FocusDir,
    memory: &mut FocusMemory,
) -> Option<T> {
    let layout = nav.layout()?;
    let current_idx = nav.active_index(current)?;

    if let Some(to) = nav.explicit_edge(current, dir) {
        if nav.active_index(to).is_some() {
            record_pick_arrival(nav, to, dir, memory);
            return Some(to);
        }
    }

    if let Some(to) = pick_inferred_line_neighbor(nav, layout, current_idx, dir, memory) {
        record_pick_arrival(nav, to, dir, memory);
        return Some(to);
    }

    if let Some(to) = pick_beam_neighbor(nav, layout, current_idx, dir, memory) {
        record_pick_arrival(nav, to, dir, memory);
        return Some(to);
    }

    if let Some(to) = pick_loose_neighbor(nav, layout, current_idx, dir, memory) {
        record_pick_arrival(nav, to, dir, memory);
        return Some(to);
    }

    None
}

/// Directional pick when the source rect is not registered as a node (e.g. walking
/// off the artifact grid onto chrome buttons).
pub fn pick_external<T: Copy + PartialEq>(
    nav: &mut FocusNav<T>,
    from_rect: [f32; 4],
    dir: FocusDir,
    memory: &mut FocusMemory,
) -> Option<T> {
    nav.ensure_layout();
    pick_external_rect(nav, from_rect, dir, memory)
}

fn pick_external_rect<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    current_rect: [f32; 4],
    dir: FocusDir,
    memory: &mut FocusMemory,
) -> Option<T> {
    let layout = nav.layout()?;
    if let Some(to) = pick_beam_from_rect(nav, layout, current_rect, dir, memory, None) {
        record_pick_arrival(nav, to, dir, memory);
        return Some(to);
    }
    if let Some(to) = pick_loose_from_rect(nav, layout, current_rect, dir, memory, None) {
        record_pick_arrival(nav, to, dir, memory);
        return Some(to);
    }
    None
}

fn infer_layout<T: Copy + PartialEq>(
    nodes: &[Node<T>],
    scope_filter: Option<FocusScope>,
) -> InferredLayout {
    let mut active = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if !node.enabled || !node.focusable {
            continue;
        }
        if scope_filter.is_some_and(|s| node.scope != s) {
            continue;
        }
        if node.rect[2] < MIN_FOCUSABLE_SIZE || node.rect[3] < MIN_FOCUSABLE_SIZE {
            continue;
        }
        active.push(i);
    }

    let heights: Vec<f32> = active.iter().map(|&i| nodes[i].rect[3]).collect();
    let widths: Vec<f32> = active.iter().map(|&i| nodes[i].rect[2]).collect();
    let median_h = median_copy(&heights).max(MIN_FOCUSABLE_SIZE);
    let median_w = median_copy(&widths).max(MIN_FOCUSABLE_SIZE);
    let row_snap_px = (median_h * 0.6).max(24.0);

    let rect_at = |ai: usize| nodes[active[ai]].rect;
    let rows = cluster_rows(&active, &rect_at, row_snap_px);
    let groups = infer_groups(&rows, &active, rect_at, median_w, median_h);

    InferredLayout {
        active,
        rows,
        groups,
        median_h,
    }
}

fn median_copy(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 48.0;
    }
    let mut v: Vec<f32> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn vertical_overlap_ratio(a: [f32; 4], b: [f32; 4]) -> f32 {
    axis_overlap_ratio(a, b, 1)
}

fn horizontal_overlap_ratio(a: [f32; 4], b: [f32; 4]) -> f32 {
    axis_overlap_ratio(a, b, 0)
}

fn axis_overlap_ratio(a: [f32; 4], b: [f32; 4], axis: u8) -> f32 {
    let (a0, a1, alen) = if axis == 0 {
        (a[0], a[0] + a[2], a[2])
    } else {
        (a[1], a[1] + a[3], a[3])
    };
    let (b0, b1, blen) = if axis == 0 {
        (b[0], b[0] + b[2], b[2])
    } else {
        (b[1], b[1] + b[3], b[3])
    };
    let top = a0.max(b0);
    let bot = a1.min(b1);
    let overlap = (bot - top).max(0.0);
    overlap / alen.min(blen).max(1.0)
}

fn rect_bottom(r: [f32; 4]) -> f32 {
    r[1] + r[3]
}

fn same_row(a: [f32; 4], b: [f32; 4], row_snap_px: f32) -> bool {
    // Center-y and bottom-y: mixed-height footer strips (tall discard panel +
    // short tally sticks) share a baseline without merging unrelated rows.
    (rect_center(a).1 - rect_center(b).1).abs() <= row_snap_px
        || (rect_bottom(a) - rect_bottom(b)).abs() <= row_snap_px
}

fn cluster_rows(
    active: &[usize],
    rect_at: &impl Fn(usize) -> [f32; 4],
    row_snap_px: f32,
) -> Vec<Vec<usize>> {
    cluster_by(
        active,
        &rect_at,
        |a, b| same_row(a, b, row_snap_px),
        |a, b| rect_center(a).0.partial_cmp(&rect_center(b).0).unwrap(),
        |a, b| rect_center(a).1.partial_cmp(&rect_center(b).1).unwrap(),
    )
}

fn cluster_by(
    active: &[usize],
    rect_at: &impl Fn(usize) -> [f32; 4],
    same_group: impl Fn([f32; 4], [f32; 4]) -> bool,
    mut within_sort: impl FnMut([f32; 4], [f32; 4]) -> std::cmp::Ordering,
    mut group_sort: impl FnMut([f32; 4], [f32; 4]) -> std::cmp::Ordering,
) -> Vec<Vec<usize>> {
    let n = active.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in i + 1..n {
            if same_group(rect_at(i), rect_at(j)) {
                union_find(&mut parent, i, j);
            }
        }
    }
    groups_from_parent(active, &parent, &mut within_sort, rect_at, &mut group_sort)
}

fn groups_from_parent(
    active: &[usize],
    parent: &[usize],
    within_sort: &mut impl FnMut([f32; 4], [f32; 4]) -> std::cmp::Ordering,
    rect_at: &impl Fn(usize) -> [f32; 4],
    group_sort: &mut impl FnMut([f32; 4], [f32; 4]) -> std::cmp::Ordering,
) -> Vec<Vec<usize>> {
    let n = active.len();
    let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        buckets.entry(find_root(parent, i)).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = buckets.into_values().collect();
    for group in &mut groups {
        group.sort_by(|&a, &b| within_sort(rect_at(a), rect_at(b)));
    }
    groups.sort_by(|ga, gb| {
        let ra = rect_at(ga[0]);
        let rb = rect_at(gb[0]);
        group_sort(ra, rb)
    });
    groups
}

fn union_find(parent: &mut [usize], a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra != rb {
        parent[rb] = ra;
    }
}

fn find_root(parent: &[usize], mut x: usize) -> usize {
    while parent[x] != x {
        x = parent[x];
    }
    x
}

fn infer_groups(
    rows: &[Vec<usize>],
    active: &[usize],
    rect_at: impl Fn(usize) -> [f32; 4],
    median_w: f32,
    median_h: f32,
) -> Vec<u32> {
    let group_gap_x = median_w * 2.5;
    let group_gap_y = median_h * 2.0;
    let mut group_ids = vec![0u32; active.len()];
    if rows.is_empty() {
        return group_ids;
    }

    let mut next_group = 0u32;
    for row in rows {
        if row.is_empty() {
            continue;
        }
        let row_group = next_group;
        next_group += 1;
        for &idx in row {
            group_ids[idx] = row_group;
        }
    }

    for w in 1..rows.len() {
        let prev = rows[w - 1].first().copied().map(|i| rect_at(i));
        let cur = rows[w].first().copied().map(|i| rect_at(i));
        if let (Some(rp), Some(rc)) = (prev, cur) {
            let gap = rc[1] - (rp[1] + rp[3]);
            if gap <= group_gap_y {
                let merge_from = group_ids[rows[w][0]];
                let merge_to = group_ids[rows[w - 1][0]];
                for idx in &rows[w] {
                    if group_ids[*idx] == merge_from {
                        group_ids[*idx] = merge_to;
                    }
                }
            }
        }
    }

    for row in rows {
        for w in 1..row.len() {
            let left = rect_at(row[w - 1]);
            let right = rect_at(row[w]);
            let gap = right[0] - (left[0] + left[2]);
            if gap > group_gap_x && group_ids[row[w]] == group_ids[row[w - 1]] {
                let old = group_ids[row[w - 1]];
                let new_group = next_group;
                next_group += 1;
                for idx in row[w..].iter() {
                    if group_ids[*idx] == old {
                        group_ids[*idx] = new_group;
                    }
                }
            }
        }
    }

    group_ids
}

fn line_step(pos: usize, len: usize, backward: bool) -> Option<usize> {
    if backward {
        if pos == 0 { None } else { Some(pos - 1) }
    } else if pos + 1 >= len {
        None
    } else {
        Some(pos + 1)
    }
}

/// Contiguous left-to-right run within a row that shares one horizontal group
/// (no large x-gap). Line stepping stays inside this run; beam search handles hops.
fn horizontal_run_in_row<'a>(line: &'a [usize], pos: usize, groups: &[u32]) -> &'a [usize] {
    let group = groups[line[pos]];
    let mut start = pos;
    while start > 0 && groups[line[start - 1]] == group {
        start -= 1;
    }
    let mut end = pos;
    while end + 1 < line.len() && groups[line[end + 1]] == group {
        end += 1;
    }
    &line[start..=end]
}

fn pick_inferred_line_neighbor<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_idx: usize,
    dir: FocusDir,
    memory: &FocusMemory,
) -> Option<T> {
    match dir {
        FocusDir::Left | FocusDir::Right => {
            let current_rect = nav.rect_at_node(layout.active[current_idx]);
            let row_snap = (layout.median_h * 0.6).max(24.0);
            let line: Vec<usize> = layout
                .rows
                .iter()
                .find(|row| row.contains(&current_idx))?
                .iter()
                .copied()
                .filter(|&i| same_row(current_rect, nav.rect_at_node(layout.active[i]), row_snap))
                .collect();
            if line.len() < 2 {
                return None;
            }
            let pos = line.iter().position(|&i| i == current_idx)?;
            let run = horizontal_run_in_row(&line, pos, &layout.groups);
            let run_pos = run.iter().position(|&i| i == current_idx)?;
            let next_pos = line_step(run_pos, run.len(), dir == FocusDir::Left)?;
            let node_i = layout.active[run[next_pos]];
            Some(nav.target_at(node_i))
        }
        FocusDir::Up | FocusDir::Down => {
            pick_inferred_grid_vertical(nav, layout, current_idx, dir, memory)
        }
    }
}

fn pick_inferred_grid_vertical<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_idx: usize,
    dir: FocusDir,
    memory: &FocusMemory,
) -> Option<T> {
    let row_idx = layout
        .rows
        .iter()
        .position(|row| row.contains(&current_idx))?;
    let col_idx = layout.rows[row_idx]
        .iter()
        .position(|&i| i == current_idx)?;
    let current_x = rect_center(nav.rect_at_node(layout.active[current_idx])).0;
    let anchor_x = memory.desired_x.unwrap_or(current_x);
    let target_row_idx = match dir {
        FocusDir::Up => {
            if row_idx == 0 {
                return None;
            }
            row_idx - 1
        }
        FocusDir::Down => {
            if row_idx + 1 >= layout.rows.len() {
                return None;
            }
            row_idx + 1
        }
        _ => unreachable!(),
    };
    pick_from_row(nav, layout, &layout.rows[target_row_idx], col_idx, anchor_x)
}

fn pick_from_row<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    row: &[usize],
    _col_idx: usize,
    anchor_x: f32,
) -> Option<T> {
    if row.is_empty() {
        return None;
    }
    if row.len() == 1 {
        return Some(nav.target_at(layout.active[row[0]]));
    }
    // Rows have different item counts (14 hand tiles vs 7 action-bar widgets).
    // Always pick the x-nearest peer — never reuse the source row's column index.
    let mut best: Option<(usize, f32)> = None;
    for &active_idx in row {
        let node_i = layout.active[active_idx];
        let dx = (rect_center(nav.rect_at_node(node_i)).0 - anchor_x).abs();
        let is_better = match best {
            None => true,
            Some((_, bd)) => dx < bd,
        };
        if is_better {
            best = Some((active_idx, dx));
        }
    }
    Some(nav.target_at(layout.active[best?.0]))
}

fn pick_beam_neighbor<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_idx: usize,
    dir: FocusDir,
    memory: &FocusMemory,
) -> Option<T> {
    pick_beam_from_rect(
        nav,
        layout,
        nav.rect_at_node(layout.active[current_idx]),
        dir,
        memory,
        Some(layout.groups[current_idx]),
    )
}

fn pick_beam_from_rect<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_rect: [f32; 4],
    dir: FocusDir,
    memory: &FocusMemory,
    current_group: Option<u32>,
) -> Option<T> {
    let current_group = current_group.unwrap_or(u32::MAX);
    let (ccx, ccy) = rect_center(current_rect);
    let band = layout.median_h * 0.55;

    let mut best: Option<(T, NavScore)> = None;
    for (i, &storage_i) in layout.active.iter().enumerate() {
        let rect = nav.rect_at_node(storage_i);
        let target = nav.target_at(storage_i);
        let (tcx, tcy) = rect_center(rect);
        let dx = tcx - ccx;
        let dy = tcy - ccy;
        let (forward, secondary, in_dir, axis_overlap) = match dir {
            FocusDir::Right => (
                dx,
                dy.abs(),
                dx > 0.0,
                vertical_overlap_ratio(current_rect, rect),
            ),
            FocusDir::Left => (
                -dx,
                dy.abs(),
                dx < 0.0,
                vertical_overlap_ratio(current_rect, rect),
            ),
            FocusDir::Down => (
                dy,
                dx.abs(),
                dy > 0.0,
                horizontal_overlap_ratio(current_rect, rect),
            ),
            FocusDir::Up => (
                -dy,
                dx.abs(),
                dy < 0.0,
                horizontal_overlap_ratio(current_rect, rect),
            ),
        };
        if !in_dir || forward <= 0.0 {
            continue;
        }

        let memory_secondary = match dir {
            FocusDir::Up | FocusDir::Down => memory
                .desired_x
                .map(|x| (tcx - x).abs())
                .unwrap_or(secondary),
            FocusDir::Left | FocusDir::Right => memory
                .desired_y
                .map(|y| (tcy - y).abs())
                .unwrap_or(secondary),
        };

        let tier = if axis_overlap >= OVERLAP_RATIO_MIN {
            2u8
        } else if memory_secondary <= band {
            3
        } else if !matches!(dir, FocusDir::Left | FocusDir::Right)
            && forward >= memory_secondary * CARDINAL_CONE_RATIO
        {
            5
        } else {
            continue;
        };

        let group_penalty =
            u32::from(current_group != u32::MAX && layout.groups[i] != current_group);

        let score = NavScore {
            tier,
            group_penalty,
            secondary: f32_to_sort_key(memory_secondary),
            forward: f32_to_sort_key(forward),
            manhattan: f32_to_sort_key(dx.abs() + dy.abs()),
        };

        let is_better = match best {
            None => true,
            Some((_, bs)) => score < bs,
        };
        if is_better {
            best = Some((target, score));
        }
    }
    best.map(|(t, _)| t)
}

fn pick_loose_neighbor<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_idx: usize,
    dir: FocusDir,
    memory: &FocusMemory,
) -> Option<T> {
    pick_loose_from_rect(
        nav,
        layout,
        nav.rect_at_node(layout.active[current_idx]),
        dir,
        memory,
        Some(layout.groups[current_idx]),
    )
}

fn pick_loose_from_rect<T: Copy + PartialEq>(
    nav: &FocusNav<T>,
    layout: &InferredLayout,
    current_rect: [f32; 4],
    dir: FocusDir,
    memory: &FocusMemory,
    current_group: Option<u32>,
) -> Option<T> {
    let current_group = current_group.unwrap_or(u32::MAX);
    let (ccx, ccy) = rect_center(current_rect);

    let mut best: Option<(T, NavScore)> = None;
    for (i, &storage_i) in layout.active.iter().enumerate() {
        let rect = nav.rect_at_node(storage_i);
        let target = nav.target_at(storage_i);
        let (tcx, tcy) = rect_center(rect);
        let dx = tcx - ccx;
        let dy = tcy - ccy;
        let (forward, secondary, in_dir) = match dir {
            FocusDir::Right => (dx, dy.abs(), dx > 0.0),
            FocusDir::Left => (-dx, dy.abs(), dx < 0.0),
            FocusDir::Down => (dy, dx.abs(), dy > 0.0),
            FocusDir::Up => (-dy, dx.abs(), dy < 0.0),
        };
        if !in_dir || forward <= 0.0 {
            continue;
        }
        let band = layout.median_h * 0.55;
        if matches!(dir, FocusDir::Left | FocusDir::Right) && secondary > band {
            continue;
        }
        if forward < secondary * CARDINAL_CONE_RATIO {
            continue;
        }

        let memory_secondary = match dir {
            FocusDir::Up | FocusDir::Down => memory
                .desired_x
                .map(|x| (tcx - x).abs())
                .unwrap_or(secondary),
            FocusDir::Left | FocusDir::Right => memory
                .desired_y
                .map(|y| (tcy - y).abs())
                .unwrap_or(secondary),
        };

        let score = NavScore {
            tier: 7,
            group_penalty: u32::from(
                current_group != u32::MAX && layout.groups[i] != current_group,
            ),
            secondary: f32_to_sort_key(memory_secondary),
            forward: f32_to_sort_key(forward),
            manhattan: f32_to_sort_key(dx.abs() + dy.abs()),
        };

        let is_better = match best {
            None => true,
            Some((_, bs)) => score < bs,
        };
        if is_better {
            best = Some((target, score));
        }
    }
    best.map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::super::session::FocusNavState;
    use super::*;

    fn row(y: f32, xs: &[(u32, f32)]) -> Vec<(u32, [f32; 4])> {
        xs.iter()
            .map(|(id, x)| (*id, [*x, y, 40.0, 40.0]))
            .collect()
    }

    fn load_nav(
        candidates: &[(u32, [f32; 4])],
        edges: &[(u32, FocusDir, u32)],
    ) -> FocusNavState<u32> {
        let mut nav = FocusNavState::new();
        let edge_triples: Vec<(u32, FocusDir, u32)> = edges.iter().copied().collect();
        nav.load_candidates(candidates, &edge_triples);
        nav
    }

    #[test]
    fn cardinal_right_prefers_same_row_over_closer_diagonal() {
        let items = vec![
            (1u32, [0.0, 100.0, 40.0, 40.0]),
            (2, [30.0, 180.0, 40.0, 40.0]), // closer but mostly down
            (3, [80.0, 105.0, 40.0, 40.0]), // farther but same row
        ];
        let mut nav = load_nav(&items, &[]);
        assert_eq!(
            nav.pick(1, FocusDir::Right),
            Some(3),
            "Right should stay on-row, not jump to diagonal neighbor"
        );
    }

    #[test]
    fn cardinal_up_prefers_same_column_on_row_above() {
        let items = vec![
            (1u32, [100.0, 200.0, 40.0, 40.0]),
            (2, [180.0, 100.0, 40.0, 40.0]), // row above, off-column
            (3, [105.0, 100.0, 40.0, 40.0]), // row above, aligned
        ];
        let mut nav = load_nav(&items, &[]);
        assert_eq!(
            nav.pick(1, FocusDir::Up),
            Some(3),
            "Up should pick the in-column target on the next row"
        );
    }

    #[test]
    fn gameplay_action_bar_right_chain_inferred() {
        let mut items: Vec<(u32, [f32; 4])> = vec![
            (100, [100.0, 900.0, 120.0, 80.0]), // Discard
            (101, [280.0, 880.0, 40.0, 100.0]), // discard tally
            (102, [400.0, 870.0, 80.0, 90.0]),  // Guide
            (103, [520.0, 870.0, 80.0, 90.0]),  // Journal
            (104, [720.0, 880.0, 40.0, 100.0]), // play tally
            (105, [860.0, 900.0, 120.0, 80.0]), // Play
        ];
        items.push((0, [400.0, 520.0, 58.0, 90.0])); // hand tile above
        let mut nav = load_nav(&items, &[]);
        let chain = [100, 101, 102, 103, 104, 105];
        for w in chain.windows(2) {
            assert_eq!(
                nav.pick(w[0], FocusDir::Right),
                Some(w[1]),
                "Right from {} should reach {}",
                w[0],
                w[1]
            );
            assert_eq!(nav.pick(w[1], FocusDir::Left), Some(w[0]));
        }
    }

    #[test]
    fn gameplay_discard_right_reaches_play_not_hand() {
        // Tall projected hand AABBs overlap the button row in Y; centers stay apart.
        let hand_y = 520.0;
        let hand_h = 220.0;
        let mut items: Vec<(u32, [f32; 4])> = (0..14u32)
            .map(|i| (i, [380.0 + i as f32 * 62.0, hand_y, 58.0, hand_h]))
            .collect();
        items.push((100, [180.0, 900.0, 140.0, 90.0])); // Discard
        items.push((101, [920.0, 910.0, 130.0, 70.0])); // Play
        let mut nav = load_nav(&items, &[]);
        assert_eq!(
            nav.pick(100, FocusDir::Right),
            Some(101),
            "Right from Discard should reach Play, not a hand tile above"
        );
    }

    #[test]
    fn hand_row_prefers_next_tile_over_button_below() {
        let mut items: Vec<(u32, [f32; 4])> =
            row(100.0, &[(1, 0.0), (2, 50.0), (3, 100.0), (4, 150.0)]);
        items.push((10, [75.0, 200.0, 80.0, 40.0]));
        let mut nav = load_nav(&items, &[]);
        assert_eq!(nav.pick(2, FocusDir::Right), Some(3));
    }

    #[test]
    fn inferred_row_left_right() {
        let items = row(0.0, &[(1, 0.0), (2, 50.0), (3, 100.0)]);
        let mut nav = load_nav(&items, &[]);
        assert_eq!(nav.pick(1, FocusDir::Right), Some(2));
        assert_eq!(nav.pick(3, FocusDir::Left), Some(2));
        assert_eq!(nav.pick(1, FocusDir::Left), None);
    }

    #[test]
    fn guide_header_no_wrap_left_of_back() {
        let items = vec![
            (1u32, [48.0, 48.0, 108.0, 52.0]),
            (2, [1500.0, 48.0, 58.0, 52.0]),
            (3, [1600.0, 48.0, 58.0, 52.0]),
        ];
        let mut nav = load_nav(&items, &[]);
        assert_eq!(nav.pick(1, FocusDir::Left), None);
        assert_eq!(nav.pick(1, FocusDir::Right), Some(2));
    }

    #[test]
    fn explicit_edge_overrides_inference() {
        let items = row(0.0, &[(1, 0.0), (2, 50.0), (3, 100.0)]);
        let mut nav = load_nav(&items, &[(1, FocusDir::Right, 3)]);
        assert_eq!(nav.pick(1, FocusDir::Right), Some(3));
    }

    #[test]
    fn shop_like_two_row_grid() {
        let mut items = row(0.0, &[(1, 0.0), (2, 50.0), (3, 100.0)]);
        items.extend(row(60.0, &[(4, 0.0), (5, 50.0), (6, 100.0)]));
        let mut nav = load_nav(&items, &[]);
        assert_eq!(nav.pick(2, FocusDir::Down), Some(5));
        assert_eq!(nav.pick(5, FocusDir::Up), Some(2));
    }

    #[test]
    fn focus_memory_preserves_column_on_uneven_grid() {
        let items = vec![
            (1u32, [0.0, 0.0, 40.0, 40.0]),
            (2, [50.0, 0.0, 40.0, 40.0]),
            (3, [100.0, 0.0, 40.0, 40.0]),
            (4, [0.0, 60.0, 140.0, 40.0]),
            (5, [0.0, 120.0, 40.0, 40.0]),
            (6, [50.0, 120.0, 40.0, 40.0]),
            (7, [100.0, 120.0, 40.0, 40.0]),
        ];
        let mut nav = load_nav(&items, &[]);
        assert_eq!(nav.pick(2, FocusDir::Down), Some(4));
        assert_eq!(nav.pick(4, FocusDir::Down), Some(6));
    }

    #[test]
    fn scope_filter_excludes_other_surfaces() {
        use super::super::scope::FocusScope;

        let mut nav = FocusNav::new();
        nav.begin_frame();
        nav.add_scoped(1u32, [0.0, 0.0, 40.0, 40.0], FocusScope::Scene);
        nav.add_scoped(2, [50.0, 0.0, 40.0, 40.0], FocusScope::Modal);
        nav.set_scope_filter(Some(FocusScope::Modal));
        nav.end_frame();
        let mut memory = FocusMemory::default();
        assert_eq!(nav.pick(2, FocusDir::Left, &mut memory), None);
        nav.set_scope_filter(Some(FocusScope::Scene));
        assert_eq!(nav.pick(1, FocusDir::Right, &mut memory), None);
    }

    #[test]
    fn layout_survives_filtered_leading_nodes() {
        // Archive registers chrome + many artifact cells; tiny rects are skipped
        // during layout. Active indices no longer match 0..active.len()-1.
        let mut nav = FocusNav::new();
        nav.begin_frame();
        for i in 0..3u32 {
            nav.add(i, [0.0, 0.0, 1.0, 1.0]); // below MIN_FOCUSABLE_SIZE
        }
        for row in 0..4 {
            for col in 0..6 {
                let id = 100 + row * 6 + col;
                nav.add(
                    id,
                    [
                        80.0 + col as f32 * 90.0,
                        120.0 + row as f32 * 90.0,
                        80.0,
                        80.0,
                    ],
                );
            }
        }
        nav.end_frame();
        let mut memory = FocusMemory::default();
        assert_eq!(nav.pick(100, FocusDir::Right, &mut memory), Some(101));
    }
}
