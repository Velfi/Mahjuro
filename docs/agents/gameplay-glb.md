# Gameplay table (`gameplay.glb`)

**Asset:** [`assets/3d/gameplay.glb`](../../assets/3d/gameplay.glb) (export from [`Gameplay.blend`](../../assets/3d/source/Gameplay.blend)).

**Runtime:** [`src/render/gameplay_glb.rs`](../../src/render/gameplay_glb.rs) decodes through the shared room GLB path (`room_glb.wgsl`). [`GameplayScene::draw_frame`](../../src/scenes/gameplay/scene_behavior.rs) pushes [`DrawCmd::GameplayEnvironment`](../../src/render/draw_cmd.rs) instead of the procedural walnut [`DrawCmd::Table`](../../src/render/draw_cmd.rs) when the file loads.

## Spawn empties (dynamic props)

Blender object names must match glTF node names exactly:

| Node | Spawn |
|------|--------|
| `hand_tiles_left` / `hand_tiles_right` | Hand rack slot layout |
| `structure_tiles_left` / `structure_tiles_right` | Open-meld showcase strip |
| `yaku_tablets_left` / `yaku_tablets_right` | Yaku bone tablets |
| `tile_plinth` / `.001` / `.002` | Dora / round-wind / boss indicator tiles |
| `discard_river` / `play_mirror` | Discard + play pick anchors (mesh is in the GLB) |
| `player_gold` | Coin pile |
| `player_discard_tally` / `player_play_tally` | Tally-stick fans |
| `player_relic` … `player_relic.004` | Relic medallions (up to five) |
| `player_consumables` / `.001` | Consumable dish slots |
| `player_yaku_journal` | Journal book anchor |

Static geometry (felt, dishes, candles, score plaque, cash-in control, …) is part of the environment mesh. Do not duplicate those with procedural `Object3d` when the GLB is active.

## Camera & lights

- Embedded perspective camera: node `Camera`.
- `light_candle*` punctual lights drive `room_glb.wgsl` and procedural flame particles (same path as shop candles; wax uses `textures/shop/candle_sss.png` on `Candle*` meshes).

## Export notes

- Export **without Draco** (`KHR_draco_mesh_compression`).
- Keep spawn nodes as **empties** (no mesh) unless you also want invisible collision — marker policy is `SkipDrawCollisionIfMarker`.
