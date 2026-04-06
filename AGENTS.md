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
