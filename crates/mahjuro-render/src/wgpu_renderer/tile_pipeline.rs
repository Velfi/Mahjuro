#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileGlbPipelineKey {
    OpaqueCullBack,
    BlendDoubleSided,
    BlendCullBack,
}

impl TileGlbPipelineKey {
    pub(crate) fn from_loaded_primitive(lp: &crate::tile_glb::LoadedPrimitive) -> Self {
        use crate::tile_glb::GltfAlphaMode::*;
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
    /// Per-primitive material bind group (frame uniform + textures). Refreshed when the
    /// showcase decal atlas or frame uniform buffer identity changes.
    pub material_bind_group: Option<wgpu::BindGroup>,
}

/// GPU resources for one player [`mahjuro_gfx_types::TileMaterial`] mesh variant.
pub(crate) struct TileMeshGpuSet {
    pub primitives: Vec<TilePrimitiveGpu>,
    pub outline_vertex_buffer: wgpu::Buffer,
    pub outline_index_buffer: wgpu::Buffer,
    pub outline_index_count: u32,
}
