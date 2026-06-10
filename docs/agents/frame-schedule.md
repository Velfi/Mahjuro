# Frame schedule (main loop)

The per-frame update pipeline lives in [`src/main/frame_phases/`](../src/main/frame_phases/). [`frame_tick`](../src/main/frame_tick.rs) is a thin orchestrator; phase order is fixed in [`frame_phases/mod.rs`](../src/main/frame_phases/mod.rs).

## Phase order

| Phase | Module | Side effects |
|---|---|---|
| Early | `early.rs` | dt, perf watchdog, anim/audio tick, room brownout, **gamepad prepare** |
| Drain bus | `bus.rs` | scoring rumble keepalive, event bus → audio/achievements/modals/deferred round end |
| Debug menu | `debug_menu.rs` | debug menu actions (debug builds only) |
| Poll input | `input.rs` | SDL/gamepad → `FrameLocals.actions`, focus, cursor hover |
| Debug overlays | `debug_overlays.rs` | intercept input when tuning/SFX/camera/… overlays open |
| Input gates | `input_gates.rs` | modal intercept, transition block, splash skip |
| Scene update | `scene_update.rs` | layout, frame picks, `scene.update(UpdateCtx)` |
| Post update | `post_update.rs` | overlay push/pop, rumble, scene replace, options sync, transitions |
| Tail | `tail.rs` | quit flag, draw, profile flush, profiler chime |

## Ordering constraints (do not reorder casually)

1. **`prepare_gamepad_frame` before bus drain** — rumble keepalive and bus handlers need `shell.pads` populated.
2. **Bus drain before input poll** — bus side effects (deferred round end, modals) settle before scene input.
3. **Debug overlays before scene update** — overlays consume/clear `actions` so scenes never see intercepted input.
4. **Modal gate before scene update** — same pattern as overlays.
5. **Scene update before post-update rumble** — shop/cash-in hold rumble must not clobber scoring pulses fired during bus drain the same tick.
6. **Shop hold rumble only when `shop_ready && hold`** — never call sync with `hold = false` globally; motors expire on their own.

## Per-frame scratch

[`FrameLocals`](../src/main/frame_phases/locals.rs) holds stack state shared across phases (`actions`, `button_clicks`, `update_result`, etc.). Long-lived frame state stays on [`App`](../src/lib.rs) (`frame_picks`, `hub_loading`, `deferred_round_end`, …).

## Adding a new phase

1. Add a module under `frame_phases/`.
2. Register it in `frame_phases/mod.rs` **in the correct position** (update this doc).
3. Keep `App` helper methods on [`frame_tick.rs`](../src/main/frame_tick.rs) if phases need private access (phases are a child module of `main_frame_tick`).
