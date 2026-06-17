//! Pre-rasterized atlas for showcase tile face decals — avoids CPU raster + texture
//! upload when tile identities churn during sorts.
//!
//! Built-in tilesets ship pre-baked at
//! `textures/tile_sets/<tileset>/showcase_decal_atlas.png`. Player mod tilesets
//! (`mod:<name>`) runtime-bake on first use and cache under the user config dir.

use rustc_hash::FxHashMap;

use crate::decal::rasterize_tile_face_decal;
use mahjuro_core::core::tile::{Suit, Tile, TileEnhancement};

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
    build_showcase_decal_lookup_for_texture(atlas_w, atlas_h)
}

fn build_showcase_decal_lookup_for_texture(
    texture_w: u32,
    texture_h: u32,
) -> FxHashMap<ShowcaseDecalKey, [f32; 4]> {
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
                lookup.insert(key, cell_uv_rect(col, row, texture_w, texture_h));
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
    pub _texture: wgpu::Texture,
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
        _texture: texture,
        view,
        lookup: build_showcase_decal_lookup(),
    }
}

pub fn upload_showcase_decal_atlas_baked(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    payload: &crate::baked_texture::BakedTexturePayload,
) -> ShowcaseDecalAtlasGpu {
    let (texture, view, _bytes) = crate::baked_texture::upload_payload(
        device,
        queue,
        label,
        payload,
        crate::baked_texture::bc7_supported(device),
    );
    ShowcaseDecalAtlasGpu {
        _texture: texture,
        view,
        lookup: build_showcase_decal_lookup_for_texture(payload.base_width, payload.base_height),
    }
}

/// Returns true when a pre-baked showcase decal atlas is available for `tileset`.
pub fn baked_showcase_decal_atlas_available(tileset: &str) -> bool {
    if mahjuro_assets::tileset_mod::is_player_tileset(tileset) {
        return mahjuro_assets::tileset_mod::mod_showcase_cache_exists(tileset);
    }
    let path = crate::baked_texture::baked_texture_asset_path(&baked_atlas_asset_path(tileset));
    mahjuro_assets::asset_path::get(&path).is_some()
}

fn decode_showcase_decal_png(png_bytes: &[u8], path_label: &str) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(png_bytes)
        .map_err(|e| anyhow::anyhow!("showcase decal atlas {path_label}: decode failed: {e}"))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    let (ew, eh) = atlas_dimensions();
    if w != ew || h != eh {
        anyhow::bail!(
            "showcase decal atlas {path_label}: expected {ew}x{eh}, got {w}x{h}; re-bake with \
             mahjuro-bake-decal-atlases"
        );
    }
    Ok(img.into_raw())
}

fn bake_mod_showcase_decal_rgba(tileset: &str) -> anyhow::Result<Vec<u8>> {
    let ui_font = crate::decal::load_ui_font().cloned();
    let emoji_font = crate::decal::load_noto_emoji_font();
    let rgba =
        rasterize_showcase_decal_atlas_rgba(ui_font.as_ref(), emoji_font.as_ref(), Some(tileset));
    let (w, h) = atlas_dimensions();
    let img = image::RgbaImage::from_raw(w, h, rgba.clone())
        .ok_or_else(|| anyhow::anyhow!("showcase decal bake buffer size mismatch for {tileset}"))?;
    if let Some(path) = mahjuro_assets::tileset_mod::mod_showcase_cache_path(tileset) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = img.save(&path) {
            log::warn!("failed to cache showcase decal atlas for {tileset}: {e}");
        }
    }
    Ok(rgba)
}

/// Load a baked showcase atlas, runtime-baking player mods when needed.
pub fn load_or_bake_showcase_decal_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tileset: &str,
) -> anyhow::Result<ShowcaseDecalAtlasGpu> {
    let rgba = if mahjuro_assets::tileset_mod::is_player_tileset(tileset) {
        if let Some(bytes) = mahjuro_assets::tileset_mod::read_mod_showcase_cache(tileset) {
            decode_showcase_decal_png(&bytes, &format!("player tileset cache for {tileset}"))?
        } else {
            log::info!("runtime-baking showcase decal atlas for player tileset {tileset}");
            bake_mod_showcase_decal_rgba(tileset)?
        }
    } else {
        let path = baked_atlas_asset_path(tileset);
        let payload = crate::baked_texture::load_baked_texture(&path)?;
        return Ok(upload_showcase_decal_atlas_baked(
            device,
            queue,
            &format!("showcase-decal-atlas-{tileset}"),
            &payload,
        ));
    };
    Ok(upload_showcase_decal_atlas_rgba(
        device,
        queue,
        &format!("showcase-decal-atlas-{tileset}"),
        &rgba,
    ))
}

/// Decode + GPU upload from a baked atlas PNG.
pub fn load_showcase_decal_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tileset: &str,
) -> anyhow::Result<ShowcaseDecalAtlasGpu> {
    load_or_bake_showcase_decal_atlas(device, queue, tileset)
}
