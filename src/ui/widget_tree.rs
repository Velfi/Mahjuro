//! Generic, immediate-mode widget tree for scene UIs.
//!
//! Each frame a scene builds a [`Tree`] from its current state, hands it to a
//! tiny persistent [`TreeState`], and gets back a single `Option<A>` describing
//! what (if anything) the user activated this frame. The tree owns layout,
//! hit-testing, hover-follow, keyboard navigation, and rendering — so scenes never duplicate layout math between
//! `update()` and `draw()`, never juggle named const indices, and never lose
//! sync between hover hit-tests and rendered rects.
//!
//! ## Action API
//!
//! `A` is a scene-defined `Copy` enum (e.g. `enum MainAction { Play, Quit, … }`).
//! Each interactive item carries an `A` value the tree returns when the item
//! activates (keyboard/gamepad confirm or mouse click). The scene's `update()`
//! then matches on `Option<A>` exhaustively.
//! No `u32` const indices, no shared `UiAction::Confirm` disambiguation by
//! cursor position.
//!
//! ## Click routing
//!
//! Each item is registered with the main loop via `ButtonDef::scene(rect, id)`
//! where `id` is the item's stable `FocusId`. Mouse clicks come back as
//! `button_clicks` ids in the `UpdateCtx` next frame and are matched against
//! the layout cache. Mouse hover updates `focused` so a keyboard `Confirm`
//! still acts on whatever the cursor is over.
//!
//! ## Per-frame flow
//!
//! ```ignore
//! // In Scene::update():
//! let tree = self.build_tree(&ctx);          // pure function from state
//! let action = self.tree_state.update(&tree, TreeInput { ... });
//! match action { Some(MainAction::Play) => ..., None => () }
//!
//! // In Scene::draw():
//! let tree = self.build_tree_from_draw(&ctx); // same shape
//! self.tree_state.draw(&tree, &mut frame);
//! ```
//!
//! The layout cache built during `update()` is reused by `draw()` so the
//! layout pass runs exactly once per frame.

use crate::render::theme::{ButtonState, ButtonVariant, metrics};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::clip::intersect_rect;
use crate::ui::focus_nav::{
    FocusDir, FocusNavDebugSnapshot, FocusNavState, debug_snapshot_from_candidates,
    push_focus_ring,
};
use crate::ui::input::UiAction;
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::widget;
use std::borrow::Cow;

// ─── Identifiers ────────────────────────────────────────────────────────────

/// Stable per-item identifier. Survives item reordering — the focused row
/// stays focused even if the developer rearranges the build_tree() call. By
/// convention, derived from a scene's `MenuAction` enum discriminant via
/// `MainAction::Play as u32`, but any unique `u32` works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusId(pub u32);

// ─── Tree shape ─────────────────────────────────────────────────────────────

/// A complete UI tree for a scene. Cheap to construct each frame.
pub struct Tree<A: Copy> {
    pub root: Node<A>,
    /// Anchor rect for the root, in window pixels. If `None`, the root fills
    /// the full window.
    pub anchor: Option<[f32; 4]>,
}

pub enum Node<A: Copy> {
    Column {
        gap: f32,
        align: HAlign,
        children: Vec<Node<A>>,
    },
    Row {
        gap: f32,
        align: VAlign,
        children: Vec<Node<A>>,
    },
    Item(Item<A>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Center,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Center,
}

pub struct Item<A: Copy> {
    pub id: FocusId,
    pub enabled: bool,
    pub tooltip: Option<Cow<'static, str>>,
    pub label: String,
    pub variant: ButtonVariant,
    pub on_activate: A,
}

// ─── Tree builders (ergonomic shortcuts) ────────────────────────────────────

impl<A: Copy> Tree<A> {
    pub fn vertical_menu(children: Vec<Node<A>>) -> Self {
        Self {
            root: Node::Column {
                gap: 0.0, // resolved at layout time from typography
                align: HAlign::Center,
                children,
            },
            anchor: None,
        }
    }

    pub fn with_anchor(mut self, rect: [f32; 4]) -> Self {
        self.anchor = Some(rect);
        self
    }
}

pub fn button_id<A: Copy>(id: FocusId, label: &str, action: A, variant: ButtonVariant) -> Node<A> {
    Node::Item(Item {
        id,
        enabled: true,
        tooltip: None,
        label: label.into(),
        variant,
        on_activate: action,
    })
}

// ─── Persistent tree state ──────────────────────────────────────────────────

/// Per-scene state that survives across frames. The only thing the scene must
/// store between `update()` and `draw()`. Cheap to construct.
pub struct TreeState {
    /// Currently focused item id. Resolved against the latest layout cache.
    focused: Option<FocusId>,
    /// Latched true during `update()` / `update_flat()` when user input moves
    /// focus to a different item. Cleared by `take_focus_changed()`.
    focus_changed: bool,
    /// Layout cache built during `update()` and reused by `draw()`.
    layout: Vec<LaidOut>,
    last_window: (f32, f32),
    /// Smooth-scroll state for autoscroll when content overflows anchor.
    scroll: SmoothScroll,
    /// The pixel offset currently applied to laid-out rects (cached for draw).
    scroll_offset_px: f32,
    /// Total content height from the last layout (before scroll).
    content_height: f32,
    /// Anchor height from the last layout.
    anchor_height: f32,
    /// Scroll viewport clip rect in screen pixels.
    scroll_clip_rect: Option<[f32; 4]>,
    /// Auto-inferred spatial navigation for flat items.
    focus_nav: FocusNavState<FocusId>,
}

#[derive(Clone, Copy)]
struct LaidOut {
    id: FocusId,
    rect: [f32; 4],
    enabled: bool,
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeState {
    pub fn new() -> Self {
        Self {
            focused: None,
            focus_changed: false,
            layout: Vec::new(),
            last_window: (0.0, 0.0),
            scroll: SmoothScroll::new(),
            scroll_offset_px: 0.0,
            content_height: 0.0,
            anchor_height: 0.0,
            scroll_clip_rect: None,
            focus_nav: FocusNavState::new(),
        }
    }

    /// Force-set the currently focused item (e.g. when a scene transitions
    /// in and wants the cursor on a specific row).
    pub fn set_focus(&mut self, id: FocusId) {
        self.focused = Some(id);
    }

    /// Return whether focus changed during the most recent update pass, then
    /// clear the latch.
    pub fn take_focus_changed(&mut self) -> bool {
        let changed = self.focus_changed;
        self.focus_changed = false;
        changed
    }

    /// Returns the currently focused item id, if any.
    pub fn focused(&self) -> Option<FocusId> {
        self.focused
    }

    /// Hover/click resolution for a flat list of pre-computed (id, rect, action)
    /// hit targets. Use this for scenes that already know their item rects
    /// (e.g. GLB-projected hand slots, custom card grids, hand-laid tab bars)
    /// and only want focus management + click routing — not the full Node tree.
    ///
    /// Companion: [`TreeState::register_flat_buttons`] pushes the corresponding
    /// `ButtonDef::scene` entries during draw.
    pub fn update_flat<A: Copy>(
        &mut self,
        items: &[FlatItem<A>],
        input: TreeInput<'_>,
    ) -> Option<A> {
        self.focus_changed = false;
        self.last_window = input.window;
        // Build the layout cache from the explicit rects.
        self.layout.clear();
        for it in items {
            self.layout.push(LaidOut {
                id: it.id,
                rect: it.rect,
                enabled: true,
            });
        }

        // Resolve focused id.
        if let Some(id) = self.focused {
            if !self.layout.iter().any(|l| l.id == id) {
                self.focused = self.layout.first().map(|l| l.id);
            }
        } else {
            self.focused = self.layout.first().map(|l| l.id);
        }

        // Mouse hover-follow (only in cursor mode).
        if input.input_mode == crate::ui::input::InputMode::Cursor {
            let (cx, cy) = input.cursor_pos;
            for l in &self.layout {
                let [x, y, w, h] = l.rect;
                if cx >= x && cx <= x + w && cy >= y && cy <= y + h {
                    self.set_focus_changed(Some(l.id));
                    break;
                }
            }
        }

        // Mouse clicks.
        for &cid in input.button_clicks {
            if let Some(it) = items.iter().find(|i| i.id.0 == cid) {
                return Some(it.action);
            }
        }

        // Keyboard / gamepad nav — spatial nearest-neighbour over item rects.
        for a in input.actions {
            match a {
                UiAction::FocusDown => self.move_focus_spatial(FocusDir::Down),
                UiAction::FocusUp => self.move_focus_spatial(FocusDir::Up),
                UiAction::FocusNext => self.move_focus_spatial(FocusDir::Right),
                UiAction::FocusPrev => self.move_focus_spatial(FocusDir::Left),
                UiAction::Confirm | UiAction::CommitDiscard => {
                    let f = self.focused?;
                    if let Some(it) = items.iter().find(|i| i.id == f) {
                        return Some(it.action);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Push `ButtonDef::scene` hit-targets for a flat list. Call from `draw()`
    /// after your scene has drawn its visuals so the main loop can route mouse
    /// clicks back as `button_clicks` ids next frame.
    pub fn register_flat_buttons<A: Copy>(
        &self,
        items: &[FlatItem<A>],
        buttons: &mut Vec<ButtonDef>,
    ) {
        for it in items {
            let mut def =
                ButtonDef::scene((it.rect[0], it.rect[1], it.rect[2], it.rect[3]), it.id.0);
            if let Some(ref t) = it.tooltip {
                def = def.with_hover_label(t.clone());
            }
            buttons.push(def);
        }
    }

    /// Snapshot of the inferred focus graph for debug overlay drawing.
    pub fn focus_nav_debug_snapshot_flat<A: Copy>(
        &self,
        items: &[FlatItem<A>],
        label: impl Fn(A) -> String,
    ) -> FocusNavDebugSnapshot {
        let candidates: Vec<(FocusId, [f32; 4])> =
            items.iter().map(|it| (it.id, it.rect)).collect();
        debug_snapshot_from_candidates(
            &candidates,
            &[],
            self.focused(),
            self.focus_nav.memory(),
            |id| {
                items
                    .iter()
                    .find(|it| it.id == id)
                    .map(|it| label(it.action))
                    .unwrap_or_else(|| format!("id {}", id.0))
            },
        )
    }

    /// Same as [`Self::focus_nav_debug_snapshot_flat`] but labels come from
    /// [`FocusId`] only (declarative trees without a flat action list).
    pub fn focus_nav_debug_snapshot_ids(
        &self,
        candidates: &[(FocusId, [f32; 4])],
        label: impl Fn(FocusId) -> String,
    ) -> FocusNavDebugSnapshot {
        debug_snapshot_from_candidates(
            candidates,
            &[],
            self.focused(),
            self.focus_nav.memory(),
            label,
        )
    }

    /// Declarative [`Tree`] layout for the current window size.
    pub fn focus_nav_debug_snapshot_tree<A: Copy>(
        &self,
        tree: &Tree<A>,
        window: (f32, f32),
        label: impl Fn(FocusId) -> String,
    ) -> FocusNavDebugSnapshot {
        let mut layout_scratch = Vec::new();
        let _ = layout_tree(tree, window, &mut layout_scratch);
        let candidates: Vec<(FocusId, [f32; 4])> =
            layout_scratch.iter().map(|l| (l.id, l.rect)).collect();
        self.focus_nav_debug_snapshot_ids(&candidates, label)
    }
}

/// One entry in a [`TreeState::update_flat`] call.
#[derive(Clone, Debug)]
pub struct FlatItem<A: Copy> {
    pub id: FocusId,
    pub rect: [f32; 4],
    pub action: A,
    /// Cursor-hover tooltip; [`None`] = none.
    pub tooltip: Option<Cow<'static, str>>,
}

impl<A: Copy> FlatItem<A> {
    pub fn new(id: FocusId, rect: [f32; 4], action: A) -> Self {
        Self {
            id,
            rect,
            action,
            tooltip: None,
        }
    }
}

// ─── Frame: where the tree pushes its output ────────────────────────────────

pub struct TreeFrame<'a> {
    pub instances: &'a mut Vec<GpuInstance>,
    pub labels: &'a mut Vec<TextLabel>,
    pub buttons: &'a mut Vec<ButtonDef>,
}

// ─── Input bundle ───────────────────────────────────────────────────────────

pub struct TreeInput<'a> {
    pub actions: &'a [UiAction],
    pub button_clicks: &'a [u32],
    pub cursor_pos: (f32, f32),
    pub window: (f32, f32),
    pub input_mode: crate::ui::input::InputMode,
    /// Mouse-wheel / trackpad scroll delta in line units.
    /// Positive = scroll down. Only used when the tree content overflows
    /// its anchor rect (autoscroll).
    pub scroll_lines: f32,
}

// ─── Layout pass ────────────────────────────────────────────────────────────

/// Result of a layout pass, used for autoscroll calculations.
struct LayoutInfo {
    content_height: f32,
    anchor_height: f32,
    anchor_rect: [f32; 4],
}

fn layout_tree<A: Copy>(tree: &Tree<A>, window: (f32, f32), out: &mut Vec<LaidOut>) -> LayoutInfo {
    out.clear();
    let (w, h) = window;
    let scale = metrics::scene_scale(w, h);

    let anchor = tree.anchor.unwrap_or_else(|| match &tree.root {
        Node::Column { .. } => {
            let cw = (260.0 * scale).min(w * 0.7);
            let cx = (w - cw) * 0.5;
            let cy = h * 0.10;
            let ch = h * 0.85;
            [cx, cy, cw, ch]
        }
        _ => [0.0, 0.0, w, h],
    });

    let content_height = root_content_height(&tree.root, anchor[2], h, scale);
    layout_node(&tree.root, anchor, scale, h, out);
    LayoutInfo {
        content_height,
        anchor_height: anchor[3],
        anchor_rect: anchor,
    }
}

/// Compute the natural content height of a root node (including gaps).
fn root_content_height<A: Copy>(
    node: &Node<A>,
    container_w: f32,
    window_h: f32,
    scale: f32,
) -> f32 {
    match node {
        Node::Column { gap, children, .. } => {
            let gap_px = if *gap > 0.0 {
                *gap
            } else {
                (12.0 * scale).max(6.0)
            };
            let mut total = 0.0f32;
            for (i, c) in children.iter().enumerate() {
                total += child_height(c, container_w, window_h, scale);
                if i + 1 < children.len() {
                    total += gap_px;
                }
            }
            total
        }
        _ => 0.0,
    }
}

fn natural_item_height(scale: f32) -> f32 {
    (38.0 * scale).max(24.0)
}

fn natural_item_width(container_w: f32, scale: f32) -> f32 {
    (220.0 * scale).min(container_w)
}

fn layout_node<A: Copy>(
    node: &Node<A>,
    rect: [f32; 4],
    scale: f32,
    window_h: f32,
    out: &mut Vec<LaidOut>,
) {
    let [x, y, w, h] = rect;
    match node {
        Node::Column {
            gap,
            align,
            children,
        } => {
            let gap_px = if *gap > 0.0 {
                *gap
            } else {
                (12.0 * scale).max(6.0)
            };
            let mut child_heights = Vec::with_capacity(children.len());
            for c in children {
                child_heights.push(child_height(c, w, window_h, scale));
            }
            let total_h: f32 = child_heights.iter().sum::<f32>()
                + gap_px * (children.len().saturating_sub(1) as f32);
            let mut cy = y + ((h - total_h) * 0.5).max(0.0);
            for (child, ch) in children.iter().zip(child_heights.iter()) {
                let cw = match align {
                    HAlign::Stretch => w,
                    HAlign::Center => child_width(child, w, scale),
                };
                let cx = match align {
                    HAlign::Stretch => x,
                    HAlign::Center => x + (w - cw) * 0.5,
                };
                layout_node(child, [cx, cy, cw, *ch], scale, window_h, out);
                cy += *ch + gap_px;
            }
        }
        Node::Row {
            gap,
            align: _align,
            children,
        } => {
            let gap_px = if *gap > 0.0 {
                *gap
            } else {
                (10.0 * scale).max(4.0)
            };
            let n = children.len().max(1) as f32;
            let total_gap = gap_px * (children.len().saturating_sub(1) as f32);
            let cw = ((w - total_gap) / n).max(0.0);
            let mut cx = x;
            for child in children {
                let ch = child_height(child, cw, window_h, scale).min(h);
                let cy = y + (h - ch) * 0.5;
                layout_node(child, [cx, cy, cw, ch], scale, window_h, out);
                cx += cw + gap_px;
            }
        }
        Node::Item(item) => {
            out.push(LaidOut {
                id: item.id,
                rect,
                enabled: item.enabled,
            });
        }
    }
}

fn child_width<A: Copy>(node: &Node<A>, container_w: f32, scale: f32) -> f32 {
    match node {
        Node::Item(_) => natural_item_width(container_w, scale),
        Node::Column { .. } | Node::Row { .. } => container_w,
    }
}

fn child_height<A: Copy>(node: &Node<A>, _container_w: f32, _window_h: f32, scale: f32) -> f32 {
    match node {
        Node::Item(_) => natural_item_height(scale),
        Node::Row { children, .. } => children
            .iter()
            .map(|c| child_height(c, _container_w, _window_h, scale))
            .fold(0.0f32, f32::max),
        Node::Column { .. } => 0.0,
    }
}

// ─── Update pass ────────────────────────────────────────────────────────────

impl TreeState {
    /// Lay out the tree, run input, return the activated action (if any).
    pub fn update<A: Copy>(&mut self, tree: &Tree<A>, input: TreeInput<'_>) -> Option<A> {
        self.focus_changed = false;
        self.last_window = input.window;
        let info = layout_tree(tree, input.window, &mut self.layout);
        self.content_height = info.content_height;
        self.anchor_height = info.anchor_height;

        // ── Autoscroll: apply pixel offset when content overflows ────
        let overflow = (self.content_height - self.anchor_height).max(0.0);
        if overflow > 0.0 {
            self.scroll_clip_rect = Some(info.anchor_rect);
            // Feed mouse-wheel input into smooth scroll.
            // SmoothScroll works in "entry units" — we use pixels directly
            // by treating 1 unit = 1 pixel of scroll.
            let line_height = if !self.layout.is_empty() {
                // Use average item height as a scroll step for keyboard nav.
                self.content_height / self.layout.len() as f32
            } else {
                40.0
            };
            self.scroll.set_max(overflow.ceil() as u32);
            if input.scroll_lines.abs() > 0.001 {
                self.scroll.scroll_by(input.scroll_lines * line_height);
            }
            // Clamp scroll target to pixel overflow range.
            let t = self.scroll.target().clamp(0.0, overflow);
            self.scroll.set_target(t);
            self.scroll_offset_px = self.scroll.tick();

            // Shift all laid-out rects up by scroll offset.
            for l in &mut self.layout {
                l.rect[1] -= self.scroll_offset_px;
            }
        } else {
            self.scroll_clip_rect = None;
            self.scroll_offset_px = 0.0;
            self.scroll.jump(0.0);
        }

        // Resolve focused id against the latest layout. If it disappeared
        // or now points at a disabled item, fall back to the first enabled
        // item (or first item as a last resort).
        let first_enabled = || {
            self.layout
                .iter()
                .find(|l| l.enabled)
                .or_else(|| self.layout.first())
                .map(|l| l.id)
        };
        if let Some(id) = self.focused {
            match self.layout.iter().find(|l| l.id == id) {
                Some(slot) if slot.enabled => {}
                _ => self.focused = first_enabled(),
            }
        } else {
            self.focused = first_enabled();
        }

        // Mouse hover-follow: if cursor is over an enabled item, focus it
        // (only in cursor mode; disabled items shouldn't steal focus).
        if input.input_mode == crate::ui::input::InputMode::Cursor {
            let (cx, cy) = input.cursor_pos;
            for l in &self.layout {
                if !l.enabled {
                    continue;
                }
                let [x, y, w, h] = l.rect;
                if cx >= x && cx <= x + w && cy >= y && cy <= y + h {
                    self.set_focus_changed(Some(l.id));
                    break;
                }
            }
        }

        // Mouse click: any incoming button_click id whose rect we own → fire.
        for &cid in input.button_clicks {
            if let Some(action) = self.activate_id(tree, FocusId(cid)) {
                return Some(action);
            }
        }

        // Keyboard / gamepad actions.
        for a in input.actions {
            if let Some(action) = self.handle_action(tree, *a) {
                return Some(action);
            }
        }

        // ── Auto-scroll to keep focused item visible ────────────────
        if overflow > 0.0
            && let Some(fid) = self.focused
            && let Some(l) = self.layout.iter().find(|l| l.id == fid)
        {
            // l.rect[1] is already shifted by scroll_offset_px.
            // We want the focused item to be within the original
            // anchor region. Recover the anchor top from the tree.
            let anchor_top = self
                .layout
                .first()
                .map(|first| {
                    // The first item's unscrolled y minus half the centering gap
                    // is roughly anchor_top; but simpler: use the tree anchor.
                    first.rect[1] + self.scroll_offset_px
                        - ((self.anchor_height - self.content_height) * 0.5).max(0.0)
                })
                .unwrap_or(0.0);
            let anchor_bottom = anchor_top + self.anchor_height;
            let item_top = l.rect[1];
            let item_bottom = item_top + l.rect[3];

            if item_top < anchor_top {
                // Item above viewport — scroll up.
                let delta = anchor_top - item_top;
                let t = (self.scroll.target() - delta).max(0.0);
                self.scroll.set_target(t);
            } else if item_bottom > anchor_bottom {
                // Item below viewport — scroll down.
                let delta = item_bottom - anchor_bottom;
                let t = (self.scroll.target() + delta).min(overflow);
                self.scroll.set_target(t);
            }
        }

        None
    }

    fn handle_action<A: Copy>(&mut self, tree: &Tree<A>, action: UiAction) -> Option<A> {
        let focused = self.focused?;
        match action {
            UiAction::FocusDown => {
                self.move_focus_spatial(FocusDir::Down);
                None
            }
            UiAction::FocusUp => {
                self.move_focus_spatial(FocusDir::Up);
                None
            }
            UiAction::FocusNext => {
                self.move_focus_spatial(FocusDir::Right);
                None
            }
            UiAction::FocusPrev => {
                self.move_focus_spatial(FocusDir::Left);
                None
            }
            UiAction::Confirm | UiAction::CommitDiscard => self.activate_id(tree, focused),
            _ => None,
        }
    }

    fn move_focus_spatial(&mut self, dir: FocusDir) {
        let Some(id) = self.focused else {
            return;
        };
        let candidates: Vec<(FocusId, [f32; 4])> = self
            .layout
            .iter()
            .filter(|l| l.enabled)
            .map(|l| (l.id, l.rect))
            .collect();
        self.focus_nav.load_candidates(&candidates, &[]);
        if let Some(next) = self.focus_nav.pick(id, dir) {
            self.set_focus_changed(Some(next));
        }
    }

    fn activate_id<A: Copy>(&mut self, tree: &Tree<A>, id: FocusId) -> Option<A> {
        // Walk the tree to find the matching item.
        find_item(&tree.root, id).and_then(|item| activate_item(item, id))
    }

    fn set_focus_changed(&mut self, next: Option<FocusId>) {
        if self.focused != next {
            self.focused = next;
            self.focus_changed = true;
        }
    }
}

fn find_item<A: Copy>(node: &Node<A>, id: FocusId) -> Option<&Item<A>> {
    match node {
        Node::Item(item) if item.id == id => Some(item),
        Node::Item(_) => None,
        Node::Column { children, .. } | Node::Row { children, .. } => {
            for c in children {
                if let Some(found) = find_item(c, id) {
                    return Some(found);
                }
            }
            None
        }
    }
}

fn activate_item<A: Copy>(item: &Item<A>, _id: FocusId) -> Option<A> {
    if !item.enabled {
        return None;
    }
    Some(item.on_activate)
}

// ─── Draw pass ──────────────────────────────────────────────────────────────

impl TreeState {
    pub fn draw<A: Copy>(&self, tree: &Tree<A>, frame: &mut TreeFrame<'_>) {
        let mut layout_scratch = Vec::with_capacity(self.layout.len());
        let _ = layout_tree(tree, self.last_window, &mut layout_scratch);

        if self.scroll_offset_px.abs() > 0.001 {
            for l in &mut layout_scratch {
                l.rect[1] -= self.scroll_offset_px;
            }
        }

        let mut idx = 0;
        let mut ctx = DrawNodeCtx {
            focused: self.focused,
            layout: &layout_scratch,
            idx: &mut idx,
            window: self.last_window,
            clip_rect: self.scroll_clip_rect,
        };
        draw_node(&tree.root, frame, &mut ctx);
    }
}

struct DrawNodeCtx<'a, 'b> {
    focused: Option<FocusId>,
    layout: &'a [LaidOut],
    idx: &'b mut usize,
    window: (f32, f32),
    clip_rect: Option<[f32; 4]>,
}

fn draw_node<A: Copy>(node: &Node<A>, frame: &mut TreeFrame<'_>, ctx: &mut DrawNodeCtx<'_, '_>) {
    match node {
        Node::Column { children, .. } | Node::Row { children, .. } => {
            for c in children {
                draw_node(c, frame, ctx);
            }
        }
        Node::Item(item) => {
            let rect = ctx
                .layout
                .get(*ctx.idx)
                .map(|l| l.rect)
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            *ctx.idx += 1;
            let is_focused = ctx.focused == Some(item.id);
            draw_item(item, rect, is_focused, ctx.clip_rect, frame, ctx.window);
        }
    }
}

fn push_scene_button_for_item(
    frame: &mut TreeFrame<'_>,
    rect: [f32; 4],
    item_id: FocusId,
    tooltip: &Option<Cow<'static, str>>,
) {
    let mut def = ButtonDef::scene((rect[0], rect[1], rect[2], rect[3]), item_id.0);
    if let Some(t) = tooltip.as_ref() {
        def = def.with_hover_label(t.clone());
    }
    frame.buttons.push(def);
}

fn draw_item<A: Copy>(
    item: &Item<A>,
    rect: [f32; 4],
    focused: bool,
    clip_rect: Option<[f32; 4]>,
    frame: &mut TreeFrame<'_>,
    window: (f32, f32),
) {
    let draw_rect = if let Some(clip) = clip_rect {
        let Some(clipped) = intersect_rect(rect, clip) else {
            return;
        };
        clipped
    } else {
        rect
    };
    let state = if !item.enabled {
        ButtonState::Disabled
    } else if focused {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };

    if focused {
        let scale = metrics::scene_scale(window.0, window.1);
        push_focus_ring(draw_rect, scale, window.0, window.1, frame.instances);
    }

    let colors = crate::render::theme::button_colors(item.variant, state);
    widget::push_panel_colored(frame.instances, draw_rect, colors.bg, colors.border);
    frame.labels.push(TextLabel {
        rect,
        text: item.label.clone(),
        color: colors.text,
        clip_rect,
        ..Default::default()
    });
    push_scene_button_for_item(frame, draw_rect, item.id, &item.tooltip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::input::{InputMode, UiAction};

    #[test]
    fn pick_chamber_focus_next_moves_focus() {
        // Replicates pick_chamber: two FlatItems (Play=id 1, Skip=id 2), focus
        // initially on Play. Pressing FocusNext should move focus to Skip.
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(1));

        let items = vec![
            FlatItem::new(FocusId(1), [100.0, 100.0, 200.0, 100.0], "play"),
            FlatItem::new(FocusId(2), [400.0, 100.0, 200.0, 100.0], "skip"),
        ];

        let actions = [UiAction::FocusNext];
        let action = tree.update_flat(
            &items,
            TreeInput {
                actions: &actions,
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (1280.0, 720.0),
                input_mode: InputMode::Controller,
                scroll_lines: 0.0,
            },
        );
        assert_eq!(action, None, "focus move alone shouldn't return action");
        assert_eq!(
            tree.focused(),
            Some(FocusId(2)),
            "focus should move to Skip"
        );
    }

    #[test]
    fn pick_chamber_cursor_mode_does_not_clobber_focus_when_cursor_off_rects() {
        // In cursor mode, hover-follow only fires when cursor is inside a rect.
        // If cursor is at (0,0) and rects are far away, focus should not change.
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(1));

        let items = vec![
            FlatItem::new(FocusId(1), [100.0, 100.0, 200.0, 100.0], "play"),
            FlatItem::new(FocusId(2), [400.0, 100.0, 200.0, 100.0], "skip"),
        ];

        let actions = [UiAction::FocusNext];
        let _ = tree.update_flat(
            &items,
            TreeInput {
                actions: &actions,
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (1280.0, 720.0),
                input_mode: InputMode::Cursor,
                scroll_lines: 0.0,
            },
        );
        assert_eq!(
            tree.focused(),
            Some(FocusId(2)),
            "FocusNext should still move focus in cursor mode"
        );
    }

    #[test]
    fn flat_spatial_nav_moves_to_nearest_rect_not_list_order() {
        // Guide header: Back (left), Prev (center-right), Next (far right).
        // Linear FocusPrev from Back would wrap to Next; spatial should stay put.
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(1));

        let items = vec![
            FlatItem::new(FocusId(1), [48.0, 48.0, 108.0, 52.0], 'b'),
            FlatItem::new(FocusId(2), [1500.0, 48.0, 58.0, 52.0], 'p'),
            FlatItem::new(FocusId(3), [1600.0, 48.0, 58.0, 52.0], 'n'),
        ];

        let _ = tree.update_flat(
            &items,
            TreeInput {
                actions: &[UiAction::FocusPrev],
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (1920.0, 1080.0),
                input_mode: InputMode::Controller,
                scroll_lines: 0.0,
            },
        );
        assert_eq!(
            tree.focused(),
            Some(FocusId(1)),
            "no spatial neighbour left of Back — focus should not wrap"
        );

        let _ = tree.update_flat(
            &items,
            TreeInput {
                actions: &[UiAction::FocusNext],
                button_clicks: &[],
                cursor_pos: (0.0, 0.0),
                window: (1920.0, 1080.0),
                input_mode: InputMode::Controller,
                scroll_lines: 0.0,
            },
        );
        assert_eq!(
            tree.focused(),
            Some(FocusId(2)),
            "right from Back should land on Prev"
        );
    }

    #[test]
    fn pick_chamber_cursor_over_rect_overrides_focus_move() {
        // Under cursor mode, hover-follow runs BEFORE the action loop. If
        // cursor is over Play and we also press FocusNext, what happens?
        // Order: layout -> hover-follow (sets focus to Play) -> click route
        // -> action loop (FocusNext → Skip). So action wins.
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(2));

        let items = vec![
            FlatItem::new(FocusId(1), [100.0, 100.0, 200.0, 100.0], "play"),
            FlatItem::new(FocusId(2), [400.0, 100.0, 200.0, 100.0], "skip"),
        ];

        let actions = [UiAction::FocusNext];
        let _ = tree.update_flat(
            &items,
            TreeInput {
                actions: &actions,
                // Cursor inside Play rect.
                cursor_pos: (150.0, 150.0),
                button_clicks: &[],
                window: (1280.0, 720.0),
                input_mode: InputMode::Cursor,
                scroll_lines: 0.0,
            },
        );
        // Hover sets focus → Play; then FocusNext moves to Skip.
        assert_eq!(tree.focused(), Some(FocusId(2)));
    }
}
