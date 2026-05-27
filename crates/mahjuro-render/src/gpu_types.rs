//! GPU type definitions shared between the renderer and relic loader threads.

use mahjuro_core::core::relic::RelicId;

use crate::lit_mesh::MeshCpu;

/// Pre-loaded relic / pack art for lit-mesh bindings. Each [`wgpu::TextureView`]
/// keeps its backing texture alive via refcount.
pub(crate) struct RelicTextureGpu {
    pub view: wgpu::TextureView,
    pub relief_view: wgpu::TextureView,
}

/// Decoded relic image data sent from the background loader thread.
pub struct DecodedRelicImage {
    pub id: RelicId,
    pub name: &'static str,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Linear RGBA relief (same UV space as albedo): R/B = height, G = specular mask.
    /// 1×1 mid-gray when height asset is missing.
    pub relief_rgba: Vec<u8>,
    pub relief_width: u32,
    pub relief_height: u32,
    /// Extruded cap mesh built on the decode thread (GPU upload only on main).
    pub mesh_cpu: Option<MeshCpu>,
}
