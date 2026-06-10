//! Named phases of the main-loop frame pipeline.
//!
//! Order is fixed in [`run`] — see `docs/agents/frame-schedule.md`.

mod bus;
mod debug_menu;
mod debug_overlays;
mod early;
mod input;
mod input_gates;
mod locals;
mod post_update;
mod scene_update;
mod tail;

pub use locals::FrameLocals;

use crate::App;
use crate::sdl_shell::SdlShell;

/// Run one frame's update pipeline (everything before/at/after scene update).
pub fn run(app: &mut App, shell: &mut SdlShell) {
    let mut locals = FrameLocals::default();
    early::run(app, shell, &mut locals);
    bus::run(app, shell, &mut locals);
    debug_menu::run(app);
    input::run(app, shell, &mut locals);
    debug_overlays::run(app, &mut locals);
    input_gates::run(app, &mut locals);
    scene_update::run(app, &mut locals);
    post_update::run(app, shell, &mut locals);
    tail::run(app, shell, &mut locals);
}
