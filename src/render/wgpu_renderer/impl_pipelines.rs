use super::*;

impl WgpuRenderer {
    pub(super) fn tile_glb_pipeline(&self, key: TileGlbPipelineKey) -> &wgpu::RenderPipeline {
        match key {
            TileGlbPipelineKey::OpaqueDoubleSided => &self.tile_pipeline_opaque_double,
            TileGlbPipelineKey::OpaqueCullBack => &self.tile_pipeline_opaque_cull,
            TileGlbPipelineKey::BlendDoubleSided => &self.tile_pipeline_blend_double,
            TileGlbPipelineKey::BlendCullBack => &self.tile_pipeline_blend_cull,
        }
    }

    #[inline]
    pub(super) fn shop_env_pipeline(&self, key: TileGlbPipelineKey) -> &wgpu::RenderPipeline {
        match key {
            TileGlbPipelineKey::OpaqueDoubleSided => &self.shop_pipeline_opaque_double,
            TileGlbPipelineKey::OpaqueCullBack => &self.shop_pipeline_opaque_cull,
            TileGlbPipelineKey::BlendDoubleSided => &self.shop_pipeline_blend_double,
            TileGlbPipelineKey::BlendCullBack => &self.shop_pipeline_blend_cull,
        }
    }

    #[inline]
    pub(super) fn shop_env_pipeline_mrt(&self, key: TileGlbPipelineKey) -> &wgpu::RenderPipeline {
        match key {
            TileGlbPipelineKey::OpaqueDoubleSided => &self.shop_pipeline_mrt_opaque_double,
            TileGlbPipelineKey::OpaqueCullBack => &self.shop_pipeline_mrt_opaque_cull,
            TileGlbPipelineKey::BlendDoubleSided => &self.shop_pipeline_mrt_blend_double,
            TileGlbPipelineKey::BlendCullBack => &self.shop_pipeline_mrt_blend_cull,
        }
    }
}
