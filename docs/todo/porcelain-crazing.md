# Porcelain crazing (subtle shatter on glazed ceramic)

## Why
Porcelain in Mahjuro currently renders as a clean glaze: a tight pinpoint highlight inside a wide wet-glaze lobe with a Fresnel rim. It reads as fresh-from-the-kiln chiclet, which is right for some props but wrong for any relic the player is meant to read as *old* — antique teacups, ancestral talismans, kiln-fired temple objects. Real aged porcelain has a Voronoi-like net of fine cracks across the glaze surface (crazing) caused by glaze/body shrinkage mismatch over decades. Without it, every porcelain object reads the same age and the material loses its potential to communicate history. This is the cheapest single addition that turns "ceramic" into "old ceramic."

## Scope
A new procedural detail layer inside the existing `is_porcelain` branch — no new resources, no pipeline changes. Three stacked tricks:

1. **Voronoi crack network.** 2D Voronoi (or Worley F2−F1) sampled in object-XY at ~5–10mm cells. Distance-to-cell-edge thresholded to a thin (~0.3px) line mask. Multiplied into albedo as a slight darken (not pure black). The same mask perturbs the surface normal a touch so the highlight breaks across each crack — that's what makes the line read as a real fracture rather than a painted decal.
2. **Stained crack tint.** Old crazing is rarely black; it's tea-stained amber/brown where the porcelain body absorbed liquid through the cracks over years. Tint the darken by roughly `vec3(0.55, 0.45, 0.35)`. Modulate stain darkness with a low-frequency noise so coverage is uneven (some areas pristine, others well-loved).
3. **Glaze-thickness break.** Where the crack lives the glaze is interrupted, so the wide-lobe and rim terms in the porcelain spec block should *dip*. Multiply both by `(1 - 0.6 * crack_mask)`. This sells "broken glaze surface" instead of "decal painted over glaze."

Per-instance knobs (probably packed into a material param or stolen from a `base_color` channel that porcelain doesn't otherwise need):
- `crazing_density` — Voronoi cell size; small = fine spider-web, large = bold cracks.
- `crazing_age` — stain darkness + coverage multiplier; 0 = pristine, 1 = antique.
- `crazing_break_glaze` — how much the glaze lobes dip in the cracks.

A fresh teacup ships with all three at zero (current look preserved); an antique relic dials them up.

Out of scope: authored crack-pattern textures, gold-fill kintsugi (separate effect — kintsugi cracks are *additive* gold leaf, not subtractive stain), per-relic crack hand-placement, any change to the porcelain wrap-SSS terms.

## Touchpoints
- [shaders/lit_mesh.wgsl:1590-1604](../../shaders/lit_mesh.wgsl#L1590-L1604) — the `is_porcelain` spec block where the wide-lobe / rim / Fresnel get computed; the crack mask multiplies into all three.
- [shaders/lit_mesh.wgsl:1399-1405](../../shaders/lit_mesh.wgsl#L1399-L1405) — porcelain wrap-SSS block; albedo darkening from the crack mask should also apply before lighting so cracks read in shadow, not just under direct candle light.
- New helper in `lit_mesh.wgsl` near `vnoise2`: a `voronoi2_edge(p) -> f32` returning F2−F1. Pair with a distance fade based on `length(view_pos - world_pos)` so crazing doesn't alias past ~30cm camera distance — same trick the felt roadmap uses for stage 3 microfiber.
- [src/render/lit_mesh.rs:98-102](../../src/render/lit_mesh.rs#L98-L102) — `MaterialKind::Porcelain` definition; if crazing knobs need a dedicated uniform field (rather than stealing base_color channels) it lands here in the per-instance material struct.
- [src/scenes/material_viewer.rs:261-262](../../src/scenes/material_viewer.rs#L261-L262) — the porcelain swatch in the material viewer; extend it to show three porcelain variants (fresh / aged / antique) so the tuning surface is visible during iteration.

## Open questions
- **Knob plumbing.** Reuse a `base_color` alpha or unused channel (cheap, no pipeline change) vs add real per-instance fields to the `Porcelain` material struct (cleaner, but touches uniform layout and bind groups). Three knobs is borderline — one unified `crazing_age` driving all three with sensible internal ratios might be enough for shipped relics and avoids the plumbing entirely.
- **Voronoi vs cellular noise.** True Voronoi gives sharp polygonal cells (most physically faithful to crazing). Cellular/Worley F2−F1 is cheaper and rounder but less "shattered." At game distance the difference may not be visible; worth a screenshot bake-off before committing.
- **Aliasing strategy.** Distance-fade is the obvious fix but porcelain props are usually small and viewed close — the fade may rarely engage. May need explicit MSAA-aware line thickening (`fwidth`-based AA on the threshold) instead of, or in addition to, the fade.
- **Interaction with kintsugi.** If/when gold-fill kintsugi gets added (see destroyed-keyword doc context), the crazing crack mask would be the natural source for the gold lines too — same Voronoi, swap subtractive stain for additive gold. Worth keeping the helper return-shape generic enough to drive both.
