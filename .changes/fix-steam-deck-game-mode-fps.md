---
category: fixed
---

Steam Deck game mode no longer caps the game to ~10 FPS. Two Linux Vulkan
swapchain paths were serializing every frame to gamescope's nested compositor:
a Windows-only frame-latency clamp that was leaking into Linux builds, and
wgpu's post-acquire fence wait (originally added for Windows DXGI pacing) that
also blocked on Linux. Both are now Windows-only.
