use super::*;

#[path = "runtime/arrange_overlay.rs"]
mod arrange_overlay;
#[path = "runtime/camera.rs"]
mod camera;
use camera::CameraFrame;
#[path = "runtime/debug_axes.rs"]
mod debug_axes;
#[path = "runtime/flame_emitters.rs"]
mod flame_emitters;
use flame_emitters::build_flame_emitters;
#[path = "runtime/gameplay_hud_uniforms.rs"]
mod gameplay_hud_uniforms;
#[path = "runtime/object3d_placement.rs"]
mod object3d_placement;
#[path = "runtime/object3d_primitive.rs"]
mod object3d_primitive;
#[path = "runtime/object3d_ribbon.rs"]
mod object3d_ribbon;
#[path = "runtime/passes/process_op.rs"]
mod passes_process_op;
use passes_process_op::ProcessOpCtx;
#[path = "runtime/op_list.rs"]
mod op_list;
#[path = "runtime/passes/shadow.rs"]
mod passes_shadow;
use op_list::{DrawKind, RenderOp, TextDraw};
#[path = "runtime/frame.rs"]
mod frame;
use frame::RenderFrame;
#[path = "runtime/render.rs"]
mod render;
#[path = "runtime/shadow_setup.rs"]
mod shadow_setup;
#[path = "runtime/shop_environment.rs"]
mod shop_environment;
#[path = "runtime/showcase_tiles.rs"]
mod showcase_tiles;
#[path = "runtime/surface.rs"]
mod surface;
