# Mahjuro — Agent Notes

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

## Mouse Input in Scenes

Scene mouse input is wired through `ButtonDef`s returned from `Scene::draw()`. The main loop hit-tests the cursor against these rects and routes the click to the next `update()` call. There are two routing modes, controlled by the `ButtonAction` variant on each `ButtonDef`:

- **`ButtonAction::Ui(action)`** (use `ButtonDef::ui(rect, action)`) — enqueues `action` into `UpdateCtx::actions`, indistinguishable from a key/gamepad press. Use this when the click is semantically the same as some keyboard input the scene already handles. This is the common case.
- **`ButtonAction::Scene(id)`** (use `ButtonDef::scene(rect, id)`) — enqueues `id` into `UpdateCtx::button_clicks`, where the scene's `update()` matches against its own named const values. Use this when N buttons each need a different effect that has no natural keyboard equivalent (e.g. tab clicks in `collection.rs`, where each tab jumps to a specific tab rather than cycling).

There is no separate "click event" channel beyond these two. Every interactive scene must handle three things:

### 1. Every clickable element needs a `ButtonDef`

If you draw a card/button/tab and don't push a `ButtonDef` for it, it will be invisible to the mouse. Pure keyboard nav is *not* enough — even single-action screens (game over, results) need at least one whole-screen `ButtonDef` so the mouse can dismiss them. The bug pattern to watch for is `buttons: vec![]` in a scene that has visible interactive elements.

### 2. Mouse hover must update the focus index *before* the action loop

Most scenes track a `cursor`/`focused_idx`/`skip_focused` field that determines what `UiAction::Confirm` acts on. When the player clicks a card, the click is enqueued as `Confirm` — but `Confirm` reads `self.cursor`, which by default still points at whatever the *keyboard* last selected. So clicking card 3 might buy card 1.

The fix is for `update()` to hit-test `ctx.cursor_pos` against the same rects `draw()` uses, and update the focus index *before* processing the action queue. Pattern (see `shop.rs`, `results.rs`, `options.rs`):

```rust
let (cx, cy) = ctx.cursor_pos;
for (i, &(rx, ry, rw, rh)) in self.item_rects(ctx.layout).iter().enumerate() {
    if cx >= rx && cx <= rx + rw && cy >= ry && cy <= ry + rh {
        self.cursor = i;
        break;
    }
}
for a in ctx.actions { /* ... */ }
```

This also makes the gold focus highlight follow the mouse, which is the expected feel. **Extract a `fn item_rects(&self, &LayoutResult) -> Vec<(f32,f32,f32,f32)>` helper** so the same layout math is used in `update()` (hit-test), `draw()` (rendering), and `draw()` again (button registration). Three copies of the math will drift.

With hover-focus in place, almost every clickable button can be a plain `ButtonDef::ui(rect, UiAction::Confirm)` — the cursor index already disambiguates *which* button is being clicked.

### 3. When to use `ButtonAction::Scene(id)`

Reach for scene click ids only when the cursor-index pattern doesn't fit — typically when clicking a button must take an action *without first focusing it*, so hover-focus isn't a viable disambiguator. The canonical example is `collection.rs` tab clicks: hovering a tab shouldn't switch tabs (that would be jarring), but clicking it should jump directly to that tab regardless of which tab is "focused".

In that case:

1. Define `const CLICK_FOO: u32 = 0; const CLICK_BAR: u32 = 1;` at the top of the scene file. Numbering is local to the scene — collisions across scenes are fine.
2. Register buttons with `ButtonDef::scene(rect, CLICK_FOO)`.
3. In `update()`, iterate `for &id in ctx.button_clicks { match id { CLICK_FOO => ..., _ => {} } }`.

Do **not** revive the old pattern of hijacking unrelated `UiAction` variants (`ScoreHand` for "Restart", `SortBySuit` for "Main Menu", etc.). It exists nowhere in the codebase anymore and shouldn't come back — `UiAction`s are global, so any scene that hijacks one becomes vulnerable to that key/gamepad binding firing the wrong action from another context.

### Where to look for examples

- `options.rs` — best-in-class hover-to-focus via `hover_row(ctx.cursor_pos)`.
- `shop.rs` / `results.rs` — cursor-index pattern with hover hit-test and shared rect helper.
- `start_screen.rs` / `profile_select.rs` / `pause_menu.rs` — hover-focus + plain `Confirm` clicks.
- `collection.rs` — `ButtonAction::Scene(id)` for tab clicks where hover-focus doesn't apply.
