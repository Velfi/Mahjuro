//! GPU type definitions shared between the renderer and relic loader threads.

use mahjuro_core::core::relic::RelicId;

use crate::lit_mesh::MeshCpu;

/// Pre-loaded relic / pack art for lit-mesh bindings. Each [`wgpu::TextureView`]
/// keeps its backing texture alive via refcount.
pub(crate) struct RelicTextureGpu {
    pub view: wgpu::TextureView,
    pub relief_view: wgpu::TextureView,
}

/// GPU byte estimate for one resident relic (textures + mesh buffers).
pub(crate) struct RelicGpuMeta {
    pub albedo_bytes: usize,
    pub relief_bytes: usize,
    pub mesh_bytes: usize,
}

/// BC7 mip chain payload from RLC2 bakes.
pub struct RelicBc7MipChain {
    pub base_width: u32,
    pub base_height: u32,
    pub mip_count: u32,
    pub bc7_bytes: Vec<u8>,
    pub fallback_rgba: Vec<u8>,
    pub fallback_width: u32,
    pub fallback_height: u32,
    pub srgb: bool,
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
    /// RLC2 albedo BC7 mip chain (`None` only on bake-time PNG decode path).
    pub albedo_bc7: Option<RelicBc7MipChain>,
    /// RLC2 relief BC7 mip chain (`None` only on bake-time PNG decode path).
    pub relief_bc7: Option<RelicBc7MipChain>,
}
