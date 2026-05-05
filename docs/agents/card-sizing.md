# Card / UI element sizing

Hand slots are each 1/14th of the window width — far too narrow for menu cards (shop items, blind choices, relic picks). Use `card_rect()` from [`scenes/mod.rs`](../../src/scenes/mod.rs) to compute wider card rects that span multiple slots:

- `card_rect(slots, center_slot, span)` — returns a `(x, y, w, h)` rect spanning `span` adjacent slots centered on `center_slot`, with vertical padding.
- For 3 cards, use centers `[2, 7, 11]` with span 3 — gives ~3/14 window width per card with good spacing.
- Place text in horizontal bands within the card (name at 20% down, detail at 55% down).
