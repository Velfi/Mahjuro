//! Row-major sprite atlas for skip-tag icons (`textures/skip_tags/atlas.toml`).
//!
//! Same fixed schema as tile-set atlases (see `scripts/pack_atlas.py` and
//! `src/render/decal.rs`). Used by [`crate::render::draw_cmd::ImageQuadSource::PackedAtlas`].

use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{LazyLock, Mutex};

struct DecodedAtlas {
    rgba: Vec<u8>,
    width: u32,
    tile_w: u32,
    tile_h: u32,
    origins: FxHashMap<String, (u32, u32)>,
}

static ATLAS_CACHE: LazyLock<Mutex<FxHashMap<String, DecodedAtlas>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));

static MISS_LOG: LazyLock<Mutex<FxHashSet<String>>> =
    LazyLock::new(|| Mutex::new(FxHashSet::default()));

/// Crop a named cell from a packed PNG + sibling `atlas.toml`. Returns
/// `(rgba8, width, height)` or `None` if assets are missing / the name is
/// unknown.
pub fn extract_sprite_rgba(sheet_png: &str, sprite_name: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut cache = ATLAS_CACHE.lock().ok()?;
    if !cache.contains_key(sheet_png) {
        let atlas = load_atlas(sheet_png)?;
        cache.insert(sheet_png.to_string(), atlas);
    }
    let atlas = cache.get(sheet_png)?;
    let Some((x, y)) = atlas.origins.get(sprite_name).copied() else {
        warn_once(format!("{sheet_png}|{sprite_name}|missing-cell"), || {
            format!(
                "skip_tag_atlas: '{sprite_name}' not in '{sheet_png}' \
                     ({} cells indexed)",
                atlas.origins.len()
            )
        });
        return None;
    };
    crop_rgba(&atlas.rgba, atlas.width, x, y, atlas.tile_w, atlas.tile_h)
        .map(|rgba| (rgba, atlas.tile_w, atlas.tile_h))
}

fn load_atlas(sheet_png: &str) -> Option<DecodedAtlas> {
    let Some(png) = crate::asset_path::get(sheet_png) else {
        warn_once(format!("{sheet_png}|png-missing"), || {
            format!("skip_tag_atlas: PNG '{sheet_png}' not found")
        });
        return None;
    };
    let img = match image::load_from_memory(&png.data) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            warn_once(format!("{sheet_png}|png-decode"), || {
                format!("skip_tag_atlas: PNG '{sheet_png}' decode failed: {e}")
            });
            return None;
        }
    };
    let (width, _height) = img.dimensions();

    let toml_asset = atlas_toml_path(sheet_png)?;
    let Some(toml) = crate::asset_path::get(&toml_asset) else {
        warn_once(format!("{toml_asset}|toml-missing"), || {
            format!("skip_tag_atlas: TOML '{toml_asset}' not found")
        });
        return None;
    };
    let toml_src = std::str::from_utf8(&toml.data).ok()?;
    let (tile_w, tile_h, columns, layout) = parse_atlas_toml(toml_src)?;
    if tile_w == 0 || tile_h == 0 || columns == 0 {
        return None;
    }

    let mut origins: FxHashMap<String, (u32, u32)> =
        FxHashMap::with_capacity_and_hasher(layout.len(), Default::default());
    for (i, code) in layout.into_iter().enumerate() {
        if code.is_empty() {
            continue;
        }
        let col = (i as u32) % columns;
        let row = (i as u32) / columns;
        origins.insert(code, (col * tile_w, row * tile_h));
    }

    Some(DecodedAtlas {
        rgba: img.into_raw(),
        width,
        tile_w,
        tile_h,
        origins,
    })
}

fn atlas_toml_path(sheet_png: &str) -> Option<String> {
    let (dir, _file) = sheet_png.rsplit_once('/')?;
    Some(format!("{dir}/atlas.toml"))
}

fn push_layout_tokens(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else {
            break;
        };
        out.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
}

fn parse_atlas_toml(src: &str) -> Option<(u32, u32, u32, Vec<String>)> {
    let mut tile_w: Option<u32> = None;
    let mut tile_h: Option<u32> = None;
    let mut columns: Option<u32> = None;
    let mut layout: Vec<String> = Vec::new();
    let mut in_layout = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if in_layout {
            push_layout_tokens(line, &mut layout);
            if line.contains(']') {
                in_layout = false;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "tile_width" => tile_w = val.parse().ok(),
                "tile_height" => tile_h = val.parse().ok(),
                "columns" => columns = val.parse().ok(),
                "layout" => {
                    in_layout = true;
                    if let Some(rest) = val.strip_prefix('[') {
                        push_layout_tokens(rest, &mut layout);
                        if rest.contains(']') {
                            in_layout = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Some((tile_w?, tile_h?, columns?, layout))
}

fn crop_rgba(rgba: &[u8], src_width: u32, x: u32, y: u32, w: u32, h: u32) -> Option<Vec<u8>> {
    const BPP: u32 = 4;
    let mut out = Vec::with_capacity((w * h * BPP) as usize);
    for row in 0..h {
        let src_row = y + row;
        let start = ((src_row * src_width + x) * BPP) as usize;
        let end = start + (w * BPP) as usize;
        if end > rgba.len() {
            return None;
        }
        out.extend_from_slice(&rgba[start..end]);
    }
    Some(out)
}

fn warn_once<F: FnOnce() -> String>(key: String, msg: F) {
    let Ok(mut seen) = MISS_LOG.lock() else {
        return;
    };
    if seen.insert(key) {
        log::warn!("{}", msg());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tag::TagKind;

    const ATLAS_TOML: &str = include_str!("../../assets/textures/skip_tags/atlas.toml");

    #[test]
    fn parser_reads_skip_tag_layout() {
        let (tw, th, cols, layout) = parse_atlas_toml(ATLAS_TOML).expect("parse");
        assert_eq!(tw, 128);
        assert_eq!(th, 128);
        assert_eq!(cols, 3);
        assert_eq!(layout.len(), 9);
        assert_eq!(layout[0], "gold_ingot");
    }

    #[test]
    fn every_tag_kind_has_atlas_cell() {
        let (_, _, _, layout) = parse_atlas_toml(ATLAS_TOML).expect("parse");
        for tag in TagKind::all() {
            assert!(
                layout.iter().any(|id| id == tag.atlas_slug()),
                "{tag:?} slug {} missing from atlas.toml",
                tag.atlas_slug()
            );
        }
    }

    #[test]
    fn every_ordeal_kind_has_ordeal_icon_atlas_cell() {
        use crate::core::ordeal::OrdealKind;

        for kind in OrdealKind::ALL {
            let got = extract_sprite_rgba("textures/ordeal_icons/atlas.png", kind.atlas_slug());
            assert!(
                got.is_some(),
                "boss icon crop failed for {:?} ({})",
                kind,
                kind.atlas_slug()
            );
        }
    }
}
