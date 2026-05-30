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
| `btn_cash_in` / `label_cash_in` | Authored cash-in control (env mesh + engraved label) |
| `player_relic` … `player_relic.004` | Relic medallions (up to five) |
| `player_consumables` / `.001` | Owned consumable spawns (ribbon / talisman; porcelain dish is static env) |
| `player_yaku_journal` | Procedural yaku journal book (`Object3dKind::Book`; spine label "Yaku"; opens yaku journal on click) |
| `player_guidebook` | Procedural guide book (`Object3dKind::Book`; spine label "Guide"; opens guide on click) |
| `score_cascade_reel` | Score odometer fly-to anchor during cascade hand-off |
| `score_pops_lerp_target` | Cascade score-popup stream destination (`+50`, `×3`, …) |
| `score_cascade_pad` | Center of the chips × mult HUD trio |
| `score_cascade_chips` / `score_cascade_mult` | Chip / mult accumulator token destinations |
| `score_cascade_src_chips` / `score_cascade_src_mult` / `score_cascade_src_misc` | Popup launch points for steps not tied to structure tiles, relics, or yaku tablets |

Static geometry (table surface, dishes, candles, score `frame`, …) is part of the environment mesh. Do not duplicate those with procedural `Object3d`.

**Unexportables** (Blender collection): authoring-only layout that must not ship. **Exclude from glTF export** when possible. If an `Unexportables` root node appears anyway, [`walk_room_env_node`](../../crates/mahjuro-render/src/room_env_gltf.rs) skips that entire subtree (no error). All other exported meshes draw unless they are boolean **`subtractor`** operands ([`skip_room_env_authoring_mesh_node_name`](../../crates/mahjuro-render/src/room_env_gltf.rs)). Round score (`0 / 500`) is **2D HUD text** only.

Dynamic spawns use [`GameplayGlbAnchors`](../../src/scenes/gameplay/glb_anchors.rs) (built from [`GameplayMarkerPose`](../../src/render/gameplay_glb.rs): surface anchor + euler rotation + scale per empty) — [`GameplayPositions`](../../src/ui/scene_layout/gameplay.rs) keeps candle tuning and score-reel lift, not prop placement.

In Blender, set **location, rotation, and scale** on each spawn empty; the exporter writes them into the glTF node transform. Uniform scale on an empty sizes hand tiles, relics, plinth indicators, coin piles, and consumable slots; rotation drives hand tilt, tally fans, pick proxies, and consumable orientation.

Hand tile **mesh** scale is derived from the `hand_tiles_*` strip span in [`hand_layout.rs`](../../src/scenes/gameplay/hand_layout.rs): ideal at **14** tiles, capped so shorter hands (12–13) do not grow past that size, and eased so 15–20 tile hands stay readable without dominating the rack.

## Camera & lights

- Embedded **glTF perspective camera**: Blender object `default` with a real Camera data-block (exports as a glTF `camera` extension on that node — not an empty). Orthographic cameras are rejected.
- `light_candle*` punctual lights drive `room_glb.wgsl` and procedural flame particles (same path as shop candles).
- When embedded punctual lights are present, gameplay uses the **same env tonemap + attenuation** as the shop.

## Export notes

- Export **without Draco** (`KHR_draco_mesh_compression`).
- Keep spawn nodes as **empties** (no mesh) unless you also want invisible collision — marker policy is `SkipDrawCollisionIfMarker`.
- All spawn empties in the table above must be present in every export; missing nodes fail load at startup.
