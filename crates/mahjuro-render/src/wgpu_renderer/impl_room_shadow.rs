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

    /// GPU-upload offline `.msh` contact AO for one room (panics when committed bakes are required).
    #[allow(dead_code)] // offline `.msh` bake tooling only — runtime uses live punctual maps
    pub(super) fn ensure_room_baked_shadow_gpu(&mut self, room: RoomGiRoom) {
        if !room_shadow_bake::committed_room_shadows_required() {
            return;
        }
        let idx = room_gi_room_index(room);
        if self.room_baked_shadow_gpu[idx].is_some() {
            return;
        }
        let bake = room_shadow_bake::require_effective_room_shadow_bake(room).unwrap_or_else(|e| {
            panic!("{e:#}");
        });
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

        let ao_bytes = bake.ao_bytes.clone().expect(
            "room shadow bake missing AO bytes after require_effective_room_shadow_bake",
        );
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

    /// Shadow & AO lab only — sync synthetic contact AO with the live punctual pass.
    pub(super) fn write_active_room_baked_shadow_globals(
        &self,
        shadow_quality: ShadowQuality,
        build: &crate::projected_light_shadow::PunctualShadowBuild,
        _window_h: f32,
    ) {
        if !self.active_lab_baked_shadow {
            return;
        }
        let Some((_, gpu)) = &self.lab_baked_shadow else {
            return;
        };
        let globals = build_shadow_globals(
            shadow_quality,
            build,
            shadow_quality.contact_ao(),
            gpu.baked_light_view_proj,
            crate::shadow_ao_lab::CONTACT_AO_WORLD_SCALE,
        );
        self.queue.write_buffer(
            &gpu.globals_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );
    }

    pub(super) fn ensure_lab_baked_shadow_gpu(&mut self, layout: crate::shadow_ao_lab::ShadowAoLabLayout) {
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
        if let Some(layout) = frame.shadow_ao_lab_layout {
            self.ensure_lab_baked_shadow_gpu(layout);
            self.active_room_baked_shadow = None;
            self.active_lab_baked_shadow = true;
            return;
        }
        self.active_lab_baked_shadow = false;
        self.active_room_baked_shadow = None;
    }

    pub(super) fn room_shadow_sample_bind_group(&self) -> &wgpu::BindGroup {
        if self.active_lab_baked_shadow
            && let Some((_, gpu)) = &self.lab_baked_shadow
        {
            return &gpu.sample_bind_group;
        }
        &self.shadow_sample_bind_group
    }

    pub(crate) fn encode_room_shadow_capture_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        room: RoomGiRoom,
        width: u32,
        height: u32,
        light_view_proj: [f32; 16],
        depth_bias: f32,
    ) -> RoomShadowCaptureStaging {
        let byte_len = (width as u64) * (height as u64) * 4;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-shadow-capture-staging"),
            size: byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.point_shadow_array.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        RoomShadowCaptureStaging {
            buffer: staging,
            byte_len,
            room,
            width,
            height,
            light_view_proj,
            depth_bias,
        }
    }

    pub(crate) fn finalize_room_shadow_capture(
        &self,
        staging: RoomShadowCaptureStaging,
    ) -> anyhow::Result<RoomShadowBake> {
        let slice = staging.buffer.slice(..staging.byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| anyhow::anyhow!("room shadow capture map channel closed"))?
            .map_err(|e| anyhow::anyhow!("room shadow capture map failed: {e:?}"))?;
        let mapped = slice.get_mapped_range();
        let depth_bytes: Arc<[u8]> = Arc::from(mapped[..].to_vec());
        drop(mapped);
        staging.buffer.unmap();
        let ao_len = (staging.width * staging.height) as usize;
        let ao: Arc<[u8]> = if staging.room == RoomGiRoom::Archive {
            // Cubby-only caster bake: contact AO darkens `main_fixture` shelf-wide.
            Arc::from(vec![255u8; ao_len])
        } else {
            Arc::from(room_shadow_bake::bake_contact_ao_from_depth(
                staging.width,
                staging.height,
                &depth_bytes,
            ))
        };
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

pub(crate) struct RoomShadowCaptureStaging {
    buffer: wgpu::Buffer,
    byte_len: u64,
    room: RoomGiRoom,
    width: u32,
    height: u32,
    light_view_proj: [f32; 16],
    depth_bias: f32,
}

impl ActiveRoomEnv {
    pub fn from_frame(frame: &UiFrame) -> Option<Self> {
        crate::wgpu_renderer::runtime::shadow_setup::active_room_env(frame)
    }
}
