//! Offline baked relic payloads (`RLC1`) — mask-cut albedo, packed relief, extruded mesh.
//!
//! Produced by `mahjuro-bake-relics` and loaded at runtime instead of decoding PNGs.

use anyhow::Context;
use mahjuro_core::core::relic::{RelicId, all_relic_defs};

use crate::gpu_types::DecodedRelicImage;
use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::tile_glb::Vertex3dTex;

pub const MAGIC: &[u8; 4] = b"RLC1";
pub const VERSION: u32 = 1;
pub const SLUG_BYTES: usize = 64;

const FLAG_HAS_MESH: u32 = 1;

/// `assets/data/relic_baked/<slug>.rlc` where `<slug>` is the relic PNG stem.
pub fn baked_relic_asset_path(id: RelicId) -> String {
    format!("data/relic_baked/{}.rlc", relic_slug(id))
}

pub fn relic_slug(id: RelicId) -> &'static str {
    static SLUGS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    SLUGS.get_or_init(|| {
        all_relic_defs()
            .iter()
            .map(|d| {
                let stem = d.id.asset_filename().trim_end_matches(".png");
                Box::leak(stem.to_string().into_boxed_str()) as &str
            })
            .collect()
    });
    let idx = all_relic_defs()
        .iter()
        .position(|d| d.id == id)
        .expect("relic slug table out of sync with all_relic_defs");
    SLUGS.get().expect("SLUGS init")[idx]
}

pub fn relic_id_from_slug(slug: &str) -> Option<RelicId> {
    all_relic_defs()
        .iter()
        .find(|d| relic_slug(d.id) == slug)
        .map(|d| d.id)
}

pub fn baked_relic_available(id: RelicId) -> bool {
    mahjuro_assets::asset_path::get(&baked_relic_asset_path(id)).is_some()
}

pub fn all_relic_bakes_available() -> bool {
    all_relic_defs().iter().all(|d| baked_relic_available(d.id))
}

/// Encode one baked relic (same bytes the runtime loader consumes).
pub fn encode_baked_relic(msg: &DecodedRelicImage) -> anyhow::Result<Vec<u8>> {
    let slug = relic_slug(msg.id);
    anyhow::ensure!(
        slug.len() < SLUG_BYTES,
        "relic slug too long for RLC1 header: {slug}"
    );
    let mut slug_buf = [0u8; SLUG_BYTES];
    slug_buf[..slug.len()].copy_from_slice(slug.as_bytes());

    let mesh = msg.mesh_cpu.as_ref();
    let flags = if mesh.is_some() { FLAG_HAS_MESH } else { 0 };
    let (vertex_count, index_count, material) = mesh
        .map(|m| {
            (
                m.vertices.len() as u32,
                m.indices.len() as u32,
                m.default_material,
            )
        })
        .unwrap_or((
            0,
            0,
            MaterialParams {
                kind: MaterialKind::Plain,
                base_color: [1.0; 4],
                specular_strength: 0.25,
                specular_power: 32.0,
            },
        ));

    let header_size = std::mem::size_of::<RelicBakeHeaderV1>();
    let albedo_len = msg.rgba.len();
    let relief_len = msg.relief_rgba.len();
    let vert_bytes = mesh.map(|m| m.vertices.len() * std::mem::size_of::<Vertex3dTex>());
    let idx_bytes = mesh.map(|m| m.indices.len() * std::mem::size_of::<u32>());
    let mut out = Vec::with_capacity(
        header_size + albedo_len + relief_len + vert_bytes.unwrap_or(0) + idx_bytes.unwrap_or(0),
    );

    let header = RelicBakeHeaderV1 {
        magic: *MAGIC,
        version: VERSION,
        slug: slug_buf,
        albedo_w: msg.width,
        albedo_h: msg.height,
        relief_w: msg.relief_width,
        relief_h: msg.relief_height,
        albedo_len: albedo_len as u32,
        relief_len: relief_len as u32,
        vertex_count,
        index_count,
        material_kind: material_kind_to_u32(material.kind),
        base_color: material.base_color,
        specular_strength: material.specular_strength,
        specular_power: material.specular_power,
        flags,
    };
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(&msg.rgba);
    out.extend_from_slice(&msg.relief_rgba);
    if let Some(m) = mesh {
        out.extend_from_slice(bytemuck::cast_slice(&m.vertices));
        out.extend_from_slice(bytemuck::cast_slice(&m.indices));
    }
    Ok(out)
}

/// Decode a baked relic blob into the same structure the PNG loader produced.
pub fn decode_baked_relic(bytes: &[u8]) -> anyhow::Result<DecodedRelicImage> {
    let header_size = std::mem::size_of::<RelicBakeHeaderV1>();
    anyhow::ensure!(bytes.len() >= header_size, "relic bake: file too small");
    let header: &RelicBakeHeaderV1 = bytemuck::try_from_bytes(&bytes[..header_size])
        .map_err(|e| anyhow::anyhow!("relic bake header: {e}"))?;
    anyhow::ensure!(header.magic == *MAGIC, "relic bake: bad magic");
    anyhow::ensure!(
        header.version == VERSION,
        "relic bake: unsupported version {}",
        header.version
    );

    let slug = read_slug(&header.slug)?;
    let id = relic_id_from_slug(slug)
        .with_context(|| format!("relic bake: unknown slug {slug:?}"))?;
    let name = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("relic");

    let albedo_off = header_size;
    let albedo_end = albedo_off + header.albedo_len as usize;
    let relief_end = albedo_end + header.relief_len as usize;
    anyhow::ensure!(
        bytes.len() >= relief_end,
        "relic bake: truncated albedo/relief"
    );

    let rgba = bytes[albedo_off..albedo_end].to_vec();
    let relief_rgba = bytes[albedo_end..relief_end].to_vec();

    let mesh_cpu = if header.flags & FLAG_HAS_MESH != 0 {
        let vert_size = header.vertex_count as usize * std::mem::size_of::<Vertex3dTex>();
        let idx_size = header.index_count as usize * std::mem::size_of::<u32>();
        let vert_off = relief_end;
        let vert_end = vert_off + vert_size;
        let idx_end = vert_end + idx_size;
        anyhow::ensure!(bytes.len() >= idx_end, "relic bake: truncated mesh");
        let vertices: Vec<Vertex3dTex> = bytemuck::cast_slice(&bytes[vert_off..vert_end]).to_vec();
        let indices: Vec<u32> = bytemuck::cast_slice(&bytes[vert_end..idx_end]).to_vec();
        Some(MeshCpu {
            vertices,
            indices,
            default_material: MaterialParams {
                kind: material_kind_from_u32(header.material_kind),
                base_color: header.base_color,
                specular_strength: header.specular_strength,
                specular_power: header.specular_power,
            },
        })
    } else {
        None
    };

    Ok(DecodedRelicImage {
        id,
        name,
        rgba,
        width: header.albedo_w,
        height: header.albedo_h,
        relief_rgba,
        relief_width: header.relief_w,
        relief_height: header.relief_h,
        mesh_cpu,
    })
}

pub fn load_baked_relic(id: RelicId) -> anyhow::Result<DecodedRelicImage> {
    let path = baked_relic_asset_path(id);
    let file = mahjuro_assets::asset_path::get(&path)
        .with_context(|| format!("missing baked relic at {path}"))?;
    decode_baked_relic(&file.data)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RelicBakeHeaderV1 {
    magic: [u8; 4],
    version: u32,
    slug: [u8; SLUG_BYTES],
    albedo_w: u32,
    albedo_h: u32,
    relief_w: u32,
    relief_h: u32,
    albedo_len: u32,
    relief_len: u32,
    vertex_count: u32,
    index_count: u32,
    material_kind: u32,
    base_color: [f32; 4],
    specular_strength: f32,
    specular_power: f32,
    flags: u32,
}

fn read_slug(buf: &[u8; SLUG_BYTES]) -> anyhow::Result<&str> {
    let nul = buf.iter().position(|&b| b == 0).unwrap_or(SLUG_BYTES);
    let slug = std::str::from_utf8(&buf[..nul]).context("relic bake: invalid slug utf-8")?;
    anyhow::ensure!(!slug.is_empty(), "relic bake: empty slug");
    Ok(slug)
}

fn material_kind_to_u32(kind: MaterialKind) -> u32 {
    kind as u32
}

fn material_kind_from_u32(v: u32) -> MaterialKind {
    if v <= MaterialKind::Unshaded as u32 {
        // SAFETY: `MaterialKind` is `#[repr(u32)]` with discriminants 0..=22.
        unsafe { std::mem::transmute::<u32, MaterialKind>(v) }
    } else {
        MaterialKind::Plain
    }
}
