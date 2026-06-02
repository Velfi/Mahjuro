//! Generic GPU resources for procedural lit meshes (candles, table).
//!
//! Each mesh primitive owns its own vertex/index buffers plus a single uniform
//! buffer + bind group that the renderer rewrites once per frame with the
//! per-instance model matrix and material parameters. The shader (`lit_mesh.wgsl`)
//! branches on `material_kind` so candles and the wood table can share one
//! pipeline.

use std::cell::RefCell;
use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::theme::color;
use crate::wgpu_renderer::{MAX_POINT_LIGHTS};
use crate::tile_glb::Vertex3dTex;

/// Axis-aligned box extents for the `push_box` family of mesh builders.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub z0: f32,
    pub z1: f32,
}

impl Aabb {
    pub const fn new(x0: f32, x1: f32, y0: f32, y1: f32, z0: f32, z1: f32) -> Self {
        Self {
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
        }
    }
}

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
    /// Booster-pack shrink wrap — clear dielectric gloss over box art.
    /// The bound texture is the cover decal on the front face; edges read
    /// as a lightly tinted plastic sleeve (no metallic streaks or holo).
    PackWrap = 7,
    /// Metallic foil — thin-film iridescence (material viewer / legacy).
    Foil = 8,
    /// Faux glass / glazed crystal. Still rendered in the opaque pass, but
    /// shaded with a strong Fresnel rim and cool internal glow so small props
    /// read as translucent under the scene lighting.
    Glass = 9,
    /// Hard-enamel lapel pin look: color from `albedo_tex`; height/ridge from
    /// `relief_tex` (binding 3, linear grayscale).
    Enamel = 10,
    /// Carved jade tablet — waxy vitreous green dielectric with broad
    /// view-dependent sheen, soft SSS, and back-transmission glow on
    /// silhouette edges. The bound texture is a grayscale heightfield;
    /// the shader perturbs the normal on the flat faces so carved motifs
    /// catch the candle highlights.
    Jade = 11,
    /// Moonstone — transparent feldspar with blue adularescence.
    /// Layered specular (tight white pinpoint + wide soft-blue schiller
    /// halo) over a clear body with cool SSS and a moon-blue Fresnel
    /// rim. `base_color` tints the body. Uses the talisman heightmap.
    Moonstone = 12,
    /// Pearl / nacre — pearlescent surface with view-dependent hue shift
    /// (pink/blue iridescence) and broad sheen. `base_color` tints the
    /// nacre (white pearl, gold honors). Uses the talisman heightmap.
    Pearl = 13,
    /// Pitted gold nugget — metallic gold conductor with procedural
    /// noise-driven normal perturbation that reads as raw cast-metal
    /// pitting. Tinted Schlick Fresnel highlights.
    GoldNugget = 14,
    /// Holographic polychrome — thin-film iridescence with a rainbow
    /// hue that shifts with viewing angle. Uses the talisman heightmap.
    Polychrome = 15,
    /// Glazed porcelain — cool-white dielectric with a crisp narrow
    /// specular highlight, subtle Fresnel rim, and a soft warm SSS wrap
    /// so the ceramic reads as glazed rather than matte plastic.
    /// Composites engraved decals (same `has_decal` path as Plain).
    Porcelain = 16,
    /// Polished brass — smooth conductor with a wider warm-gold rim halo
    /// than `Metal` and a touch more diffuse retained, so brass fittings
    /// read as bright polished metal in overhead light without going
    /// near-black off-axis. Use for hanging brass props (bells, rails).
    Brass = 17,
    /// Bookbinding leather — warm dielectric with procedural grain,
    /// broad soft sheen (no tight pinpoint), subtle Fresnel rim, and a
    /// little wrap-SSS so the shadow side stays warm rather than dead
    /// black. `base_color` drives the leather tint (oxblood, cordovan,
    /// tan). Composites engraved decals via the same `has_decal` path
    /// as Plain/LacqueredWoodFlat.
    Leather = 18,
    /// Legacy material slot (procedural green baize removed). Kept so
    /// `#[repr(u32)]` discriminants stay stable.
    FeltGreen = 19,
    /// Unlit emission added on top of the usual lit path. `specular_strength`
    /// scales `base_color.rgb` in the shader (`lit_mesh.wgsl` emissive term).
    Emissive = 20,
    /// Talisman tablets — abalone shell (oily iridescence / memorial stone-pearl).
    /// `base_color` tints the sheen; `material_params.w` is the kind index (memorial
    /// adds 128). Uses the talisman heightmap for carved relief and iridescence phase.
    Chitin = 21,
    /// Flat texture read — boss ordeal icons and other extruded decals that should
    /// match their 2D atlas art without scene lighting or specular.
    Unshaded = 22,
}

/// Whether instances using this material should participate in the
/// directional shadow map (lamps, filaments, and other pure emitters opt out).
#[inline]
pub fn material_casts_shadow(kind: MaterialKind) -> bool {
    !matches!(kind, MaterialKind::Emissive | MaterialKind::Unshaded)
}

/// Balance for props on the shop shelf when embedded punctual lights and HDR
/// tonemap are active (`SsrGlobals.shop_punctual.y == DISPLAY_CASE_STOREROOM`).
///
/// Spec-forward materials (pack wrap, foil, talismans) pull back direct lit in
/// `lit_mesh.wgsl`; art-forward materials (enamel relics) use [`AMBIENT_MUL`].
pub mod shop_catalog_balance {
    /// `SsrGlobals.shop_punctual.y` — storeroom shelf row balance active.
    pub const DISPLAY_CASE_STOREROOM: f32 = 1.0;
    /// Hemispheric ambient multiplier for art-forward catalog props.
    pub const AMBIENT_MUL: f32 = 0.22;
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
            base_color: color::WALNUT_RAISED,
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

    /// Warm lamp filament / bulb: `intensity` drives HDR-style glow in the
    /// lit-mesh path (still tonemapped like the rest of the table scene).
    pub fn emissive_lamp(tint_rgb: [f32; 3], intensity: f32) -> Self {
        Self {
            kind: MaterialKind::Emissive,
            base_color: [tint_rgb[0], tint_rgb[1], tint_rgb[2], 1.0],
            specular_strength: intensity,
            specular_power: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
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
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowCasterUniform {
    pub light_view_proj: [f32; 16],
    pub model: [f32; 16],
}

/// GPU + bind-group context needed by `LitMeshInstance::set_decal` to
/// upload or rebind a per-instance decal texture. Groups the handles
/// that travel together so the call site stays compact.
#[derive(Copy, Clone)]
pub struct DecalUploadCtx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub layout: &'a wgpu::BindGroupLayout,
    pub sampler: &'a wgpu::Sampler,
    pub relief_view: &'a wgpu::TextureView,
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

    pub shadow_uniform_buffer: wgpu::Buffer,

    pub shadow_bind_group: wgpu::BindGroup,
    /// Optional per-instance decal texture (used by yaku/wood tablets to
    /// engrave a label on top of the procedural base material). When set,
    /// `bind_group` binds this texture at slot 1 instead of the shared
    /// transparent placeholder. The cached `(width, height, label hash)` lets
    /// the renderer skip work when nothing has changed.
    pub decal_texture: Option<wgpu::Texture>,
    pub decal_label_hash: u64,
    pub decal_size: (u32, u32),
    /// Generation stamp for the relief-slot view this instance's bind
    /// group was last bound against. The journal book body uses the
    /// renderer's `journal_scene_view_generation` here so a recreated
    /// (post-resize) view forces re-binding before the next draw — the
    /// alternative is a "Texture has been destroyed" validation error
    /// the first frame after resize. Other lit-mesh instances bind a
    /// stable per-renderer view at slot 3 and ignore this field.
    pub relief_view_generation: u32,
    /// Last CPU-side copy of the main-pass uniform; skips redundant
    /// `queue.write_buffer` when nothing changed (big win for static shop/menu).
    last_main_uniform: RefCell<MeshUniform>,
    /// Last shadow caster uniform — paired with shadow passes skipping when idle.
    last_shadow_uniform: RefCell<ShadowCasterUniform>,
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
        let initial_main = MeshUniform {
            view_proj: identity,
            model: identity,
            base_color: [1.0; 4],
            material_params: [0.0; 4],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit-mesh-uniform"),
            contents: bytemuck::bytes_of(&initial_main),
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
        let initial_shadow = ShadowCasterUniform {
            light_view_proj: identity,
            model: identity,
        };
        let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit-mesh-shadow-uniform"),
            contents: bytemuck::bytes_of(&initial_shadow),
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
            relief_view_generation: u32::MAX,
            last_main_uniform: RefCell::new(initial_main),
            last_shadow_uniform: RefCell::new(initial_shadow),
        }
    }

    /// Upload an RGBA8 decal texture for this instance and rebind it at
    /// slot 1 of the material bind group. Used by the tablet decal pass to
    /// engrave per-instance labels on bone/wood tablets without changing the
    /// pipeline layout. The instance keeps ownership of the texture so it
    /// stays alive for as long as the bind group references it.
    pub fn set_decal(&mut self, ctx: DecalUploadCtx<'_>, rgba: &[u8], width: u32, height: u32) {
        let DecalUploadCtx {
            device,
            queue,
            layout,
            sampler,
            relief_view,
        } = ctx;
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
    /// Upload shadow caster uniform if it differs from the last upload.
    /// Returns `true` when the GPU buffer was written (shadow map must redraw).
    pub fn write_shadow_uniform(
        &self,
        queue: &wgpu::Queue,
        light_view_proj: [f32; 16],
        model: glam::Mat4,
    ) -> bool {
        let u = ShadowCasterUniform {
            light_view_proj,
            model: model.to_cols_array(),
        };
        let mut last = self.last_shadow_uniform.borrow_mut();
        if *last == u {
            return false;
        }
        *last = u;
        queue.write_buffer(&self.shadow_uniform_buffer, 0, bytemuck::bytes_of(&u));
        true
    }

    /// Update only the light view-projection for an already-uploaded shadow caster.
    pub fn rewrite_shadow_light_view_proj(&self, queue: &wgpu::Queue, light_view_proj: [f32; 16]) {
        let mut last = self.last_shadow_uniform.borrow_mut();
        if last.light_view_proj == light_view_proj {
            return;
        }
        last.light_view_proj = light_view_proj;
        queue.write_buffer(
            &self.shadow_uniform_buffer,
            0,
            bytemuck::bytes_of(&*last),
        );
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
    /// in `material_params.w`. Talisman rendering passes the per-kind
    /// heightmap-texture index here so the shader's heightmap-driven
    /// normal perturbation samples the right relief carving.
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
        let mut last = self.last_main_uniform.borrow_mut();
        if *last == u {
            return;
        }
        *last = u;
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
        let mut last = self.last_main_uniform.borrow_mut();
        if *last == u {
            return;
        }
        *last = u;
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
    }
}

/// Per-primitive camera uniform buffers for imported room GLB meshes.
/// Each primitive gets its own buffer so glTF node TRS deltas can be baked
/// before the render pass (mid-pass `write_buffer` on a shared uniform is unreliable).
pub fn create_room_env_camera_uniform_buffers(
    device: &wgpu::Device,
    count: usize,
    label: &str,
) -> Vec<wgpu::Buffer> {
    use crate::tile_body;
    use crate::wgpu_renderer::CameraUniform;
    let identity = glam::Mat4::IDENTITY.to_cols_array();
    let initial = CameraUniform {
        view_proj: identity,
        model: identity,
        base_color_factor: [1.0, 0.0, 0.0, tile_body::TEXTURED_BASE_MAP_BODY_KIND],
        cam_pos: [0.0; 3],
        tile_seed: 0.0,
        decal_atlas_uv: [0.0, 0.0, 1.0, 1.0],
        hdr_tonemap: [0.0; 4],
    };
    (0..count)
        .map(|i| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{label}-{i}")),
                contents: bytemuck::bytes_of(&initial),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        })
        .collect()
}

/// Per-primitive shadow caster buffers + bind groups for room GLB depth passes.
pub fn create_room_env_shadow_gpu_batch(
    device: &wgpu::Device,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    count: usize,
    label: &str,
) -> (Vec<wgpu::Buffer>, Vec<wgpu::BindGroup>) {
    let identity = glam::Mat4::IDENTITY.to_cols_array();
    let initial = ShadowCasterUniform {
        light_view_proj: identity,
        model: identity,
    };
    let mut buffers = Vec::with_capacity(count);
    let mut bind_groups = Vec::with_capacity(count);
    for i in 0..count {
        let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-{i}")),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{label}-bg-{i}")),
            layout: shadow_caster_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_uniform_buffer.as_entire_binding(),
            }],
        });
        buffers.push(shadow_uniform_buffer);
        bind_groups.push(shadow_bind_group);
    }
    (buffers, bind_groups)
}

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

/// Group 1 of `shaders/shadow.wgsl` — same `HallwayDistortion` layout as `room_glb` @binding(8).
/// Zeroed buffer ⇒ warp disabled (tiles, lit meshes, shop room, etc.).
pub fn create_shadow_warp_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let sz = std::mem::size_of::<crate::hallway_glb::HallwayDistortion>() as u64;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow-warp-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(sz),
            },
            count: None,
        }],
    })
}

pub fn create_shadow_warp_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    distortion_uniform: &wgpu::Buffer,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: distortion_uniform.as_entire_binding(),
        }],
    })
}

/// Frame-shared shadow sampling uniform consumed by lit_mesh.wgsl /
/// tile_3d.wgsl / room_glb.wgsl in the main pass via group 2.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowGlobals {
    /// x = enabled (0/1), y = depth bias, z = point texel size, w = unused.
    pub params: [f32; 4],
    /// x = shadow caster count, y/z/w = unused (spot shadows removed).
    pub counts: [f32; 4],
    /// Dense 0..caster_count-1 view-projection matrices.
    pub point_view_proj: [[f32; 16]; MAX_POINT_LIGHTS],
    /// Lighting index → shadow layer packed as four vec4 rows (std140 aligned), or -1.
    pub point_light_layer: [[f32; 4]; 4],
    /// Offline `.msh` contact-AO projection (independent of live point lights).
    pub contact_ao_view_proj: [f32; 16],
}

impl ShadowGlobals {
    pub fn empty() -> Self {
        Self {
            params: [0.0, 0.005, 0.0, 0.0],
            counts: [0.0; 4],
            point_view_proj: [[0.0; 16]; MAX_POINT_LIGHTS],
            point_light_layer: [[-1.0; 4]; 4],
            contact_ao_view_proj: [0.0; 16],
        }
    }
}

/// GPU depth 2D array for projected punctual shadows.
pub struct ShadowDepthArrayGpu {
    pub texture: wgpu::Texture,
    pub layer_views: Vec<wgpu::TextureView>,
    pub array_view: wgpu::TextureView,
    pub size: u32,
}

pub fn create_shadow_depth_array(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    layers: u32,
) -> ShadowDepthArrayGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.max(1),
            height: size.max(1),
            depth_or_array_layers: layers.max(1),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let layer_views: Vec<_> = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("{label}-layer-{layer}")),
                format: Some(wgpu::TextureFormat::Depth32Float),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&format!("{label}-array")),
        format: Some(wgpu::TextureFormat::Depth32Float),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    ShadowDepthArrayGpu {
        texture,
        layer_views,
        array_view,
        size,
    }
}

/// Bind-group layout for the shadow-sampling group (group 2) shared by
/// all 3D scene shaders.
pub fn create_shadow_sample_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow-sample-layout"),
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
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                },
                count: None,
            },
        ],
    })
}

/// Shared shadow-sampling bind group (projected point/spot arrays + contact AO + baked depth).
pub fn create_shadow_sample_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    globals_buffer: &wgpu::Buffer,
    point_depth: &wgpu::TextureView,
    spot_depth: &wgpu::TextureView,
    compare_sampler: &wgpu::Sampler,
    ao_view: &wgpu::TextureView,
    ao_sampler: &wgpu::Sampler,
    baked_depth_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(point_depth),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(spot_depth),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(compare_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(ao_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(ao_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(baked_depth_view),
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
    /// Matches [`crate::wgpu_renderer::uniforms::CameraUniform::hdr_tonemap`]:
    /// x = ACES HDR path when **1**; y = linear exposure before ACES;
    /// z = hemispheric ambient scale; w = reserved.
    pub hdr_tonemap: [f32; 4],
    /// x = `1/room_env_world_scale` for embedded glTF punctual attenuation in `lit_mesh`
    /// (document-space distance; **0** = world-space / gameplay).
    /// y = shop display-case material tuning (1 = shop + embedded punctual only); zw unused.
    pub shop_punctual: [f32; 4],
}

/// Bind-group layout for lit_mesh group 3: spotlights (binding 0, same
/// uniform as `tile_3d.wgsl` / the tile pipeline) plus SSR
/// globals + history textures (bindings 1–4). Merged so the lit_mesh
/// pipeline stays within WebGPU's four bind-group limit.
pub fn create_lit_mesh_spot_ssr_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lit-mesh-spot-ssr-layout"),
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
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
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
            wgpu::BindGroupLayoutEntry {
                binding: 4,
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
pub fn push_box(vertices: &mut Vec<Vertex3dTex>, indices: &mut Vec<u32>, aabb: Aabb) {
    let Aabb {
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
    } = aabb;
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
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Vertex/index targets for procedural lit-mesh builders.
pub struct LitMeshBuffers<'a> {
    pub vertices: &'a mut Vec<Vertex3dTex>,
    pub indices: &'a mut Vec<u32>,
}

/// Right circular cylinder with axis parallel to **+Y**.
#[derive(Clone, Copy, Debug)]
pub struct CylinderYParams {
    pub cx: f32,
    pub cz: f32,
    pub y0: f32,
    pub y1: f32,
    pub radius: f32,
    pub segments: usize,
}

/// Right circular cylinder with axis parallel to **+Z**.
#[derive(Clone, Copy, Debug)]
pub struct CylinderZParams {
    pub cx: f32,
    pub cy: f32,
    pub z0: f32,
    pub z1: f32,
    pub radius: f32,
    pub segments: usize,
}

/// Right circular cylinder with axis parallel to **+Y**, spanning `y0..y1`,
/// circular footprint centered at `(cx, cz)` in XZ. Includes both caps and a
/// smooth-shaded barrel (radial normals). UVs zeroed.
pub fn push_cylinder_y(buffers: &mut LitMeshBuffers<'_>, params: &CylinderYParams) {
    let CylinderYParams {
        cx,
        cz,
        y0,
        y1,
        radius,
        segments,
    } = *params;
    let vertices = &mut *buffers.vertices;
    let indices = &mut *buffers.indices;
    debug_assert!(y1 > y0);
    let n = segments.max(4);
    let two_pi = std::f32::consts::TAU;

    // Barrel (radial normals).
    let barrel_bot = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, y0, cz + radius * st],
            normal: [ct, 0.0, st],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    let barrel_top = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, y1, cz + radius * st],
            normal: [ct, 0.0, st],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = i as u32;
        let i1 = ((i + 1) % n) as u32;
        let b0 = barrel_bot + i0;
        let b1 = barrel_bot + i1;
        let t0 = barrel_top + i0;
        let t1 = barrel_top + i1;
        indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
    }

    // Bottom cap (−Y).
    let c_bot = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [cx, y0, cz],
        normal: [0.0, -1.0, 0.0],
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let ring_bot = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, y0, cz + radius * st],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = ring_bot + i as u32;
        let i1 = ring_bot + ((i + 1) % n) as u32;
        indices.extend_from_slice(&[c_bot, i0, i1]);
    }

    // Top cap (+Y).
    let c_top = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [cx, y1, cz],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let ring_top = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, y1, cz + radius * st],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = ring_top + i as u32;
        let i1 = ring_top + ((i + 1) % n) as u32;
        indices.extend_from_slice(&[c_top, i1, i0]);
    }
}

/// Right circular cylinder with axis parallel to **+Z**, spanning `z0..z1`,
/// circular footprint centered at `(cx, cy)` in XY. Includes both caps and a
/// smooth-shaded barrel (radial XY normals). UVs zeroed.
pub fn push_cylinder_z(buffers: &mut LitMeshBuffers<'_>, params: &CylinderZParams) {
    let CylinderZParams {
        cx,
        cy,
        z0,
        z1,
        radius,
        segments,
    } = *params;
    let vertices = &mut *buffers.vertices;
    let indices = &mut *buffers.indices;
    debug_assert!(z1 > z0);
    let n = segments.max(4);
    let two_pi = std::f32::consts::TAU;

    let barrel_bot = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, cy + radius * st, z0],
            normal: [ct, st, 0.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    let barrel_top = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, cy + radius * st, z1],
            normal: [ct, st, 0.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = i as u32;
        let i1 = ((i + 1) % n) as u32;
        let b0 = barrel_bot + i0;
        let b1 = barrel_bot + i1;
        let t0 = barrel_top + i0;
        let t1 = barrel_top + i1;
        indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
    }

    // Bottom cap (−Z).
    let c_bot = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [cx, cy, z0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let ring_bot = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, cy + radius * st, z0],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = ring_bot + i as u32;
        let i1 = ring_bot + ((i + 1) % n) as u32;
        indices.extend_from_slice(&[c_bot, i1, i0]);
    }

    // Top cap (+Z).
    let c_top = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [cx, cy, z1],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let ring_top = vertices.len() as u32;
    for i in 0..n {
        let t = two_pi * i as f32 / n as f32;
        let (ct, st) = (t.cos(), t.sin());
        vertices.push(Vertex3dTex {
            position: [cx + radius * ct, cy + radius * st, z1],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..n {
        let i0 = ring_top + i as u32;
        let i1 = ring_top + ((i + 1) % n) as u32;
        indices.extend_from_slice(&[c_top, i0, i1]);
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
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
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
