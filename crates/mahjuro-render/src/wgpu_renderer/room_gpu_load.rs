//! Lazy GPU upload for deferred room GLB environments.

use super::*;

use crate::scene_keys;

use std::time::{Duration, Instant};

use crate::gltf_helpers::{
    GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE, GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW,
    GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL, GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT, GltfPbrUniform,
    build_sampler_descriptor,
};
use crate::room_env_gltf::RoomEnvPrimitiveCpu;
use crate::wgpu_renderer::resources::RoomEnvTextureCache;
use wgpu::util::DeviceExt;

pub(super) const ROOM_SHOP: u8 = 1 << 0;
pub(super) const ROOM_HALLWAY: u8 = 1 << 1;
pub(super) const ROOM_STAIRCASE: u8 = 1 << 2;
pub(super) const ROOM_ARCHIVE: u8 = 1 << 3;
pub(super) const ROOM_GAMEPLAY: u8 = 1 << 4;
pub(super) const ROOM_MAIN_MENU: u8 = 1 << 5;

use crate::score_roller_layout::{self, GAMEPLAY_SCORE_ROLLER_SLOT_COUNT};

/// Max main-thread wall time for one gameplay env upload slice during prefetch / fade.
const GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS: f32 = 6.0;

#[inline]
fn hallway_env_shader_flags(node_name: Option<&str>) -> u32 {
    if node_name == Some(crate::hallway_glb::HALLWAY_WALLS_NODE) {
        GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT
    } else {
        0
    }
}

#[inline]
fn archive_env_shader_flags(node_name: Option<&str>) -> u32 {
    if matches!(
        node_name,
        Some(
            crate::archive_glb::SIGN_DESCRIPTION_LEFT
                | crate::archive_glb::SIGN_DESCRIPTION_RIGHT
                | crate::archive_glb::INSPECT_PLAQUE
        )
    ) {
        GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
    } else {
        0
    }
}

#[inline]
fn main_menu_env_shader_flags(node_name: Option<&str>) -> u32 {
    let Some(name) = node_name else {
        return 0;
    };
    if crate::main_menu_glb::is_main_menu_moon_env_node(name) {
        GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE
    } else if crate::main_menu_glb::is_main_menu_star_env_node(name) {
        GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW
    } else {
        0
    }
}

#[inline]
fn room_env_shader_flags(scene_key: &str, node_name: Option<&str>) -> u32 {
    match scene_key {
        scene_keys::HALLWAY => hallway_env_shader_flags(node_name),
        scene_keys::ARCHIVE => archive_env_shader_flags(node_name),
        scene_keys::MAIN_MENU => main_menu_env_shader_flags(node_name),
        _ => 0,
    }
}

#[inline]
fn room_env_pbr_uniform(
    prim: &crate::tile_glb::LoadedPrimitive,
    scene_key: &str,
    node_name: Option<&str>,
) -> GltfPbrUniform {
    let mut pbr_uniform = GltfPbrUniform::from_loaded(
        prim.metallic_factor,
        prim.roughness_factor,
        prim.emissive_factor,
        prim.alpha_mode,
        prim.alpha_cutoff,
    );
    pbr_uniform.add_flags(room_env_shader_flags(scene_key, node_name));
    pbr_uniform
}

pub(super) struct GameplayRoomGpuUpload {
    prim_count: usize,
    next_prim: usize,
    prims: Vec<TilePrimitiveGpu>,
    room_tex_cache: RoomEnvTextureCache,
    _white_albedo_tex: wgpu::Texture,
    white_albedo_view: wgpu::TextureView,
    _decal_tex: wgpu::Texture,
    gameplay_decal_view: wgpu::TextureView,
    cash_in_prim_indices: Vec<usize>,
    score_roller_prim_groups: Vec<Vec<usize>>,
    score_roller_pivots_doc: Vec<[f32; 3]>,
    score_roller_axes_doc: Vec<[f32; 3]>,
    score_roller_found: [bool; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
    started_at: Instant,
}

fn require_room_environment_loaded(
    glb: &str,
    prims: &[TilePrimitiveGpu],
    gpu: &Option<ShopEnvironmentGpu>,
) {
    if gpu.is_none() || prims.is_empty() {
        panic!("{glb} environment failed to load (required for this scene)");
    }
}

pub(super) struct RoomGpuUploadCtx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub tile_material_layout: &'a wgpu::BindGroupLayout,
    pub shadow_caster_layout: &'a wgpu::BindGroupLayout,
    pub shadow_warp_layout: &'a wgpu::BindGroupLayout,
    pub tile_default_normal_view: &'a wgpu::TextureView,
    pub tile_glb_default_mr_view: &'a wgpu::TextureView,
    pub tile_glb_default_emissive_view: &'a wgpu::TextureView,
}

fn load_shop_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    crate::room_gltf_anim::RoomGltfAnimGpu,
    Vec<usize>,
) {
    crate::room_glb::with_shop_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let mut shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
        let mut shop_eyeball_prim_indices = Vec::new();
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap, shop_gltf_anim, shop_eyeball_prim_indices);
        };
        shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::from_room_cpu(
            &cpu.gltf_anim_library,
            &cpu.environment_primitives,
            "shop.glb",
        );
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                if env_prim.gltf_node_name.as_deref() == Some("Eyeball") {
                    shop_eyeball_prim_indices.push(i);
                }
                let prim = &env_prim.mesh;
                let vb = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("shop-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ib = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("shop-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("shop-env-albedo-{i}"),
                    prim.albedo_rgba.as_ref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("shop-env-normal-{i}"),
                    prim.normal_rgba.as_ref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("shop-env-mr-{i}"),
                    prim.metallic_roughness_rgba.as_ref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("shop-env-emissive-{i}"),
                    prim.emissive_rgba.as_ref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform =
                    room_env_pbr_uniform(prim, scene_keys::SHOP, env_prim.gltf_node_name.as_deref());
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("shop-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                });
            }
            let (_white_tex, shop_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers =
                create_room_env_camera_uniform_buffers(ctx.device, prims.len(), "shop-env-uniform");
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("shop-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("shop-env-bg"),
                        layout: ctx.tile_material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform_buffers[pi].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&shop_decal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&p.normal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: p.pbr_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(
                                    &p.metallic_roughness_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: distortion_buffer.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "shop-env-shadow",
            );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "shop-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            if shop_eyeball_prim_indices.is_empty() {
                if let Some(bindings) = shop_gltf_anim.clip_prim_bindings.get("eyeball_travel") {
                    shop_eyeball_prim_indices = bindings.iter().map(|(pi, _)| *pi).collect();
                    log::info!(
                        "shop.glb GPU: Eyeball prims from eyeball_travel bindings {:?}",
                        shop_eyeball_prim_indices
                    );
                }
            } else {
                log::info!(
                    "shop.glb GPU: Eyeball primitive indices {:?}",
                    shop_eyeball_prim_indices
                );
            }
            log::info!("shop.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap, shop_gltf_anim, shop_eyeball_prim_indices)
    })
}

fn load_hallway_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::hallway_glb::with_hallway_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let vb = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("hallway-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ib = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("hallway-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("hallway-env-albedo-{i}"),
                    prim.albedo_rgba.as_ref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("hallway-env-normal-{i}"),
                    prim.normal_rgba.as_ref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("hallway-env-mr-{i}"),
                    prim.metallic_roughness_rgba.as_ref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("hallway-env-emissive-{i}"),
                    prim.emissive_rgba.as_ref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::HALLWAY,
                    env_prim.gltf_node_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("hallway-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                });
            }
            let (_white_tex, hallway_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "hallway-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("hallway-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("hallway-env-bg"),
                        layout: ctx.tile_material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform_buffers[pi].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&hallway_decal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&p.normal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: p.pbr_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(
                                    &p.metallic_roughness_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: distortion_buffer.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "hallway-env-shadow",
            );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "hallway-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("hallway.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn load_main_menu_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::main_menu_glb::with_main_menu_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let vb = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("main_menu-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ib = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("main_menu-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("main_menu-env-albedo-{i}"),
                    prim.albedo_rgba.as_ref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("main_menu-env-normal-{i}"),
                    prim.normal_rgba.as_ref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("main_menu-env-mr-{i}"),
                    prim.metallic_roughness_rgba.as_ref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("main_menu-env-emissive-{i}"),
                    prim.emissive_rgba.as_ref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::MAIN_MENU,
                    env_prim.gltf_node_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("main_menu-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                });
            }
            let (_white_tex, main_menu_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "main_menu-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("main_menu-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("main_menu-env-bg"),
                        layout: ctx.tile_material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform_buffers[pi].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&main_menu_decal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&p.normal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: p.pbr_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(
                                    &p.metallic_roughness_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: distortion_buffer.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "main_menu-env-shadow",
            );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "main_menu-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("main_menu.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn load_staircase_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::staircase_glb::with_staircase_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let vb = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("staircase-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ib = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("staircase-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("staircase-env-albedo-{i}"),
                    prim.albedo_rgba.as_ref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("staircase-env-normal-{i}"),
                    prim.normal_rgba.as_ref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("staircase-env-mr-{i}"),
                    prim.metallic_roughness_rgba.as_ref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("staircase-env-emissive-{i}"),
                    prim.emissive_rgba.as_ref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::STAIRWAY,
                    env_prim.gltf_node_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("staircase-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                });
            }
            let (_white_tex, staircase_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "staircase-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("staircase-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("staircase-env-bg"),
                        layout: ctx.tile_material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform_buffers[pi].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&staircase_decal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&p.normal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: p.pbr_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(
                                    &p.metallic_roughness_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: distortion_buffer.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "staircase-env-shadow",
            );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "staircase-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("staircase.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn create_cleared_archive_decal_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes_per_row = crate::wgpu_renderer::resources::rgba8_copy_bytes_per_row(width);
    let clear = vec![0u8; (bytes_per_row * height) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &clear,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    tex
}

fn load_archive_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Vec<usize>,
    Vec<usize>,
) {
    crate::archive_glb::with_archive_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let mut sign_l = None;
        let mut sign_r = None;
        let mut inspect_plaque = None;
        let mut plaque_backing = None;
        let mut page_left = Vec::new();
        let mut page_right = Vec::new();
        let Some(cpu) = cpu_opt else {
            return (
                prims,
                gpu_wrap,
                sign_l,
                sign_r,
                inspect_plaque,
                plaque_backing,
                page_left,
                page_right,
            );
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                if let Some(ref name) = env_prim.gltf_node_name {
                    if name == crate::archive_glb::SIGN_DESCRIPTION_LEFT {
                        sign_l = Some(i);
                    } else if name == crate::archive_glb::SIGN_DESCRIPTION_RIGHT {
                        sign_r = Some(i);
                    } else if name == crate::archive_glb::INSPECT_PLAQUE {
                        inspect_plaque = Some(i);
                    } else if name == crate::archive_glb::PLAQUE_BACKING {
                        plaque_backing = Some(i);
                    } else if name == crate::archive_glb::BTN_PAGE_LEFT {
                        page_left.push(i);
                    } else if name == crate::archive_glb::BTN_PAGE_RIGHT {
                        page_right.push(i);
                    }
                }
                let prim = &env_prim.mesh;
                let vb = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("archive-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ib = ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("archive-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("archive-env-albedo-{i}"),
                    prim.albedo_rgba.as_ref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("archive-env-normal-{i}"),
                    prim.normal_rgba.as_ref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("archive-env-mr-{i}"),
                    prim.metallic_roughness_rgba.as_ref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = room_tex_cache.upload_slot(
                    ctx.device,
                    ctx.queue,
                    format!("archive-env-emissive-{i}"),
                    prim.emissive_rgba.as_ref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::ARCHIVE,
                    env_prim.gltf_node_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("archive-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                });
            }
            let decal_layout = crate::primitive::DecalLayout::Fit {
                target_short_edge: crate::decal::PLAQUE_DECAL_HEIGHT,
            };
            let (sign_decal_w, sign_decal_h) = crate::decal::decal_dimensions(
                &decal_layout,
                crate::archive_glb::archive_sign_description_decal_extents_for(cpu),
            );
            let (inspect_decal_w, inspect_decal_h) = crate::decal::decal_dimensions(
                &decal_layout,
                crate::archive_glb::archive_inspect_plaque_decal_extents_for(cpu),
            );
            let archive_sign_decal_tex = create_cleared_archive_decal_texture(
                ctx.device,
                ctx.queue,
                "archive-sign-decal",
                sign_decal_w,
                sign_decal_h,
            );
            let archive_inspect_decal_tex = create_cleared_archive_decal_texture(
                ctx.device,
                ctx.queue,
                "archive-inspect-plaque-decal",
                inspect_decal_w,
                inspect_decal_h,
            );
            let archive_sign_decal_view =
                archive_sign_decal_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let archive_inspect_decal_view =
                archive_inspect_decal_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "archive-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("archive-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    let decal_view = if sign_l == Some(pi) || sign_r == Some(pi) {
                        &archive_sign_decal_view
                    } else if inspect_plaque == Some(pi) {
                        &archive_inspect_decal_view
                    } else {
                        &white_albedo_view
                    };
                    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("archive-env-bg"),
                        layout: ctx.tile_material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform_buffers[pi].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(decal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&p.normal_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: p.pbr_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: wgpu::BindingResource::TextureView(
                                    &p.metallic_roughness_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: distortion_buffer.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "archive-env-shadow",
            );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "archive-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                archive_sign_decal_texture: Some(archive_sign_decal_tex),
                archive_sign_decal_size: Some((sign_decal_w, sign_decal_h)),
                archive_inspect_plaque_decal_texture: Some(archive_inspect_decal_tex),
                archive_inspect_plaque_decal_size: Some((inspect_decal_w, inspect_decal_h)),
            });
            log::info!("archive.glb GPU: {} primitive draw(s)", prims.len());
        }
        (
            prims,
            gpu_wrap,
            sign_l,
            sign_r,
            inspect_plaque,
            plaque_backing,
            page_left,
            page_right,
        )
    })
}

fn record_gameplay_env_prim_metadata(
    upload: &mut GameplayRoomGpuUpload,
    i: usize,
    env_prim: &RoomEnvPrimitiveCpu,
    cpu: &crate::room_glb::RoomGlbCpu,
) {
    if let Some(ref name) = env_prim.gltf_node_name {
        if matches!(
            name.as_str(),
            crate::gameplay_glb::BTN_CASH_IN | crate::gameplay_glb::LABEL_CASH_IN
        ) {
            upload.cash_in_prim_indices.push(i);
        }
        if let Some(raw_idx) = score_roller_layout::gameplay_score_roller_raw_index(name)
            && let Some(slot) = score_roller_layout::gameplay_score_roller_slot_remap(raw_idx)
        {
            upload.score_roller_prim_groups[slot].push(i);
            upload.score_roller_found[slot] = true;
            if let Some(bind) = cpu.node_bind_poses.get(name) {
                let pivot = bind.bind_world_doc.w_axis.truncate();
                upload.score_roller_pivots_doc[slot] = pivot.to_array();
                let axis = bind
                    .bind_world_doc
                    .transform_vector3(glam::Vec3::X)
                    .normalize_or_zero();
                upload.score_roller_axes_doc[slot] = axis.to_array();
            }
        }
    }
}

fn upload_gameplay_env_prim_gpu(
    i: usize,
    env_prim: &RoomEnvPrimitiveCpu,
    ctx: &RoomGpuUploadCtx<'_>,
    white_albedo_view: &wgpu::TextureView,
    room_tex_cache: &mut RoomEnvTextureCache,
) -> TilePrimitiveGpu {
    let prim = &env_prim.mesh;
    let vb = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("gameplay-env-verts-{i}")),
            contents: bytemuck::cast_slice(&prim.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let ib = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("gameplay-env-idx-{i}")),
            contents: bytemuck::cast_slice(&prim.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
    let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
    let albedo_view = room_tex_cache.upload_slot(
        ctx.device,
        ctx.queue,
        format!("gameplay-env-albedo-{i}"),
        prim.albedo_rgba.as_ref(),
        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        white_albedo_view,
    );
    let normal_view = room_tex_cache.upload_slot(
        ctx.device,
        ctx.queue,
        format!("gameplay-env-normal-{i}"),
        prim.normal_rgba.as_ref(),
        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_default_normal_view,
    );
    let metallic_roughness_view = room_tex_cache.upload_slot(
        ctx.device,
        ctx.queue,
        format!("gameplay-env-mr-{i}"),
        prim.metallic_roughness_rgba.as_ref(),
        prim.metallic_roughness_mip_chain
            .as_deref()
            .map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_glb_default_mr_view,
    );
    let emissive_view = room_tex_cache.upload_slot(
        ctx.device,
        ctx.queue,
        format!("gameplay-env-emissive-{i}"),
        prim.emissive_rgba.as_ref(),
        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        ctx.tile_glb_default_emissive_view,
    );
    let pbr_uniform =
        room_env_pbr_uniform(prim, scene_keys::GAMEPLAY, env_prim.gltf_node_name.as_deref());
    let pbr_uniform_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("gameplay-pbr-{i}")),
            contents: bytemuck::bytes_of(&pbr_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let sampler = ctx
        .device
        .create_sampler(&build_sampler_descriptor(prim.sampler, None));
    TilePrimitiveGpu {
        vertex_buffer: vb,
        index_buffer: ib,
        index_count: prim.indices.len() as u32,
        albedo_view,
        normal_view,
        metallic_roughness_view,
        emissive_view,
        pbr_uniform_buffer,
        sampler,
        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
    }
}

fn begin_gameplay_room_gpu_upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Option<GameplayRoomGpuUpload> {
    crate::gameplay_glb::with_gameplay_glb_cpu(|cpu_opt| {
        let cpu = cpu_opt?;
        if cpu.environment_primitives.is_empty() {
            return None;
        }
        let prim_count = cpu.environment_primitives.len();
        let (white_tex, white_view) = white_albedo(device, queue);
        let (decal_tex, decal_view) = white_albedo(device, queue);
        Some(GameplayRoomGpuUpload {
            prim_count,
            next_prim: 0,
            prims: Vec::with_capacity(prim_count),
            room_tex_cache: RoomEnvTextureCache::new(),
            _white_albedo_tex: white_tex,
            white_albedo_view: white_view,
            _decal_tex: decal_tex,
            gameplay_decal_view: decal_view,
            cash_in_prim_indices: Vec::new(),
            score_roller_prim_groups: vec![Vec::new(); GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_pivots_doc: vec![[0.0, 0.0, 0.0]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_axes_doc: vec![[1.0, 0.0, 0.0]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_found: [false; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            started_at: Instant::now(),
        })
    })
}

fn finalize_gameplay_room_gpu_upload(
    upload: GameplayRoomGpuUpload,
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    Vec<usize>,
    Vec<Vec<usize>>,
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
) {
    let GameplayRoomGpuUpload {
        prim_count,
        prims,
        gameplay_decal_view,
        cash_in_prim_indices,
        score_roller_prim_groups,
        score_roller_pivots_doc,
        score_roller_axes_doc,
        score_roller_found,
        ..
    } = upload;
    debug_assert_eq!(prims.len(), prim_count);

    let distortion_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gameplay-env-distortion"),
            contents: bytemuck::bytes_of(&crate::hallway_glb::HallwayDistortion::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let uniform_buffers =
        create_room_env_camera_uniform_buffers(ctx.device, prim_count, "gameplay-env-uniform");
    let bind_groups: Vec<wgpu::BindGroup> = prims
        .iter()
        .enumerate()
        .map(|(pi, p)| {
            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gameplay-env-bg"),
                layout: ctx.tile_material_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffers[pi].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&p.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&gameplay_decal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&p.normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: p.pbr_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&p.metallic_roughness_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: distortion_buffer.as_entire_binding(),
                    },
                ],
            })
        })
        .collect();
    let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
        ctx.device,
        ctx.shadow_caster_layout,
        prim_count,
        "gameplay-env-shadow",
    );
    let shadow_warp_bind_group = create_shadow_warp_bind_group(
        ctx.device,
        ctx.shadow_warp_layout,
        &distortion_buffer,
        "gameplay-env-shadow-warp",
    );
    let gpu_wrap = Some(ShopEnvironmentGpu {
        uniform_buffers,
        distortion_buffer,
        shadow_uniform_buffers,
        shadow_bind_groups,
        shadow_warp_bind_group,
        bind_groups,
        archive_sign_decal_texture: None,
        archive_sign_decal_size: None,
        archive_inspect_plaque_decal_texture: None,
        archive_inspect_plaque_decal_size: None,
    });
    log::info!("gameplay.glb GPU: {prim_count} primitive draw(s)");
    let mapped_rolls = score_roller_found.iter().filter(|&&b| b).count();
    log::debug!(
        "gameplay.glb score rollers: all_found={} ({mapped_rolls}/{GAMEPLAY_SCORE_ROLLER_SLOT_COUNT})",
        mapped_rolls == GAMEPLAY_SCORE_ROLLER_SLOT_COUNT
    );
    if mapped_rolls != GAMEPLAY_SCORE_ROLLER_SLOT_COUNT {
        log::warn!(
            "gameplay.glb score rollers missing: found {mapped_rolls}/{GAMEPLAY_SCORE_ROLLER_SLOT_COUNT}"
        );
    }
    (
        prims,
        gpu_wrap,
        cash_in_prim_indices,
        score_roller_prim_groups,
        score_roller_pivots_doc,
        score_roller_axes_doc,
    )
}

impl WgpuRenderer {
    fn cancel_gameplay_room_gpu_upload(&mut self) {
        self.gameplay_room_gpu_upload = None;
    }

    /// Upload gameplay room env prims across frames when `budget_ms` is finite.
    /// Returns `true` once the room is resident on the GPU.
    fn tick_gameplay_room_gpu_upload(&mut self, budget_ms: f32) -> bool {
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
            self.cancel_gameplay_room_gpu_upload();
            return true;
        }
        if !crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload() {
            crate::room_preload::join_gameplay_cpu_prefetch_blocking();
            if !crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload() {
                crate::gameplay_glb::decode_gameplay_glb_into_cache();
            }
        }

        if self.gameplay_room_gpu_upload.is_none() {
            let ctx = self.room_gpu_upload_ctx();
            self.gameplay_room_gpu_upload =
                begin_gameplay_room_gpu_upload(ctx.device, ctx.queue);
        }
        let Some(mut upload) = self.gameplay_room_gpu_upload.take() else {
            panic!("gameplay.glb environment failed to load (required for this scene)");
        };

        let unlimited = budget_ms >= 1.0e6;
        let deadline = if unlimited {
            None
        } else {
            Some(Instant::now() + Duration::from_secs_f32((budget_ms / 1000.0).max(0.0)))
        };

        while upload.next_prim < upload.prim_count {
            if let Some(dl) = deadline
                && Instant::now() >= dl
            {
                break;
            }
            let i = upload.next_prim;
            let ctx = self.room_gpu_upload_ctx();
            let prim_gpu = crate::gameplay_glb::with_gameplay_glb_cpu(|cpu_opt| {
                let cpu = cpu_opt.expect("gameplay CPU cache missing during GPU upload");
                let env_prim = &cpu.environment_primitives[i];
                record_gameplay_env_prim_metadata(&mut upload, i, env_prim, cpu);
                upload_gameplay_env_prim_gpu(
                    i,
                    env_prim,
                    &ctx,
                    &upload.white_albedo_view,
                    &mut upload.room_tex_cache,
                )
            });
            upload.prims.push(prim_gpu);
            upload.next_prim += 1;
        }

        if upload.next_prim < upload.prim_count {
            self.gameplay_room_gpu_upload = Some(upload);
            return false;
        }

        let started_at = upload.started_at;
        self.ensure_gameplay_hud_pools();
        let payload = crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let _cpu = crate::cpu_profiler::scope("wgpu.room.gameplay");
        let _startup = crate::startup_profile::scope("wgpu.room.gameplay");
        let finalize_t0 = Instant::now();
        let (prims, gpu_wrap, cash_in, roller_groups, roller_pivots, roller_axes) = {
            let ctx = self.room_gpu_upload_ctx();
            finalize_gameplay_room_gpu_upload(upload, ctx)
        };
        require_room_environment_loaded("gameplay.glb", &prims, &gpu_wrap);
        self.gameplay_env_primitives = prims;
        self.gameplay_environment = gpu_wrap;
        self.gameplay_cash_in_prim_indices = cash_in;
        self.gameplay_score_roller_prim_groups = roller_groups;
        self.gameplay_score_roller_pivots_doc = roller_pivots;
        self.gameplay_score_roller_axes_doc = roller_axes;
        self.gameplay_score_roller_drive_initialized
            .replace([false; 2]);
        self.gameplay_env_collision_meshes = crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
            o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
        });
        crate::gameplay_glb::release_gameplay_environment_cpu_sources_after_gpu_upload();
        self.rooms_gpu_loaded |= ROOM_GAMEPLAY;
        self.note_room_gpu_resident(ROOM_GAMEPLAY);
        if crate::room_gpu_profile::enabled() {
            let finalize_ms = finalize_t0.elapsed().as_secs_f64() * 1000.0;
            let total_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let mb = payload.total_bytes() as f64 / (1024.0 * 1024.0);
            let hitch = if frame_dt_ms >= 33.0 {
                "HITCH"
            } else if frame_dt_ms >= 20.0 {
                "slow"
            } else {
                "ok"
            };
            log::info!(
                "room gpu profile: gameplay.glb GPU upload — finalize {finalize_ms:.1} ms | \
                 {total_ms:.1} ms total wall | {prims} prims | {total_mb:.2} MiB CPU payload | \
                 prev frame dt {frame_dt_ms:.1} ms ({hitch})",
                prims = payload.primitives,
                total_mb = mb,
            );
        }
        crate::startup_profile::log_sample("wgpu.room.gameplay", "first gameplay GPU upload");
        true
    }

    fn drive_gameplay_room_gpu_upload(&mut self, budget_ms: f32) {
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
            return;
        }
        if budget_ms >= 1.0e6 {
            while !self.tick_gameplay_room_gpu_upload(budget_ms) {}
        } else {
            let _ = self.tick_gameplay_room_gpu_upload(budget_ms);
        }
    }
}

impl WgpuRenderer {
    pub(super) fn ensure_room_gpu_for_draw_cmds(&mut self, cmds: &[DrawCmd]) {
        let mut need = 0u8;
        for cmd in cmds {
            match cmd {
                DrawCmd::ShopEnvironment => need |= ROOM_SHOP,
                DrawCmd::MainMenuEnvironment => need |= ROOM_MAIN_MENU,
                DrawCmd::HallwayEnvironment => need |= ROOM_HALLWAY,
                DrawCmd::StaircaseEnvironment => need |= ROOM_STAIRCASE,
                DrawCmd::ArchiveEnvironment => need |= ROOM_ARCHIVE,
                DrawCmd::GameplayEnvironment => need |= ROOM_GAMEPLAY,
                _ => {}
            }
        }
        if need & ROOM_SHOP != 0 {
            self.ensure_shop_room_gpu();
        }
        if need & ROOM_MAIN_MENU != 0 {
            self.ensure_main_menu_room_gpu();
        }
        if need & ROOM_HALLWAY != 0 {
            self.ensure_hallway_room_gpu();
        }
        if need & ROOM_STAIRCASE != 0 {
            self.ensure_staircase_room_gpu();
        }
        if need & ROOM_ARCHIVE != 0 {
            self.ensure_archive_room_gpu();
        }
        if need & ROOM_GAMEPLAY != 0 {
            self.ensure_gameplay_room_gpu();
        }
    }

    pub fn ensure_gameplay_room_gpu_for_resume(&mut self) {
        self.ensure_gameplay_room_gpu();
    }

    /// Preload room GPU data for picking before [`Self::render`] builds ops.
    pub fn ensure_rooms_for_scene_key(&mut self, key: Option<&str>) {
        match key {
            Some(scene_keys::MAIN_MENU) | Some("main_menu_exterior") => {
                self.ensure_main_menu_room_gpu();
            }
            Some(scene_keys::SHOP) | Some("showcase") | Some(scene_keys::HALLWAY) => {
                self.ensure_shop_room_gpu();
            }
            Some(scene_keys::STAIRWAY) => self.ensure_staircase_room_gpu(),
            Some(scene_keys::ARCHIVE) => self.ensure_archive_room_gpu(),
            Some(scene_keys::GAMEPLAY) | Some(scene_keys::VICTORY) | Some(scene_keys::DEFEAT) => {
                self.ensure_gameplay_room_gpu()
            }
            // Legacy aliases
            Some("pick_chamber") => self.ensure_shop_room_gpu(),
            Some("staircase") => self.ensure_staircase_room_gpu(),
            Some("collection") => self.ensure_archive_room_gpu(),
            Some("game_over") => self.ensure_gameplay_room_gpu(),
            _ => {}
        }
    }

    /// Start background CPU decode for the next room in the run chain.
    pub fn prefetch_room_chain_next(&mut self, scene: crate::room_preload::RoomSceneChain) {
        crate::room_preload::start_room_cpu_prefetch(scene);
    }

    /// Join finished prefetch workers and upload room GPU data when CPU caches are ready.
    ///
    /// Only uploads rooms that match the current hub/run chain — avoids parking shop,
    /// hallway, and gameplay environments on the GPU while the main menu is idle.
    ///
    /// `frame_dt_ms` is the previous frame wall time; logged as a hitch proxy when
    /// [`crate::room_gpu_profile`] is enabled.
    /// `warm_gameplay_for_resume` — when true (Continue → gameplay), also upload
    /// `gameplay.glb` on the main menu once its CPU prefetch finishes.
    ///
    /// `pending_scene_key` — scene we are fading into (`App::pending_scene`), if any;
    /// uploads the destination room during fade-out so the black-frame swap does not stall.
    pub fn poll_room_prefetch_gpu_uploads(
        &mut self,
        scene_key: Option<&str>,
        frame_dt_ms: f32,
        warm_gameplay_for_resume: bool,
        pending_scene_key: Option<&str>,
    ) {
        self.room_profile_frame_dt_ms = frame_dt_ms;
        crate::room_preload::try_drain_room_cpu_prefetch_threads();

        let upload_shop = |this: &mut Self| {
            if this.rooms_gpu_loaded & ROOM_SHOP == 0
                && crate::room_glb::shop_cpu_ready_for_gpu_upload()
            {
                this.ensure_shop_room_gpu();
            }
        };
        let upload_hallway = |this: &mut Self| {
            if this.rooms_gpu_loaded & ROOM_HALLWAY == 0
                && crate::hallway_glb::hallway_cpu_ready_for_gpu_upload()
            {
                this.ensure_hallway_room_gpu();
            }
        };
        let upload_archive = |this: &mut Self| {
            if this.rooms_gpu_loaded & ROOM_ARCHIVE == 0
                && crate::archive_glb::archive_cpu_ready_for_gpu_upload()
            {
                this.ensure_archive_room_gpu();
            }
        };
        let upload_gameplay = |this: &mut Self| {
            if this.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
                return;
            }
            if !crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload() {
                return;
            }
            this.drive_gameplay_room_gpu_upload(GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS);
        };

        match scene_key {
            Some(scene_keys::MAIN_MENU) | Some("main_menu_exterior") => {
                self.ensure_main_menu_room_gpu();
                // CPU-only hub chain (shop → archive). Do not upload shop/archive
                // environments on the menu — that blocks the main thread and pins
                // ~1.5 GiB of textures while relics are still streaming.
                crate::room_preload::advance_hub_cpu_prefetch_chain();
                if warm_gameplay_for_resume {
                    upload_gameplay(self);
                }
            }
            Some(scene_keys::SHOP)
            | Some("showcase")
            | Some(scene_keys::HALLWAY)
            | Some("pick_chamber") => {
                upload_shop(self);
                upload_hallway(self);
                if self.rooms_gpu_loaded & ROOM_HALLWAY != 0
                    && self.rooms_gpu_loaded & ROOM_GAMEPLAY == 0
                {
                    crate::room_preload::start_gameplay_cpu_prefetch();
                }
                if matches!(scene_key, Some(scene_keys::HALLWAY) | Some("pick_chamber")) {
                    upload_gameplay(self);
                } else if matches!(scene_key, Some(scene_keys::SHOP) | Some("showcase"))
                    && self.graphics_mode.max_room_gpu_residents() >= 3
                {
                    upload_gameplay(self);
                }
            }
            Some(scene_keys::ARCHIVE) | Some("collection") => upload_archive(self),
            Some(scene_keys::GAMEPLAY)
            | Some(scene_keys::VICTORY)
            | Some(scene_keys::DEFEAT)
            | Some("game_over")
            | Some("tutorial") => upload_gameplay(self),
            _ => {}
        }

        if let Some(pending) = pending_scene_key {
            match pending {
                scene_keys::GAMEPLAY
                | scene_keys::VICTORY
                | scene_keys::DEFEAT
                | "game_over"
                | "tutorial" => {
                    crate::room_preload::start_gameplay_cpu_prefetch();
                    upload_gameplay(self);
                }
                scene_keys::HALLWAY | "pick_chamber" => {
                    crate::room_preload::start_hallway_cpu_prefetch();
                    upload_hallway(self);
                }
                scene_keys::SHOP | "showcase" => {
                    crate::room_preload::start_shop_cpu_prefetch();
                    upload_shop(self);
                }
                scene_keys::ARCHIVE | "collection" => {
                    crate::room_preload::start_archive_cpu_prefetch();
                    upload_archive(self);
                }
                scene_keys::STAIRWAY | "staircase" => {
                    self.ensure_staircase_room_gpu();
                }
                _ => {}
            }
        }
    }

    fn room_gpu_upload_ctx(&self) -> RoomGpuUploadCtx<'_> {
        RoomGpuUploadCtx {
            device: &self.device,
            queue: &self.queue,
            tile_material_layout: &self.tile_material_layout,
            shadow_caster_layout: &self.shadow_caster_layout,
            shadow_warp_layout: &self.shadow_warp_layout,
            tile_default_normal_view: &self.tile_env_normal_view,
            tile_glb_default_mr_view: &self.tile_env_mr_view,
            tile_glb_default_emissive_view: &self.tile_env_emissive_view,
        }
    }

    pub(super) fn ensure_main_menu_room_gpu(&mut self) {
        if self.rooms_gpu_loaded & ROOM_MAIN_MENU != 0 {
            return;
        }
        if !crate::main_menu_glb::main_menu_cpu_ready_for_gpu_upload() {
            crate::room_preload::join_main_menu_cpu_prefetch_blocking();
            if !crate::main_menu_glb::main_menu_cpu_ready_for_gpu_upload() {
                crate::main_menu_glb::decode_main_menu_glb_into_cache();
            }
        }
        let payload = crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        crate::room_gpu_profile::measure_gpu_upload(
            "main_menu.glb",
            "wgpu.room.main_menu",
            payload,
            frame_dt_ms,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap) = load_main_menu_room_gpu(ctx);
                require_room_environment_loaded("main_menu.glb", &prims, &gpu_wrap);
                self.main_menu_env_primitives = prims;
                self.main_menu_environment = gpu_wrap;
                self.main_menu_env_collision_meshes =
                    crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                        o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
                    });
                crate::main_menu_glb::release_main_menu_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= ROOM_MAIN_MENU;
                self.note_room_gpu_resident(ROOM_MAIN_MENU);
            },
        );
    }

    pub(super) fn ensure_shop_room_gpu(&mut self) {
        if self.rooms_gpu_loaded & ROOM_SHOP != 0 {
            return;
        }
        if !crate::room_glb::shop_cpu_ready_for_gpu_upload() {
            crate::room_preload::join_shop_cpu_prefetch_blocking();
            if !crate::room_glb::shop_cpu_ready_for_gpu_upload() {
                crate::room_glb::decode_shop_glb_into_cache();
            }
        }
        self.ensure_talisman_textures();
        let payload = crate::room_glb::with_shop_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        crate::room_gpu_profile::measure_gpu_upload(
            "shop.glb",
            "wgpu.room.shop",
            payload,
            frame_dt_ms,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap, anim, eyeball) = load_shop_room_gpu(ctx);
                require_room_environment_loaded("shop.glb", &prims, &gpu_wrap);
                self.shop_env_primitives = prims;
                self.shop_environment = gpu_wrap;
                self.shop_gltf_anim = anim;
                self.shop_eyeball_prim_indices = eyeball;
                self.shop_env_collision_meshes = crate::room_glb::with_shop_glb_cpu(|o| {
                    o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
                });
                crate::room_glb::release_shop_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= ROOM_SHOP;
                self.note_room_gpu_resident(ROOM_SHOP);
            },
        );
    }

    pub(super) fn ensure_hallway_room_gpu(&mut self) {
        if self.rooms_gpu_loaded & ROOM_HALLWAY != 0 {
            return;
        }
        if !crate::hallway_glb::hallway_cpu_ready_for_gpu_upload() {
            crate::room_preload::join_hallway_cpu_prefetch_blocking();
            if !crate::hallway_glb::hallway_cpu_ready_for_gpu_upload() {
                crate::hallway_glb::decode_hallway_glb_into_cache();
            }
        }
        let payload = crate::hallway_glb::with_hallway_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        crate::room_gpu_profile::measure_gpu_upload(
            "hallway.glb",
            "wgpu.room.hallway",
            payload,
            frame_dt_ms,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap) = load_hallway_room_gpu(ctx);
                require_room_environment_loaded("hallway.glb", &prims, &gpu_wrap);
                self.hallway_env_primitives = prims;
                self.hallway_environment = gpu_wrap;
                crate::hallway_glb::release_hallway_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= ROOM_HALLWAY;
                self.note_room_gpu_resident(ROOM_HALLWAY);
            },
        );
    }

    pub(super) fn ensure_staircase_room_gpu(&mut self) {
        if self.rooms_gpu_loaded & ROOM_STAIRCASE != 0 {
            return;
        }
        let payload = crate::staircase_glb::with_staircase_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        crate::room_gpu_profile::measure_gpu_upload(
            "staircase.glb",
            "wgpu.room.staircase",
            payload,
            frame_dt_ms,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap) = load_staircase_room_gpu(ctx);
                require_room_environment_loaded("staircase.glb", &prims, &gpu_wrap);
                self.staircase_env_primitives = prims;
                self.staircase_environment = gpu_wrap;
                crate::staircase_glb::release_staircase_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= ROOM_STAIRCASE;
                self.note_room_gpu_resident(ROOM_STAIRCASE);
            },
        );
    }

    pub(super) fn ensure_archive_room_gpu(&mut self) {
        if self.rooms_gpu_loaded & ROOM_ARCHIVE != 0 {
            return;
        }
        if !crate::archive_glb::archive_cpu_ready_for_gpu_upload() {
            crate::room_preload::join_archive_cpu_prefetch_blocking();
            if !crate::archive_glb::archive_cpu_ready_for_gpu_upload() {
                crate::archive_glb::decode_archive_glb_into_cache();
            }
        }
        self.ensure_ordeal_icon_pool();
        self.ensure_talisman_textures();
        let payload = crate::archive_glb::with_archive_glb_cpu(|o| {
            o.map(|c| crate::room_gpu_profile::count_cpu_payload(&c.environment_primitives))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        crate::room_gpu_profile::measure_gpu_upload(
            "archive.glb",
            "wgpu.room.archive",
            payload,
            frame_dt_ms,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (
                    prims,
                    gpu_wrap,
                    sign_l,
                    sign_r,
                    inspect_plaque,
                    plaque_backing,
                    page_left,
                    page_right,
                ) = load_archive_room_gpu(ctx);
                require_room_environment_loaded("archive.glb", &prims, &gpu_wrap);
                self.archive_env_primitives = prims;
                self.archive_environment = gpu_wrap;
                self.archive_sign_left_prim_idx = sign_l;
                self.archive_sign_right_prim_idx = sign_r;
                self.archive_inspect_plaque_prim_idx = inspect_plaque;
                self.archive_plaque_backing_prim_idx = plaque_backing;
                self.archive_page_left_prim_indices = page_left;
                self.archive_page_right_prim_indices = page_right;
                crate::archive_glb::release_archive_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= ROOM_ARCHIVE;
                self.note_room_gpu_resident(ROOM_ARCHIVE);
            },
        );
    }

    pub(super) fn ensure_gameplay_room_gpu(&mut self) {
        self.drive_gameplay_room_gpu_upload(f32::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_env_shader_flags_hallway_walls_only() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::HALLWAY,
                Some(crate::hallway_glb::HALLWAY_WALLS_NODE)
            ),
            GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT
        );
        assert_eq!(
            room_env_shader_flags(scene_keys::HALLWAY, Some("ceiling")),
            0
        );
    }

    #[test]
    fn room_env_shader_flags_archive_decal_hosts() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::SIGN_DESCRIPTION_LEFT)
            ),
            GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::SIGN_DESCRIPTION_RIGHT)
            ),
            GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::INSPECT_PLAQUE)
            ),
            GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::PLAQUE_BACKING)
            ),
            0
        );
    }

    #[test]
    fn room_env_shader_flags_main_menu_moon_and_star() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::MAIN_MENU,
                Some(crate::main_menu_glb::MAIN_MENU_MOON_MESH_NODE)
            ),
            GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE
        );
        assert_eq!(
            room_env_shader_flags(scene_keys::MAIN_MENU, Some("star.001")),
            GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW
        );
        assert_eq!(room_env_shader_flags(scene_keys::MAIN_MENU, Some("dock")), 0);
    }

    #[test]
    fn room_env_shader_flags_scene_scoped() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::SHOP,
                Some(crate::hallway_glb::HALLWAY_WALLS_NODE)
            ),
            0
        );
        assert_eq!(
            room_env_shader_flags("unknown_scene", Some("star.001")),
            0
        );
    }
}
