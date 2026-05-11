#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileGlbPipelineKey {
    /// Not selected by [`Self::from_loaded_primitive`]; pipelines kept for symmetry.
    #[allow(dead_code)]
    OpaqueDoubleSided,
    OpaqueCullBack,
    BlendDoubleSided,
    BlendCullBack,
}

impl TileGlbPipelineKey {
    pub(crate) fn from_loaded_primitive(lp: &crate::render::tile_glb::LoadedPrimitive) -> Self {
        use crate::render::tile_glb::GltfAlphaMode::*;
        // glTF `doubleSided` on opaque/mask solids (e.g. mahjong tiles) makes interior
        // back-faces visible on thin-walled geometry; only honor it for blend materials.
        match lp.alpha_mode {
            Blend if lp.double_sided => Self::BlendDoubleSided,
            Blend => Self::BlendCullBack,
            Opaque | Mask => Self::OpaqueCullBack,
        }
    }

    #[inline]
    pub(crate) fn is_blend(self) -> bool {
        matches!(self, Self::BlendDoubleSided | Self::BlendCullBack)
    }
}

/// One material slot of the tile mesh — vertex/index buffers + the primitive's
/// own albedo texture.  A tile may consist of several of these (e.g. an ivory
/// face primitive and a bamboo back primitive).
pub(crate) struct TilePrimitiveGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub albedo_view: wgpu::TextureView,
    /// Tangent-space normal map (`Rgba8Unorm`); flat `(128,128,255)` when unused.
    pub normal_view: wgpu::TextureView,
    pub metallic_roughness_view: wgpu::TextureView,
    pub emissive_view: wgpu::TextureView,
    pub pbr_uniform_buffer: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    pub pipeline_key: TileGlbPipelineKey,
}
