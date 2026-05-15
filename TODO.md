# Current TODO

- **Rendering / Windows Vulkan:** Investigate `STATUS_ACCESS_VIOLATION` in AMD Win32 WSI during first `Surface::configure` / `vkCreateSwapchainKHR` (e.g. RX 7900 XT, LLPC). Today we run `mahjuro vulkan-wsi-probe` and fall back to DX12 when it faults. Follow [wgpu#8354](https://github.com/gfx-rs/wgpu/issues/8354) (DXGI Vulkan swapchain), retry after driver/wgpu bumps with `MAHJURO_SKIP_VULKAN_WSI_PROBE=1`, and whether SDL + `Instance::create_surface` becomes viable (`Send`/`Sync` on `Window` vs unsafe `RawHandle`).
- **Audio:** Add `assets/audio/yaku_kokushi_musou.ogg` for the Kokushi Musō yaku stinger (wired in `src/audio.rs`; missing file no-ops today).
- Revisit relic rearranging on the shop screen.
- Investigate mirror- and shadow-hand interactions while touching shop relic order.
- Rebalance `WildWinds`; it may still be too strong.
- fix tile pack celebrations
- fix zodiac celebrations
- fix shop lighting (needs a tuning pass)
- fix pick_blind lighting (needs a tuning pass)
- gold piles need work, revisit different objects for different denominations
- controller rumble broken
  - We switched to SDL to avoid gilrs problems. Let's make sure everything we think we got out of the switch is actually real.
- can we get back to dynamically linking SDL3 instead of statically? There was some issue with the build script that was causing problems. Is linking still an issue?
- Change inspect to rotate object instead of camera, show shop/collection in background, but put blur filter on everything but the showcased object
- 