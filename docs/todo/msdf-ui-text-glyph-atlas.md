# MSDF UI text and baked glyph atlas

## Why

HUD copy is already laid out in drawable pixels, but each [`TextLabel`](../../src/render/wgpu_renderer/internal_slots.rs) is still rasterized with fontdue into a full-string RGBA texture, then drawn with the **linear** [`tile_sampler`](../../src/render/wgpu_renderer/init/build.rs). That softens glyph edges at 1:1 and makes sharpness depend on fractional quad placement. The per-label [`text_label_cache`](../../src/render/wgpu_renderer/runtime/render.rs) also grows with strings × rect shapes × font sizes. Tile and plaque labels use **fixed-resolution** CPU bakes (e.g. 192×256 in [`showcase_decal_atlas.rs`](../../src/render/showcase_decal_atlas.rs)), so table-facing text cannot stay crisp when tiles are large on HiDPI. A single GPU glyph pipeline (MSDF atlas + instanced quads) fixes HUD sharpness long-term; a build bake step covers closed sets (core charset, tile faces) while a small runtime cache handles dynamic UI.

## Scope

1. **Near-term bridge (optional, can ship alone):** Add a dedicated `text_sampler` (`FilterMode::Nearest`) for [`make_text_draw`](../../src/render/wgpu_renderer/runtime/render.rs) only; pixel-snap label rects before rasterize and draw. Skip or relax snap when `scroll_offset != 0` to avoid scroll jitter.
2. **Build bake — core atlas:** Script or `mahjuro bake-ui-glyphs` CLI (pattern: [`build/room_shadow_bake.rs`](../../build/room_shadow_bake.rs), [`src/main/cli.rs`](../../src/main/cli.rs)) emitting `assets/data/ui_glyphs/` — MSDF atlas PNG + metrics JSON for Instrument Serif regular/italic at a reference em size, plus a manifest charset (ASCII, UI punctuation, bounded CJK if needed).
3. **Packaging:** Include baked glyphs in release packs via [`tools/bake_assets/bake_assets.py`](../../tools/bake_assets/bake_assets.py) / `pack_rules.json`; stamp inputs in `build.rs` so dev rebuilds stay fresh.
4. **Runtime loader + cache:** Load baked atlas at init; `ensure_glyph(ch, face)` uploads misses (scores, names, rare emoji) into atlas slots — do **not** prebake every string the player will see.
5. **Draw path migration:** Replace per-label RGBA uploads with instanced glyph quads (extend [`shaders/text_quad.wgsl`](../../shaders/text_quad.wgsl) or successor): MSDF threshold, pixel-snapped positions, preserve [`TextEffectId`](../../src/render/text_effect.rs) in the fragment stage. Route [`push_styled_text_block`](../../src/ui/styled_text.rs) / widget tree output through glyph runs instead of [`TextLabel`](../../src/render/wgpu_renderer/internal_slots.rs) bitmaps; retire or shrink `text_label_cache`.
6. **Tile / plaque decals:** Rebake showcase cells at `base_px × pixel_density` **or** sample the shared MSDF atlas on face UVs; wire into [`prebake_showcase_decal_atlases_for_all_player_tilesets`](../../src/render/wgpu_renderer/impl_public.rs) or move that work fully into the build bake so splash does not pay CPU cost.
7. **Emoji:** Color bitmap strip in bake for common UI symbols; runtime fallback for rare codepoints (same split as today’s Noto path in [`decal.rs`](../../src/render/decal.rs)).

Out of scope: changing 3D score popups ([`glyph_mesh.rs`](../../src/render/glyph_mesh.rs) vector extrusion — already resolution-independent). Out of scope: rebaking room shadows / GI. Out of scope: hinting in fontdue (no TrueType hints; MSDF replaces that path for flat UI).

## Touchpoints

- [src/render/wgpu_renderer/runtime/render.rs](../../src/render/wgpu_renderer/runtime/render.rs) — `make_text_draw`, `text_label_cache`, `DrawCmd::Text` handling.
- [src/render/wgpu_renderer/init/build.rs](../../src/render/wgpu_renderer/init/build.rs) — `text_bind_group_layout`, `text_sampler`, text pipelines.
- [shaders/text_quad.wgsl](../../shaders/text_quad.wgsl) — vertex placement, MSDF sampling, text effects.
- [src/render/wgpu_renderer/internal_slots.rs](../../src/render/wgpu_renderer/internal_slots.rs) — `TextLabel`, `TextLabelShapeKey`, `CachedTextLabel`.
- [src/render/decal.rs](../../src/render/decal.rs) — `rasterize_label_*`, `load_ui_font`, tile/plaque decal raster paths.
- [src/render/showcase_decal_atlas.rs](../../src/render/showcase_decal_atlas.rs) — `DECAL_W`/`DECAL_H`, `build_showcase_decal_atlas_texture`.
- [src/ui/styled_text.rs](../../src/ui/styled_text.rs), [src/ui/widget_tree.rs](../../src/ui/widget_tree.rs) — layout → label emission.
- [src/render/theme.rs](../../src/render/theme.rs) — `typography::size` tiers (scale quads, not per-size atlas bakes).
- [src/sdl_shell.rs](../../src/sdl_shell.rs) — `drawable_size`, `high_pixel_density`, `pixel_density` (coordinate space stays drawable pixels).
- [src/main/cli.rs](../../src/main/cli.rs), [src/main/commands.rs](../../src/main/commands.rs) — new `BakeUiGlyphs` command.
- [build/](../../build/) — optional `ui_glyphs_bake.rs` stamp hook (mirror room shadow bake).
- [tools/bake_assets/bake_assets.py](../../tools/bake_assets/bake_assets.py) — pack baked `assets/data/ui_glyphs/`.
- [fonts/](../../fonts/) — Instrument Serif sources for the bake manifest.

## Open questions

- **glyphon vs custom MSDF.** Use [`glyphon`](https://github.com/grovesNL/glyphon) (cosmic-text + wgpu) for atlas/layout vs a small in-repo baker (msdfgen / export tool). Custom keeps styled-markup control; glyphon is less glue for basic UI.
- **Build charset size.** Ship a minimal Latin+punctuation bake only, or include a large CJK block up front to avoid runtime uploads on first Chinese UI string?
- **Pixel snap during scroll.** Snap all HUD labels for sharpness vs exempt scrolled widget-tree labels to avoid 1px stepping — hybrid rules need a clear policy.
- **Tile decals: density bake vs atlas UVs.** Short-term 2× raster bake into existing atlas is cheap; sharing the UI MSDF atlas on tile faces is the unified end state but needs UV/shader work on tile quads.
- **Migration order.** Spike one screen (e.g. pause menu) on glyph instances before converting chronicle/shop, or bridge with nearest+snap until MSDF lands?
