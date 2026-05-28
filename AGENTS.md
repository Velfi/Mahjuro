# Mahjuro — Agent Notes

Short pointers to deeper context. Read the linked file before working in the area.

- **CI locally** — `./scripts/check.sh` mirrors the GitHub `build-and-test` job (bake tools → `cargo build` → `cargo test`; Linux needs `xvfb-run`). Optional `--extras` for fmt/clippy/Python (not in CI).

- [3D world space](docs/agents/world-space.md) — Z-up frame, table at `z = 0`, anchors, screen vs world placement.
- [Widget tree (scene input)](docs/agents/widget-tree.md) — `Tree<A>` / `FlatItem<A>` immediate-mode UI; single source of truth for rects, hover/keyboard nav.
- [Font & text layout](docs/agents/font-scaling.md) — label auto-shrink, colored/plain block height helpers.
- [Chart guidelines](docs/agents/chart-guidelines.md) — Duke top-ten rules for Chronicle/dashboard charts.
- [Room shadows & baking](docs/agents/room-shadows-and-baking.md) — offline `.msh` / `.mgi`, live prop shadows, rebake commands, headless tools, relic RLC1.
- [macOS dylibs / app bundle](docs/agents/macos-dylibs.md) — `libsteam_api.dylib`, static SDL3, `@loader_path` / `@rpath`, CI vs `package-macos.sh`.
- [Scoring](docs/agents/scoring.md) — play / cash-in loop, melds, chips, multipliers.
- [Gameplay table (`gameplay.glb`)](docs/agents/gameplay-glb.md) — authored room draw + spawn empties; required at runtime.
- [Tile pack opening](docs/agents/tpos2-art-direction.md) — `Arrival` → `Anticipation` → `Unseal` → `Deal` celebration phases.
- [Talisman tablet art](docs/agents/memorial-talisman-art.md) — shop + memorial heightmaps, octagon `_mask.png`, `scripts/generate_talisman_art.py`.
