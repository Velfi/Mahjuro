---
category: changed
---

Windowing and input now run on SDL3 instead of the previous winit-based stack, with gamepads handled through SDL3’s controller APIs. On macOS, SDL hints are set so Xbox-style controllers are picked up more reliably.
