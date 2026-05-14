//! Pre-rasterized atlas for showcase tile face decals — avoids CPU raster + texture
//! upload when tile identities churn during sorts.

use rustc_hash::FxHashMap;

use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::render::decal::rasterize_tile_face_decal;

pub type ShowcaseDecalKey = (Suit, u8, Option<TileEnhancement>, bool);

const DECAL_W: u32 = 192;
const DECAL_H: u32 = 256;
const ATLAS_COLS: u32 = 21;
/// Base faces: 42 tile types × 4 enhancements × 2 debuff states = 336 cells (21×16).
const ATLAS_ROWS: u32 = 16;

fn all_base_faces() -> Vec<(Suit, u8)> {
    let mut v = Vec::with_capacity(42);
    for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
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

/// Build one RGBA atlas texture + lookup from `(suit, rank, enhancement, debuffed)` → UV rect.
pub struct ShowcaseDecalAtlasGpu {
    /// Kept so the atlas stays allocated while bind groups reference `view`.
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub lookup: FxHashMap<ShowcaseDecalKey, [f32; 4]>,
}

pub fn build_showcase_decal_atlas_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile_set: Option<&str>,
) -> ShowcaseDecalAtlasGpu {
    let atlas_w = ATLAS_COLS * DECAL_W;
    let atlas_h = ATLAS_ROWS * DECAL_H;
    let mut atlas_rgba = vec![0u8; (atlas_w * atlas_h * 4) as usize];
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
                let key: ShowcaseDecalKey = (suit, rank, e, debuffed);
                lookup.insert(key, cell_uv_rect(col, row, atlas_w, atlas_h));
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, 336);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showcase-decal-atlas"),
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
        &atlas_rgba,
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
        lookup,
    }
}
