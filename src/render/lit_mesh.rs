//! Generic GPU resources for procedural lit meshes (candles, table).
//!
//! Each mesh primitive owns its own vertex/index buffers plus a single uniform
//! buffer + bind group that the renderer rewrites once per frame with the
//! per-instance model matrix and material parameters. The shader (`lit_mesh.wgsl`)
//! branches on `material_kind` so candles and the wood table can share one
//! pipeline.

use wgpu::util::DeviceExt;

use crate::render::tile_glb::Vertex3dTex;

/// Material variants understood by `lit_mesh.wgsl`.
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum MaterialKind {
    Plain = 0,
    Wax = 1,
    Wick = 2,
    LacqueredWood = 3,
}

/// Compact per-mesh material parameters.
#[derive(Clone, Copy, Debug)]
pub struct MaterialParams {
    pub kind: MaterialKind,
    pub base_color: [f32; 4],
    pub specular_strength: f32,
    pub specular_power: f32,
}

impl MaterialParams {
    pub fn wax() -> Self {
        Self {
            kind: MaterialKind::Wax,
            base_color: [0.94, 0.86, 0.62, 1.0],
            specular_strength: 0.0,
            specular_power: 16.0,
        }
    }
    pub fn wick() -> Self {
        Self {
            kind: MaterialKind::Wick,
            base_color: [0.12, 0.08, 0.05, 1.0],
            specular_strength: 0.0,
            specular_power: 8.0,
        }
    }
    pub fn lacquered_wood() -> Self {
        Self {
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0], // ignored — wood shader is procedural
            specular_strength: 0.55,
            specular_power: 96.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshUniform {
    pub view_proj: [f32; 16],
    pub model: [f32; 16],
    pub base_color: [f32; 4],
    /// (kind, specular_strength, specular_power, _pad)
    pub material_params: [f32; 4],
}

/// CPU-side mesh data ready to be uploaded.
pub struct MeshCpu {
    pub vertices: Vec<Vertex3dTex>,
    pub indices: Vec<u32>,
    pub default_material: MaterialParams,
}

/// GPU resources for a single lit-mesh primitive (vertex + index buffers).
/// Per-instance uniform buffers + bind groups live in [`LitMeshInstance`].
pub struct LitMeshGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub default_material: MaterialParams,
}

impl LitMeshGpu {
    pub fn new(device: &wgpu::Device, mesh: &MeshCpu, label: &str) -> Self {
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-vb")),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-ib")),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: mesh.indices.len() as u32,
            default_material: mesh.default_material,
        }
    }
}

/// Per-instance state: a uniform buffer (rewritten each frame) + a bind group
/// that points at the buffer plus a shared 1×1 white albedo texture/sampler.
pub struct LitMeshInstance {
    pub uniform_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

impl LitMeshInstance {
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        white_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let identity = glam::Mat4::IDENTITY.to_cols_array();
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit-mesh-uniform"),
            contents: bytemuck::bytes_of(&MeshUniform {
                view_proj: identity,
                model: identity,
                base_color: [1.0; 4],
                material_params: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            uniform_buffer,
            bind_group,
        }
    }

    pub fn write_uniform(
        &self,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        model: glam::Mat4,
        material: MaterialParams,
    ) {
        let u = MeshUniform {
            view_proj,
            model: model.to_cols_array(),
            base_color: material.base_color,
            material_params: [
                material.kind as u32 as f32,
                material.specular_strength,
                material.specular_power,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
    }
}

/// Build the bind-group layout shared by every lit-mesh primitive.
pub fn create_lit_mesh_material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lit-mesh-material-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}
