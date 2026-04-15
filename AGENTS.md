# Mahjuro — Agent Notes

## 3D world space (HUD props)

**World space** is the shared right-handed **Z-up** 3D frame for gameplay and shop. The **table** (felt, wood, dishes) lies in the **XY** plane near **`z = 0`**. Meshes use packed [`WorldSurfaceAnchor`](src/render/draw_cmd.rs) values: `[pixel_x, pixel_y, lift]` where `lift` is height in world **+Z**. The renderer maps them with [`pixel_to_world`](src/render/world_space.rs). Lit-mesh model matrices use [`translate_rot_scale`](src/render/table_transform.rs) with world-space centers and rotations from the same helpers (no separate “Z-up wrapper” layer). Gameplay bottom-bar spacing and lift live in [`action_bar_layout.rs`](src/scenes/gameplay/action_bar_layout.rs).

**Conventions:** **+X** right, **+Z** up; larger layout `py` moves along **+Y** (screen-down on the felt). Mesh orientation uses [`table_transform.rs`](src/render/table_transform.rs); each `DrawCmd` placement type matches one rotation recipe there; do not invent parallel Euler orders.

## Font Scaling

Text labels are rasterized via `rasterize_label` (src/render/decal.rs), which sizes the font at `min(height * 0.55, width * 1.5 / char_count)`. This means:

- **Tall, narrow rects with long text produce tiny fonts.** The width cap dominates and the text becomes illegible.
- **Always use screen-proportioned rects for text.** Don't pass raw hand-slot rects directly as text label rects when the text is more than a few characters.
- **Split long content into multiple labels.** For example, put the name in one label and the price in another, each with its own sub-region of the card.
- **Use short, wide rects** (e.g. 20-25% of slot height) for readable text bands within a UI card or slot.

## Card / UI Element Sizing

Hand slots are each 1/14th of the window width — far too narrow for menu cards (shop items, blind choices, relic picks). Use `card_rect()` from `scenes/mod.rs` to compute wider card rects that span multiple slots:

- `card_rect(slots, center_slot, span)` — returns a `(x, y, w, h)` rect spanning `span` adjacent slots centered on `center_slot`, with vertical padding.
- For 3 cards, use centers `[2, 7, 11]` with span 3 — gives ~3/14 window width per card with good spacing.
- Place text in horizontal bands within the card (name at 20% down, detail at 55% down).

## Relic Display Row

Use `relic_row(relics, score_panel, window_w)` from `scenes/mod.rs` to render a horizontal row of relic badges below the score panel. Returns `(Vec<GpuInstance>, Vec<TextLabel>)` to extend into the scene output. Used in gameplay, shop, and pick-blind scenes.

## Mouse / Keyboard Input in Scenes — Widget Tree

Scene UI input is driven by **`src/ui/widget_tree.rs`** — a generic, immediate-mode widget system parameterized over a scene-defined `Action` enum. Each frame the scene builds a tiny `Tree<A>` (or list of `FlatItem<A>`) from current state, hands it to a persistent `TreeState`, and gets back `Option<A>` describing what (if anything) the user activated.

`TreeState` owns layout, hit-testing, hover-follow, keyboard navigation, slider/toggle/cycle adjustment, and click-id registration. Scenes never duplicate layout math between `update()` and `draw()`, never juggle named `usize` const indices, and never lose sync between hover hit-tests and rendered rects.

### Two flavors

There are two entry points depending on how a scene's geometry is computed:

1. **`Tree<A>` — declarative layout** (`Tree::vertical_menu`, `Tree::anchored`, with `wt::button_id`, `wt::slider`, `wt::toggle`, `wt::cycle`, `wt::tab` builders). Use when you want the tree to *compute* the rects: vertical menus, anchored modals, generic Column/Row/Grid containers. See [`start_screen.rs`](src/scenes/start_screen.rs), [`pause_menu.rs`](src/scenes/pause_menu.rs).

2. **`FlatItem<A>` — bring-your-own rects** (`TreeState::update_flat`, `TreeState::register_flat_buttons`). Use when the rects come from external geometry — `LayoutResult::hand_slots`, custom card grids, hand-laid tab bars. The scene supplies `Vec<FlatItem { id, rect, action }>`; the tree handles hover, focus, click routing, and keyboard linear nav. See [`shop.rs`](src/scenes/shop.rs), [`options.rs`](src/scenes/options.rs), [`collection.rs`](src/scenes/collection.rs), [`results.rs`](src/scenes/results.rs), [`pick_blind.rs`](src/scenes/pick_blind.rs), [`profile_select.rs`](src/scenes/profile_select.rs), [`game_over.rs`](src/scenes/game_over.rs).

### The contract

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

### Rules

1. **Every interactive element must appear in the flat-item list / tree.** The tree is what registers `ButtonDef::scene` hit targets with the main loop — anything not in the list is invisible to the mouse. Even single-action screens (game over) need at least one whole-screen item.

2. **Build the item list from a single helper** (`flat_items` / `build_tree`). Both `update()` and `draw()` call it. This is the *whole point* of the system — there should be exactly one place that knows the rects.

3. **Each item's `FocusId` must be stable across rebuilds.** Derive it from the action discriminant (`Action::Foo as u32 + 1`) so the focused item survives a rebuild that reorders the list. The tree resolves the focused id against the latest layout each frame; if the id no longer exists it falls back to the first item.

4. **Hover-follow is automatic.** The tree updates its focused id from `cursor_pos` *before* processing actions, so a keyboard `Confirm` always acts on whatever the mouse is over.

5. **Keyboard nav is automatic.** `FocusUp/Down/Prev/Next` move linearly through the layout-cache order. `Confirm`/`CommitDiscard` activate the focused item.

6. **`Esc`/`Pause` shortcuts stay in the scene.** The tree only handles activation. Scenes still loop over `ctx.actions` themselves to handle `Cancel`/`Pause`/scene-specific shortcuts.

7. **Heterogeneous rows (sliders/toggles/cycles) — handle adjustment in the scene.** `update_flat` only fires `Click(Row)` actions. For arrow-key slider adjustment or cycle stepping, read `self.tree.focused()` and apply the change in the scene's own action loop. See [`options.rs`](src/scenes/options.rs) `update_input` for the canonical pattern.

### Adding or reordering an item

1. Add an enum variant.
2. Add one line to the items list (or move an existing line — order is whatever you want).
3. Add one match arm in `update()` for the new action.

That's it. No const-index renumber, no `_COUNT` sibling, no parallel-array bookkeeping, no hover-rect duplication. The compiler enforces exhaustive action matching.

### Anti-patterns to avoid

- **Manual `cursor: usize` index across heterogeneous items.** Use the action enum + `FocusId` instead. The pre-migration shop scene had a unified `cursor` ranging across cards / owned-relics / next-button — fragile and replaced by `ShopFocus` derived from `tree.focused()`.
- **Duplicating layout math in `update()` and `draw()`.** AGENTS.md used to recommend a `fn item_rects()` helper as a workaround. The widget tree replaces that workaround entirely.
- **Direct `ButtonDef::ui` / `ButtonDef::scene` pushes from scene draw code.** Always go through `register_flat_buttons` (or `TreeState::draw` for full-tree scenes) so the click-id registration matches the action enum exactly. The only remaining direct callers should be `widget::push_button` for visuals — pop the synthetic `ButtonDef` it creates and let the tree register the real one.
- **Hijacking unrelated `UiAction` variants** (`ScoreHand` for "Restart", `SortBySuit` for "Main Menu", etc.). Doesn't exist anywhere now and shouldn't come back — `UiAction`s are global, so hijacking one makes the scene vulnerable to misfires from any other context.

### Where to look for examples

- [`start_screen.rs`](src/scenes/start_screen.rs) / [`pause_menu.rs`](src/scenes/pause_menu.rs) — vertical menu via `Tree::vertical_menu` with declarative `wt::button_id`.
- [`options.rs`](src/scenes/options.rs) — heterogeneous rows (sliders + toggles + cycles + back button) via `FlatItem` + scene-side adjustment loop. Best example of killing const-index bookkeeping.
- [`shop.rs`](src/scenes/shop.rs) — mixed flat list (buy cards + sell badges + next-round button) with `ShopFocus` for visual highlighting derived from `tree.focused()`.
- [`collection.rs`](src/scenes/collection.rs) — tabs + footer arrows + back button as flat items, with scene-owned grid card rendering (cards aren't interactive).
- [`results.rs`](src/scenes/results.rs) / [`profile_select.rs`](src/scenes/profile_select.rs) / [`game_over.rs`](src/scenes/game_over.rs) — small scenes, `FlatItem` from layout slots or window-sized dismiss targets.
- [`gameplay.rs`](src/scenes/gameplay.rs) — `GameplayButton` enum for the bottom button bar; uses `ButtonDef::ui` directly because each button maps to a unique semantic `UiAction` already.
