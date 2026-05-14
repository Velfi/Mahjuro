---
category: changed
---

Added two named theme tokens — **`LAPIS`** (sky-blue, the cool counterpart to `RUBY`) and **`PORCELAIN_AGED`** (well-loved ceramic cream, distinct from `PARCHMENT` paper) — and a `CascadeTokenKind::color()` helper that funnels the score-popup, cascade-HUD label, and 3D cascade-token meshes through a single Chips→`LAPIS` / Mult→`RUBY` mapping. Score-popup constants now resolve to `LAPIS / RUBY / RELIC_GOLD / TALLOW` instead of four drifting literals; the consumable dish, action-prompt legend, and shop legend ceramic surfaces now share `PORCELAIN_AGED`. See `COLOR_THEME.md` and `python3 tools/color_inventory.py` for the audit.
