# Font scaling

Text labels are rasterized via `rasterize_label` ([src/render/decal.rs](../../src/render/decal.rs)), which sizes the font at `min(height * 0.55, width * 1.5 / char_count)`. This means:

- **Tall, narrow rects with long text produce tiny fonts.** The width cap dominates and the text becomes illegible.
- **Always use screen-proportioned rects for text.** Don't pass raw hand-slot rects directly as text label rects when the text is more than a few characters.
- **Split long content into multiple labels.** For example, put the name in one label and the price in another, each with its own sub-region of the card.
- **Use short, wide rects** (e.g. 20-25% of slot height) for readable text bands within a UI card or slot.
