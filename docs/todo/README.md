# Deferred work

Follow-up tasks that are scoped but not scheduled. Each entry links to a doc with the *why*, *scope*, and *touchpoints* so a future contributor can pick it up cold.

## Refactors

- [Finish the wgpu render-runtime split](render-runtime-finish-split.md) — Object3d dispatch + encoder/passes are still in one ~5,000-line `render()`; first pass landed seven sibling modules.

## Rendering / shaders

- [Emissive materials — global illumination transport](emissive-materials-gi-transport.md) — glTF emissive should contribute incident light via SSGI / bake / area-light techniques, not only self-emission and bloom; shop/hallway first.
- [Journal prepass vs SSR history isolation](journal-prepass-ssr-history-isolation.md) — offscreen journal render must not publish `scene_prev` / SSR depth; split main vs auxiliary `render_to` or gate snapshot so lacquer SSR stays stable while the shop book is focused.
- [Porcelain crazing](porcelain-crazing.md) — Voronoi crack network + tea-stain tint + glaze-break in the porcelain shader branch, so antique ceramic relics can read as old rather than fresh-from-the-kiln.
- [Global tonemapper pass](tonemapper.md) — render the 3D pass into an offscreen Rgba16Float target and tonemap to the surface, so SDR and HDR look consistent without per-scene light retuning.

## Catalog / content

- [Silk Moth successor relic](silk-moth-successor-relic.md) — post-destruction successor for Silk Thread; cocoon → moth transformation, art rewrite for the parent, and a Steam achievement for the emergence.
- [Taotie successor relic](taotie-successor-relic.md) — post-destruction successor for Melting Ice; ice-thaws-to-bronze-glutton transformation, art rewrite for the parent, and a Steam achievement for the awakening.

