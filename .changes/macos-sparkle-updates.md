---
category: changed
---

macOS auto-update now uses Sparkle, the standard macOS update framework. Previously, updates failed on macOS because Gatekeeper blocks any app from rewriting its own bundle in `/Applications`. Sparkle handles the download and bundle swap externally, so updates install cleanly without manual drag-replace. Linux and Windows continue to use the in-game updater.
