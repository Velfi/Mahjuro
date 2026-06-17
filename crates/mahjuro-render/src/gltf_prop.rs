//! Instanced props decoded from standalone GLB files and drawn through the
//! glTF PBR path in `tile_3d.wgsl` (same material slots as hand tiles / shop room).

use wgpu::util::DeviceExt;

use crate::gltf_helpers::{GltfPbrUniform, build_sampler_descriptor};
use crate::tile_glb::{LoadedPrimitive, LoadedTile, Vertex3dTex};
use crate::wgpu_renderer::{
    TileGlbPipelineKey, TileMeshGpuSet, TilePrimitiveGpu,
    resources::{TextureUploadParams, upload_rgba_texture_with_mips, white_albedo},
};

fn upload_baked_gltf_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    source_path: Option<&str>,
) -> Option<wgpu::TextureView> {
    let source_path = source_path?;
    let payload = crate::baked_texture::load_baked_texture(source_path).ok()?;
    let (_texture, view, _bytes) = crate::baked_texture::upload_payload(
        device,
        queue,
        label,
        &payload,
        crate::baked_texture::bc7_supported(device),
    );
    Some(view)
}

/// `tile_visual_params.w` — selects the imported glTF PBR branch
/// in `tile_3d.wgsl` (albedo + normal + metallic-roughness, no mahjong decal).
pub const GLTF_PROP_BODY_KIND: f32 = 4.0;

pub(crate) fn create_tile_material_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame_uniform: &wgpu::Buffer,
    prim: &TilePrimitiveGpu,
    decal_view: &wgpu::TextureView,
    distortion_placeholder: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-material-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&prim.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(decal_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&prim.normal_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: prim.pbr_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&prim.metallic_roughness_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&prim.emissive_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: distortion_placeholder.as_entire_binding(),
            },
        ],
    })
}

pub struct GltfTileGpuDefaults<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub default_normal_view: &'a wgpu::TextureView,
    pub default_mr_view: &'a wgpu::TextureView,
    pub default_emissive_view: &'a wgpu::TextureView,
}

pub(crate) fn upload_gltf_tile_primitives(
    defaults: &GltfTileGpuDefaults<'_>,
    label_prefix: &str,
    primitives: &[LoadedPrimitive],
) -> Vec<TilePrimitiveGpu> {
    let GltfTileGpuDefaults {
        device,
        queue,
        default_normal_view,
        default_mr_view,
        default_emissive_view,
    } = *defaults;
    let mut out = Vec::with_capacity(primitives.len());
    for (i, prim) in primitives.iter().enumerate() {
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}-vb-{i}")),
            contents: bytemuck::cast_slice(&prim.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}-ib-{i}")),
            contents: bytemuck::cast_slice(&prim.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
        let (_albedo_texture, albedo_view) = if let Some(view) = upload_baked_gltf_slot(
            device,
            queue,
            &format!("{label_prefix}-albedo-{i}"),
            prim.albedo_btx_source_path.as_deref(),
        ) {
            (white_albedo(device, queue).0, view)
        } else {
            match prim.albedo_rgba.as_deref() {
                Some((rgba, w, h)) => upload_rgba_texture_with_mips(&TextureUploadParams {
                    device,
                    queue,
                    label: format!("{label_prefix}-albedo-{i}"),
                    rgba,
                    width: *w,
                    height: *h,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                }),
                None => white_albedo(device, queue),
            }
        };
        let normal_view = match prim.normal_rgba.as_deref() {
            Some((rgba, w, h)) => upload_baked_gltf_slot(
                device,
                queue,
                &format!("{label_prefix}-normal-{i}"),
                prim.normal_btx_source_path.as_deref(),
            )
            .unwrap_or_else(|| {
                upload_rgba_texture_with_mips(&TextureUploadParams {
                    device,
                    queue,
                    label: format!("{label_prefix}-normal-{i}"),
                    rgba,
                    width: *w,
                    height: *h,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                })
                .1
            }),
            None => default_normal_view.clone(),
        };
        let metallic_roughness_view = match prim.metallic_roughness_rgba.as_deref() {
            Some((rgba, w, h)) => upload_baked_gltf_slot(
                device,
                queue,
                &format!("{label_prefix}-mr-{i}"),
                prim.metallic_roughness_btx_source_path.as_deref(),
            )
            .unwrap_or_else(|| {
                upload_rgba_texture_with_mips(&TextureUploadParams {
                    device,
                    queue,
                    label: format!("{label_prefix}-mr-{i}"),
                    rgba,
                    width: *w,
                    height: *h,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                })
                .1
            }),
            None => default_mr_view.clone(),
        };
        let emissive_view = match prim.emissive_rgba.as_deref() {
            Some((rgba, w, h)) => upload_baked_gltf_slot(
                device,
                queue,
                &format!("{label_prefix}-emissive-{i}"),
                prim.emissive_btx_source_path.as_deref(),
            )
            .unwrap_or_else(|| {
                upload_rgba_texture_with_mips(&TextureUploadParams {
                    device,
                    queue,
                    label: format!("{label_prefix}-emissive-{i}"),
                    rgba,
                    width: *w,
                    height: *h,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                })
                .1
            }),
            None => default_emissive_view.clone(),
        };
        let pbr_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}-pbr-{i}")),
            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                prim.metallic_factor,
                prim.roughness_factor,
                prim.emissive_factor,
                prim.alpha_mode,
                prim.alpha_cutoff,
            )),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let sampler = device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
        out.push(TilePrimitiveGpu {
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
            material_bind_group: None,
        });
    }
    out
}

pub(crate) fn upload_tile_mesh_gpu_set(
    defaults: &GltfTileGpuDefaults<'_>,
    label_prefix: &str,
    mesh: &LoadedTile,
) -> TileMeshGpuSet {
    let mut merge_vertices: Vec<Vertex3dTex> = Vec::new();
    let mut merge_indices: Vec<u32> = Vec::new();
    for prim in &mesh.primitives {
        let base = merge_vertices.len() as u32;
        merge_vertices.extend_from_slice(&prim.vertices);
        merge_indices.extend(prim.indices.iter().map(|&ix| ix + base));
    }
    let primitives = upload_gltf_tile_primitives(defaults, label_prefix, &mesh.primitives);
    let device = defaults.device;
    let (outline_vertex_buffer, outline_index_buffer, outline_index_count) =
        if merge_indices.is_empty() {
            let dummy_outline_vertex = Vertex3dTex {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            };
            let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{label_prefix}-outline-verts-dummy")),
                contents: bytemuck::cast_slice(&[dummy_outline_vertex; 3]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{label_prefix}-outline-idx-dummy")),
                contents: bytemuck::cast_slice(&[0u32, 1, 2]),
                usage: wgpu::BufferUsages::INDEX,
            });
            (vb, ib, 0u32)
        } else {
            let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{label_prefix}-outline-verts")),
                contents: bytemuck::cast_slice(&merge_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{label_prefix}-outline-idx")),
                contents: bytemuck::cast_slice(&merge_indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            (vb, ib, merge_indices.len() as u32)
        };
    TileMeshGpuSet {
        primitives,
        outline_vertex_buffer,
        outline_index_buffer,
        outline_index_count,
    }
}
