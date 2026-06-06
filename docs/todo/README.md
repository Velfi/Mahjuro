# Deferred work

Follow-up tasks that are scoped but not scheduled. Each entry links to a doc with the *why*, *scope*, and *touchpoints* so a future contributor can pick it up cold.

## Refactors

- [Event-driven run mutations (subscribers beyond gold)](event-driven-run-mutations.md) — centralize relic destruction, optional plays/discards and tile-supply hooks the same way gold now uses `notify_run_gold_changed` + `GameEvent::GoldChanged`.
- [Room-bake compile surface trim](room-bake-compile-trim.md) — shrink what `mahjuro-bake` links: split SDL input, cfg-gate scene modules, optional minimal render frames; `RoomBakeApp` + `mahjuro::room_bake` already landed.

## Tooling / quality

- [Clippy — structural refactors](clippy-structural-refactors.md) — param structs for 8+ arg functions, `GltfMipChain` alias, box heavy `Scene` variants; ~56 warnings after mechanical fixes; no crate-level allows.

## Rendering / shaders

- [MSDF UI text and baked glyph atlas](msdf-ui-text-glyph-atlas.md) — replace per-label fontdue bitmaps with a baked MSDF atlas + runtime glyph cache; optional nearest/pixel-snap bridge; density-aware tile decal bakes.
- [Archive offline baked directional shadows](archive-offline-baked-shadows.md) — re-enable `archive.msh` after a proper caster/receiver split in `archive.glb`; current workaround is punctual-only room lighting.
- [Emissive materials — global illumination transport](emissive-materials-gi-transport.md) — glTF emissive should contribute incident light via SSGI / bake / area-light techniques, not only self-emission and bloom; shop/hallway first.
- [Journal prepass vs SSR history isolation](journal-prepass-ssr-history-isolation.md) — offscreen journal render must not publish `scene_prev` / SSR depth; split main vs auxiliary `render_to` or gate snapshot so lacquer SSR stays stable while the shop book is focused.

## Catalog / content

- [Unified HTML editor for ship game data JSON](game-data-json-editor.md) — extend the relic flavor editor shell to `bosses.json` / `yaku.json` (row pick + manifest-driven fields) while keeping `flavor_spans` WYSIWYG for relics.

