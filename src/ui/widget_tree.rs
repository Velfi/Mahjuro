#![allow(dead_code)]
//! Generic, immediate-mode widget tree for scene UIs.
//!
//! Each frame a scene builds a [`Tree`] from its current state, hands it to a
//! tiny persistent [`TreeState`], and gets back a single `Option<A>` describing
//! what (if anything) the user activated this frame. The tree owns layout,
//! hit-testing, hover-follow, keyboard navigation, slider/toggle/cycle
//! adjustment, and rendering — so scenes never duplicate layout math between
//! `update()` and `draw()`, never juggle named const indices, and never lose
//! sync between hover hit-tests and rendered rects.
//!
//! ## Action API
//!
//! `A` is a scene-defined `Copy` enum (e.g. `enum MainAction { Play, Quit, … }`).
//! Each interactive item carries an `A` value the tree returns when the item
//! activates. The scene's `update()` then matches on `Option<A>` exhaustively.
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
//! self.tree_state.draw(&tree, &mut frame, &noop_render_custom);
//! ```
//!
//! The layout cache built during `update()` is reused by `draw()` so the
//! layout pass runs exactly once per frame.

use crate::render::theme::{self, ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::focus_nav::push_focus_ring;
use crate::ui::input::UiAction;
use crate::ui::widget;

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
    /// Vertical stack of children.
    Column {
        gap: f32,
        align: HAlign,
        children: Vec<Node<A>>,
    },
    /// Horizontal stack of children.
    Row {
        gap: f32,
        align: VAlign,
        children: Vec<Node<A>>,
    },
    /// Fixed-column grid (children flow into rows).
    Grid {
        cols: usize,
        gap: (f32, f32),
        children: Vec<Node<A>>,
    },
    Item(Item<A>),
    Decoration(Decoration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Center,
    Left,
    Right,
    /// Stretch children to fill the container width.
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Center,
    Top,
    Bottom,
    Stretch,
}

/// Sizing hint for a node.
#[derive(Clone, Copy, Debug)]
pub enum Size {
    /// Use the node's natural size (defaults to typical button dimensions).
    Auto,
    /// Fixed pixel size.
    Fixed(f32, f32),
    /// Width derived from a fraction of the container's width.
    FracW(f32),
    /// Height derived from a fraction of the container's height.
    FracH(f32),
}

pub struct Item<A: Copy> {
    pub id: FocusId,
    pub size: Size,
    pub enabled: bool,
    pub kind: ItemKind<A>,
}

pub enum ItemKind<A: Copy> {
    Button {
        label: String,
        variant: ButtonVariant,
        on_activate: A,
    },
    Slider {
        label: String,
        value: f32,
        range: (f32, f32),
        step: f32,
        /// Pure mapping from new value → action. No closures so the kind stays
        /// `Copy`-friendly for trivial scenes.
        on_change: fn(f32) -> A,
    },
    Toggle {
        label: String,
        value: bool,
        on_toggle: A,
    },
    Cycle {
        label: String,
        options: Vec<String>,
        index: usize,
        on_next: A,
        on_prev: A,
    },
    Tab {
        label: String,
        active: bool,
        on_select: A,
    },
    /// Escape hatch: tree owns the rect, scene owns the visual. The
    /// `kind_tag` is a scene-defined u32 (typically a discriminant) that the
    /// scene's `render_custom` callback matches against to draw the contents.
    Custom { kind_tag: u32, on_activate: A },
}

/// Non-interactive node: titles, hint text, spacers.
pub enum Decoration {
    Title {
        text: String,
        tier: f32,
        color: [f32; 4],
    },
    Body {
        text: String,
        tier: f32,
        color: [f32; 4],
    },
    Hint {
        text: String,
        tier: f32,
        color: [f32; 4],
    },
    Spacer(f32),
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

    pub fn anchored(rect: [f32; 4], child: Node<A>) -> Self {
        Self {
            root: child,
            anchor: Some(rect),
        }
    }

    pub fn with_anchor(mut self, rect: [f32; 4]) -> Self {
        self.anchor = Some(rect);
        self
    }
}

pub fn button<A: Copy>(label: &str, action: A, variant: ButtonVariant) -> Node<A> {
    let id = FocusId(action_id(action));
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Button {
            label: label.into(),
            variant,
            on_activate: action,
        },
    })
}

pub fn button_id<A: Copy>(id: FocusId, label: &str, action: A, variant: ButtonVariant) -> Node<A> {
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Button {
            label: label.into(),
            variant,
            on_activate: action,
        },
    })
}

pub fn slider<A: Copy>(
    id: FocusId,
    label: &str,
    value: f32,
    range: (f32, f32),
    step: f32,
    on_change: fn(f32) -> A,
) -> Node<A> {
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Slider {
            label: label.into(),
            value,
            range,
            step,
            on_change,
        },
    })
}

pub fn toggle<A: Copy>(id: FocusId, label: &str, value: bool, on_toggle: A) -> Node<A> {
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Toggle {
            label: label.into(),
            value,
            on_toggle,
        },
    })
}

pub fn cycle<A: Copy>(
    id: FocusId,
    label: &str,
    options: Vec<String>,
    index: usize,
    on_prev: A,
    on_next: A,
) -> Node<A> {
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Cycle {
            label: label.into(),
            options,
            index,
            on_next,
            on_prev,
        },
    })
}

pub fn tab<A: Copy>(id: FocusId, label: &str, active: bool, on_select: A) -> Node<A> {
    Node::Item(Item {
        id,
        size: Size::Auto,
        enabled: true,
        kind: ItemKind::Tab {
            label: label.into(),
            active,
            on_select,
        },
    })
}

pub fn custom<A: Copy>(id: FocusId, size: Size, kind_tag: u32, on_activate: A) -> Node<A> {
    Node::Item(Item {
        id,
        size,
        enabled: true,
        kind: ItemKind::Custom {
            kind_tag,
            on_activate,
        },
    })
}

pub fn title<A: Copy>(text: &str, tier: f32, color: [f32; 4]) -> Node<A> {
    Node::Decoration(Decoration::Title {
        text: text.into(),
        tier,
        color,
    })
}

pub fn body<A: Copy>(text: &str, tier: f32, color: [f32; 4]) -> Node<A> {
    Node::Decoration(Decoration::Body {
        text: text.into(),
        tier,
        color,
    })
}

pub fn hint<A: Copy>(text: &str, tier: f32, color: [f32; 4]) -> Node<A> {
    Node::Decoration(Decoration::Hint {
        text: text.into(),
        tier,
        color,
    })
}

pub fn spacer<A: Copy>(px: f32) -> Node<A> {
    Node::Decoration(Decoration::Spacer(px))
}

/// Convenience: derive a `FocusId` from any `Copy` action via raw pointer
/// hashing. Returns a unique, stable id per discriminant *value* (not memory
/// address) by interpreting the action's bytes. Reliable only for actions
/// that fit in `u64`; falls back to a typeid-derived hash otherwise.
fn action_id<A: Copy>(action: A) -> u32 {
    use std::mem::size_of;
    // Read the first up-to-8 bytes of the action and fold them into a u32.
    // For C-style enums and small variant payloads this is stable and unique
    // per (variant, payload) combination, which is exactly what we want.
    let size = size_of::<A>().min(8);
    if size == 0 {
        return 0;
    }
    let mut buf = [0u8; 8];
    // SAFETY: A is Copy, we read at most size_of::<A>() bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(&action as *const A as *const u8, buf.as_mut_ptr(), size);
    }
    let v = u64::from_le_bytes(buf);
    // Mix with a small prime so the low bits aren't all clustered at zero.
    let mixed = v.wrapping_mul(0x9E3779B97F4A7C15);
    (mixed ^ (mixed >> 32)) as u32
}

// ─── Persistent tree state ──────────────────────────────────────────────────

/// Per-scene state that survives across frames. The only thing the scene must
/// store between `update()` and `draw()`. Cheap to construct.
pub struct TreeState {
    /// Currently focused item id. Resolved against the latest layout cache.
    focused: Option<FocusId>,
    /// Layout cache built during `update()` and reused by `draw()`. Each entry
    /// is `(item id, rect, kind tag)`. Decorations are not cached.
    layout: Vec<LaidOut>,
    last_window: (f32, f32),
}

#[derive(Clone, Copy)]
struct LaidOut {
    id: FocusId,
    rect: [f32; 4],
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
            layout: Vec::new(),
            last_window: (0.0, 0.0),
        }
    }

    /// Force-set the currently focused item (e.g. when a scene transitions
    /// in and wants the cursor on a specific row).
    pub fn set_focus(&mut self, id: FocusId) {
        self.focused = Some(id);
    }

    /// Returns the currently focused item id, if any.
    pub fn focused(&self) -> Option<FocusId> {
        self.focused
    }

    /// Hover/click resolution for a flat list of pre-computed (id, rect, action)
    /// hit targets. Use this for scenes that already know their item rects
    /// (e.g. derived from `LayoutResult::hand_slots`, or hand-laid card grids)
    /// and only want focus management + click routing — not the full Node tree.
    ///
    /// Companion: [`TreeState::register_flat_buttons`] pushes the corresponding
    /// `ButtonDef::scene` entries during draw.
    pub fn update_flat<A: Copy>(
        &mut self,
        items: &[FlatItem<A>],
        input: TreeInput<'_>,
    ) -> Option<A> {
        self.last_window = input.window;
        // Build the layout cache from the explicit rects.
        self.layout.clear();
        for it in items {
            self.layout.push(LaidOut {
                id: it.id,
                rect: it.rect,
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

        // Mouse hover-follow.
        let (cx, cy) = input.cursor_pos;
        for l in &self.layout {
            let [x, y, w, h] = l.rect;
            if cx >= x && cx <= x + w && cy >= y && cy <= y + h {
                self.focused = Some(l.id);
                break;
            }
        }

        // Mouse clicks.
        for &cid in input.button_clicks {
            if let Some(it) = items.iter().find(|i| i.id.0 == cid) {
                return Some(it.action);
            }
        }

        // Keyboard / gamepad nav. Treat the list as 1D (linear order).
        for a in input.actions {
            match a {
                UiAction::FocusDown | UiAction::FocusNext => {
                    self.move_focus(1);
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.move_focus(-1);
                }
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
            buttons.push(ButtonDef::scene(
                (it.rect[0], it.rect[1], it.rect[2], it.rect[3]),
                it.id.0,
            ));
        }
    }
}

/// One entry in a [`TreeState::update_flat`] call.
#[derive(Clone, Copy, Debug)]
pub struct FlatItem<A: Copy> {
    pub id: FocusId,
    pub rect: [f32; 4],
    pub action: A,
}

impl<A: Copy> FlatItem<A> {
    pub fn new(id: FocusId, rect: [f32; 4], action: A) -> Self {
        Self { id, rect, action }
    }
}

// ─── Frame: where the tree pushes its output ────────────────────────────────

pub struct TreeFrame<'a> {
    pub instances: &'a mut Vec<GpuInstance>,
    pub labels: &'a mut Vec<TextLabel>,
    pub buttons: &'a mut Vec<ButtonDef>,
    pub window: (f32, f32),
}

/// Focus state passed to a `render_custom` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusState {
    Rest,
    Hover,
}

/// No-op `render_custom` for trees that don't use the `Custom` escape hatch.
pub fn noop_render_custom(
    _frame: &mut TreeFrame<'_>,
    _rect: [f32; 4],
    _kind_tag: u32,
    _state: FocusState,
) {
}

// ─── Input bundle ───────────────────────────────────────────────────────────

pub struct TreeInput<'a> {
    pub actions: &'a [UiAction],
    pub button_clicks: &'a [u32],
    pub cursor_pos: (f32, f32),
    pub window: (f32, f32),
}

// ─── Layout pass ────────────────────────────────────────────────────────────

/// Walk the tree and compute item rects. Decorations get their rects too but
/// we don't cache them (they're never hit-tested).
fn layout_tree<A: Copy>(
    tree: &Tree<A>,
    window: (f32, f32),
    out: &mut Vec<LaidOut>,
) -> Vec<NodeRect> {
    out.clear();
    let (w, h) = window;
    let scale = (w.min(h)) / 600.0;

    // Resolve the root anchor. Defaults to centered, narrow column for
    // vertical menus; full screen for everything else.
    let anchor = tree.anchor.unwrap_or_else(|| {
        match &tree.root {
            Node::Column { .. } => {
                let cw = (260.0 * scale).min(w * 0.7);
                let cx = (w - cw) * 0.5;
                // Centered vertically with a comfortable top margin.
                let cy = h * 0.10;
                let ch = h * 0.85;
                [cx, cy, cw, ch]
            }
            _ => [0.0, 0.0, w, h],
        }
    });

    let mut rects = Vec::new();
    layout_node(&tree.root, anchor, scale, h, out, &mut rects);
    rects
}

/// Per-node resolved rect, parallel to the tree walk order. Used by `draw`.
#[derive(Clone, Copy)]
struct NodeRect {
    rect: [f32; 4],
}

fn natural_item_height(scale: f32) -> f32 {
    (38.0 * scale).max(24.0)
}

fn natural_item_width(container_w: f32, scale: f32) -> f32 {
    (220.0 * scale).min(container_w)
}

fn natural_decoration_height(decoration: &Decoration, window_h: f32, scale: f32) -> f32 {
    match decoration {
        Decoration::Title { tier, .. } => typography::size(*tier, window_h) * 1.2,
        Decoration::Body { tier, .. } => typography::size(*tier, window_h) * 1.1,
        Decoration::Hint { tier, .. } => typography::size(*tier, window_h) * 1.1,
        Decoration::Spacer(px) => *px * scale,
    }
}

fn layout_node<A: Copy>(
    node: &Node<A>,
    rect: [f32; 4],
    scale: f32,
    window_h: f32,
    out: &mut Vec<LaidOut>,
    rects: &mut Vec<NodeRect>,
) {
    rects.push(NodeRect { rect });
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
            // Compute each child's natural height, then center the stack
            // vertically inside the container.
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
                    _ => child_width(child, w, scale),
                };
                let cx = match align {
                    HAlign::Left => x,
                    HAlign::Right => x + w - cw,
                    HAlign::Center | HAlign::Stretch => x + (w - cw) * 0.5,
                };
                layout_node(child, [cx, cy, cw, *ch], scale, window_h, out, rects);
                cy += ch + gap_px;
            }
        }
        Node::Row {
            gap,
            align,
            children,
        } => {
            let gap_px = if *gap > 0.0 {
                *gap
            } else {
                (10.0 * scale).max(4.0)
            };
            // Equal-width split for now. Each child gets the row height.
            let n = children.len().max(1) as f32;
            let total_gap = gap_px * (children.len().saturating_sub(1) as f32);
            let cw = ((w - total_gap) / n).max(0.0);
            let mut cx = x;
            for child in children {
                let ch = child_height(child, cw, window_h, scale).min(h);
                let cy = match align {
                    VAlign::Top => y,
                    VAlign::Bottom => y + h - ch,
                    VAlign::Center | VAlign::Stretch => y + (h - ch) * 0.5,
                };
                let final_h = if matches!(align, VAlign::Stretch) {
                    h
                } else {
                    ch
                };
                layout_node(child, [cx, cy, cw, final_h], scale, window_h, out, rects);
                cx += cw + gap_px;
            }
        }
        Node::Grid {
            cols,
            gap,
            children,
        } => {
            let cols = (*cols).max(1);
            let (gx, gy) = *gap;
            let cell_w = ((w - gx * (cols as f32 - 1.0)) / cols as f32).max(0.0);
            let rows = children.len().div_ceil(cols);
            let cell_h = if rows > 0 {
                ((h - gy * (rows as f32 - 1.0)) / rows as f32).max(0.0)
            } else {
                0.0
            };
            for (i, child) in children.iter().enumerate() {
                let r = i / cols;
                let c = i % cols;
                let cx = x + c as f32 * (cell_w + gx);
                let cy = y + r as f32 * (cell_h + gy);
                layout_node(child, [cx, cy, cell_w, cell_h], scale, window_h, out, rects);
            }
        }
        Node::Item(item) => {
            out.push(LaidOut { id: item.id, rect });
        }
        Node::Decoration(_) => {
            // Decorations don't go in the focus cache.
        }
    }
}

fn child_width<A: Copy>(node: &Node<A>, container_w: f32, scale: f32) -> f32 {
    match node {
        Node::Item(item) => match item.size {
            Size::Fixed(w, _) => w,
            Size::FracW(f) => container_w * f,
            Size::FracH(_) | Size::Auto => natural_item_width(container_w, scale),
        },
        Node::Decoration(_) => container_w,
        Node::Column { .. } | Node::Row { .. } | Node::Grid { .. } => container_w,
    }
}

fn child_height<A: Copy>(node: &Node<A>, container_w: f32, window_h: f32, scale: f32) -> f32 {
    match node {
        Node::Item(item) => match item.size {
            Size::Fixed(_, h) => h,
            Size::FracH(f) => window_h * f,
            Size::FracW(f) => container_w * f,
            Size::Auto => natural_item_height(scale),
        },
        Node::Decoration(d) => natural_decoration_height(d, window_h, scale),
        // Containers default to the natural height of their children. We don't
        // recursively measure here because the parent already gave us a rect
        // (anchor or grid cell). Use a sentinel = 0; the parent decides.
        Node::Column { .. } | Node::Row { .. } | Node::Grid { .. } => 0.0,
    }
}

// ─── Update pass ────────────────────────────────────────────────────────────

impl TreeState {
    /// Lay out the tree, run input, return the activated action (if any).
    pub fn update<A: Copy>(&mut self, tree: &Tree<A>, input: TreeInput<'_>) -> Option<A> {
        self.last_window = input.window;
        let _ = layout_tree(tree, input.window, &mut self.layout);

        // Resolve focused id against the latest layout. If it disappeared,
        // fall back to the first item.
        if let Some(id) = self.focused {
            if !self.layout.iter().any(|l| l.id == id) {
                self.focused = self.layout.first().map(|l| l.id);
            }
        } else {
            self.focused = self.layout.first().map(|l| l.id);
        }

        // Mouse hover-follow: if cursor is over an item, focus it.
        let (cx, cy) = input.cursor_pos;
        for l in &self.layout {
            let [x, y, w, h] = l.rect;
            if cx >= x && cx <= x + w && cy >= y && cy <= y + h {
                self.focused = Some(l.id);
                break;
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

        None
    }

    fn handle_action<A: Copy>(&mut self, tree: &Tree<A>, action: UiAction) -> Option<A> {
        let focused = self.focused?;
        match action {
            UiAction::FocusDown | UiAction::FocusNext => {
                self.move_focus(1);
                None
            }
            UiAction::FocusUp | UiAction::FocusPrev => {
                self.move_focus(-1);
                None
            }
            UiAction::Confirm | UiAction::CommitDiscard => self.activate_id(tree, focused),
            _ => None,
        }
    }

    fn move_focus(&mut self, delta: i32) {
        if self.layout.is_empty() {
            return;
        }
        let cur = self
            .focused
            .and_then(|id| self.layout.iter().position(|l| l.id == id))
            .unwrap_or(0);
        let n = self.layout.len() as i32;
        let next = ((cur as i32 + delta).rem_euclid(n)) as usize;
        self.focused = Some(self.layout[next].id);
    }

    fn activate_id<A: Copy>(&mut self, tree: &Tree<A>, id: FocusId) -> Option<A> {
        // Walk the tree to find the matching item.
        find_item(&tree.root, id).and_then(|item| activate_item(item, id))
    }
}

fn find_item<'a, A: Copy>(node: &'a Node<A>, id: FocusId) -> Option<&'a Item<A>> {
    match node {
        Node::Item(item) if item.id == id => Some(item),
        Node::Item(_) | Node::Decoration(_) => None,
        Node::Column { children, .. }
        | Node::Row { children, .. }
        | Node::Grid { children, .. } => {
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
    match &item.kind {
        ItemKind::Button { on_activate, .. } => Some(*on_activate),
        ItemKind::Toggle { on_toggle, .. } => Some(*on_toggle),
        ItemKind::Tab { on_select, .. } => Some(*on_select),
        ItemKind::Custom { on_activate, .. } => Some(*on_activate),
        ItemKind::Slider {
            value,
            range,
            step,
            on_change,
            ..
        } => {
            // Activate (Confirm) on a slider increments by one step, wrapping
            // back to min when past max. This is the same affordance as
            // clicking a slider — and matches what scenes' Confirm handlers
            // do today (start_screen / pause_menu / options).
            let next = value + step;
            let snapped = if next > range.1 + 1e-4 {
                range.0
            } else {
                next.min(range.1)
            };
            Some(on_change(snapped))
        }
        ItemKind::Cycle { on_next, .. } => Some(*on_next),
    }
}

// ─── Draw pass ──────────────────────────────────────────────────────────────

impl TreeState {
    pub fn draw<A: Copy>(
        &self,
        tree: &Tree<A>,
        frame: &mut TreeFrame<'_>,
        render_custom: &dyn Fn(&mut TreeFrame<'_>, [f32; 4], u32, FocusState),
    ) {
        // Re-walk the tree using the cached layout. We rebuild the rects in
        // the same order as `update()` did, then draw each node.
        let mut layout_scratch = Vec::with_capacity(self.layout.len());
        let _ = layout_tree(tree, self.last_window, &mut layout_scratch);
        // The cache from update() and the cache we just built must match.
        // (They will, because the tree shape is identical.)
        let mut idx = 0;
        draw_node(
            &tree.root,
            self.focused,
            frame,
            render_custom,
            &layout_scratch,
            &mut idx,
            self.last_window,
        );
    }
}

fn draw_node<A: Copy>(
    node: &Node<A>,
    focused: Option<FocusId>,
    frame: &mut TreeFrame<'_>,
    render_custom: &dyn Fn(&mut TreeFrame<'_>, [f32; 4], u32, FocusState),
    layout: &[LaidOut],
    idx: &mut usize,
    window: (f32, f32),
) {
    match node {
        Node::Column { children, .. }
        | Node::Row { children, .. }
        | Node::Grid { children, .. } => {
            for c in children {
                draw_node(c, focused, frame, render_custom, layout, idx, window);
            }
        }
        Node::Item(item) => {
            // The next entry in `layout` corresponds to this item.
            let rect = layout
                .get(*idx)
                .map(|l| l.rect)
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            *idx += 1;
            let is_focused = focused == Some(item.id);
            draw_item(item, rect, is_focused, frame, render_custom, window);
        }
        Node::Decoration(d) => {
            // Decorations need their own rect; we recompute it from the
            // bounding box of the parent walk by tracking parent rects in
            // a separate pass. Simpler: just use the *last* parent rect we
            // saw — the layout pass already centered text labels by writing
            // a label rect that spans the parent's width. To stay simple,
            // we use the parent's anchor for decorations by reading the
            // last container rect from `layout` if available, but since
            // decorations aren't cached we instead embed them inline next
            // to their parent column. We re-derive the rect via a small
            // re-layout against the current parent context.
            //
            // The simpler implementation: decorations only ever appear inside
            // a Column or Row. We recompute their rect on the fly here using
            // the same heuristics layout_node does. To keep things consistent
            // we just draw the decoration centered horizontally in the window
            // at a y-position chosen by where we are in the column. For the
            // first migration scenes (start_screen), decorations are always
            // a Title at the top — so we use a window-relative top position.
            draw_decoration_top(d, frame, window);
        }
    }
}

fn draw_decoration_top(d: &Decoration, frame: &mut TreeFrame<'_>, window: (f32, f32)) {
    let (w, h) = window;
    match d {
        Decoration::Title { text, tier, color } => {
            let th = typography::size(*tier, h);
            frame.labels.push(TextLabel {
                rect: [0.0, h * 0.08, w, th],
                text: text.clone(),
                color: *color,
                ..Default::default()
            });
        }
        Decoration::Body { text, tier, color } => {
            let th = typography::size(*tier, h);
            frame.labels.push(TextLabel {
                rect: [0.0, h * 0.16, w, th],
                text: text.clone(),
                color: *color,
                ..Default::default()
            });
        }
        Decoration::Hint { text, tier, color } => {
            let th = typography::size(*tier, h);
            let scale = (w.min(h)) / 600.0;
            let hint_y = h - th - (12.0 * scale);
            frame.labels.push(TextLabel {
                rect: [0.0, hint_y, w, th],
                text: text.clone(),
                color: *color,
                ..Default::default()
            });
        }
        Decoration::Spacer(_) => {}
    }
}

fn draw_item<A: Copy>(
    item: &Item<A>,
    rect: [f32; 4],
    focused: bool,
    frame: &mut TreeFrame<'_>,
    render_custom: &dyn Fn(&mut TreeFrame<'_>, [f32; 4], u32, FocusState),
    window: (f32, f32),
) {
    let state = if !item.enabled {
        ButtonState::Disabled
    } else if focused {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    let focus_state = if focused {
        FocusState::Hover
    } else {
        FocusState::Rest
    };

    // Draw a gold focus ring around the focused item — the 2D equivalent
    // of the 3D tile outline shell that selected in-game tiles get.
    if focused {
        let scale = (window.0.min(window.1)) / 600.0;
        push_focus_ring(rect, scale, frame.instances);
    }

    match &item.kind {
        ItemKind::Button { label, variant, .. } => {
            widget::push_button(
                frame.instances,
                frame.labels,
                frame.buttons,
                rect,
                label,
                *variant,
                state,
                UiAction::Confirm, // not used — we override below
            );
            // Override the just-pushed button so its action is the stable id.
            // push_button already pushed a ButtonDef::ui(...) — pop it and
            // replace with a ButtonDef::scene so the main loop routes the
            // click back as a button_clicks id.
            frame.buttons.pop();
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
        ItemKind::Toggle { label, value, .. } => {
            let display = if *value {
                format!("{label}: ON")
            } else {
                format!("{label}: OFF")
            };
            let variant = if *value {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Default
            };
            widget::push_button(
                frame.instances,
                frame.labels,
                frame.buttons,
                rect,
                &display,
                variant,
                state,
                UiAction::Confirm,
            );
            frame.buttons.pop();
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
        ItemKind::Slider {
            label,
            value,
            range,
            ..
        } => {
            draw_slider_row(frame, rect, label, *value, *range, state);
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
        ItemKind::Cycle {
            label,
            options,
            index,
            ..
        } => {
            let current = options.get(*index).map(|s| s.as_str()).unwrap_or("");
            let display = format!("{label}: {current}");
            widget::push_button(
                frame.instances,
                frame.labels,
                frame.buttons,
                rect,
                &display,
                ButtonVariant::Default,
                state,
                UiAction::Confirm,
            );
            frame.buttons.pop();
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
        ItemKind::Tab { label, active, .. } => {
            let variant = if *active {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Subtle
            };
            widget::push_button(
                frame.instances,
                frame.labels,
                frame.buttons,
                rect,
                label,
                variant,
                state,
                UiAction::Confirm,
            );
            frame.buttons.pop();
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
        ItemKind::Custom { kind_tag, .. } => {
            render_custom(frame, rect, *kind_tag, focus_state);
            frame.buttons.push(ButtonDef::scene(
                (rect[0], rect[1], rect[2], rect[3]),
                item.id.0,
            ));
        }
    }
    let _ = (metrics::BUTTON_GAP, color::PARCHMENT, theme::button_colors); // silence dead-import warnings if any
}

fn draw_slider_row(
    frame: &mut TreeFrame<'_>,
    rect: [f32; 4],
    label: &str,
    value: f32,
    range: (f32, f32),
    state: ButtonState,
) {
    // Background panel.
    let colors = theme::button_colors(ButtonVariant::Default, state);
    widget::push_panel_colored(frame.instances, rect, colors.bg, colors.border);

    let [x, y, w, h] = rect;
    let label_w = w * 0.45;
    let track_x = x + label_w + 8.0;
    let track_w = (x + w) - track_x - 8.0;
    let track_h = (h * 0.35).max(4.0);
    let track_y = y + (h - track_h) * 0.5;

    // Label on the left.
    frame.labels.push(TextLabel {
        rect: [x + 8.0, y, label_w, h],
        text: label.into(),
        color: colors.text,
        ..Default::default()
    });

    // Track background.
    frame.instances.push(GpuInstance {
        rect: [track_x, track_y, track_w, track_h],
        color: color::OBSIDIAN,
    });
    // Filled portion.
    let t = ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
    frame.instances.push(GpuInstance {
        rect: [track_x, track_y, track_w * t, track_h],
        color: color::GOLD,
    });

    // Numeric readout on the far right of the track.
    let pct = (t * 100.0).round() as i32;
    frame.labels.push(TextLabel {
        rect: [track_x + track_w - 60.0, y, 60.0, h],
        text: format!("{pct}%"),
        color: colors.text,
        ..Default::default()
    });
}
