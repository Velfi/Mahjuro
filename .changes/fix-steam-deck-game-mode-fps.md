---
category: fixed
---

Steam Deck game mode no longer caps the game to ~10 FPS. The Vulkan swapchain
latency clamp was meant only for Windows AMD drivers; on Linux it forced a
2-image swapchain that gamescope's nested compositor could throttle to its own
pacing.
