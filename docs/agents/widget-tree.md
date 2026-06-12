# Mouse / Keyboard input in scenes — Widget Tree

Scene UI input is driven by **[`src/ui/widget_tree.rs`](../../src/ui/widget_tree.rs)** — a generic, immediate-mode widget system parameterized over a scene-defined `Action` enum. Each frame the scene builds a tiny `Tree<A>` (or list of `FlatItem<A>`) from current state, hands it to a persistent `TreeState`, and gets back `Option<A>` describing what (if anything) the user activated.

`TreeState` owns layout, hit-testing, hover-follow, keyboard navigation, slider/toggle/cycle adjustment, and click-id registration. Scenes never duplicate layout math between `update()` and `draw()`, never juggle named `usize` const indices, and never lose sync between hover hit-tests and rendered rects.

Shared spatial-nav stash for hybrid 3D scenes: **[`RectFocusSession`](../../src/ui/focus_nav/rect_focus.rs)** in `focus_nav/` (draw-stashed rects + cursor hit-test). Shop stock focus uses it alongside `chrome_tree: TreeState` for HUD buttons.

## Two flavors

There are two entry points depending on how a scene's geometry is computed:

1. **`Tree<A>` — declarative layout** (`Tree::vertical_menu`, `Tree::anchored`, with `wt::button_id`, `wt::slider`, `wt::toggle`, `wt::cycle`, `wt::tab` builders). Use when you want the tree to *compute* the rects: vertical menus, anchored modals, generic Column/Row/Grid containers. See [`pause_menu.rs`](../../src/scenes/pause_menu.rs).

2. **`FlatItem<A>` — bring-your-own rects** (`TreeState::update_flat`, `TreeState::register_flat_buttons`, optionally `update_flat_with_edges`). Use when the rects come from external geometry — GLB-projected hand slots, custom card grids, hand-laid tab bars. The scene supplies `Vec<FlatItem { id, rect, action }>`; the tree handles hover, focus, click routing, and keyboard linear nav. See [`main_menu.rs`](../../src/scenes/main_menu.rs), [`options.rs`](../../src/scenes/options.rs) (chrome only), [`shop/focus.rs`](../../src/scenes/shop/focus.rs) (HUD chrome), [`archive.rs`](../../src/scenes/archive.rs), [`profile_select.rs`](../../src/scenes/profile_select.rs), [`run_summary.rs`](../../src/scenes/run_summary.rs), [`credits.rs`](../../src/scenes/credits.rs).

## Hybrid scenes (tree + custom nav)

Some scenes combine widget-tree chrome with domain-specific focus graphs:

- **Options** — `FlatItem` for TOC, bottom bar, and all content rows; slider/toggle adjustment stays manual.
- **Shop** — `chrome_tree` for Leave/Restock/Journal/Wall HUD; stock/inventory uses `RectFocusSession` + [`shop_pool_nav`](../../src/scenes/shop/focus.rs) and 3D picks.
- **Archive** — flat items for chrome + artifact grid; `update_flat_with_edges` for tab links; grid paging stays manual.
- **Gameplay** — bottom action bar + 3D picks; input lives under [`gameplay/input/`](../../src/scenes/gameplay/input/) (not FlatItem — heterogeneous `FocusTarget`).

## The contract

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainAction { Play, Options, Quit }

impl MainAction { fn id(self) -> FocusId { FocusId(self as u32 + 1) } }

pub struct MyScene { tree: TreeState, /* … */ }

impl MyScene {
    /// Single source of truth — used by both update() and draw().
    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<MainAction>> {
        vec![
            FlatItem::new(MainAction::Play.id(),    [/*rect*/], MainAction::Play),
            FlatItem::new(MainAction::Options.id(), [/*rect*/], MainAction::Options),
            FlatItem::new(MainAction::Quit.id(),    [/*rect*/], MainAction::Quit),
        ]
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(&items, TreeInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (ctx.layout.window_w, ctx.layout.window_h),
            input_mode: ctx.input_mode,
            scroll_lines: 0.0,
        });
        match action {
            Some(MainAction::Play)    => Some(/*…*/),
            Some(MainAction::Options) => Some(/*…*/),
            Some(MainAction::Quit)    => { *ctx.quit_requested = true; None }
            None => None,
        }
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let mut buttons = Vec::new();
        // … render visuals using the SAME rects from flat_items() …
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        self.tree.register_flat_buttons(&items, &mut buttons);
        SceneDrawOutput { buttons, /* … */ }
    }
}
```

## Rules

1. **Every interactive element must appear in the flat-item list / tree.** The tree is what registers `ButtonDef::scene` hit targets with the main loop — anything not in the list is invisible to the mouse. Even single-action screens (run summary) need at least one whole-screen item.

2. **Build the item list from a single helper** (`flat_items` / `build_tree`). Both `update()` and `draw()` call it. This is the *whole point* of the system — there should be exactly one place that knows the rects.

3. **Each item's `FocusId` must be stable across rebuilds.** Derive it from the action discriminant so the focused item survives a rebuild that reorders the list.

4. **Hover-follow is automatic** (cursor mode). Keyboard `Confirm` acts on the tree's focused item.

5. **Keyboard nav is automatic** for flat lists via inferred rows/columns (`focus_nav::graph`).

6. **`Esc`/`Pause` shortcuts stay in the scene** when they need special semantics.

7. **Heterogeneous rows (sliders/toggles/cycles) — handle adjustment in the scene.** See [`options.rs`](../../src/scenes/options.rs) `update_input`.

8. **Scrollable flat lists (keyboard/controller):**
   - Put **every** focus target in the flat list with scroll-adjusted rects (see [`yaku_journal.rs`](../../src/scenes/yaku_journal.rs), [`options.rs`](../../src/scenes/options.rs) content rows) — not only the visible viewport slice.
   - When focus moves via keyboard, call [`clamp_index_into_viewport`](../../src/ui/focus_nav/flat_scroll.rs) and rebuild items **before** `update_flat`.
   - `TreeState::update_flat` preserves an offscreen focus id; it no longer jumps to the first on-screen item.

9. **Cursor-hover tooltips are opt-in** via `FlatItem::with_tooltip`.

## Anti-patterns to avoid

- **Manual `cursor: usize` index across heterogeneous items.** Use action enum + `FocusId`.
- **Duplicating layout math in `update()` and `draw()`.**
- **Direct `ButtonDef::scene` for tree-managed chrome** — use `register_flat_buttons`.
- **Hijacking unrelated `UiAction` variants** for scene-specific buttons.

## Where to look for examples

- [`main_menu.rs`](../../src/scenes/main_menu.rs) — hub menu via `FlatItem<HubFocus>`.
- [`pause_menu.rs`](../../src/scenes/pause_menu.rs) — declarative `Tree::vertical_menu`.
- [`options.rs`](../../src/scenes/options.rs) — unified `FlatItem` nav + manual row adjustment.
- [`shop/focus.rs`](../../src/scenes/shop/focus.rs) — HUD chrome flat items + stock session.
- [`archive.rs`](../../src/scenes/archive.rs) — hybrid chrome + grid (`update_flat_with_edges` for tabs).
- [`wall/focus.rs`](../../src/scenes/wall/focus.rs) — grid edges via `update_flat_with_edges`.
- [`profile_select.rs`](../../src/scenes/profile_select.rs) / [`run_summary.rs`](../../src/scenes/run_summary.rs) / [`credits.rs`](../../src/scenes/credits.rs) — small flat-item scenes.
