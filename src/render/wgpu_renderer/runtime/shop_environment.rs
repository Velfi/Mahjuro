use super::camera::CameraFrame;
use super::*;

struct GltfRoomEnvUniformParams<'a> {
    frame: &'a crate::render::draw_cmd::UiFrame,
    camera: &'a CameraFrame,
    embedded_gltf_punctual: bool,
    hallway_env: bool,
    archive_env: bool,
    main_menu_env: bool,
    bloom_linear_hdr_output: bool,
    model: Mat4,
    gpu: &'a ShopEnvironmentGpu,
    shadow_upload: Option<([f32; 16], &'a mut bool)>,
}

impl WgpuRenderer {
    fn draw_gltf_room_env_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        room_hdr_mrt_emissive: bool,
        skip_prim: impl Fn(usize) -> bool,
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
                if skip_prim(pi) {
                    continue;
                }
                if prim.pipeline_key.is_blend() != blend_phase {
                    continue;
                }
                if last_key != Some(prim.pipeline_key) {
                    let pipe = if frame.uses_room_glb_shader() {
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

    /// Draw [`shop.glb`] environment primitives through `room_glb.wgsl` / `tile_3d.wgsl`
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
            |_| false,
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
            |_| false,
        );
    }

    /// Draw [`archive.glb`] Archive room.
    pub(super) fn draw_archive_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.archive_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.archive_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |pi| self.archive_env_skip_description_prim(pi, frame),
        );
    }

    fn write_gltf_room_env_uniforms<'a>(&self, p: GltfRoomEnvUniformParams<'a>) {
        let GltfRoomEnvUniformParams {
            frame,
            camera,
            embedded_gltf_punctual,
            hallway_env,
            archive_env,
            main_menu_env,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
        } = p;
        let height_scale = if main_menu_env {
            crate::render::main_menu_glb::main_menu_env_height_scale(self.room_gltf_height_scale)
        } else {
            self.room_gltf_height_scale
        };
        let s = crate::render::room_glb::room_env_world_scale(camera.h, height_scale);
        let inv_doc_scale = if embedded_gltf_punctual || frame.shop_inspect_lit_mesh_hdr {
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let lit_base = self.tile_hdr_tonemap(frame);
        let mut hdr_tonemap = if frame.shop_inspect_lit_mesh_hdr {
            let linear_hdr = lit_base[1] * crate::render::room_glb::SHOP_INSPECT_ENV_VS_LIT_LINEAR;
            let ambient = lit_base[2] * crate::render::room_glb::SHOP_INSPECT_ENV_VS_LIT_AMBIENT;
            [1.0, linear_hdr, ambient, 0.0]
        } else {
            lit_base
        };
        // `hdr_tonemap.w` is reserved / no-op now: `room_glb.wgsl::fs_main` and
        // `fs_main_emissive` both write linear HDR to their target, and
        // `tonemap_composite.wgsl` is the single ACES pass. `bloom_linear_hdr_output`
        // is informational only — the emissive pre-pass uses the same uniforms.
        let _ = bloom_linear_hdr_output;
        hdr_tonemap[3] = 1.0;
        let (exposure, ambient_x) = if embedded_gltf_punctual {
            let mut e = self.shop_env_linear_exposure
                * crate::render::room_glb::ROOM_GLB_LINEAR_EXPOSURE_BASE;
            let mut a = self.shop_env_ambient_scale;
            if hallway_env {
                e *= crate::render::hallway_glb::HALLWAY_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::render::hallway_glb::HALLWAY_ENV_AMBIENT_SCALE_MIN);
            }
            if archive_env {
                e *= crate::render::archive_glb::ARCHIVE_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::render::archive_glb::ARCHIVE_ENV_AMBIENT_SCALE_MIN);
            }
            if main_menu_env {
                e *= crate::render::main_menu_glb::MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::render::main_menu_glb::MAIN_MENU_ENV_AMBIENT_SCALE_MIN);
            }
            if !hallway_env && !archive_env && !main_menu_env {
                a = a.max(crate::render::room_glb::SHOP_ENV_DIELECTRIC_AMBIENT_MIN);
            }
            (e, a)
        } else if frame.shop_inspect_lit_mesh_hdr {
            (
                lit_base[1] * crate::render::room_glb::SHOP_INSPECT_STOREROOM_GLB_TILE_SEED_MUL,
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
        if let Some((lvp, changed)) = shadow_upload {
            self.write_room_env_shadow_caster(gpu, lvp, model, changed);
        }
    }

    /// Depth-only draws for imported room GLB opaque primitives.
    pub(super) fn draw_gltf_room_env_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        skip_prim: impl Fn(usize) -> bool,
    ) {
        if prims.is_empty() {
            return;
        }
        pass.set_pipeline(&self.shadow_pipeline_room_env);
        pass.set_bind_group(0, &gpu.shadow_bind_group, &[]);
        pass.set_bind_group(1, &gpu.shadow_warp_bind_group, &[]);
        for (pi, prim) in prims.iter().enumerate() {
            if skip_prim(pi) || prim.pipeline_key.is_blend() {
                continue;
            }
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, 0..1);
        }
    }

    pub(super) fn write_shop_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let s =
            crate::render::room_glb::room_env_world_scale(camera.h, self.room_gltf_height_scale);
        let model = crate::render::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    self.room_gltf_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
        });
    }

    pub(super) fn write_hallway_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        let s =
            crate::render::room_glb::room_env_world_scale(camera.h, self.room_gltf_height_scale);
        let model = crate::render::hallway_glb::with_hallway_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    self.room_gltf_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: true,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
        });
        let mut dist = frame.hallway_distortion.unwrap_or_default();
        dist.time_pulse[0] = self.creation_time.elapsed().as_secs_f32();
        self.queue
            .write_buffer(&gpu.distortion_buffer, 0, bytemuck::bytes_of(&dist));
    }

    /// Rasterize focused catalog copy into the archive room decal atlas (bound at group0
    /// `binding(3)` for every archive primitive; only `sign_description_*` vertices sample it).
    pub(super) fn sync_archive_description_decal_texture(
        &mut self,
        frame: &crate::render::draw_cmd::UiFrame,
    ) {
        let Some(gpu) = self.archive_environment.as_ref() else {
            return;
        };
        let Some(tex) = gpu.archive_sign_decal_texture.as_ref() else {
            return;
        };
        use crate::render::archive_glb::archive_sign_description_decal_extents;
        use crate::render::decal::{
            PLAQUE_DECAL_HEIGHT, PlaqueDecalStyle, decal_dimensions, rasterize_plaque_decal_styled,
        };
        use crate::render::primitive::DecalLayout;

        let layout = DecalLayout::Fit {
            target_short_edge: PLAQUE_DECAL_HEIGHT,
        };
        let (dw, dh) = decal_dimensions(&layout, archive_sign_description_decal_extents());
        let key = match frame.archive_sign_description_decal_text.as_ref() {
            None => u64::MAX,
            Some(t) => super::super::tablet_label_hash(t, dw, dh),
        };
        if key == self.archive_sign_decal_upload_key {
            return;
        }
        self.archive_sign_decal_upload_key = key;

        let rgba = match frame.archive_sign_description_decal_text.as_ref() {
            None => vec![0u8; (dw * dh * 4) as usize],
            Some(text) => rasterize_plaque_decal_styled(
                text,
                self.ui_font.as_ref(),
                self.emoji_font.as_ref(),
                dw,
                dh,
                PlaqueDecalStyle::WalnutInkOnLight,
            ),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dw),
                rows_per_image: Some(dh),
            },
            wgpu::Extent3d {
                width: dw,
                height: dh,
                depth_or_array_layers: 1,
            },
        );
    }

    pub(super) fn write_archive_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.archive_environment else {
            return;
        };
        let s =
            crate::render::room_glb::room_env_world_scale(camera.h, self.room_gltf_height_scale);
        let model = crate::render::archive_glb::with_archive_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    self.room_gltf_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            archive_env: true,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
        });
    }

    /// Draw [`main_menu.glb`] hub waterfront.
    pub(super) fn draw_main_menu_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::render::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.main_menu_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.main_menu_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |_| false,
        );
    }

    pub(super) fn write_main_menu_environment_uniforms(
        &self,
        frame: &crate::render::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.main_menu_environment else {
            return;
        };
        let env_h =
            crate::render::main_menu_glb::main_menu_env_height_scale(self.room_gltf_height_scale);
        let s = crate::render::room_glb::room_env_world_scale(camera.h, env_h);
        let model = crate::render::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            archive_env: false,
            main_menu_env: true,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
        });
    }

    /// Upload shop collision AABBs for per-punctual ray occlusion in `room_glb.wgsl`.
    pub(super) fn write_shop_room_punctual_occluders(&self, camera: &CameraFrame) {
        if self.shop_env_collision_meshes.is_empty() {
            return;
        }
        let model = crate::render::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    self.room_gltf_height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| {
            let s = crate::render::room_glb::room_env_world_scale(
                camera.h,
                self.room_gltf_height_scale,
            );
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let occ =
            TileOccludersBuf::from_room_collision_meshes(model, &self.shop_env_collision_meshes);
        self.queue
            .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
    }

    #[inline]
    pub(super) fn archive_env_skip_description_prim(
        &self,
        pi: usize,
        frame: &crate::render::draw_cmd::UiFrame,
    ) -> bool {
        match (
            self.archive_sign_left_prim_idx,
            self.archive_sign_right_prim_idx,
            frame.archive_description_sign_use_left,
        ) {
            (Some(_li), Some(ri), Some(true)) => pi == ri,
            (Some(li), Some(_ri), Some(false)) => pi == li,
            _ => false,
        }
    }
}
