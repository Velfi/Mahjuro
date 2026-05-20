# Mahjuro — Agent Notes

Short pointers to deeper context. Read the linked file before working in the area.

- [3D world space](docs/agents/world-space.md) — Z-up frame, table at `z = 0`, `WorldSurfaceAnchor` packing, `table_transform.rs` for mesh orientation.
- [Scene placement](docs/agents/scene-placement.md) — screen vs world coordinates, `Object3d` euler vs `Placement` degrees, `PlacementAnchor`.
- [Font scaling](docs/agents/font-scaling.md) — `rasterize_label` width cap shrinks text in tall/narrow rects; use wide rects and split long content.
- [Widget tree (scene input)](docs/agents/widget-tree.md) — `Tree<A>` / `FlatItem<A>` immediate-mode UI; single source of truth for rects, automatic hover/keyboard nav. **Styled copy:** `widget::push_text_block` runs safe inline markup (`**bold**`, `*italic*`, `__underline__`, `{{effect:rainbow}}…{{/effect}}`); see [`src/ui/styled_text.rs`](src/ui/styled_text.rs). `widget::push_button` labels stay plain.
- [macOS dylibs / app bundle](docs/agents/macos-dylibs.md) — `libsteam_api.dylib`, static SDL3, `@loader_path` / `@rpath`, CI vs `package-macos.sh`.
- **Room vertex warp + shadows** — `shaders/hallway_vertex_warp.wgsl` is prepended into `room_glb` / `tile_3d` / `shadow.wgsl` (`embedded_wgsl.rs`). The shadow depth pass binds the same `HallwayDistortion` bytes as the lit room pass (`ShopEnvironmentGpu.distortion_buffer` → group 1); tiles and lit-mesh casters use `shadow_warp_disabled_bind_group`. Any new scene warp must keep lit and shadow VS in sync.