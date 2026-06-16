//! Run-state fixtures and scene setup for offline room lighting bakes.

mod fixtures;
mod scenes;

pub(crate) use fixtures::{bake_player_progress, bake_render_settings};
#[cfg(feature = "screenshot")]
pub(crate) use fixtures::{setup_gameplay_bake_state, setup_shop_state};
pub use scenes::scene_for_room;
