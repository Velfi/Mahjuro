---
name: Journal prepass vs SSR history isolation
description: Architectural split so offscreen renders (shop journal → book texture) cannot publish scene_prev / SSR depth; complements any minimal skip-snapshot guard
type: project
---

# Journal prepass vs SSR history isolation

## Why
The shop draws an embedded `YakuJournalScene` into `journal_scene_texture` before the main frame (`render_journal_prepass` → `render_to` with `output_override`). That path shares `scene_color_texture`, depth, and—critically—the **SSR snapshot** that copies into `scene_prev_texture` and `ssr_prev_depth_texture`. Lacquered surfaces sample those buffers for screen-space reflections. When the journal prepass runs first in a frame, it overwrites SSR history with the journal render; the following shop pass reflects wrong content until focus moves away and the prepass stops. The minimal fix is to skip SSR snapshot on prepass; the durable fix is to make **main presentation** the only path allowed to publish temporal SSR inputs, so future offscreen passes cannot regress this accidentally.

## Scope
1. **Invariant:** `scene_prev_texture` and `ssr_prev_depth_texture` are updated only by the render that produces the **visible** swapchain image for that frame (or an explicitly named “primary” path), never by auxiliary `render_to(..., output_override)` calls such as the journal prepass.
2. **Minimal implementation:** gate the SSR snapshot block in [`render_to`](../../src/render/wgpu_renderer/runtime/render.rs) on `!is_prepass` (or equivalent flag), and add a one-line comment tying it to lacquer SSR + journal prepass ordering.
3. **Structural follow-up (pick one direction, not both unless needed):**
   - **API split:** expose `render_main_frame` vs `render_offscreen(frame, target, …)` so auxiliary entry points cannot run frame-global stages; or
   - **Dedicated scratch:** optional separate intermediate color/depth for offscreen full pipelines so two submissions in one frame cannot stomp shared history (higher memory cost, maximum isolation).
4. Verify shop lacquer SSR with journal focused: counter reflections stay stable across frames.

Out of scope: changing how the book mesh samples `journal_scene_texture`, journal UI layout, or SSR quality settings.

## Touchpoints
- [src/render/wgpu_renderer/runtime/render.rs](../../src/render/wgpu_renderer/runtime/render.rs) — `render_journal_prepass`, `render_to`, `is_prepass`, and the **SSR snapshot** `copy_texture_to_texture` pair after Pass A (~“SSR snapshot” comment). Primary place for the guard or refactor.
- [src/main/draw.rs](../../src/main/draw.rs) — ordering: `journal_prepass_frame` → `render_journal_prepass` before `renderer.render`; documents why prepass must not publish SSR history.
- [src/scenes/shop/draw.rs](../../src/scenes/shop/draw.rs) — sets `journal_prepass_frame` when `journal_open_amount > 0.001`; no change required once renderer contract is correct.

## Open questions
- **API vs guard-only.** Is a `!is_prepass` check enough long-term, or should journal prepass move to a dedicated helper that never touches SSR snapshot code paths? The latter reduces repeat bugs if more prepasses are added.
