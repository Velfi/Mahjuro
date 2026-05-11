use super::camera::CameraFrame;
use super::*;

impl WgpuRenderer {
    fn draw_gltf_room_env_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        room_hdr_mrt_emissive: bool,
    ) {
        if prims.is_empty() {
            return;
        }
        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
        pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
        for blend_phase in [false, true] {
            let mut last_pi: Option<usize> = None;
            let mut last_key = None;
            for (pi, prim) in prims.iter().enumerate() {
                if prim.pipeline_key.is_blend() != blend_phase {
                    continue;
                }
                if last_key != Some(prim.pipeline_key) {
                    let pipe = if frame.room_uses_shop_glb_shader() {
                        if room_hdr_mrt_emissive {
                            self.shop_env_pipeline_mrt(prim.pipeline_key)
                        } else {
                            self.shop_env_pipeline(prim.pipeline_key)
                        }
                    } else {
                        self.tile_glb_pipeline(prim.pipeline_key)
                    };
                    pass.set_pipeline(pipe);
                    last_key = Some(prim.pipeline_key);
                }
                if last_pi != Some(pi) {
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    last_pi = Some(pi);
                }
                let Some(bg) = gpu.bind_groups.get(pi) else {
                    continue;
                };
                pass.set_bind_group(0, bg, &[]);
                pass.draw_indexed(0..prim.index_count, 0, 0..1);
            }
        }
    }

    /// Draw [`Shop.glb`] environment primitives through `shop_glb.wgsl` / `tile_3d.wgsl`
    /// (same routing as [`RenderOp::ShopEnvironment`]).
    pub(super) fn draw_shop_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.shop_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
        );
    }

    /// Draw [`hallway.glb`] (pick-blind room).
    pub(super) fn draw_hallway_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.hallway_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
        );
    }

    fn write_gltf_room_env_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        embedded_gltf_punctual: bool,
        hallway_env: bool,
        bloom_linear_hdr_output: bool,
        model: Mat4,
        gpu: &ShopEnvironmentGpu,
    ) {
        let s = crate::render::shop_glb::shop_env_world_scale(camera.h, self.shop_env_height_scale);
        let inv_doc_scale = if embedded_gltf_punctual || frame.shop_inspect_lit_mesh_hdr {
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let lit_base = self.tile_hdr_tonemap(frame);
        let mut hdr_tonemap = if frame.shop_inspect_lit_mesh_hdr {
            let linear_hdr = lit_base[1] * crate::render::shop_glb::SHOP_INSPECT_ENV_VS_LIT_LINEAR;
            let ambient = lit_base[2] * crate::render::shop_glb::SHOP_INSPECT_ENV_VS_LIT_AMBIENT;
            [1.0, linear_hdr, ambient, 0.0]
        } else {
            lit_base
        };
        if bloom_linear_hdr_output {
            hdr_tonemap[3] = 1.0;
        }
        let (exposure, ambient_x) = if embedded_gltf_punctual {
            let mut e = self.shop_env_linear_exposure
                * crate::render::shop_glb::SHOP_ENV_LINEAR_EXPOSURE_BASE;
            let mut a = self.shop_env_ambient_scale;
            if hallway_env {
                e *= crate::render::hallway_glb::HALLWAY_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::render::hallway_glb::HALLWAY_ENV_AMBIENT_SCALE_MIN);
            }
            (e, a)
        } else if frame.shop_inspect_lit_mesh_hdr {
            (
                lit_base[1] * crate::render::shop_glb::SHOP_INSPECT_STOREROOM_GLB_TILE_SEED_MUL,
                hdr_tonemap[2],
            )
        } else {
            (0.0, 0.0)
        };
        self.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: camera.view_proj_arr,
                model: model.to_cols_array(),
                base_color_factor: [
                    1.0,
                    0.0,
                    0.0,
                    crate::render::tile_body::TEXTURED_BASE_MAP_BODY_KIND,
                ],
                cam_pos: camera.cam_pos.to_array(),
                tile_seed: exposure,
                decal_atlas_uv: [ambient_x, inv_doc_scale, self.shop_gltf_emissive_scale, 1.0],
                hdr_tonemap,
            }),
        );
    }

    pub(super) fn write_shop_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
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
        self.write_gltf_room_env_uniforms(
            frame,
            camera,
            frame.scene_lighting.embedded_gltf_punctual,
            false,
            bloom_linear_hdr_output,
            model,
            gpu,
        );
    }

    pub(super) fn write_hallway_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        let s = crate::render::shop_glb::shop_env_world_scale(camera.h, self.shop_env_height_scale);
        let model = crate::render::hallway_glb::with_hallway_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::shop_glb::shop_env_model_matrix_from_cpu(
                    camera.h,
                    self.shop_env_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(
            frame,
            camera,
            frame.scene_lighting.embedded_gltf_punctual,
            true,
            bloom_linear_hdr_output,
            model,
            gpu,
        );
    }
}
