# Mahjuro — Agent Notes

Short pointers to deeper context. Read the linked file before working in the area.

- **CI locally** — `./scripts/check.sh` mirrors the GitHub `build-and-test` job (bake tools → `cargo build` → `cargo test`; Linux needs `xvfb-run`). Optional `--extras` for fmt/clippy/Python (not in CI). Hot-build wall times: `./scripts/profile-build.sh --hot` (add `--timings` for per-crate HTML under `target/cargo-timings/`).

- [3D world space](docs/agents/world-space.md) — Z-up frame, table at `z = 0`, anchors, screen vs world placement.
- [Cap-mesh coordinates](docs/agents/cap-mesh-coordinates.md) — image → cap-local → UV for relic pins (+Y) and talismans (+Z); `cap_extrude.rs` source of truth.
- [Frame schedule (main loop)](docs/agents/frame-schedule.md) — `frame_phases/` pipeline order, `FrameLocals`, ordering constraints.
- [Widget tree (scene input)](docs/agents/widget-tree.md) — `Tree<A>` / `FlatItem<A>` immediate-mode UI; single source of truth for rects, hover/keyboard nav.
- [Font & text layout](docs/agents/font-scaling.md) — label auto-shrink, colored/plain block height helpers.
- [Chart guidelines](docs/agents/chart-guidelines.md) — Duke top-ten rules for Chronicle/dashboard charts.
- [Launch options](docs/agents/launch-options.md) — CLI flags and `MAHJURO_*` env vars (runtime + build).
- [Tileset mods](docs/agents/tileset-mods.md) — player-installed `atlas.png` + `atlas.toml` under config dir; `mod:<name>` namespace.
- [GPU memory / 4 GB preset](docs/agents/gpu-memory.md) — Low memory graphics mode, profiling soak, residency caps.
- [Memory / loading budgets](docs/agents/memory-loading-budgets.md) — Phase 0 baseline capture (`scripts/memory-loading-baseline.sh`), soak metrics, targets.
- [Room shadows & baking](docs/agents/room-shadows-and-baking.md) — offline `.msh` / `.mgi`, live prop shadows, rebake commands, headless tools, relic RLC1.
- [macOS dylibs / app bundle](docs/agents/macos-dylibs.md) — `libsteam_api.dylib`, static SDL3, `@loader_path` / `@rpath`, CI vs `package-macos.sh`.
- [Windows build / DXC redist](docs/agents/windows-build.md) — `dxcompiler.dll` + `dxil.dll`, DX12 vs FXC, release packaging.
- [Multi-store distribution](docs/agents/distribution.md) — `dist-steam` / `dist-mas` / `dist-msstore`, achievements, sandbox I/O, packaging.
- [Mac App Store release](docs/agents/macos-app-store.md) — Connect alignment, signing, Transporter upload, review notes.
- [Microsoft Store release](docs/agents/windows-app-store.md) — Partner Center, MSIX signing, upload.
- [Scoring](docs/agents/scoring.md) — play / cash-in loop, melds, chips, multipliers.
- [Blind targets](docs/agents/blind-targets.md) — formula, multipliers, season scaling, current values.
- [Gameplay table (`gameplay.glb`)](docs/agents/gameplay-glb.md) — authored room draw + spawn empties; required at runtime.
- [Tile pack opening](docs/agents/tpos2-art-direction.md) — `Arrival` → `Anticipation` → `Unseal` → `Deal` celebration phases.
- [Talisman carving art](docs/agents/memorial-talisman-art.md) — carved jade relief, heightmaps + organic masks, `scripts/generate_talisman_art.py`.
