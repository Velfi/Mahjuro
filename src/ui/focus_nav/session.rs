//! Scene-owned focus navigation session (`begin_frame` → `add` → `end_frame`).

use super::debug::FocusNavDebugSnapshot;
use super::graph::FocusNav;
use super::scope::FocusScope;
use super::{FocusDir, FocusMemory};

/// Persistent per-scene focus graph + axis memory.
///
/// Typical frame:
/// ```ignore
/// focus_nav.begin_frame();
/// focus_nav.add(target, rect);
/// focus_nav.end_frame();
/// let next = focus_nav.pick(current, FocusDir::Right);
/// ```
pub struct FocusNavState<T: Copy + PartialEq> {
    nav: FocusNav<T>,
    memory: FocusMemory,
}

impl<T: Copy + PartialEq> Default for FocusNavState<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + PartialEq> FocusNavState<T> {
    pub fn new() -> Self {
        Self {
            nav: FocusNav::new(),
            memory: FocusMemory::default(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.nav.begin_frame();
    }

    /// Precompute inferred rows/groups for this frame's registered rects.
    pub fn end_frame(&mut self) {
        self.nav.end_frame();
    }

    pub fn set_scope(&mut self, scope: Option<FocusScope>) {
        self.nav.set_scope_filter(scope);
    }

    pub fn add(&mut self, target: T, rect: [f32; 4]) {
        self.nav.add(target, rect);
    }

    pub fn add_scoped(&mut self, target: T, rect: [f32; 4], scope: FocusScope) {
        self.nav.add_scoped(target, rect, scope);
    }

    pub fn edge(&mut self, from: T, dir: FocusDir, to: T) {
        self.nav.edge(from, dir, to);
    }

    pub fn clear_edges(&mut self) {
        self.nav.clear_edges();
    }

    /// Replace the graph for this frame from explicit rects and optional edges.
    pub fn load_candidates(&mut self, candidates: &[(T, [f32; 4])], edges: &[(T, FocusDir, T)]) {
        self.begin_frame();
        self.clear_edges();
        for &(target, rect) in candidates {
            self.add(target, rect);
        }
        for &(from, dir, to) in edges {
            self.edge(from, dir, to);
        }
        self.end_frame();
    }

    pub fn pick(&mut self, current: T, dir: FocusDir) -> Option<T> {
        self.nav.pick(current, dir, &mut self.memory)
    }

    /// Pick from an off-graph screen rect (artifact cell → chrome bar, etc.).
    pub fn pick_from_rect(&mut self, from_rect: [f32; 4], dir: FocusDir) -> Option<T> {
        super::graph::pick_external(&mut self.nav, from_rect, dir, &mut self.memory)
    }

    pub fn memory(&self) -> &FocusMemory {
        &self.memory
    }

    pub fn debug_snapshot(
        &self,
        current: Option<T>,
        label: impl Fn(T) -> String,
    ) -> FocusNavDebugSnapshot {
        let mut snap = self.nav.debug_snapshot(current, label);
        snap.desired_x = self.memory.desired_x;
        snap.desired_y = self.memory.desired_y;
        snap
    }
}
