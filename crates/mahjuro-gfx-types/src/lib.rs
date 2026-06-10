//! Graphics-related settings and JSON tuning overrides (shared by render + game).

#![deny(unused_imports)]

pub mod effects;
pub mod graphics_mode;
pub mod shadow;
pub mod tile;
pub mod tuning;

pub use effects::EffectsQuality;
pub use graphics_mode::{
    AdapterMemoryProbe, BandwidthClass, GraphicsMemoryModel, GraphicsMode, AUTO_LOW_MEMORY_VRAM_THRESHOLD_MIB,
    MIN_RENDER_HEIGHT, MIN_RENDER_WIDTH, MIN_SUPPORTED_GPU_MEMORY_MIB,
};
pub use shadow::ShadowQuality;
pub use tile::{TileMaterial, TilePreset};
pub use tuning::{
    clear_tuning_override, has_tuning_override, load_tuning_override, save_tuning_override,
};
