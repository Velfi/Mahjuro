use super::*;

impl WgpuRenderer {
    pub(super) fn write_shop_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        shop_gltf_punctual: bool,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let s = crate::render::shop_glb::shop_env_world_scale(camera.h, self.shop_env_height_scale);
        let model = crate::render::shop_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::shop_glb::shop_env_model_matrix_from_cpu(
                    camera.h,
                    self.shop_env_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        let (exposure, ambient_x) = if shop_gltf_punctual {
            (
                self.shop_env_linear_exposure
                    * crate::render::shop_glb::SHOP_ENV_LINEAR_EXPOSURE_BASE,
                self.shop_env_ambient_scale,
            )
        } else {
            (0.0, 0.0)
        };
        let inv_doc_scale = if shop_gltf_punctual {
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let hdr_tonemap = self.tile_hdr_tonemap(frame);
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
                // `.y` = 1/world_scale: inverse-square in document space (matches glTF units).
                decal_atlas_uv: [ambient_x, inv_doc_scale, 1.0, 1.0],
                hdr_tonemap,
            }),
        );
    }
}
