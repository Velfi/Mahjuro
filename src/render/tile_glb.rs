//! Load mesh + PBR base color from a GLB (all primitives of the first mesh).

use anyhow::Context;
use glam::Mat4;
use gltf::image::Format;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex3dTex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// One material-slot from the GLB (maps to one glTF primitive).
pub struct LoadedPrimitive {
    pub vertices: Vec<Vertex3dTex>,
    pub indices: Vec<u32>,
    /// glTF `baseColorFactor` (linear).
    pub base_color_factor: [f32; 4],
    /// Decoded RGBA8, row-major.
    pub albedo_rgba: Option<(Vec<u8>, u32, u32)>,
}

/// All primitives of the first mesh in the GLB.
pub struct LoadedTile {
    pub primitives: Vec<LoadedPrimitive>,
}

/// Convert imported glTF image to RGBA8 for GPU upload.
pub fn gltf_image_to_rgba8(img: &gltf::image::Data) -> Option<(Vec<u8>, u32, u32)> {
    let w = img.width;
    let h = img.height;
    match img.format {
        Format::R8G8B8A8 => Some((img.pixels.clone(), w, h)),
        Format::R8G8B8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for chunk in img.pixels.chunks(3) {
                v.extend_from_slice(chunk);
                v.push(255);
            }
            Some((v, w, h))
        }
        Format::R8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for &g in &img.pixels {
                v.extend_from_slice(&[g, g, g, 255]);
            }
            Some((v, w, h))
        }
        Format::R8G8 => {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for chunk in img.pixels.chunks_exact(2) {
                v.push(chunk[0]);
                v.push(chunk[1]);
                v.push(0);
                v.push(255);
            }
            Some((v, w, h))
        }
        _ => {
            log::warn!(
                "unsupported glTF image format {:?}; use a PNG/JPEG base color or RGBA8",
                img.format
            );
            None
        }
    }
}

/// Center mesh at origin and scale so the largest AABB extent is 1.0.
/// Uses a shared AABB across all primitives so they stay in the same space.
pub fn normalize_mesh(tile: &mut LoadedTile) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut any = false;

    for prim in &tile.primitives {
        for v in &prim.vertices {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
    }

    if !any {
        return;
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];

    let extent = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(1e-6);

    let s = 1.0 / extent;
    for prim in &mut tile.primitives {
        for v in &mut prim.vertices {
            v.position[0] = (v.position[0] - center[0]) * s;
            v.position[1] = (v.position[1] - center[1]) * s;
            v.position[2] = (v.position[2] - center[2]) * s;
        }
    }
}

/// Walk the scene graph and return the accumulated world-space transform for
/// the first node that references `mesh_index`.  Blender GLB exports store the
/// Z-up → Y-up coordinate conversion as a node transform (typically a −90° X
/// rotation) rather than baking it into vertex data, so we must apply it here.
fn find_node_transform(document: &gltf::Document, mesh_index: usize) -> Option<Mat4> {
    let scene = document.default_scene().or_else(|| document.scenes().next())?;
    fn walk(node: gltf::Node<'_>, parent: Mat4, target: usize) -> Option<Mat4> {
        let local = Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;
        if node.mesh().map(|m| m.index()) == Some(target) {
            return Some(world);
        }
        for child in node.children() {
            if let Some(m) = walk(child, world, target) {
                return Some(m);
            }
        }
        None
    }
    for node in scene.nodes() {
        if let Some(m) = walk(node, Mat4::IDENTITY, mesh_index) {
            return Some(m);
        }
    }
    None
}

pub fn load_glb_tile_from_bytes(data: &[u8]) -> anyhow::Result<LoadedTile> {
    let (document, buffers, images) =
        gltf::import_slice(data).context("gltf::import_slice(Tile.glb)")?;

    let mesh = document.meshes().next().context("GLB has no meshes")?;

    // Apply the scene-graph node transform so Blender's Z-up → Y-up conversion
    // (stored as a node rotation, not baked into vertex data) takes effect.
    let node_xform = find_node_transform(&document, mesh.index()).unwrap_or(Mat4::IDENTITY);
    // Normals transform by the inverse-transpose of the model matrix.
    let normal_xform = node_xform.inverse().transpose();

    let mut primitives = Vec::new();

    for primitive in mesh.primitives() {
        let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

        let positions: Vec<[f32; 3]> = reader
            .read_positions()
            .context("primitive has no POSITION attribute")?
            .collect();

        let normals: Vec<[f32; 3]> = if let Some(n) = reader.read_normals() {
            n.collect()
        } else {
            vec![[0.0, 1.0, 0.0]; positions.len()]
        };

        anyhow::ensure!(
            normals.len() == positions.len(),
            "NORMAL count does not match POSITION count"
        );

        let tex_coord_set = primitive
            .material()
            .pbr_metallic_roughness()
            .base_color_texture()
            .map(|t| t.tex_coord())
            .unwrap_or(0);

        let mut uvs: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(tex_coord_set) {
            tc.into_f32().collect()
        } else {
            vec![[0.0, 0.0]; positions.len()]
        };

        anyhow::ensure!(
            uvs.len() == positions.len(),
            "TEXCOORD count does not match POSITION count"
        );

        // Apply KHR_texture_transform if present on the base color texture.
        if let Some(tex_info) = primitive.material().pbr_metallic_roughness().base_color_texture() {
            if let Some(xform) = tex_info.texture_transform() {
                let [ox, oy] = xform.offset();
                let [sx, sy] = xform.scale();
                let r = xform.rotation();
                let (sin_r, cos_r) = r.sin_cos();
                for uv in &mut uvs {
                    let u = uv[0];
                    let v = uv[1];
                    uv[0] = u * sx * cos_r - v * sy * sin_r + ox;
                    uv[1] = u * sx * sin_r + v * sy * cos_r + oy;
                }
            }
        }

        let vertices: Vec<Vertex3dTex> = positions
            .into_iter()
            .zip(normals)
            .zip(uvs)
            .map(|((position, normal), uv)| {
                let p = node_xform.transform_point3(glam::Vec3::from(position));
                let n = normal_xform
                    .transform_vector3(glam::Vec3::from(normal))
                    .normalize_or_zero();
                Vertex3dTex {
                    position: p.into(),
                    normal: n.into(),
                    uv,
                }
            })
            .collect();

        let indices: Vec<u32> = if let Some(ids) = reader.read_indices() {
            ids.into_u32().collect()
        } else {
            (0..vertices.len() as u32).collect()
        };

        let pbr = primitive.material().pbr_metallic_roughness();
        let base_color_factor = pbr.base_color_factor();

        let albedo_rgba = pbr.base_color_texture().and_then(|tex_info| {
            let img_index = tex_info.texture().source().index();
            images.get(img_index).and_then(gltf_image_to_rgba8)
        });

        if albedo_rgba.is_none() && pbr.base_color_texture().is_some() {
            log::warn!(
                "primitive {}: base color texture present but image could not be decoded",
                primitive.index()
            );
        }

        primitives.push(LoadedPrimitive {
            vertices,
            indices,
            base_color_factor,
            albedo_rgba,
        });
    }

    anyhow::ensure!(!primitives.is_empty(), "mesh has no primitives");
    Ok(LoadedTile { primitives })
}
