//! Run-state fixtures for offline room lighting bakes.

mod fixtures;
mod scenes;

pub use fixtures::{setup_gameplay_bake_state, setup_shop_state};
pub use scenes::scene_for_room;
