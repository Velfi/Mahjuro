//! Graphics-related settings and JSON tuning overrides (shared by render + game).

#![deny(unused_imports)]

pub mod effects;
pub mod shadow;
pub mod tile;
pub mod tuning;

pub use effects::EffectsQuality;
pub use shadow::ShadowQuality;
pub use tile::{TileMaterial, TilePreset};
pub use tuning::{clear_tuning_override, has_tuning_override, load_tuning_override, save_tuning_override};
