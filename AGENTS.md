# Mahjuro — Agent Notes

Short pointers to deeper context. Read the linked file before working in the area.

- [3D world space](docs/agents/world-space.md) — Z-up frame, table at `z = 0`, `WorldSurfaceAnchor` packing, `table_transform.rs` for mesh orientation.
- [Scene placement](docs/agents/scene-placement.md) — screen vs world coordinates, `Object3d` euler vs `Placement` degrees, `PlacementAnchor` + arrange mode workflow.
- [Font scaling](docs/agents/font-scaling.md) — `rasterize_label` width cap shrinks text in tall/narrow rects; use wide rects and split long content.
- [Card / UI sizing](docs/agents/card-sizing.md) — `card_rect()` from `scenes/mod.rs` for menu cards spanning multiple hand slots.
- [Relic display](docs/agents/relic-row.md) — gameplay horizontal 3D tray vs shop 3D props (no shared `relic_row()` helper).
- [Widget tree (scene input)](docs/agents/widget-tree.md) — `Tree<A>` / `FlatItem<A>` immediate-mode UI; single source of truth for rects, automatic hover/keyboard nav.
- [macOS dylibs / app bundle](docs/agents/macos-dylibs.md) — `libsteam_api.dylib`, static SDL3, `@loader_path` / `@rpath`, CI vs `package-macos.sh`.
- **Asset packs** — `build.rs` runs `tools/bake_assets/bake_assets.py` into the Cargo output dir; runtime: `src/asset_sources.rs`, `tools/bake_assets/README.md`. Override with `MAHJURO_ASSETS` / `MAHJURO_SKIP_ASSET_BAKE`.
- Headless **`screenshot`** subcommand — use **`cargo build --release`** and `./target/release/mahjuro screenshot …`; debug builds pay a large cost each run (shader/pipeline init). Each invocation cold-starts `WgpuRenderer`; batch many outputs in one process only if we add a batch mode later.
