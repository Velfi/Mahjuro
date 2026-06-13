use std::sync::Arc;

use super::*;
use crate::lit_mesh::create_shadow_sample_bind_group;
use crate::room_gi_bake::{RoomGiRoom, room_gi_room_index};
use crate::room_shadow_bake::{self, RoomShadowBake};
use crate::wgpu_renderer::runtime::shadow_setup::{ActiveRoomEnv, build_shadow_globals};
use crate::wgpu_renderer::runtime::{agent_shadow_log, probe_baked_ao_at_world};
use mahjuro_gfx_types::ShadowQuality;

pub(crate) struct RoomBakedShadowGpu {
    pub sample_bind_group: wgpu::BindGroup,
    pub globals_buffer: wgpu::Buffer,
    pub baked_light_view_proj: [f32; 16],
    _ao_texture: wgpu::Texture,
    _depth_texture: wgpu::Texture,
}

impl WgpuRenderer {
    /// Recreate point/spot depth arrays when shadow quality tier changes resolution.
    pub(super) fn recreate_shadow_depth_arrays_if_needed(
        &mut self,
        quality: ShadowQuality,
    ) -> bool {
        if !quality.active() {
            return false;
        }
        let point_size = quality.point_map_size();
        let spot_size = quality.spot_map_size();
        if self.point_shadow_array.size == point_size && self.spot_shadow_array.size == spot_size {
            return false;
        }

        use crate::lit_mesh::{create_shadow_depth_array, create_shadow_sample_bind_group};
        use crate::wgpu_renderer::constants::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS};

        self.point_shadow_array = create_shadow_depth_array(
            &self.device,
            "point-shadow-array",
            point_size,
            MAX_POINT_LIGHTS as u32,
        );
        self.spot_shadow_array = create_shadow_depth_array(
            &self.device,
            "spot-shadow-array",
            spot_size,
            MAX_SPOT_LIGHTS as u32,
        );
        self.shadow_sample_bind_group = create_shadow_sample_bind_group(
            &self.device,
            &self.shadow_sample_layout,
            "shadow-sample-bg",
            &self.shadow_globals_buffer,
            &self.point_shadow_array.array_view,
            &self.spot_shadow_array.array_view,
            &self.shadow_compare_sampler,
            &self.shadow_ao_white_view,
            &self.shadow_ao_sampler,
            &self.shadow_baked_depth_dummy_view,
        );
        for gpu in self.room_baked_shadow_gpu.iter_mut().flatten() {
            let ao_view = gpu._ao_texture.create_view(&Default::default());
            let depth_view = gpu._depth_texture.create_view(&Default::default());
            gpu.sample_bind_group = create_shadow_sample_bind_group(
                &self.device,
                &self.shadow_sample_layout,
                "room-baked-shadow-sample-bg",
                &gpu.globals_buffer,
                &self.point_shadow_array.array_view,
                &self.spot_shadow_array.array_view,
                &self.shadow_compare_sampler,
                &ao_view,
                &self.shadow_ao_sampler,
                &depth_view,
            );
        }
        if let Some((_, gpu)) = self.lab_baked_shadow.as_mut() {
            let ao_view = gpu._ao_texture.create_view(&Default::default());
            let depth_view = gpu._depth_texture.create_view(&Default::default());
            gpu.sample_bind_group = create_shadow_sample_bind_group(
                &self.device,
                &self.shadow_sample_layout,
                "lab-baked-shadow-sample-bg",
                &gpu.globals_buffer,
                &self.point_shadow_array.array_view,
                &self.spot_shadow_array.array_view,
                &self.shadow_compare_sampler,
                &ao_view,
                &self.shadow_ao_sampler,
                &depth_view,
            );
        }
        self.cached_projected_shadow_hash = 0;
        true
    }

    pub fn request_room_shadow_capture(&mut self, room: RoomGiRoom) {
        self.room_shadow_capture_pending = Some(room);
        self.room_shadow_captured = None;
    }

    pub fn take_room_shadow_capture(&mut self) -> Option<RoomShadowBake> {
        self.room_shadow_captured.take()
    }

    /// GPU-upload offline `.msh` contact AO for one room.
    pub(super) fn ensure_room_baked_shadow_gpu(&mut self, room: RoomGiRoom) -> bool {
        let idx = room_gi_room_index(room);
        if self.room_baked_shadow_gpu[idx].is_some() {
            return true;
        }
        let Some(bake) = room_shadow_bake::cached_room_shadow_bake(room) else {
            return false;
        };
        match Self::upload_room_baked_shadow_gpu(
            &self.device,
            &self.queue,
            &self.shadow_sample_layout,
            &self.point_shadow_array.array_view,
            &self.spot_shadow_array.array_view,
            &self.shadow_compare_sampler,
            &self.shadow_ao_sampler,
            room,
            &bake,
        ) {
            Ok(gpu) => {
                // #region agent log
                if let Ok(bake) = room_shadow_bake::require_room_shadow_bake(room) {
                    if let Some(ao) = bake.ao_bytes.as_ref() {
                        let mut dark = 0u32;
                        for &b in ao.iter() {
                            if b < 128 {
                                dark += 1;
                            }
                        }
                        agent_shadow_log(
                            "H1",
                            "impl_room_shadow.rs:ensure_room_baked_shadow_gpu",
                            "room baked shadow GPU loaded",
                            serde_json::json!({
                                "room": format!("{room:?}"),
                                "gpu_loaded": true,
                                "ao_dark_frac": dark as f32 / ao.len().max(1) as f32,
                                "origin_probe": probe_baked_ao_at_world(
                                    gpu.baked_light_view_proj,
                                    ao,
                                    bake.width,
                                    bake.height,
                                    glam::Vec3::ZERO,
                                )
                                .map(|(ndc, uv, a): (glam::Vec3, [f32; 2], u8)| {
                                    serde_json::json!({
                                        "ndc": ndc.to_array(),
                                        "uv": uv,
                                        "ao": a,
                                    })
                                }),
                            }),
                        );
                    }
                }
                // #endregion
                self.room_baked_shadow_gpu[idx] = Some(gpu);
                true
            }
            Err(e) => {
                panic!("room shadow GPU upload for {room:?}: {e:#}");
            }
        }
    }

    fn upload_room_baked_shadow_gpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shadow_sample_layout: &wgpu::BindGroupLayout,
        point_depth_view: &wgpu::TextureView,
        spot_depth_view: &wgpu::TextureView,
        compare_sampler: &wgpu::Sampler,
        ao_sampler: &wgpu::Sampler,
        _room: RoomGiRoom,
        bake: &RoomShadowBake,
    ) -> anyhow::Result<RoomBakedShadowGpu> {
        let w = bake.width;
        let h = bake.height;

        let ao_bytes = bake
            .ao_bytes
            .clone()
            .expect("room shadow bake missing AO bytes after require_effective_room_shadow_bake");
        let ao_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-baked-shadow-ao"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ao_view = ao_texture.create_view(&Default::default());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &ao_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &ao_bytes,
            wgpu::TexelCopyBufferLayout {
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
                ..Default::default()
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-baked-shadow-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&Default::default());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &depth_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bake.depth_bytes,
            wgpu::TexelCopyBufferLayout {
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
                ..Default::default()
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let globals = build_shadow_globals(
            ShadowQuality::Medium,
            &crate::projected_light_shadow::PunctualShadowBuild::empty(),
            true,
            bake.light_view_proj,
            crate::room_shadow_bake::contact_ao_world_scale_ratio(
                crate::room_shadow_bake::ROOM_SHADOW_BAKE_REFERENCE_H,
            ),
            0.0,
        );
        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-baked-shadow-globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sample_bind_group = create_shadow_sample_bind_group(
            device,
            shadow_sample_layout,
            "room-baked-shadow-sample-bg",
            &globals_buffer,
            point_depth_view,
            spot_depth_view,
            compare_sampler,
            &ao_view,
            ao_sampler,
            &depth_view,
        );
        Ok(RoomBakedShadowGpu {
            sample_bind_group,
            globals_buffer,
            baked_light_view_proj: bake.light_view_proj,
            _ao_texture: ao_texture,
            _depth_texture: depth_texture,
        })
    }

    /// Sync active baked contact AO globals with the live punctual pass.
    pub(super) fn write_active_room_baked_shadow_globals(
        &self,
        shadow_quality: ShadowQuality,
        build: &crate::projected_light_shadow::PunctualShadowBuild,
        window_h: f32,
        contact_ao_active: bool,
        dynamic_receiver_shadow_strength: f32,
    ) {
        let (gpu, contact_ao_world_scale) = if self.active_lab_baked_shadow {
            let Some((_, gpu)) = &self.lab_baked_shadow else {
                return;
            };
            (gpu, crate::shadow_ao_lab::CONTACT_AO_WORLD_SCALE)
        } else {
            let Some(room) = self.active_room_baked_shadow else {
                return;
            };
            let Some(gpu) = self.room_baked_shadow_gpu[room_gi_room_index(room)].as_ref() else {
                return;
            };
            (
                gpu,
                crate::room_shadow_bake::contact_ao_world_scale_ratio(window_h),
            )
        };
        let globals = build_shadow_globals(
            shadow_quality,
            build,
            contact_ao_active,
            gpu.baked_light_view_proj,
            contact_ao_world_scale,
            dynamic_receiver_shadow_strength,
        );
        self.queue
            .write_buffer(&gpu.globals_buffer, 0, bytemuck::bytes_of(&globals));
    }

    pub(super) fn ensure_lab_baked_shadow_gpu(
        &mut self,
        layout: crate::shadow_ao_lab::ShadowAoLabLayout,
    ) {
        if self
            .lab_baked_shadow
            .as_ref()
            .is_some_and(|(l, _)| *l == layout)
        {
            return;
        }
        let bake = crate::shadow_ao_lab::synthetic_bake(layout);
        match Self::upload_room_baked_shadow_gpu(
            &self.device,
            &self.queue,
            &self.shadow_sample_layout,
            &self.point_shadow_array.array_view,
            &self.spot_shadow_array.array_view,
            &self.shadow_compare_sampler,
            &self.shadow_ao_sampler,
            RoomGiRoom::Hallway,
            &bake,
        ) {
            Ok(gpu) => {
                self.lab_baked_shadow = Some((layout, gpu));
            }
            Err(e) => panic!("lab shadow GPU upload: {e:#}"),
        }
    }

    pub(super) fn upload_active_room_baked_shadow_globals(&mut self, frame: &UiFrame) {
        if self.room_shadow_capture_pending.is_some() {
            self.active_room_baked_shadow = None;
            self.active_lab_baked_shadow = false;
            return;
        }
        if let Some(layout) = frame.shadow_ao_lab_layout {
            self.ensure_lab_baked_shadow_gpu(layout);
            self.active_room_baked_shadow = None;
            self.active_lab_baked_shadow = true;
            return;
        }
        self.active_lab_baked_shadow = false;
        if let Some(room) = crate::wgpu_renderer::runtime::shadow_setup::active_baked_shadow_room(
            frame,
            self.active_scene_key.map(|key| key as &str),
        ) {
            self.active_room_baked_shadow = if self.ensure_room_baked_shadow_gpu(room) {
                Some(room)
            } else {
                None
            };
        } else {
            self.active_room_baked_shadow = None;
        }
    }

    pub(super) fn room_shadow_sample_bind_group(&self) -> &wgpu::BindGroup {
        if self.active_lab_baked_shadow
            && let Some((_, gpu)) = &self.lab_baked_shadow
        {
            return &gpu.sample_bind_group;
        }
        if let Some(room) = self.active_room_baked_shadow
            && let Some(gpu) = self.room_baked_shadow_gpu[room_gi_room_index(room)].as_ref()
        {
            return &gpu.sample_bind_group;
        }
        &self.shadow_sample_bind_group
    }

    pub(crate) fn encode_room_shadow_capture_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UiFrame,
        room: RoomGiRoom,
        width: u32,
        height: u32,
        light_view_proj: [f32; 16],
        depth_bias: f32,
        camera_h: f32,
    ) -> RoomShadowCaptureStaging {
        let depth_byte_len = super::resources::depth_copy_buffer_size(width, height);
        let depth_staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-shadow-capture-staging"),
            size: depth_byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mask_byte_len = super::resources::rgba8_copy_buffer_size(width, height);
        let mask_staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-shadow-mask-capture-staging"),
            size: mask_byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let normal_byte_len = super::resources::rgba8_copy_buffer_size(width, height);
        let normal_staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-shadow-normal-capture-staging"),
            size: normal_byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let extent = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let capture_depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-shadow-capture-depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let capture_depth_view = capture_depth_texture.create_view(&Default::default());
        let capture_mask_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-shadow-capture-mask"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let capture_mask_view = capture_mask_texture.create_view(&Default::default());
        let capture_normal_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room-shadow-capture-normal"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let capture_normal_view = capture_normal_texture.create_view(&Default::default());
        let (capture_depth_r32_texture, capture_depth_r32_view) =
            super::resources::create_depth_r32_snapshot(
                &self.device,
                width,
                height,
                "room-shadow-capture-depth-r32",
            );
        let active_env = active_room_env_for_shadow_bake(room);
        let room_model = self
            .room_env_shadow_base_model(active_env, camera_h)
            .to_cols_array();
        let prim_deltas = self.room_env_shadow_prim_deltas(active_env, frame);
        if let Some((_, gpu)) = self.room_shadow_capture_env(room) {
            let mut changed = false;
            self.write_room_env_shadow_caster(
                gpu,
                light_view_proj,
                glam::Mat4::from_cols_array(&room_model),
                &prim_deltas,
                &mut changed,
            );
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("room-shadow-capture-mask-pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &capture_mask_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &capture_normal_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &capture_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.draw_room_shadow_capture_mask(&mut pass, room, frame);
        }
        self.encode_blit_depth_view_to_r32(
            encoder,
            &capture_depth_view,
            &capture_depth_r32_view,
            width,
            height,
        );
        super::resources::copy_r32_texture_to_buffer(
            encoder,
            &depth_staging,
            &capture_depth_r32_texture,
            width,
            height,
        );
        super::resources::copy_rgba8_texture_to_buffer(
            encoder,
            &mask_staging,
            &capture_mask_texture,
            width,
            height,
        );
        super::resources::copy_rgba8_texture_to_buffer(
            encoder,
            &normal_staging,
            &capture_normal_texture,
            width,
            height,
        );
        let width = width.max(1);
        let height = height.max(1);
        RoomShadowCaptureStaging {
            depth_buffer: depth_staging,
            depth_byte_len,
            mask_buffer: mask_staging,
            mask_byte_len,
            normal_buffer: normal_staging,
            normal_byte_len,
            room,
            width,
            height,
            light_view_proj,
            depth_bias,
            _capture_depth_texture: capture_depth_texture,
            _capture_mask_texture: capture_mask_texture,
            _capture_normal_texture: capture_normal_texture,
            _capture_depth_r32_texture: capture_depth_r32_texture,
        }
    }

    pub(crate) fn finalize_room_shadow_capture(
        &self,
        staging: RoomShadowCaptureStaging,
    ) -> anyhow::Result<RoomShadowBake> {
        let depth_slice = staging.depth_buffer.slice(..staging.depth_byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        depth_slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| anyhow::anyhow!("room shadow capture map channel closed"))?
            .map_err(|e| anyhow::anyhow!("room shadow capture map failed: {e:?}"))?;
        let mapped = depth_slice.get_mapped_range();
        let depth_bytes: Arc<[u8]> = Arc::from(tight_r32_from_padded(
            &mapped,
            staging.width,
            staging.height,
        ));
        drop(mapped);
        staging.depth_buffer.unmap();

        let mask_slice = staging.mask_buffer.slice(..staging.mask_byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        mask_slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| anyhow::anyhow!("room shadow mask capture map channel closed"))?
            .map_err(|e| anyhow::anyhow!("room shadow mask capture map failed: {e:?}"))?;
        let mapped = mask_slice.get_mapped_range();
        let mask_bytes = tight_rgba8_from_padded(&mapped, staging.width, staging.height);
        drop(mapped);
        staging.mask_buffer.unmap();

        let normal_slice = staging.normal_buffer.slice(..staging.normal_byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        normal_slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| anyhow::anyhow!("room shadow normal capture map channel closed"))?
            .map_err(|e| anyhow::anyhow!("room shadow normal capture map failed: {e:?}"))?;
        let mapped = normal_slice.get_mapped_range();
        let normal_bytes = tight_rgba8_from_padded(&mapped, staging.width, staging.height);
        drop(mapped);
        staging.normal_buffer.unmap();

        let ao: Arc<[u8]> = Arc::from(room_shadow_bake::bake_contact_ao_for_room_from_mask(
            staging.room,
            staging.width,
            staging.height,
            &depth_bytes,
            &mask_bytes,
            &normal_bytes,
        )?);
        Ok(RoomShadowBake {
            room: staging.room,
            width: staging.width,
            height: staging.height,
            light_view_proj: staging.light_view_proj,
            depth_bias: staging.depth_bias,
            depth_bytes,
            ao_bytes: Some(ao),
        })
    }
}

impl WgpuRenderer {
    fn room_shadow_capture_env(
        &self,
        room: RoomGiRoom,
    ) -> Option<(&[TilePrimitiveGpu], &ShopEnvironmentGpu)> {
        match room {
            RoomGiRoom::Shop => Some((
                self.shop_env_primitives.as_slice(),
                self.shop_environment.as_ref()?,
            )),
            RoomGiRoom::Hallway => Some((
                self.hallway_env_primitives.as_slice(),
                self.hallway_environment.as_ref()?,
            )),
            RoomGiRoom::Archive => Some((
                self.archive_env_primitives.as_slice(),
                self.archive_environment.as_ref()?,
            )),
            RoomGiRoom::MainMenu => Some((
                self.main_menu_env_primitives.as_slice(),
                self.main_menu_environment.as_ref()?,
            )),
            RoomGiRoom::Stairway => Some((
                self.staircase_env_primitives.as_slice(),
                self.staircase_environment.as_ref()?,
            )),
            RoomGiRoom::Gameplay => Some((
                self.gameplay_env_primitives.as_slice(),
                self.gameplay_environment.as_ref()?,
            )),
            RoomGiRoom::ShadowTestRoom => Some((
                self.shadow_test_room_env_primitives.as_slice(),
                self.shadow_test_room_environment.as_ref()?,
            )),
        }
    }

    fn draw_room_shadow_capture_mask(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        room: RoomGiRoom,
        frame: &UiFrame,
    ) -> u32 {
        let Some((prims, gpu)) = self.room_shadow_capture_env(room) else {
            return 0;
        };
        if prims.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.room_shadow_mask_pipeline);
        pass.set_bind_group(1, &gpu.shadow_warp_bind_group, &[]);
        let mut draws = 0u32;
        for (pi, prim) in prims.iter().enumerate() {
            if self.room_shadow_capture_skip_prim(room, frame, pi)
                || prim.pipeline_key.is_blend()
                || prim.index_count == 0
            {
                continue;
            }
            if prim.vertex_buffer.size() == 0 || prim.index_buffer.size() == 0 {
                continue;
            }
            let (Some(shadow_bg), Some(mask_bg)) = (
                gpu.shadow_bind_groups.get(pi),
                gpu.shadow_mask_bind_groups.get(pi),
            ) else {
                continue;
            };
            pass.set_bind_group(0, shadow_bg, &[]);
            pass.set_bind_group(2, mask_bg, &[]);
            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.index_count, 0, 0..1);
            draws += 1;
        }
        draws
    }

    fn room_shadow_capture_skip_prim(&self, room: RoomGiRoom, frame: &UiFrame, pi: usize) -> bool {
        match room {
            RoomGiRoom::Shop => {
                frame.shop_env_eyeball_only
                    && !self.shop_eyeball_prim_indices.is_empty()
                    && !self.shop_eyeball_prim_indices.contains(&pi)
            }
            RoomGiRoom::Hallway | RoomGiRoom::Stairway => false,
            RoomGiRoom::Archive => self.archive_env_skip_shadow_prim(pi, frame),
            RoomGiRoom::MainMenu => self.main_menu_env_skip_prim(pi, frame),
            RoomGiRoom::Gameplay => self.gameplay_env_skip_prim(pi, frame),
            RoomGiRoom::ShadowTestRoom => false,
        }
    }
}

fn tight_rgba8_from_padded(mapped: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = width.max(1) as usize * 4;
    let padded_row = super::resources::rgba8_copy_bytes_per_row(width) as usize;
    let height = height.max(1) as usize;
    if row == padded_row {
        return mapped[..row * height].to_vec();
    }
    let mut out = vec![0u8; row * height];
    for y in 0..height {
        let src = y * padded_row;
        let dst = y * row;
        out[dst..dst + row].copy_from_slice(&mapped[src..src + row]);
    }
    out
}

fn tight_r32_from_padded(mapped: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = width.max(1) as usize * 4;
    let padded_row = super::resources::depth_copy_buffer_size(width, 1) as usize;
    let height = height.max(1) as usize;
    if row == padded_row {
        return mapped[..row * height].to_vec();
    }
    let mut out = vec![0u8; row * height];
    for y in 0..height {
        let src = y * padded_row;
        let dst = y * row;
        out[dst..dst + row].copy_from_slice(&mapped[src..src + row]);
    }
    out
}

pub(crate) struct RoomShadowCaptureStaging {
    depth_buffer: wgpu::Buffer,
    depth_byte_len: u64,
    mask_buffer: wgpu::Buffer,
    mask_byte_len: u64,
    normal_buffer: wgpu::Buffer,
    normal_byte_len: u64,
    room: RoomGiRoom,
    width: u32,
    height: u32,
    light_view_proj: [f32; 16],
    depth_bias: f32,
    _capture_depth_texture: wgpu::Texture,
    _capture_mask_texture: wgpu::Texture,
    _capture_normal_texture: wgpu::Texture,
    _capture_depth_r32_texture: wgpu::Texture,
}

impl ActiveRoomEnv {
    pub fn from_frame(frame: &UiFrame) -> Option<Self> {
        crate::wgpu_renderer::runtime::shadow_setup::active_room_env(frame)
    }
}

fn active_room_env_for_shadow_bake(room: RoomGiRoom) -> ActiveRoomEnv {
    match room {
        RoomGiRoom::Shop => ActiveRoomEnv::Shop,
        RoomGiRoom::Hallway => ActiveRoomEnv::Hallway,
        RoomGiRoom::Archive => ActiveRoomEnv::Archive,
        RoomGiRoom::MainMenu => ActiveRoomEnv::MainMenu,
        RoomGiRoom::Stairway => ActiveRoomEnv::Stairway,
        RoomGiRoom::Gameplay => ActiveRoomEnv::Gameplay,
        RoomGiRoom::ShadowTestRoom => ActiveRoomEnv::ShadowTest,
    }
}
