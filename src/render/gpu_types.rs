//! GPU type definitions shared between the renderer and relic loader threads.

use crate::core::relic::RelicId;

/// Pre-loaded relic / pack art for lit-mesh bindings. Each [`wgpu::TextureView`]
/// keeps its backing texture alive via refcount.
pub(crate) struct RelicTextureGpu {
    pub view: wgpu::TextureView,
    pub relief_view: wgpu::TextureView,
}

/// Decoded relic image data sent from the background loader thread.
pub(crate) struct DecodedRelicImage {
    pub id: RelicId,
    pub name: &'static str,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mesh_rgba: Option<Vec<u8>>,
    pub mesh_width: u32,
    pub mesh_height: u32,
    /// Linear RGBA relief (same UV space as albedo); 1×1 mid-gray when height asset is missing.
    pub relief_rgba: Vec<u8>,
    pub relief_width: u32,
    pub relief_height: u32,
}
