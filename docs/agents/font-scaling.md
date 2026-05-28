# Font scaling and text layout

## GPU labels (`TextLabel` / `rasterize_label`)

When `font_px` is `None`, the renderer auto-shrinks from the rect:

```text
font_px ≈ min(rect.h * 0.55, rect.w * 1.5 / char_count)
```

Tall/narrow rects therefore produce tiny text. Prefer **wide rects**, split long copy across lines, or pin `font_px = Some(px)` and let the rasterizer shrink uniformly down to the readable floor when overflow remains.

See [`TextLabel`](../../crates/mahjuro-render/src/wgpu_renderer/internal_slots.rs) and [`widget::push_text_block`](../../src/ui/widget.rs).

## CPU layout height helpers

Do **not** invent ad-hoc vertical multipliers at call sites (e.g. `line_h * 1.4`). Use the canonical helpers so `update()` reserve math matches `draw()`:

| Content | Measure | Push / wrap |
|---------|---------|-------------|
| Tinted vocabulary lines | [`colored_keywords::colored_line_block_height`](../../src/ui/colored_keywords.rs) | [`push_colored_line_left`](../../src/ui/colored_keywords.rs), [`colored_row_line_step`](../../src/ui/colored_keywords.rs) |
| Plain `wrap_text` blocks | [`widget::plain_text_block_height`](../../src/ui/widget.rs) | [`widget::PLAIN_TEXT_LINE_STEP_MUL`](../../src/ui/widget.rs) (1.22) |

For word-wrap at a fixed font size, use [`widget::wrap_text`](../../src/ui/widget.rs) — it measures at the render font size instead of going through auto-shrink measurement (which would never break lines).

## Styled blocks vs plain buttons

[`widget::push_text_block`](../../src/ui/widget.rs) runs safe inline markup via [`styled_text.rs`](../../src/ui/styled_text.rs). [`widget::push_button`](../../src/ui/widget.rs) labels stay plain.
