use super::*;

mod camera;
use camera::CameraFrame;
mod debug_axes;
mod debug_rain_hit;
mod flame_emitters;
use flame_emitters::build_flame_emitters;
mod gameplay_hud_uniforms;
mod object3d_placement;
mod object3d_primitive;
mod object3d_ribbon;
#[path = "runtime/passes/process_op.rs"]
mod passes_process_op;
use passes_process_op::ProcessOpCtx;
mod op_list;
#[path = "runtime/passes/shadow.rs"]
mod passes_shadow;
pub use op_list::DrawKind;
use op_list::{RenderOp, TextDraw};
mod acquire_telemetry;
pub(super) use acquire_telemetry::{AcquireOutcome, AcquireTelemetry};
mod frame;
use frame::RenderFrame;
mod render;
pub(crate) mod shadow_setup;
pub(crate) mod shop_environment;
mod shadow_debug_probe;
mod shadow_debug_session;
pub(crate) use shadow_debug_session::{
    agent_shadow_log, probe_baked_ao_at_world, probe_baked_ao_at_world_scaled,
    shadow_caster_z_up_probe,
};
mod showcase_tiles;
mod surface;
