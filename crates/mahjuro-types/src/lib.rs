//! Small shared types used by the game shell and GPU renderer.

#![deny(unused_imports)]

pub mod game_over;
pub mod scene_draw;
pub mod shop_pick;
pub mod theme_tokens;
pub mod ui_action;

pub use game_over::GameOverReason;

pub use scene_draw::{BackgroundId, ButtonAction, ButtonDef};
pub use ui_action::UiAction;
