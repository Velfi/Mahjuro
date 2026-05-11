//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

mod embedded_wgsl;
mod init;
mod init_phases;
pub(crate) mod resources;
mod runtime;
mod showcase;

mod constants;
mod hash_util;
mod internal_slots;
mod layout_instances;
mod lighting_buffers;
mod moon;
mod picking_types;
mod projection;
mod renderer;
mod screenshot;
mod targets;
mod tile_pipeline;
mod ui_instances;
mod uniforms;

mod impl_arrange;
mod impl_loaders;
mod impl_pipelines;
mod impl_public;
mod impl_screenshot;

use self::resources::*;
use self::showcase::*;

pub use constants::{
    MAIN_MENU_PICK_OPTIONS, MAIN_MENU_PICK_PLAY, MAIN_MENU_PICK_QUIT, MAX_BOOK_SLOTS,
    MAX_BOWL_SLOTS, MAX_BUG_SLOTS, MAX_CASCADE_TOKEN_SLOTS, MAX_DORA_PLINTH_SLOTS,
    MAX_EXTRUDED_GLYPH_SLOTS, MAX_MIRROR_SLOTS, MAX_ORB_SLOTS, MAX_POINT_LIGHTS, MAX_RELIC_SLOTS,
    MAX_RIBBON_SLOTS, MAX_SHRINE_SLOTS, MAX_SPOT_LIGHTS, MAX_TALISMAN_SLOTS, MAX_TALLY_FAN_SLOTS,
    MAX_TALLY_STICK_SLOTS, MAX_TILE_OCCLUDERS, MAX_WALL_TILE_SLOTS, MAX_WOOD_TABLET_SLOTS,
    MAX_YAKU_TABLET_SLOTS,
};
pub use internal_slots::{RelicIcon, TextAlign, TextLabel};
pub use layout_instances::build_instances_from_layout;
pub use picking_types::{GameplayPick, MainMenuPick, ShopHit};
pub use projection::{DebugArrangeOverride, ProjectionCache};
pub use renderer::WgpuRenderer;
pub use targets::TargetInit;
pub use ui_instances::{GpuInstance, GradientQuadInstance, RenderSettings};

pub(crate) use constants::{MAX_RENDER_DIMENSION, clamp_render_physical_size};
pub(crate) use hash_util::tablet_label_hash;
pub use lighting_buffers::{PointLight, SpotLight};
pub(crate) use lighting_buffers::{
    PointLightsBuf, SpotLightsBuf, TileOccluderGpu, TileOccludersBuf,
};
pub(crate) use moon::current_moon_phase;
pub(crate) use resources::{create_depth, create_depth_copy};
pub(crate) use screenshot::ScreenshotStaging;
pub(crate) use targets::RenderTarget;
pub(crate) use tile_pipeline::{TileGlbPipelineKey, TilePrimitiveGpu};
pub(crate) use uniforms::{
    BloomParams, CameraUniform, FlameViewUniform, Globals, HazeUniform, ProbeGiFrameUniform,
    TileOutlineFrameUniform, TileOutlineInstance, TonemapParams,
};

pub(super) use constants::{
    LOCAL_X_EXTENT, LOCAL_Y_EXTENT, LOCAL_Z_EXTENT, MAX_SHOWCASE_TILE_SLOTS, TEXT_CACHE_TTL_FRAMES,
};
pub(super) use projection::PickCamera;

pub(crate) use internal_slots::{
    CachedTextLabel, DepartingTile, HandTileGpu, ShopEnvironmentGpu, ShowcaseTileGpu,
    TextLabelShapeKey, TileFaceOverlayGpu,
};
