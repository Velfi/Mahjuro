//! Offline baked relic payloads (`RLC2`) — mask-cut albedo, packed relief, extruded mesh.
//!
//! Produced by `mahjuro-bake-relics` and loaded at runtime instead of decoding PNGs.

use anyhow::Context;
use mahjuro_core::core::relic::{RelicId, all_relic_defs};

use crate::gpu_types::{DecodedRelicImage, RelicBc7MipChain};
use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::tile_glb::Vertex3dTex;

pub const MAGIC: &[u8; 4] = b"RLC2";
pub const VERSION: u32 = 2;
pub const SLUG_BYTES: usize = 64;

const FLAG_HAS_MESH: u32 = 1;

struct RelicSlugTables {
    by_id: rustc_hash::FxHashMap<RelicId, &'static str>,
    by_slug: rustc_hash::FxHashMap<&'static str, RelicId>,
}

fn slug_tables() -> &'static RelicSlugTables {
    static TABLES: std::sync::OnceLock<RelicSlugTables> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut by_id = rustc_hash::FxHashMap::default();
        let mut by_slug = rustc_hash::FxHashMap::default();
        for def in all_relic_defs() {
            let stem = def.id.asset_filename().trim_end_matches(".png");
            let slug = Box::leak(stem.to_string().into_boxed_str()) as &'static str;
            by_id.insert(def.id, slug);
            by_slug.insert(slug, def.id);
        }
        RelicSlugTables { by_id, by_slug }
    })
}

/// `assets/data/relic_baked/<slug>.rlc` where `<slug>` is the relic PNG stem.
pub fn baked_relic_asset_path(id: RelicId) -> String {
    format!("data/relic_baked/{}.rlc", relic_slug(id))
}

pub fn relic_slug(id: RelicId) -> &'static str {
    *slug_tables()
        .by_id
        .get(&id)
        .expect("relic slug table out of sync with all_relic_defs")
}

pub fn relic_id_from_slug(slug: &str) -> Option<RelicId> {
    slug_tables().by_slug.get(slug).copied()
}

pub fn baked_relic_available(id: RelicId) -> bool {
    mahjuro_assets::asset_path::get_shared(&baked_relic_asset_path(id)).is_some()
}

pub fn all_relic_bakes_available() -> bool {
    all_relic_defs().iter().all(|d| baked_relic_available(d.id))
}

pub fn mip_chain_count(mut w: u32, mut h: u32) -> u32 {
    let mut count = 0u32;
    while w > 0 && h > 0 {
        count += 1;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    count.max(1)
}

/// Pad RGBA to BC7 block dimensions (transparent pixels on the right/bottom).
fn pad_rgba_to_bc7_blocks(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    use crate::relic_gpu_residency::{align_bc7_dim, bc7_block_aligned};

    let aligned_w = align_bc7_dim(width.max(1));
    let aligned_h = align_bc7_dim(height.max(1));
    if bc7_block_aligned(width, height) {
        return (rgba.to_vec(), width, height);
    }
    let mut out = vec![0u8; (aligned_w as usize) * (aligned_h as usize) * 4];
    for y in 0..height {
        let src = (y * width * 4) as usize;
        let dst = (y * aligned_w * 4) as usize;
        let row = (width * 4) as usize;
        out[dst..dst + row].copy_from_slice(&rgba[src..src + row]);
    }
    (out, aligned_w, aligned_h)
}

#[cfg(feature = "relic_bc7_bake")]
fn rgba_mip_chain_bc7(rgba: &[u8], width: u32, height: u32) -> Vec<(Vec<u8>, u32, u32)> {
    let mut chain = Vec::new();
    let mut w = width.max(1);
    let mut h = height.max(1);
    let mut level = rgba.to_vec();
    loop {
        chain.push((level.clone(), w, h));
        if w <= 4 && h <= 4 {
            break;
        }
        let img = image::RgbaImage::from_raw(w, h, level).expect("relic mip rgba");
        w = (w / 2).max(4);
        h = (h / 2).max(4);
        level = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle).into_raw();
    }
    chain
}

#[cfg(feature = "relic_bc7_bake")]
fn encode_bc7_mip_chain(
    rgba: &[u8],
    width: u32,
    height: u32,
    srgb: bool,
) -> anyhow::Result<(Vec<u8>, u32, u32, u32, Vec<u8>, u32, u32)> {
    use intel_tex_2::{RgbaSurface, bc7};

    let (bc7_rgba, bc7_w, bc7_h) = pad_rgba_to_bc7_blocks(rgba, width, height);
    let chain = rgba_mip_chain_bc7(&bc7_rgba, bc7_w, bc7_h);
    let mip_count = chain.len() as u32;
    let (base_w, base_h) = (bc7_w, bc7_h);
    let settings = if srgb {
        bc7::opaque_fast_settings()
    } else {
        bc7::alpha_fast_settings()
    };

    let mut bc7_out = Vec::new();
    for (level_rgba, lw, lh) in &chain {
        let stride = lw * 4;
        let surface = RgbaSurface {
            data: level_rgba,
            width: *lw,
            height: *lh,
            stride,
        };
        let level_bytes = bc7::compress_blocks(&settings, &surface);
        bc7_out.extend_from_slice(&level_bytes);
    }

    Ok((
        bc7_out,
        base_w,
        base_h,
        mip_count,
        rgba.to_vec(),
        width,
        height,
    ))
}

#[cfg(not(feature = "relic_bc7_bake"))]
fn encode_bc7_mip_chain(
    rgba: &[u8],
    width: u32,
    height: u32,
    _srgb: bool,
) -> anyhow::Result<(Vec<u8>, u32, u32, u32, Vec<u8>, u32, u32)> {
    Ok((Vec::new(), width, height, 1, rgba.to_vec(), width, height))
}

/// Validate the structure of a baked relic blob without materializing pixel/mesh Vecs.
pub fn validate_baked_relic(id: RelicId) -> anyhow::Result<()> {
    let path = baked_relic_asset_path(id);
    let data = mahjuro_assets::asset_path::get_shared(&path)
        .with_context(|| format!("missing baked relic at {path}"))?;
    validate_baked_relic_bytes(id, data.as_ref())
}

fn validate_baked_relic_bytes(expected_id: RelicId, bytes: &[u8]) -> anyhow::Result<()> {
    let header_size = std::mem::size_of::<RelicBakeHeader>();
    anyhow::ensure!(bytes.len() >= header_size, "relic bake: file too small");
    let header: &RelicBakeHeader = bytemuck::try_from_bytes(&bytes[..header_size])
        .map_err(|e| anyhow::anyhow!("relic bake header: {e}"))?;
    anyhow::ensure!(header.magic == *MAGIC, "relic bake: bad magic (expected RLC2)");
    anyhow::ensure!(
        header.version == VERSION,
        "relic bake: unsupported version {}",
        header.version
    );
    let slug = read_slug(&header.slug)?;
    let slug_id =
        relic_id_from_slug(slug).with_context(|| format!("relic bake: unknown slug {slug:?}"))?;
    anyhow::ensure!(slug_id == expected_id, "relic bake: slug/id mismatch");
    let mut off = header_size;
    off = off
        .checked_add(header.albedo_bc7_len as usize)
        .context("relic bake: albedo bc7 overflow")?;
    off = off
        .checked_add(header.albedo_fallback_len as usize)
        .context("relic bake: albedo fallback overflow")?;
    off = off
        .checked_add(header.relief_bc7_len as usize)
        .context("relic bake: relief bc7 overflow")?;
    off = off
        .checked_add(header.relief_fallback_len as usize)
        .context("relic bake: relief fallback overflow")?;
    anyhow::ensure!(bytes.len() >= off, "relic bake: truncated textures");
    anyhow::ensure!(
        crate::relic_gpu_residency::bc7_block_aligned(header.albedo_base_w, header.albedo_base_h),
        "relic bake: albedo BC7 size {}x{} is not 4-aligned",
        header.albedo_base_w,
        header.albedo_base_h
    );
    anyhow::ensure!(
        crate::relic_gpu_residency::bc7_block_aligned(header.relief_base_w, header.relief_base_h),
        "relic bake: relief BC7 size {}x{} is not 4-aligned",
        header.relief_base_w,
        header.relief_base_h
    );
    if header.flags & FLAG_HAS_MESH != 0 {
        validate_mesh_tail(bytes, off, header.vertex_count, header.index_count)?;
    }
    Ok(())
}

fn validate_mesh_tail(
    bytes: &[u8],
    vert_off: usize,
    vertex_count: u32,
    index_count: u32,
) -> anyhow::Result<()> {
    let vert_size = (vertex_count as usize)
        .checked_mul(std::mem::size_of::<Vertex3dTex>())
        .context("relic bake: vertex length overflow")?;
    let idx_size = (index_count as usize)
        .checked_mul(std::mem::size_of::<u32>())
        .context("relic bake: index length overflow")?;
    let vert_end = vert_off
        .checked_add(vert_size)
        .context("relic bake: mesh vertex section overflow")?;
    let idx_end = vert_end
        .checked_add(idx_size)
        .context("relic bake: mesh index section overflow")?;
    anyhow::ensure!(bytes.len() >= idx_end, "relic bake: truncated mesh");
    Ok(())
}

/// Encode one baked relic (RLC2).
pub fn encode_baked_relic(msg: &DecodedRelicImage) -> anyhow::Result<Vec<u8>> {
    let slug = relic_slug(msg.id);
    anyhow::ensure!(
        slug.len() < SLUG_BYTES,
        "relic slug too long for RLC2 header: {slug}"
    );
    let mut slug_buf = [0u8; SLUG_BYTES];
    slug_buf[..slug.len()].copy_from_slice(slug.as_bytes());

    let (albedo_bc7, albedo_base_w, albedo_base_h, albedo_mips, albedo_fb, albedo_fb_w, albedo_fb_h) =
        encode_bc7_mip_chain(&msg.rgba, msg.width, msg.height, true)?;
    let (relief_bc7, relief_base_w, relief_base_h, relief_mips, relief_fb, relief_fb_w, relief_fb_h) =
        encode_bc7_mip_chain(
            &msg.relief_rgba,
            msg.relief_width,
            msg.relief_height,
            false,
        )?;

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

    let header_size = std::mem::size_of::<RelicBakeHeader>();
    let vert_bytes = mesh.map(|m| m.vertices.len() * std::mem::size_of::<Vertex3dTex>());
    let idx_bytes = mesh.map(|m| m.indices.len() * std::mem::size_of::<u32>());
    let mut out = Vec::with_capacity(
        header_size
            + albedo_bc7.len()
            + albedo_fb.len()
            + relief_bc7.len()
            + relief_fb.len()
            + vert_bytes.unwrap_or(0)
            + idx_bytes.unwrap_or(0),
    );

    let header = RelicBakeHeader {
        magic: *MAGIC,
        version: VERSION,
        slug: slug_buf,
        flags,
        albedo_base_w: albedo_base_w,
        albedo_base_h: albedo_base_h,
        albedo_mip_count: albedo_mips,
        albedo_bc7_len: albedo_bc7.len() as u32,
        albedo_fallback_w: albedo_fb_w,
        albedo_fallback_h: albedo_fb_h,
        albedo_fallback_len: albedo_fb.len() as u32,
        relief_base_w: relief_base_w,
        relief_base_h: relief_base_h,
        relief_mip_count: relief_mips,
        relief_bc7_len: relief_bc7.len() as u32,
        relief_fallback_w: relief_fb_w,
        relief_fallback_h: relief_fb_h,
        relief_fallback_len: relief_fb.len() as u32,
        vertex_count,
        index_count,
        material_kind: material_kind_to_u32(material.kind),
        base_color: material.base_color,
        specular_strength: material.specular_strength,
        specular_power: material.specular_power,
    };
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(&albedo_bc7);
    out.extend_from_slice(&albedo_fb);
    out.extend_from_slice(&relief_bc7);
    out.extend_from_slice(&relief_fb);
    if let Some(m) = mesh {
        out.extend_from_slice(bytemuck::cast_slice(&m.vertices));
        out.extend_from_slice(bytemuck::cast_slice(&m.indices));
    }
    Ok(out)
}

/// Decode an RLC2 baked relic blob.
pub fn decode_baked_relic(bytes: &[u8]) -> anyhow::Result<DecodedRelicImage> {
    let header_size = std::mem::size_of::<RelicBakeHeader>();
    anyhow::ensure!(bytes.len() >= header_size, "relic bake: file too small");
    let header: &RelicBakeHeader = bytemuck::try_from_bytes(&bytes[..header_size])
        .map_err(|e| anyhow::anyhow!("relic bake header: {e}"))?;
    anyhow::ensure!(header.magic == *MAGIC, "relic bake: bad magic (expected RLC2)");
    anyhow::ensure!(
        header.version == VERSION,
        "relic bake: unsupported version {}",
        header.version
    );

    let slug = read_slug(&header.slug)?;
    let id =
        relic_id_from_slug(slug).with_context(|| format!("relic bake: unknown slug {slug:?}"))?;
    let name = all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("relic");

    let albedo_bc7_end = header_size + header.albedo_bc7_len as usize;
    let albedo_fb_end = albedo_bc7_end + header.albedo_fallback_len as usize;
    let relief_bc7_end = albedo_fb_end + header.relief_bc7_len as usize;
    let relief_fb_end = relief_bc7_end + header.relief_fallback_len as usize;
    anyhow::ensure!(bytes.len() >= relief_fb_end, "relic bake: truncated textures");

    let albedo_bc7 = RelicBc7MipChain {
        base_width: header.albedo_base_w,
        base_height: header.albedo_base_h,
        mip_count: header.albedo_mip_count,
        bc7_bytes: bytes[header_size..albedo_bc7_end].to_vec(),
        fallback_rgba: bytes[albedo_bc7_end..albedo_fb_end].to_vec(),
        fallback_width: header.albedo_fallback_w,
        fallback_height: header.albedo_fallback_h,
        srgb: true,
    };
    let relief_bc7 = RelicBc7MipChain {
        base_width: header.relief_base_w,
        base_height: header.relief_base_h,
        mip_count: header.relief_mip_count,
        bc7_bytes: bytes[albedo_fb_end..relief_bc7_end].to_vec(),
        fallback_rgba: bytes[relief_bc7_end..relief_fb_end].to_vec(),
        fallback_width: header.relief_fallback_w,
        fallback_height: header.relief_fallback_h,
        srgb: false,
    };

    let mesh_cpu = if header.flags & FLAG_HAS_MESH != 0 {
        decode_mesh_from_header(
            bytes,
            relief_fb_end,
            header.vertex_count,
            header.index_count,
            header.material_kind,
            header.base_color,
            header.specular_strength,
            header.specular_power,
        )?
    } else {
        None
    };

    Ok(DecodedRelicImage {
        id,
        name,
        rgba: albedo_bc7.fallback_rgba.clone(),
        width: albedo_bc7.fallback_width,
        height: albedo_bc7.fallback_height,
        relief_rgba: relief_bc7.fallback_rgba.clone(),
        relief_width: relief_bc7.fallback_width,
        relief_height: relief_bc7.fallback_height,
        mesh_cpu,
        albedo_bc7: Some(albedo_bc7),
        relief_bc7: Some(relief_bc7),
    })
}

fn decode_mesh_from_header(
    bytes: &[u8],
    vert_off: usize,
    vertex_count: u32,
    index_count: u32,
    material_kind: u32,
    base_color: [f32; 4],
    specular_strength: f32,
    specular_power: f32,
) -> anyhow::Result<Option<MeshCpu>> {
    let vert_size = vertex_count as usize * std::mem::size_of::<Vertex3dTex>();
    let idx_size = index_count as usize * std::mem::size_of::<u32>();
    let vert_end = vert_off + vert_size;
    let idx_end = vert_end + idx_size;
    anyhow::ensure!(bytes.len() >= idx_end, "relic bake: truncated mesh");
    let vertices: Vec<Vertex3dTex> = bytemuck::cast_slice(&bytes[vert_off..vert_end]).to_vec();
    let indices: Vec<u32> = bytemuck::cast_slice(&bytes[vert_end..idx_end]).to_vec();
    Ok(Some(MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: material_kind_from_u32(material_kind),
            base_color,
            specular_strength,
            specular_power,
        },
    }))
}

pub fn load_baked_relic(id: RelicId) -> anyhow::Result<DecodedRelicImage> {
    let path = baked_relic_asset_path(id);
    let data = mahjuro_assets::asset_path::get_shared(&path)
        .with_context(|| format!("missing baked relic at {path}"))?;
    decode_baked_relic(data.as_ref())
}

/// Uncached read for one-shot batch / on-demand loads (avoids churning the asset byte LRU).
pub fn load_baked_relic_uncached(id: RelicId) -> anyhow::Result<DecodedRelicImage> {
    let path = baked_relic_asset_path(id);
    let data = mahjuro_assets::asset_path::load_asset_bytes(&path)
        .with_context(|| format!("missing baked relic at {path}"))?;
    decode_baked_relic(&data)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RelicBakeHeader {
    magic: [u8; 4],
    version: u32,
    slug: [u8; SLUG_BYTES],
    flags: u32,
    albedo_base_w: u32,
    albedo_base_h: u32,
    albedo_mip_count: u32,
    albedo_bc7_len: u32,
    albedo_fallback_w: u32,
    albedo_fallback_h: u32,
    albedo_fallback_len: u32,
    relief_base_w: u32,
    relief_base_h: u32,
    relief_mip_count: u32,
    relief_bc7_len: u32,
    relief_fallback_w: u32,
    relief_fallback_h: u32,
    relief_fallback_len: u32,
    vertex_count: u32,
    index_count: u32,
    material_kind: u32,
    base_color: [f32; 4],
    specular_strength: f32,
    specular_power: f32,
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
