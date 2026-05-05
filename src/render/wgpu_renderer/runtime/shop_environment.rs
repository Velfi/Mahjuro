use super::*;

impl WgpuRenderer {
    pub(super) fn write_shop_environment_uniforms(
        &self,
        camera: &CameraFrame,
        shop_gltf_punctual: bool,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let s = crate::render::shop_glb::shop_env_world_scale(camera.h, self.shop_env_height_scale);
        let model = Mat4::from_scale(glam::Vec3::splat(s));
        let (exposure, ambient_x) = if shop_gltf_punctual {
            (self.shop_env_linear_exposure, self.shop_env_ambient_scale)
        } else {
            (0.0, 0.0)
        };
        self.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: camera.view_proj_arr,
                model: model.to_cols_array(),
                // xyz = tile semantics for legacy `tile_3d` shop path; `shop_glb.wgsl` uses
                // `tile_seed` / `decal_atlas_uv.x` for exposure / ambient — see `shop_glb.rs`.
                base_color_factor: [
                    1.0,
                    0.0,
                    0.0,
                    crate::render::tile_body::TEXTURED_BASE_MAP_BODY_KIND,
                ],
                cam_pos: camera.cam_pos.to_array(),
                tile_seed: exposure,
                decal_atlas_uv: [ambient_x, 0.0, 1.0, 1.0],
            }),
        );
    }
}
