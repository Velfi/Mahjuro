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
    /// Same wood albedo branch as `LacqueredWood`, but with no vertex
    /// displacement and no SSR. Used for thin upright slabs (e.g. the
    /// hanging score plaque) where the table-tuned heightfield amplitude
    /// would push vertices through the slab thickness and produce
    /// rectangular ghost artifacts on the face.
    LacqueredWoodFlat = 4,
    /// Polished metal: tinted specular driven by Schlick Fresnel against the
    /// instance base colour (no separate clearcoat). Diffuse is suppressed
    /// almost entirely so the surface reads as a metallic conductor rather
    /// than a brightly painted plastic.
    Metal = 5,
    /// Discard-river surface: a single mesh that mixes a stone trough and
    /// a flowing water plane in one draw call. Per-fragment branch is
    /// driven by the vertex `uv.y` channel — `uv.y > 0.5` is the water
    /// surface (procedural scrolling normals + indigo/foam tint),
    /// otherwise the fragment is treated as dark stone. Reads `extras.y`
    /// from the point-light buffer for an animated time uniform.
    Water = 6,
    /// Jade-tablet talisman: dielectric material (like Plain) with a
    /// heightmap-driven normal perturbation on the flat faces. The bound
    /// texture is treated as a grayscale heightfield — the shader samples
    /// finite differences and perturbs the surface normal so carved motifs
    /// catch the candle highlights. Uses screen-space derivative tangent
    /// basis so it works regardless of the tablet's world-space orientation
    /// (upright on the wall or laid flat in the tray).
    Talisman = 7,
    /// Metallic foil wrapping — semi-conductor with thin-film iridescence.
    /// The bound texture is sampled as full-colour albedo (pack box art),
    /// overlaid with a view-dependent rainbow sheen that shifts as the
    /// light sweeps across the surface. Specular is high and tinted by
    /// the albedo so the foil reads as a reflective metallic wrapper.
    Foil = 8,
    /// Faux glass / glazed crystal. Still rendered in the opaque pass, but
    /// shaded with a strong Fresnel rim and cool internal glow so small props
    /// read as translucent under the scene lighting.
    Glass = 9,
    /// Hard-enamel lapel pin look: color from `albedo_tex`; height/ridge from
    /// `relief_tex` (binding 3, linear grayscale).
    Enamel = 10,
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

/// Uniform written into a caster's shadow bind group each frame: the
/// light's view-projection matrix paired with the caster's world-space
/// model matrix. The shadow vertex shader (`shaders/shadow.wgsl`) reads
/// this and emits clip positions in light space.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowCasterUniform {
    pub light_view_proj: [f32; 16],
    pub model: [f32; 16],
}

/// Per-instance state: a uniform buffer (rewritten each frame) + a bind group
/// that points at the buffer plus a shared 1×1 white albedo texture/sampler.
///
/// Also owns a sibling shadow-caster uniform + bind group used by the
/// shadow pre-pass. Both buffers are rewritten in lockstep with the same
/// model matrix every frame.
pub struct LitMeshInstance {
    pub uniform_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    pub shadow_uniform_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    pub shadow_bind_group: wgpu::BindGroup,
    /// Optional per-instance decal texture (used by yaku/wood tablets to
    /// engrave a label on top of the procedural base material). When set,
    /// `bind_group` binds this texture at slot 1 instead of the shared
    /// transparent placeholder. The cached `(width, height, label hash)` lets
    /// the renderer skip work when nothing has changed.
    pub decal_texture: Option<wgpu::Texture>,
    pub decal_label_hash: u64,
    pub decal_size: (u32, u32),
}

impl LitMeshInstance {
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shadow_caster_layout: &wgpu::BindGroupLayout,
        albedo_view: &wgpu::TextureView,
        relief_view: &wgpu::TextureView,
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
                    resource: wgpu::BindingResource::TextureView(albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(relief_view),
                },
            ],
        });
        let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit-mesh-shadow-uniform"),
            contents: bytemuck::bytes_of(&ShadowCasterUniform {
                light_view_proj: identity,
                model: identity,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-shadow-bg"),
            layout: shadow_caster_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_uniform_buffer.as_entire_binding(),
            }],
        });
        Self {
            uniform_buffer,
            bind_group,
            shadow_uniform_buffer,
            shadow_bind_group,
            decal_texture: None,
            decal_label_hash: 0,
            decal_size: (0, 0),
        }
    }

    /// Upload an RGBA8 decal texture for this instance and rebind it at
    /// slot 1 of the material bind group. Used by the tablet decal pass to
    /// engrave per-instance labels on bone/wood tablets without changing the
    /// pipeline layout. The instance keeps ownership of the texture so it
    /// stays alive for as long as the bind group references it.
    pub fn set_decal(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        relief_view: &wgpu::TextureView,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) {
        // Reuse the existing texture if its dimensions match — only the bytes
        // change. Otherwise (or first time) allocate a fresh texture.
        let needs_alloc = self
            .decal_texture
            .as_ref()
            .map(|_| self.decal_size != (width, height))
            .unwrap_or(true);
        if needs_alloc {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("lit-mesh-decal"),
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
            self.decal_texture = Some(tex);
            self.decal_size = (width, height);
        }
        let tex = self.decal_texture.as_ref().expect("decal texture present");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-bg-decal"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(relief_view),
                },
            ],
        });
    }

    /// Rebind the material bind group with an externally-owned texture view.
    /// Used by the talisman pass to swap heightmap textures per-instance
    /// without uploading new pixel data every frame.
    pub fn rebind_texture(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        albedo_view: &wgpu::TextureView,
        relief_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-bg-rebind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(relief_view),
                },
            ],
        });
    }

    /// Write the per-instance shadow caster uniform with the current
    /// frame's light view-projection and the instance's model matrix.
    #[allow(dead_code)]
    pub fn write_shadow_uniform(
        &self,
        queue: &wgpu::Queue,
        light_view_proj: [f32; 16],
        model: glam::Mat4,
    ) {
        let u = ShadowCasterUniform {
            light_view_proj,
            model: model.to_cols_array(),
        };
        queue.write_buffer(&self.shadow_uniform_buffer, 0, bytemuck::bytes_of(&u));
    }

    pub fn write_uniform(
        &self,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        model: glam::Mat4,
        material: MaterialParams,
    ) {
        self.write_uniform_with_decal(queue, view_proj, model, material, false);
    }

    /// Same as [`write_uniform`] but also sets the per-instance "has engraved
    /// decal" flag in `material_params.w`. The shader treats the bound
    /// texture as a transparent overlay (composited via mix) instead of a
    /// multiplicative albedo when this flag is set.
    pub fn write_uniform_with_decal(
        &self,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        model: glam::Mat4,
        material: MaterialParams,
        has_decal: bool,
    ) {
        self.write_uniform_raw_w(
            queue,
            view_proj,
            model,
            material,
            if has_decal { 1.0 } else { 0.0 },
        );
    }

    /// Lowest-level uniform write: caller supplies the raw `f32` that lands
    /// in `material_params.w`. Talisman rendering uses this to pass the
    /// sub-kind index (0=jade, 1=pearl, 2=gilded, 3=polychrome) so the
    /// shader can select per-kind sheen effects.
    pub fn write_uniform_raw_w(
        &self,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        model: glam::Mat4,
        material: MaterialParams,
        params_w: f32,
    ) {
        let u = MeshUniform {
            view_proj,
            model: model.to_cols_array(),
            base_color: material.base_color,
            material_params: [
                material.kind as u32 as f32,
                material.specular_strength,
                material.specular_power,
                params_w,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
    }

    /// Write with an explicit RGBA `base_color` override, bypassing the
    /// material's default. Used by ghost/trail passes that need per-instance
    /// alpha + tint without mutating the shared mesh material.
    pub fn write_uniform_tinted(
        &self,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        model: glam::Mat4,
        material: MaterialParams,
        base_color: [f32; 4],
    ) {
        let u = MeshUniform {
            view_proj,
            model: model.to_cols_array(),
            base_color,
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

/// Bind-group layout for the per-caster shadow uniform consumed by
/// `shaders/shadow.wgsl` during the shadow pre-pass. A single uniform
/// containing `(light_view_proj, model)`.
pub fn create_shadow_caster_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow-caster-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Frame-shared shadow sampling uniform consumed by lit_mesh.wgsl /
/// tile_3d.wgsl / tile_outline.wgsl in the main pass via group 2.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowGlobals {
    pub light_view_proj: [f32; 16],
    /// x = enabled (0/1), y = depth bias, z = texel size, w = unused.
    pub params: [f32; 4],
}

/// Bind-group layout for the shadow-sampling group (group 2) shared by
/// all 3D scene shaders. Layout: uniform + depth texture + comparison
/// sampler.
pub fn create_shadow_sample_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow-sample-layout"),
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
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    })
}

/// Frame-shared SSR globals consumed by `lit_mesh.wgsl` (group 3) for
/// the lacquered-wood reflection march. The camera is fixed, so this is
/// rewritten once per frame with the current view-projection inverse.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SsrGlobals {
    pub inv_view_proj: [f32; 16],
    pub view_proj: [f32; 16],
    /// xyz = camera world position, w = unused
    pub view_pos: [f32; 4],
    /// x = enabled (0/1), y = max_distance (world units), z = stride
    /// (world units per step), w = max_steps
    pub params: [f32; 4],
}

/// Bind-group layout for the lit_mesh SSR group (group 3): SSR globals
/// uniform + previous-frame scene colour + scene depth + a filtering
/// sampler shared by both textures.
pub fn create_lit_mesh_ssr_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lit-mesh-ssr-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
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
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Append a colored axis-aligned box to (vertices, indices). 6 quads, 24
/// verts (each face has its own normal so the lit shader reads flat).
/// Shared helper for procedural mesh builders that compose from boxes
/// (plaque, ofuda, tablets, peg block). The standalone curio
/// cabinet keeps its own private copy because it predates this helper.
pub fn push_box(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    z0: f32,
    z1: f32,
) {
    let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
        // +X
        (
            [1.0, 0.0, 0.0],
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        ),
        // -X
        (
            [-1.0, 0.0, 0.0],
            [[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]],
        ),
        // +Y
        (
            [0.0, 1.0, 0.0],
            [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        ),
        // -Y
        (
            [0.0, -1.0, 0.0],
            [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
        ),
        // +Z
        (
            [0.0, 0.0, 1.0],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        ),
        // -Z
        (
            [0.0, 0.0, -1.0],
            [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        ),
    ];
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: *normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Append a single flat quad to (vertices, indices) with explicit corners
/// and a shared face normal. UVs are zeroed so decal textures don't bleed;
/// callers can overwrite them after the fact if they want a mapped face.
///
/// Corners must be wound counter-clockwise when viewed from the direction
/// the normal points. Used by procedural meshes that need non-axis-aligned
/// faces (e.g. chamfered bevels).
pub fn push_quad(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    v0: [f32; 3],
    v1: [f32; 3],
    v2: [f32; 3],
    v3: [f32; 3],
    normal: [f32; 3],
) {
    let base = vertices.len() as u32;
    for pos in [v0, v1, v2, v3] {
        vertices.push(Vertex3dTex {
            position: pos,
            normal,
            uv: [0.0, 0.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
            // Grayscale relief / height for relic enamel (binding 1 = color).
            // Other materials bind a 1×1 mid-gray stub.
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
        ],
    })
}
