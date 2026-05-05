# Relic display (gameplay / shop / pick blind)

There is **no** shared `relic_row()` helper in [`scenes/mod.rs`](../../src/scenes/mod.rs) anymore.

- **Gameplay** — Active relics are **3D enamel medallions** in a **horizontal tray** across the upper screen. Built in [`build_relic_tray_and_wind`](../../src/scenes/gameplay/input_handler.rs); arrange leaf [`gameplay.relic_col`](../../src/ui/scene_layout/gameplay.rs). Cascade popups use the same geometry via [`relic_tray_screen_center_xy`](../../src/scenes/gameplay/input_handler.rs).
- **Shop** — For-sale and owned relics are **3D props** on the counter / dishes (see shop draw path), not a 2D badge row under the score panel.
- **Pick blind** — Minimal HUD; relic/consumable strips were intentionally dropped (see comments in [`pick_blind.rs`](../../src/scenes/pick_blind.rs)).

For **2D modifier-strip tokens** (chips / mult pills during scoring), see [`GameplayScene::cascade_token_layout`](../../src/scenes/gameplay.rs).
