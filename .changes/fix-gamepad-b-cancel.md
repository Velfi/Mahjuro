---
category: fixed
---

Releasing the gamepad A or B button no longer fires a spurious Cancel. Previously, letting go of B (or A with swap-AB) emitted a Cancel action on release, which could back out of menus unintentionally. Only the confirm-side button's release now emits anything, and only as the ConfirmRelease paired with its press.
