---
name: Global tonemapper pass
description: Add a tonemap step at composite so SDR and HDR look consistent without per-scene retuning of light intensities
type: project
---

# Global tonemapper pass

## Why
Lighting in scenes like `pick_blind` is currently authored to look right on whichever monitor the work was done on, then looks wrong on the other. The shrine spotlight is the obvious case: the upcoming-shrine focal point stacks four warm lights (spot 2.20 + close fill 1.30 + floor bounce 0.90 + plaque 1.45, all multiplied by a 1.15 focus boost) and `lit_mesh.wgsl` accumulates them with no tone curve. On HDR (Rgba16Float scRGB) the OS tonemaps the >1.0 values into a soft highlight; on SDR (Rgba8UnormSrgb) those same values clip to white and the shrine reads as blown out. Without a tonemapper every scene has to be hand-tuned twice, and we'll keep hitting this every time someone composes warm light stacks.

## Scope
1. Render the main 3D pass into an offscreen `Rgba16Float` color target instead of the surface, regardless of HDR/SDR mode. The HDR path already tolerates this; the SDR path currently writes lighting straight into an sRGB8 surface where >1.0 values clip.
2. Add a fullscreen tonemap composite pass that samples the offscreen target and writes to the surface. Reinhard or AgX (AgX is the modern default — neutral midtones, soft highlight rolloff). Implementation lives next to the existing `bloom_composite` shader since both are fullscreen post passes.
3. Branch on `surface_format`:
   - SDR sRGB8: tonemap → sRGB encode handled by surface format.
   - HDR Rgba16Float: pass linear scRGB through with an exposure scale (or a softer curve tuned for HDR) so the OS tonemapper still has headroom to work with.
4. Expose an `exposure` uniform on the composite so we have a single global knob instead of retuning per-scene light intensities. Default `1.0`.
5. Re-check the upcoming-shrine spotlight in [pick_blind.rs:755-793](../../src/scenes/pick_blind.rs#L755-L793) on both SDR and HDR after the tonemapper lands and pull the four light values back in line if they were over-compensating for SDR clipping.

Out of scope: per-scene exposure animation, auto-exposure / eye adaptation, bloom retuning (bloom currently extracts >1.0 and that behavior should be preserved), changing any albedo / material values.

## Touchpoints
- [src/render/wgpu_renderer/init.rs:61-92](../../src/render/wgpu_renderer/init.rs#L61-L92) — surface format selection. The offscreen target should always be Rgba16Float; the surface stays whatever the swapchain picked.
- [src/render/wgpu_renderer/resources.rs](../../src/render/wgpu_renderer/resources.rs) — allocate the new offscreen color target alongside the existing bloom textures; resize hook needs to follow the same pattern.
- [src/render/wgpu_renderer/runtime/passes/](../../src/render/wgpu_renderer/runtime/passes/) — the main 3D pass currently targets `surface_view`; switch it to the offscreen target. New `tonemap.rs` pass between bloom composite and the surface present.
- [shaders/lit_mesh.wgsl:1320-1714](../../shaders/lit_mesh.wgsl#L1320-L1714) — light accumulation loops. No code changes here; just verify after the tonemapper that the existing intensity values produce a clean rolloff instead of clipping.
- [shaders/bloom_composite.wgsl](../../shaders/bloom_composite.wgsl) — closest existing fullscreen-composite shader; mirror its bind layout for the new tonemap shader.
- [src/scenes/pick_blind.rs:755-793](../../src/scenes/pick_blind.rs#L755-L793) — the four upcoming-shrine lights to re-evaluate once the tonemapper is in place.

## Open questions
- **Reinhard vs AgX vs ACES.** AgX is the current "neutral, modern" default and matches what Blender ships. ACES has a heavier filmic look that may fight the temple aesthetic. Reinhard is simplest but desaturates highlights. Pick by visual A/B on the pick_blind shrine and the shop scene, not by reputation.
- **Bloom ordering.** Tonemap before or after bloom composite? Bloom currently expects linear values >1.0 to extract; tonemapping before bloom kills that. Order should be: scene → bloom (extract + blur + composite onto scene) → tonemap → surface.
- **HDR path behavior.** When the surface is already Rgba16Float, do we tonemap at all, or pass through and let the OS handle it? Probably pass through with just an exposure scale — re-tonemapping on HDR would defeat the format choice. Worth testing on a real HDR display before deciding.
- **Per-scene exposure overrides.** Does any scene legitimately need a different exposure (e.g. the run-end darker mood)? If so, the exposure uniform should be settable from scene code, not just a global constant. Decide once we see a scene that needs it.
