use super::camera::CameraFrame;
use super::*;

struct GltfRoomEnvUniformParams<'a> {
    frame: &'a crate::draw_cmd::UiFrame,
    camera: &'a CameraFrame,
    env_scene_key: &'static str,
    embedded_gltf_punctual: bool,
    hallway_env: bool,
    staircase_env: bool,
    archive_env: bool,
    main_menu_env: bool,
    bloom_linear_hdr_output: bool,
    model: Mat4,
    gpu: &'a ShopEnvironmentGpu,
    shadow_upload: Option<([f32; 16], &'a mut bool)>,
    /// Cache uniform + model for shop glTF node TRS per-primitive draws.
    cache_shop_env_uniform: bool,
}

impl WgpuRenderer {
    fn shop_gltf_anim_prim_deltas(
        &self,
        frame: &crate::draw_cmd::UiFrame,
    ) -> rustc_hash::FxHashMap<usize, glam::Mat4> {
        if frame.shop_gltf_anim_samples.is_empty() {
            return rustc_hash::FxHashMap::default();
        }
        let deltas = self
            .shop_gltf_anim
            .resolve_prim_deltas(&frame.shop_gltf_anim_samples);
        if deltas.is_empty() && !self.shop_gltf_anim_missing_clip_warned.replace(true) {
            log::warn!(
                "shop glTF anim: playback requested but no clip/primitive bindings matched"
            );
        } else if !deltas.is_empty() {
            self.shop_gltf_anim_missing_clip_warned.set(false);
        }
        deltas
    }

    fn draw_gltf_room_env_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        prims: &[TilePrimitiveGpu],
        gpu: &ShopEnvironmentGpu,
        room_hdr_mrt_emissive: bool,
        skip_prim: impl Fn(usize) -> bool,
    ) {
        if prims.is_empty() {
            return;
        }
        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
        pass.set_bind_group(2, self.room_shadow_sample_bind_group(), &[]);
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
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let eyeball_only = frame.shop_env_eyeball_only;
        let eyeball_indices: Vec<usize> = if !self.shop_eyeball_prim_indices.is_empty() {
            self.shop_eyeball_prim_indices.clone()
        } else {
            self.shop_gltf_anim
                .clip_prim_bindings
                .get("eyeball_travel")
                .map(|b| b.iter().map(|(pi, _)| *pi).collect())
                .unwrap_or_default()
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.shop_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |pi| eyeball_only && !eyeball_indices.is_empty() && !eyeball_indices.contains(&pi),
        );
    }

    /// Draw [`gameplay.glb`] table room.
    pub(super) fn draw_gameplay_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.gameplay_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.gameplay_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |pi| self.gameplay_env_skip_cash_in_prim(pi, frame),
        );
    }

    /// Draw [`hallway.glb`] (pick-blind room).
    pub(super) fn draw_hallway_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
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

    /// Draw [`staircase.glb`] (post-ordeal interstitial).
    pub(super) fn draw_staircase_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
        room_hdr_mrt_emissive: bool,
    ) {
        let Some(ref gpu) = self.staircase_environment else {
            return;
        };
        self.draw_gltf_room_env_meshes(
            pass,
            frame,
            &self.staircase_env_primitives,
            gpu,
            room_hdr_mrt_emissive,
            |_| false,
        );
    }

    /// Draw [`archive.glb`] Archive room.
    pub(super) fn draw_archive_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
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
            |pi| self.archive_env_skip_archive_prim(pi, frame),
        );
    }

    fn write_gltf_room_env_uniforms<'a>(&self, p: GltfRoomEnvUniformParams<'a>) {
        let GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key,
            embedded_gltf_punctual,
            hallway_env,
            staircase_env,
            archive_env,
            main_menu_env,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform,
        } = p;
        let env_tune = self.env_tune_for(env_scene_key);
        let height_scale = if main_menu_env {
            crate::main_menu_glb::main_menu_env_height_scale(env_tune.height_scale)
        } else {
            env_tune.height_scale
        };
        let s = crate::room_glb::room_env_world_scale(camera.h, height_scale);
        let inv_doc_scale = if embedded_gltf_punctual {
            1.0 / s.max(1e-6)
        } else {
            0.0
        };
        let mut hdr_tonemap = self.tile_hdr_tonemap(frame);
        // `hdr_tonemap.w` is reserved / no-op now: `room_glb.wgsl::fs_main` and
        // `fs_main_emissive` both write linear HDR to their target, and
        // `tonemap_composite.wgsl` is the single ACES pass. `bloom_linear_hdr_output`
        // is informational only — the emissive pre-pass uses the same uniforms.
        let _ = bloom_linear_hdr_output;
        hdr_tonemap[3] = 1.0;
        let (exposure, ambient_x) = if embedded_gltf_punctual {
            let gameplay_table = matches!(env_scene_key, "gameplay" | "tutorial");
            // `gameplay.glb` room + tiles share `ROOM_GLB_LINEAR_EXPOSURE_BASE` × this mul via `tile_hdr_tonemap`.
            let mut e = env_tune.linear_exposure
                * crate::room_glb::ROOM_GLB_LINEAR_EXPOSURE_BASE;
            if gameplay_table {
                e *= crate::gameplay_glb::GAMEPLAY_ENV_LINEAR_EXPOSURE_MUL;
            }
            let mut a = env_tune.ambient_scale;
            if hallway_env {
                e *= crate::hallway_glb::HALLWAY_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::hallway_glb::HALLWAY_ENV_AMBIENT_SCALE_MIN);
            }
            if staircase_env {
                e *= crate::staircase_glb::STAIRCASE_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::staircase_glb::STAIRCASE_ENV_AMBIENT_SCALE_MIN);
            }
            if archive_env {
                e *= crate::archive_glb::ARCHIVE_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::archive_glb::ARCHIVE_ENV_AMBIENT_SCALE_MIN);
            }
            if main_menu_env {
                e *= crate::main_menu_glb::MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL;
                a = a.max(crate::main_menu_glb::MAIN_MENU_ENV_AMBIENT_SCALE_MIN);
            }
            // Gameplay/tutorial GLB: windowless interior — authored ambient only (usually 0).
            if !gameplay_table && !hallway_env && !staircase_env && !archive_env && !main_menu_env {
                a = a.max(crate::room_glb::SHOP_ENV_DIELECTRIC_AMBIENT_MIN);
            }
            (e, a)
        } else {
            (0.0, 0.0)
        };
        let uniform = CameraUniform {
            view_proj: camera.view_proj_arr,
            model: model.to_cols_array(),
            base_color_factor: [
                1.0,
                if frame.shop_env_unlit_debug { 1.0 } else { 0.0 },
                0.0,
                crate::tile_body::TEXTURED_BASE_MAP_BODY_KIND,
            ],
            cam_pos: camera.cam_pos.to_array(),
            tile_seed: if frame.shop_env_unlit_debug {
                1.0
            } else {
                exposure
            },
            decal_atlas_uv: [
                if frame.shop_env_unlit_debug {
                    1.0
                } else {
                    ambient_x
                },
                inv_doc_scale,
                env_tune.gltf_emissive_scale,
                1.0,
            ],
            hdr_tonemap,
        };
        let anim_deltas = if cache_shop_env_uniform {
            self.shop_gltf_anim_prim_deltas(frame)
        } else {
            rustc_hash::FxHashMap::default()
        };
        for (pi, buf) in gpu.uniform_buffers.iter().enumerate() {
            let prim_model = if let Some(delta) = anim_deltas.get(&pi) {
                model * *delta
            } else {
                model
            };
            let mut u = uniform;
            u.model = prim_model.to_cols_array();
            self.queue.write_buffer(buf, 0, bytemuck::bytes_of(&u));
        }
        if let Some((lvp, changed)) = shadow_upload {
            self.write_room_env_shadow_caster(gpu, lvp, model, &anim_deltas, changed);
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
        pass.set_bind_group(1, &gpu.shadow_warp_bind_group, &[]);
        for (pi, prim) in prims.iter().enumerate() {
            if skip_prim(pi) || prim.pipeline_key.is_blend() || prim.index_count == 0 {
                continue;
            }
            if prim.vertex_buffer.size() == 0 || prim.index_buffer.size() == 0 {
                continue;
            }
            let Some(bg) = gpu.shadow_bind_groups.get(pi) else {
                continue;
            };
            pass.set_bind_group(0, bg, &[]);
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, 0..1);
        }
    }

    pub(super) fn draw_shop_environment_shadow(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let eyeball_only = frame.shop_env_eyeball_only;
        let eyeball_indices: Vec<usize> = if !self.shop_eyeball_prim_indices.is_empty() {
            self.shop_eyeball_prim_indices.clone()
        } else {
            self.shop_gltf_anim
                .clip_prim_bindings
                .get("eyeball_travel")
                .map(|b| b.iter().map(|(pi, _)| *pi).collect())
                .unwrap_or_default()
        };
        self.draw_gltf_room_env_shadow(
            pass,
            &self.shop_env_primitives,
            gpu,
            |pi| eyeball_only && !eyeball_indices.is_empty() && !eyeball_indices.contains(&pi),
        );
    }

    pub(super) fn write_shop_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.shop_environment else {
            return;
        };
        let height = self.env_tune_for("shop").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, height, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "shop",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform: true,
        });
    }

    pub(super) fn write_gameplay_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.gameplay_environment else {
            return;
        };
        let env_key = if self.active_scene_key == Some("tutorial") {
            "tutorial"
        } else {
            "gameplay"
        };
        let height = self.env_tune_for(env_key).height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::gameplay_glb::with_gameplay_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, height, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: env_key,
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            staircase_env: false,
            cache_shop_env_uniform: false,
        });
    }

    pub(super) fn write_hallway_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.hallway_environment else {
            return;
        };
        let height = self.env_tune_for("pick_chamber").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::hallway_glb::with_hallway_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "pick_chamber",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: true,
            staircase_env: false,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform: false,
        });
        let mut dist = frame.hallway_distortion.unwrap_or_default();
        dist.time_pulse[0] = self.creation_time.elapsed().as_secs_f32();
        self.queue
            .write_buffer(&gpu.distortion_buffer, 0, bytemuck::bytes_of(&dist));
    }

    pub(super) fn write_staircase_environment_uniforms(
        &self,
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.staircase_environment else {
            return;
        };
        let height = self.env_tune_for("staircase").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::staircase_glb::with_staircase_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "staircase",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: true,
            archive_env: false,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform: false,
        });
    }

    /// Rasterize focused catalog copy into the archive room decal atlas (bound at group0
    /// `binding(3)` for every archive primitive; only `sign_description_*` vertices sample it).
    pub(super) fn sync_archive_description_decal_texture(
        &mut self,
        frame: &crate::draw_cmd::UiFrame,
    ) {
        let Some(gpu) = self.archive_environment.as_ref() else {
            return;
        };
        let Some(tex) = gpu.archive_sign_decal_texture.as_ref() else {
            return;
        };
        use crate::archive_glb::archive_sign_description_decal_extents;
        use crate::decal::{
            PLAQUE_DECAL_HEIGHT, PlaqueDecalStyle, decal_dimensions, rasterize_plaque_decal_styled,
        };
        use crate::primitive::DecalLayout;

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
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.archive_environment else {
            return;
        };
        let height = self.env_tune_for("collection").height_scale;
        let s = crate::room_glb::room_env_world_scale(camera.h, height);
        let model = crate::archive_glb::with_archive_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "collection",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: true,
            main_menu_env: false,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform: false,
        });
    }

    /// Draw [`main_menu.glb`] hub waterfront.
    pub(super) fn draw_main_menu_environment_meshes(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &crate::draw_cmd::UiFrame,
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
        frame: &crate::draw_cmd::UiFrame,
        camera: &CameraFrame,
        bloom_linear_hdr_output: bool,
        shadow_upload: Option<([f32; 16], &mut bool)>,
    ) {
        let Some(ref gpu) = self.main_menu_environment else {
            return;
        };
        let height = self.env_tune_for("main_menu_exterior").height_scale;
        let env_h =
            crate::main_menu_glb::main_menu_env_height_scale(height);
        let s = crate::room_glb::room_env_world_scale(camera.h, env_h);
        let model = crate::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(glam::Vec3::splat(s)));
        self.write_gltf_room_env_uniforms(GltfRoomEnvUniformParams {
            frame,
            camera,
            env_scene_key: "main_menu_exterior",
            embedded_gltf_punctual: frame.scene_lighting.embedded_gltf_punctual,
            hallway_env: false,
            staircase_env: false,
            archive_env: false,
            main_menu_env: true,
            bloom_linear_hdr_output,
            model,
            gpu,
            shadow_upload,
            cache_shop_env_uniform: false,
        });
    }

    /// Upload shop collision AABBs for per-punctual ray occlusion in `room_glb.wgsl`.
    pub(super) fn write_shop_room_punctual_occluders(&self, camera: &CameraFrame) {
        if self.shop_env_collision_meshes.is_empty() {
            return;
        }
        let height = self.env_tune_for("shop").height_scale;
        let model = crate::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    camera.h,
                    height,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| {
            let s = crate::room_glb::room_env_world_scale(
                camera.h,
                self.env_tune_for("shop").height_scale,
            );
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let occ =
            TileOccludersBuf::from_room_collision_meshes(model, &self.shop_env_collision_meshes);
        self.queue
            .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
    }

    /// Upload main-menu roof AABBs so porch punctuals respect `rooflet` / main roof shells.
    pub(super) fn write_main_menu_room_punctual_occluders(&self, camera: &CameraFrame) {
        if self.main_menu_env_collision_meshes.is_empty() {
            return;
        }
        let height = self.env_tune_for("main_menu_exterior").height_scale;
        let env_h =
            crate::main_menu_glb::main_menu_env_height_scale(height);
        let model = crate::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(camera.h, env_h, cpu)
            })
        })
        .unwrap_or_else(|| {
            let s = crate::room_glb::room_env_world_scale(camera.h, env_h);
            glam::Mat4::from_scale(glam::Vec3::splat(s))
        });
        let occ = TileOccludersBuf::from_room_collision_meshes(
            model,
            &self.main_menu_env_collision_meshes,
        );
        self.queue
            .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
    }

    /// Lit pass: hide authored cash-in control when structure cannot be scored.
    #[inline]
    fn gameplay_env_skip_cash_in_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        !frame.gameplay_cash_in_button_visible && self.gameplay_cash_in_prim_indices.contains(&pi)
    }

    #[inline]
    fn archive_env_is_description_sign_prim(&self, pi: usize) -> bool {
        self.archive_sign_left_prim_idx == Some(pi) || self.archive_sign_right_prim_idx == Some(pi)
    }

    /// Lit pass: draw only the active description board (opposite the focus ref).
    #[inline]
    pub(super) fn archive_env_skip_archive_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        self.archive_env_skip_description_prim(pi, frame)
            || self.archive_env_skip_page_button_prim(pi, frame)
    }

    #[inline]
    fn archive_env_skip_page_button_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        if self.archive_page_left_prim_indices.contains(&pi) {
            return !frame.archive_page_left_visible;
        }
        if self.archive_page_right_prim_indices.contains(&pi) {
            return !frame.archive_page_right_visible;
        }
        false
    }

    /// Lit pass: draw only the active description board (opposite the focus ref).
    #[inline]
    pub(super) fn archive_env_skip_description_prim(
        &self,
        pi: usize,
        frame: &crate::draw_cmd::UiFrame,
    ) -> bool {
        if !self.archive_env_is_description_sign_prim(pi) {
            return false;
        }
        match frame.archive_description_sign_use_left {
            Some(true) => self.archive_sign_right_prim_idx == Some(pi),
            Some(false) => self.archive_sign_left_prim_idx == Some(pi),
            _ => false,
        }
    }

    /// Shadow pre-pass / offline bake: never cast from either sign — they are flat
    /// decal boards and project hard silhouettes onto the featured pedestal.
    #[inline]
    pub(super) fn archive_env_skip_room_shadow_caster(&self, pi: usize) -> bool {
        self.archive_env_shadow_caster_mask
            .get(pi)
            .is_some_and(|casts| !*casts)
    }
}
