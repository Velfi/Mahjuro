//! Generic baked texture payloads: BC7 mip chains for shipped sampled art.
//!
//! Relic RLC2 bakes use the same payload shape inside their relic-specific
//! container. Static art bakes use the standalone BTX1 container below.

use anyhow::Context;

pub const MAGIC: &[u8; 4] = b"BTX1";
pub const VERSION: u32 = 6;

const FLAG_SRGB: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BakedTextureColor {
    Srgb,
    Linear,
    NormalLinear,
}

impl BakedTextureColor {
    #[inline]
    pub fn is_srgb(self) -> bool {
        matches!(self, Self::Srgb)
    }

    #[inline]
    pub fn is_normal(self) -> bool {
        matches!(self, Self::NormalLinear)
    }
}

#[derive(Clone, Debug)]
pub struct BakedTexturePayload {
    pub base_width: u32,
    pub base_height: u32,
    pub mip_count: u32,
    pub bc7_bytes: Vec<u8>,
    pub srgb: bool,
}

impl BakedTexturePayload {
    #[inline]
    pub fn color(&self) -> BakedTextureColor {
        if self.srgb {
            BakedTextureColor::Srgb
        } else {
            BakedTextureColor::Linear
        }
    }
}

pub fn baked_texture_asset_path(source_asset_path: &str) -> String {
    let mut path = String::from("data/texture_baked/");
    for ch in source_asset_path.chars() {
        match ch {
            '/' | '\\' => path.push('/'),
            ':' => path.push('_'),
            _ => path.push(ch),
        }
    }
    path.push_str(".btx");
    path
}

pub fn gltf_slot_source_path(asset_label: &str, primitive_ordinal: usize, slot: &str) -> String {
    format!("3d_gltf/{asset_label}/prim_{primitive_ordinal:04}_{slot}.png")
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
#[cfg(feature = "texture_bc7_bake")]
fn pad_rgba_to_bc7_blocks(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    use crate::relic_gpu_residency::{align_bc7_base_dim, bc7_upload_chain_valid};

    let aligned_w = align_bc7_base_dim(width);
    let aligned_h = align_bc7_base_dim(height);
    if aligned_w == width
        && aligned_h == height
        && bc7_upload_chain_valid(width, height, mip_chain_count(width, height))
    {
        return (rgba.to_vec(), width, height);
    }
    let mut out = vec![0u8; (aligned_w as usize) * (aligned_h as usize) * 4];
    for y in 0..height.min(aligned_h) {
        let src = (y * width * 4) as usize;
        let dst = (y * aligned_w * 4) as usize;
        let row = (width * 4) as usize;
        out[dst..dst + row].copy_from_slice(&rgba[src..src + row]);
    }
    (out, aligned_w, aligned_h)
}

#[cfg(feature = "texture_bc7_bake")]
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
        let img = image::RgbaImage::from_raw(w, h, level).expect("baked texture mip rgba");
        w = (w / 2).max(4);
        h = (h / 2).max(4);
        level =
            image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle).into_raw();
    }
    chain
}

#[cfg(feature = "texture_bc7_bake")]
fn decode_normal_u8(px: &[u8]) -> [f32; 3] {
    [
        (px[0] as f32 / 255.0) * 2.0 - 1.0,
        (px[1] as f32 / 255.0) * 2.0 - 1.0,
        (px[2] as f32 / 255.0) * 2.0 - 1.0,
    ]
}

#[cfg(feature = "texture_bc7_bake")]
fn encode_normal_u8(n: [f32; 3]) -> [u8; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    [
        (((n[0] / len) * 0.5 + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (((n[1] / len) * 0.5 + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (((n[2] / len) * 0.5 + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(feature = "texture_bc7_bake")]
fn downsample_normal_rgba8(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    for dy in 0..dh {
        for dx in 0..dw {
            let x0 = dx * 2;
            let y0 = dy * 2;
            let mut n = [0.0f32; 3];
            let mut alpha = 0.0f32;
            let mut count = 0.0f32;
            for oy in 0..2 {
                for ox in 0..2 {
                    let sx = (x0 + ox).min(sw - 1);
                    let sy = (y0 + oy).min(sh - 1);
                    let i = ((sy * sw + sx) * 4) as usize;
                    let p = decode_normal_u8(&src[i..i + 4]);
                    n[0] += p[0];
                    n[1] += p[1];
                    n[2] += p[2];
                    alpha += src[i + 3] as f32;
                    count += 1.0;
                }
            }
            let rgb = encode_normal_u8(n);
            let o = ((dy * dw + dx) * 4) as usize;
            out[o] = rgb[0];
            out[o + 1] = rgb[1];
            out[o + 2] = rgb[2];
            out[o + 3] = (alpha / count).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(feature = "texture_bc7_bake")]
fn normal_mip_chain_bc7(rgba: &[u8], width: u32, height: u32) -> Vec<(Vec<u8>, u32, u32)> {
    let mut chain = Vec::new();
    let mut w = width.max(1);
    let mut h = height.max(1);
    let mut level = rgba.to_vec();
    loop {
        chain.push((level.clone(), w, h));
        if w <= 4 && h <= 4 {
            break;
        }
        let nw = (w / 2).max(4);
        let nh = (h / 2).max(4);
        level = downsample_normal_rgba8(&level, w, h, nw, nh);
        w = nw;
        h = nh;
    }
    chain
}

#[cfg(feature = "texture_bc7_bake")]
pub fn encode_rgba_bc7_mip_chain(
    rgba: &[u8],
    width: u32,
    height: u32,
    color: BakedTextureColor,
) -> anyhow::Result<BakedTexturePayload> {
    use intel_tex_2::{RgbaSurface, bc7};

    anyhow::ensure!(
        rgba.len() == (width as usize) * (height as usize) * 4,
        "baked texture RGBA length mismatch for {width}x{height}"
    );

    let (bc7_rgba, bc7_w, bc7_h) = pad_rgba_to_bc7_blocks(rgba, width, height);
    let chain = if color.is_normal() {
        normal_mip_chain_bc7(&bc7_rgba, bc7_w, bc7_h)
    } else {
        rgba_mip_chain_bc7(&bc7_rgba, bc7_w, bc7_h)
    };
    let mip_count = chain.len() as u32;
    let settings = if color.is_srgb() {
        bc7::opaque_fast_settings()
    } else if color.is_normal() {
        bc7::opaque_basic_settings()
    } else {
        bc7::alpha_fast_settings()
    };

    let mut bc7_out = Vec::new();
    for (level_rgba, lw, lh) in &chain {
        let surface = RgbaSurface {
            data: level_rgba,
            width: *lw,
            height: *lh,
            stride: lw * 4,
        };
        bc7_out.extend_from_slice(&bc7::compress_blocks(&settings, &surface));
    }

    Ok(BakedTexturePayload {
        base_width: bc7_w,
        base_height: bc7_h,
        mip_count,
        bc7_bytes: bc7_out,
        srgb: color.is_srgb(),
    })
}

#[cfg(not(feature = "texture_bc7_bake"))]
pub fn encode_rgba_bc7_mip_chain(
    rgba: &[u8],
    width: u32,
    height: u32,
    color: BakedTextureColor,
) -> anyhow::Result<BakedTexturePayload> {
    let _ = (rgba, width, height, color);
    anyhow::bail!("BTX1 texture bake requires the texture_bc7_bake feature")
}

pub fn encode_btx(payload: &BakedTexturePayload) -> anyhow::Result<Vec<u8>> {
    let header_size = std::mem::size_of::<BtxHeader>();
    let mut out = Vec::with_capacity(header_size + payload.bc7_bytes.len());
    let header = BtxHeader {
        magic: *MAGIC,
        version: VERSION,
        flags: if payload.srgb { FLAG_SRGB } else { 0 },
        base_w: payload.base_width,
        base_h: payload.base_height,
        mip_count: payload.mip_count,
        bc7_len: payload.bc7_bytes.len() as u32,
    };
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(&payload.bc7_bytes);
    Ok(out)
}

pub fn decode_btx(bytes: &[u8]) -> anyhow::Result<BakedTexturePayload> {
    let header_size = std::mem::size_of::<BtxHeader>();
    anyhow::ensure!(bytes.len() >= header_size, "BTX1 texture: file too small");
    let header: &BtxHeader = bytemuck::try_from_bytes(&bytes[..header_size])
        .map_err(|e| anyhow::anyhow!("BTX1 texture header: {e}"))?;
    anyhow::ensure!(header.magic == *MAGIC, "BTX1 texture: bad magic");
    anyhow::ensure!(
        header.version == VERSION,
        "BTX1 texture: unsupported version {}",
        header.version
    );

    let bc7_end = header_size
        .checked_add(header.bc7_len as usize)
        .context("BTX1 texture: BC7 length overflow")?;
    anyhow::ensure!(bytes.len() >= bc7_end, "BTX1 texture: truncated payload");
    anyhow::ensure!(
        crate::relic_gpu_residency::bc7_block_aligned(header.base_w, header.base_h),
        "BTX1 texture: BC7 size {}x{} is not 4-aligned",
        header.base_w,
        header.base_h
    );
    anyhow::ensure!(
        !bytes[header_size..bc7_end].is_empty(),
        "BTX1 texture: missing BC7 payload"
    );

    Ok(BakedTexturePayload {
        base_width: header.base_w,
        base_height: header.base_h,
        mip_count: header.mip_count,
        bc7_bytes: bytes[header_size..bc7_end].to_vec(),
        srgb: header.flags & FLAG_SRGB != 0,
    })
}

pub fn load_baked_texture(source_asset_path: &str) -> anyhow::Result<BakedTexturePayload> {
    let path = baked_texture_asset_path(source_asset_path);
    let data = mahjuro_assets::asset_path::get_shared(&path)
        .with_context(|| format!("missing baked texture at {path}"))?;
    decode_btx(data.as_ref())
}

pub fn load_rgba_for_cpu(source_asset_path: &str) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let file = mahjuro_assets::asset_path::get(source_asset_path)
        .ok_or_else(|| anyhow::anyhow!("asset missing at {source_asset_path}"))?;
    let img = image::load_from_memory(&file.data)
        .with_context(|| format!("decode PNG fallback for {source_asset_path}"))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

pub fn bc7_supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
}

pub fn upload_payload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    payload: &BakedTexturePayload,
    bc7_supported: bool,
) -> (wgpu::Texture, wgpu::TextureView, usize) {
    use crate::relic_gpu_residency::{
        bc7_mip_level_count, bc7_next_mip_dim, bc7_upload_chain_bytes, bc7_upload_chain_valid,
    };

    assert!(
        bc7_supported,
        "{label}: BTX1 texture requires TEXTURE_COMPRESSION_BC; use a compatibility build/pack for non-BC GPUs"
    );
    assert!(
        !payload.bc7_bytes.is_empty()
            && bc7_upload_chain_valid(payload.base_width, payload.base_height, payload.mip_count),
        "{label}: invalid BTX1 BC7 mip chain"
    );

    let upload_mip_count =
        bc7_mip_level_count(payload.base_width, payload.base_height).min(payload.mip_count.max(1));
    let format = if payload.srgb {
        wgpu::TextureFormat::Bc7RgbaUnormSrgb
    } else {
        wgpu::TextureFormat::Bc7RgbaUnorm
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: payload.base_width.max(1),
            height: payload.base_height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: upload_mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut off = 0usize;
    let mut w = payload.base_width.max(1);
    let mut h = payload.base_height.max(1);
    for mip in 0..payload.mip_count.max(1) {
        let level_bytes = crate::relic_gpu_residency::bc7_mip_bytes(w, h);
        if mip < upload_mip_count {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &payload.bc7_bytes[off..off + level_bytes],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w.div_ceil(4) * 16),
                    rows_per_image: Some(h.div_ceil(4)),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        off += level_bytes;
        w = bc7_next_mip_dim(w);
        h = bc7_next_mip_dim(h);
    }
    let bytes = bc7_upload_chain_bytes(payload.base_width, payload.base_height);
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view, bytes)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BtxHeader {
    magic: [u8; 4],
    version: u32,
    flags: u32,
    base_w: u32,
    base_h: u32,
    mip_count: u32,
    bc7_len: u32,
}

#[cfg(all(test, feature = "texture_bc7_bake"))]
mod tests {
    use super::*;

    fn normal_len(px: &[u8]) -> f32 {
        let n = decode_normal_u8(px);
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
    }

    #[test]
    fn normal_mips_are_renormalized() {
        let n_up = [128, 128, 255, 255];
        let n_side = [255, 128, 128, 255];
        let rgba = [
            n_up, n_side, n_side, n_up, n_side, n_up, n_up, n_side, n_up, n_side, n_side, n_up,
            n_side, n_up, n_up, n_side,
        ]
        .concat();

        let chain = normal_mip_chain_bc7(&rgba, 4, 4);
        assert_eq!(chain.len(), 1, "4x4 is the smallest uploaded BC7 mip");

        let down = downsample_normal_rgba8(&rgba, 4, 4, 2, 2);
        for px in down.chunks_exact(4) {
            assert!(
                (normal_len(px) - 1.0).abs() < 0.01,
                "normal mip texel should stay unit length, got {:?}",
                &px[0..3]
            );
        }
    }

    #[test]
    fn normal_payloads_use_linear_bc7_header() {
        let rgba = [[128, 128, 255, 255]; 16].concat();
        let payload =
            encode_rgba_bc7_mip_chain(&rgba, 4, 4, BakedTextureColor::NormalLinear).unwrap();
        let encoded = encode_btx(&payload).unwrap();
        let decoded = decode_btx(&encoded).unwrap();
        assert!(!decoded.srgb);
        assert_eq!(decoded.base_width, 4);
        assert_eq!(decoded.base_height, 4);
        assert_eq!(decoded.mip_count, 1);
    }
}
