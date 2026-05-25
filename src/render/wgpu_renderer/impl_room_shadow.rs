use std::sync::Arc;

use super::*;
use crate::render::lit_mesh::{ShadowGlobals, create_shadow_sample_bind_group};
use crate::render::room_gi_bake::RoomGiRoom;
use crate::render::room_shadow_bake::{self, RoomShadowBake};
use crate::render::wgpu_renderer::runtime::shadow_setup::{self, ActiveRoomEnv};

pub(crate) struct RoomBakedShadowGpu {
    pub sample_bind_group: wgpu::BindGroup,
    pub globals_buffer: wgpu::Buffer,
    pub baked_light_view_proj: [f32; 16],
    pub depth_bias: f32,
    pub texel: f32,
    _depth_texture: wgpu::Texture,
    _ao_texture: wgpu::Texture,
}

impl WgpuRenderer {
    pub fn request_room_shadow_capture(&mut self, room: RoomGiRoom) {
        self.room_shadow_capture_pending = Some(room);
        self.room_shadow_captured = None;
    }

    pub fn take_room_shadow_capture(&mut self) -> Option<RoomShadowBake> {
        self.room_shadow_captured.take()
    }

    pub(super) fn load_room_baked_shadows(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shadow_sample_layout: &wgpu::BindGroupLayout,
        dynamic_depth_view: &wgpu::TextureView,
        compare_sampler: &wgpu::Sampler,
        ao_sampler: &wgpu::Sampler,
    ) -> [Option<RoomBakedShadowGpu>; crate::render::room_gi_bake::ROOM_GI_ROOM_COUNT] {
        [
            RoomGiRoom::Shop,
            RoomGiRoom::Hallway,
            RoomGiRoom::Archive,
            RoomGiRoom::MainMenu,
            RoomGiRoom::Staircase,
            RoomGiRoom::Gameplay,
        ]
        .map(|room| {
            room_shadow_bake::cached_room_shadow_bake(room).and_then(|bake| {
                Self::upload_room_baked_shadow_gpu(
                    device,
                    queue,
                    shadow_sample_layout,
                    dynamic_depth_view,
                    compare_sampler,
                    ao_sampler,
                    room,
                    &bake,
                )
                .ok()
            })
        })
    }

    fn upload_room_baked_shadow_gpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shadow_sample_layout: &wgpu::BindGroupLayout,
        dynamic_depth_view: &wgpu::TextureView,
        compare_sampler: &wgpu::Sampler,
        ao_sampler: &wgpu::Sampler,
        _room: RoomGiRoom,
        bake: &RoomShadowBake,
    ) -> anyhow::Result<RoomBakedShadowGpu> {
        let w = bake.width;
        let h = bake.height;
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
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
                ..Default::default()
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let ao_bytes = bake
            .ao_bytes
            .clone()
            .unwrap_or_else(|| Arc::from(vec![255u8; (w * h) as usize]));
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

        let texel = 1.0 / w.max(h) as f32;
        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-baked-shadow-globals"),
            contents: bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
                params: [1.0, bake.depth_bias, texel, 1.0],
                room_baked_light_view_proj: bake.light_view_proj,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sample_bind_group = create_shadow_sample_bind_group(
            device,
            shadow_sample_layout,
            "room-baked-shadow-sample-bg",
            &globals_buffer,
            dynamic_depth_view,
            compare_sampler,
            &depth_view,
            &ao_view,
            ao_sampler,
        );
        Ok(RoomBakedShadowGpu {
            sample_bind_group,
            globals_buffer,
            baked_light_view_proj: bake.light_view_proj,
            depth_bias: bake.depth_bias,
            texel,
            _depth_texture: depth_texture,
            _ao_texture: ao_texture,
        })
    }

    /// Keep baked room shadow sampling in sync with the live prop shadow pass.
    pub(super) fn write_active_room_baked_shadow_globals(
        &self,
        queue: &wgpu::Queue,
        light_view_proj: [f32; 16],
        shadows_enabled: bool,
    ) {
        let Some(room) = self.active_room_baked_shadow else {
            return;
        };
        let Some(gpu) = self.room_baked_shadow_gpu[crate::render::room_gi_bake::room_gi_room_index(room)].as_ref() else {
            return;
        };
        let enabled = if shadows_enabled { 1.0 } else { 0.0 };
        let baked_mode = if room == RoomGiRoom::Archive {
            2.0
        } else {
            1.0
        };
        queue.write_buffer(
            &gpu.globals_buffer,
            0,
            bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj,
                params: [enabled, gpu.depth_bias, gpu.texel, baked_mode],
                room_baked_light_view_proj: gpu.baked_light_view_proj,
            }),
        );
    }

    pub(super) fn upload_active_room_baked_shadow_globals(&mut self, frame: &UiFrame) {
        let active_env = ActiveRoomEnv::from_frame(frame);
        let loaded = active_env.and_then(|env| {
            let room = env.to_room_gi()?;
            self.room_baked_shadow_gpu[crate::render::room_gi_bake::room_gi_room_index(room)]
                .as_ref()
                .map(|_| room)
        });
        self.active_room_baked_shadow = active_env.and_then(|env| {
            shadow_setup::room_env_uses_offline_baked_shadow(Some(env), loaded).then_some(loaded?)
        });
    }

    pub(super) fn room_shadow_sample_bind_group(&self) -> &wgpu::BindGroup {
        if let Some(room) = self.active_room_baked_shadow
            && let Some(gpu) = self.room_baked_shadow_gpu
                [crate::render::room_gi_bake::room_gi_room_index(room)]
                .as_ref()
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
                texture: &self.shadow_map_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
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
        crate::render::wgpu_renderer::runtime::shadow_setup::active_room_env(frame)
    }
}
