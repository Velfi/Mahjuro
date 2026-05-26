# Gameplay table (`gameplay.glb`)

**Asset:** [`assets/3d/gameplay.glb`](../../assets/3d/gameplay.glb) (export from [`Gameplay.blend`](../../assets/3d/source/Gameplay.blend)).

**Runtime:** [`src/render/gameplay_glb.rs`](../../src/render/gameplay_glb.rs) decodes through the shared room GLB path (`room_glb.wgsl`). [`GameplayScene::draw_frame`](../../src/scenes/gameplay/scene_behavior.rs) pushes [`DrawCmd::GameplayEnvironment`](../../src/render/draw_cmd.rs) when the file loads and passes validation. If the file is present but any required empty is missing, gameplay shows an error screen — no procedural fallback.

## Spawn empties (dynamic props)

Blender object names must match glTF node names exactly:

| Node | Spawn |
|------|--------|
| `hand_tiles_left` / `hand_tiles_right` | Hand rack layout (position lerped; rotation slerped per slot; scale from left) |
| `structure_tiles_left` / `structure_tiles_right` | Open-meld showcase tiles (anchor + rotation lerped per tile along the row; scale from left) |
| `yaku_tablets_left` / `yaku_tablets_right` | Yaku bone tablets (anchor + rotation lerped per tablet; scale from left) |
| `tile_plinth` / `.001` / `.002` | Dora / round-wind / boss indicator tiles (anchor + rotation + scale per empty) |
| `discard_river` / `play_mirror` | Procedural discard bowl + play mirror (`Object3dKind::Bowl` / `Mirror`) |
| `player_gold` | Coin pile |
| `player_discard_tally` / `player_play_tally` | Discard / play tally-stick fans |
| `player_cash_in` | Cash-in button spawn |
| `player_relic` … `player_relic.004` | Relic medallions (up to five) |
| `player_consumables` / `.001` | Owned consumable spawns (ribbon / talisman; porcelain dish is static env) |
| `player_yaku_journal` | Procedural journal book (`Object3dKind::Book`; opens yaku journal on click) |

Static geometry (table surface, dishes, candles, …) is part of the environment mesh. Do not duplicate those with procedural `Object3d`.

**Unexportables** (Blender collection): layout-preview meshes for tiles, relics, score plaque parts, bowl, mirror, wood tablet, etc. Keep them in the `.blend` for authoring but **disable that collection for glTF export** — if any Unexportables mesh is present, [`load_gameplay_glb_from_bytes`](../../src/render/gameplay_glb.rs) fails at decode. The round score (`0 / 500`) is **2D HUD text** only.

Dynamic spawns use [`GameplayGlbAnchors`](../../src/scenes/gameplay/glb_anchors.rs) (built from [`GameplayMarkerPose`](../../src/render/gameplay_glb.rs): surface anchor + euler rotation + scale per empty) — [`GameplayPositions`](../../src/ui/scene_layout/gameplay.rs) keeps candle tuning and score-reel lift, not prop placement.

In Blender, set **location, rotation, and scale** on each spawn empty; the exporter writes them into the glTF node transform. Uniform scale on an empty sizes hand tiles, relics, plinth indicators, coin piles, and consumable slots; rotation drives hand tilt, tally fans, pick proxies, and consumable orientation.

## Camera & lights

- Embedded **glTF perspective camera**: Blender object `default` with a real Camera data-block (exports as a glTF `camera` extension on that node — not an empty). Orthographic cameras are rejected.
- `light_candle*` punctual lights drive `room_glb.wgsl` and procedural flame particles (same path as shop candles; wax uses `textures/shop/candle_sss.png` on `Candle*` meshes).
- When embedded punctual lights are present, gameplay uses the **same env tonemap + attenuation** as the shop.

## Export notes

- Export **without Draco** (`KHR_draco_mesh_compression`).
- Keep spawn nodes as **empties** (no mesh) unless you also want invisible collision — marker policy is `SkipDrawCollisionIfMarker`.
- All spawn empties in the table above must be present in every export; missing nodes fail load at startup.
