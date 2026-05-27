//! Pre-rasterized atlas for showcase tile face decals — avoids CPU raster + texture
//! upload when tile identities churn during sorts.
//!
//! Offline PNGs live at `textures/tile_sets/<tileset>/showcase_decal_atlas.png`
//! (baked by `mahjuro-bake-decal-atlases` during `cargo build`). Runtime loads
//! those files only — there is no CPU raster fallback.

use rustc_hash::FxHashMap;

use mahjuro_core::core::tile::{Suit, Tile, TileEnhancement};
use crate::decal::rasterize_tile_face_decal;

pub type ShowcaseDecalKey = (Suit, u8, Option<TileEnhancement>, bool);

pub const DECAL_W: u32 = 192;
pub const DECAL_H: u32 = 256;
const ATLAS_COLS: u32 = 21;
/// Base faces: 42 tile types × 4 enhancements × 2 debuff states = 336 cells (21×16).
const ATLAS_ROWS: u32 = 16;

pub fn atlas_dimensions() -> (u32, u32) {
    (ATLAS_COLS * DECAL_W, ATLAS_ROWS * DECAL_H)
}

/// Asset path relative to the assets root (or pack prefix).
pub fn baked_atlas_asset_path(tileset: &str) -> String {
    format!("textures/tile_sets/{tileset}/showcase_decal_atlas.png")
}

fn all_base_faces() -> Vec<(Suit, u8)> {
    let mut v = Vec::with_capacity(42);
    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
        for rank in 1..=9 {
            v.push((suit, rank));
        }
    }
    for rank in 1..=4 {
        v.push((Suit::Wind, rank));
    }
    for rank in 1..=3 {
        v.push((Suit::Dragon, rank));
    }
    for rank in 1..=4 {
        v.push((Suit::Flower, rank));
    }
    for rank in 1..=4 {
        v.push((Suit::Season, rank));
    }
    debug_assert_eq!(v.len(), 42);
    v
}

fn enhancements() -> [Option<TileEnhancement>; 4] {
    [
        None,
        Some(TileEnhancement::Pearl),
        Some(TileEnhancement::Gilded),
        Some(TileEnhancement::Polychrome),
    ]
}

/// Normalized atlas UV rect: xy = origin, zw = size (each axis 0..1).
fn cell_uv_rect(col: u32, row: u32, atlas_w: u32, atlas_h: u32) -> [f32; 4] {
    let ox = (col * DECAL_W) as f32 / atlas_w as f32;
    let oy = (row * DECAL_H) as f32 / atlas_h as f32;
    let sx = DECAL_W as f32 / atlas_w as f32;
    let sy = DECAL_H as f32 / atlas_h as f32;
    [ox, oy, sx, sy]
}

/// Deterministic UV lookup for a baked atlas grid (no raster required).
pub fn build_showcase_decal_lookup() -> FxHashMap<ShowcaseDecalKey, [f32; 4]> {
    let (atlas_w, atlas_h) = atlas_dimensions();
    let mut lookup = FxHashMap::default();
    let bases = all_base_faces();
    let enh = enhancements();
    let mut idx: u32 = 0;
    for &(suit, rank) in &bases {
        for &e in &enh {
            for &debuffed in &[false, true] {
                let col = idx % ATLAS_COLS;
                let row = idx / ATLAS_COLS;
                debug_assert!(row < ATLAS_ROWS);
                let key: ShowcaseDecalKey = (suit, rank, e, debuffed);
                lookup.insert(key, cell_uv_rect(col, row, atlas_w, atlas_h));
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, 336);
    lookup
}

fn blit_cell(atlas: &mut [u8], atlas_w: u32, rgba: &[u8], col: u32, row: u32) {
    let ox = (col * DECAL_W) as usize;
    let oy = (row * DECAL_H) as usize;
    let astride = atlas_w as usize * 4;
    let row_bytes = DECAL_W as usize * 4;
    for dy in 0..DECAL_H as usize {
        let src = dy * row_bytes;
        let dst = (oy + dy) * astride + ox * 4;
        atlas[dst..dst + row_bytes].copy_from_slice(&rgba[src..src + row_bytes]);
    }
}

/// CPU raster of the full showcase decal atlas (336 cells). Used by the offline baker only.
pub fn rasterize_showcase_decal_atlas_rgba(
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile_set: Option<&str>,
) -> Vec<u8> {
    let (atlas_w, atlas_h) = atlas_dimensions();
    let mut atlas_rgba = vec![0u8; (atlas_w * atlas_h * 4) as usize];
    let bases = all_base_faces();
    let enh = enhancements();
    let mut idx: u32 = 0;
    for &(suit, rank) in &bases {
        for &e in &enh {
            for &debuffed in &[false, true] {
                let col = idx % ATLAS_COLS;
                let row = idx / ATLAS_COLS;
                debug_assert!(row < ATLAS_ROWS);
                let tile = Tile {
                    suit,
                    rank,
                    id: 0,
                    enhancement: e,
                    debuffed_visual: debuffed,
                };
                let rgba = rasterize_tile_face_decal(
                    &tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, false,
                );
                blit_cell(&mut atlas_rgba, atlas_w, &rgba, col, row);
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, 336);
    atlas_rgba
}

/// Build one RGBA atlas texture + lookup from `(suit, rank, enhancement, debuffed)` → UV rect.
pub struct ShowcaseDecalAtlasGpu {
    /// Kept so the atlas stays allocated while bind groups reference `view`.
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub lookup: FxHashMap<ShowcaseDecalKey, [f32; 4]>,
}

pub fn upload_showcase_decal_atlas_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
) -> ShowcaseDecalAtlasGpu {
    let (atlas_w, atlas_h) = atlas_dimensions();
    let expected = (atlas_w * atlas_h * 4) as usize;
    debug_assert_eq!(rgba.len(), expected);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: atlas_w,
            height: atlas_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas_w * 4),
            rows_per_image: Some(atlas_h),
        },
        wgpu::Extent3d {
            width: atlas_w,
            height: atlas_h,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    ShowcaseDecalAtlasGpu {
        texture,
        view,
        lookup: build_showcase_decal_lookup(),
    }
}

/// Returns true when the baked PNG for `tileset` is present in mounted assets.
pub fn baked_showcase_decal_atlas_available(tileset: &str) -> bool {
    mahjuro_assets::asset_path::get(&baked_atlas_asset_path(tileset)).is_some()
}

/// Decode + GPU upload from a baked atlas PNG.
pub fn load_showcase_decal_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tileset: &str,
) -> anyhow::Result<ShowcaseDecalAtlasGpu> {
    let path = baked_atlas_asset_path(tileset);
    let file = mahjuro_assets::asset_path::get(&path).ok_or_else(|| {
        anyhow::anyhow!(
            "missing baked showcase decal atlas at {path}; run `cargo build` (needs \
             mahjuro-bake-decal-atlases in target/<profile>/) or \
             `cargo run -p mahjuro-render --bin mahjuro-bake-decal-atlases`"
        )
    })?;
    let img = image::load_from_memory(&file.data)
        .map_err(|e| anyhow::anyhow!("showcase decal atlas {path}: decode failed: {e}"))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    let (ew, eh) = atlas_dimensions();
    if w != ew || h != eh {
        anyhow::bail!(
            "showcase decal atlas {path}: expected {ew}x{eh}, got {w}x{h}; re-bake with \
             mahjuro-bake-decal-atlases"
        );
    }
    Ok(upload_showcase_decal_atlas_rgba(
        device,
        queue,
        &format!("showcase-decal-atlas-{tileset}"),
        img.as_raw(),
    ))
}
