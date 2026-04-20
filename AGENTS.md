# Mahjuro — Agent Notes

Short pointers to deeper context. Read the linked file before working in the area.

- [3D world space](docs/agents/world-space.md) — Z-up frame, table at `z = 0`, `WorldSurfaceAnchor` packing, `table_transform.rs` for mesh orientation.
- [Font scaling](docs/agents/font-scaling.md) — `rasterize_label` width cap shrinks text in tall/narrow rects; use wide rects and split long content.
- [Card / UI sizing](docs/agents/card-sizing.md) — `card_rect()` from `scenes/mod.rs` for menu cards spanning multiple hand slots.
- [Relic display row](docs/agents/relic-row.md) — `relic_row()` helper for the badge strip below the score panel.
- [Widget tree (scene input)](docs/agents/widget-tree.md) — `Tree<A>` / `FlatItem<A>` immediate-mode UI; single source of truth for rects, automatic hover/keyboard nav.
