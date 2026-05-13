//! Sprite-atlas glyph extraction for Kenney Input Prompts.
//!
//! Each style (`Xbox Series`, `PlayStation Series`, `Nintendo Switch`, …) ships
//! as one PNG sheet plus an XML index of `SubTexture` rectangles. On first
//! lookup the PNG is decoded once and the XML is parsed once; subsequent
//! lookups crop the named sub-rect into a per-glyph RGBA buffer.
//!
//! The GPU layer caches the cropped buffer as its own `TileFaceOverlayGpu`
//! (one bind-group/texture per glyph), so a small set of frequently-shown
//! glyphs costs roughly the same as the previous SVG pipeline at draw time —
//! just without rasterising SVGs at runtime, and without shipping individual
//! SVG files in the asset pack.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

struct DecodedSheet {
    rgba: Vec<u8>,
    width: u32,
    sub_textures: HashMap<String, SubRect>,
}

#[derive(Clone, Copy)]
struct SubRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

static SHEET_CACHE: LazyLock<Mutex<HashMap<String, DecodedSheet>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns `(rgba8, width, height)` for the named sprite within `sheet_asset`.
/// `sheet_asset` is an asset-relative path to the `_sheet_double.png`; the
/// matching XML index is found by swapping the `.png` extension for `.xml`.
/// Returns `None` if the sheet asset is missing, the XML index lacks the
/// requested name, or the PNG fails to decode. On the first failure for a
/// given `(sheet, sprite)` pair, a granular `log::warn!` identifies which
/// step failed; subsequent failures are silenced via [`MISS_LOG`] so the
/// negative cache in the renderer doesn't double up on the warning.
pub fn extract_sprite_rgba(sheet_asset: &str, sprite_name: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut cache = SHEET_CACHE.lock().ok()?;
    if !cache.contains_key(sheet_asset) {
        let sheet = load_sheet(sheet_asset)?;
        cache.insert(sheet_asset.to_string(), sheet);
    }
    let sheet = cache.get(sheet_asset)?;
    let Some(rect) = sheet.sub_textures.get(sprite_name).copied() else {
        warn_once(format!("{sheet_asset}|{sprite_name}|missing-sub-texture"), || {
            format!(
                "kenney_atlas: '{sprite_name}' not in '{sheet_asset}' \
                 ({} entries indexed)",
                sheet.sub_textures.len()
            )
        });
        return None;
    };
    match crop_rgba(&sheet.rgba, sheet.width, rect) {
        Some(rgba) => Some((rgba, rect.w, rect.h)),
        None => {
            warn_once(format!("{sheet_asset}|{sprite_name}|crop-oob"), || {
                format!(
                    "kenney_atlas: crop out-of-bounds for '{sprite_name}' in '{sheet_asset}' \
                     (rect={}x{}+{}+{}, sheet width={})",
                    rect.w, rect.h, rect.x, rect.y, sheet.width
                )
            });
            None
        }
    }
}

fn load_sheet(sheet_asset: &str) -> Option<DecodedSheet> {
    let Some(png) = crate::asset_path::get(sheet_asset) else {
        warn_once(format!("{sheet_asset}|png-missing"), || {
            format!("kenney_atlas: PNG '{sheet_asset}' not found in asset packs / loose tree")
        });
        return None;
    };
    let img = match image::load_from_memory(&png.data) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            warn_once(format!("{sheet_asset}|png-decode"), || {
                format!("kenney_atlas: PNG '{sheet_asset}' decode failed: {e}")
            });
            return None;
        }
    };
    let (width, _height) = img.dimensions();

    let Some(stripped) = sheet_asset.strip_suffix(".png") else {
        warn_once(format!("{sheet_asset}|extension"), || {
            format!("kenney_atlas: sheet path '{sheet_asset}' missing `.png` suffix")
        });
        return None;
    };
    let xml_asset = format!("{stripped}.xml");
    let Some(xml) = crate::asset_path::get(&xml_asset) else {
        warn_once(format!("{xml_asset}|xml-missing"), || {
            format!("kenney_atlas: XML '{xml_asset}' not found in asset packs / loose tree")
        });
        return None;
    };
    let xml_text = match std::str::from_utf8(&xml.data) {
        Ok(s) => s,
        Err(e) => {
            warn_once(format!("{xml_asset}|xml-utf8"), || {
                format!("kenney_atlas: XML '{xml_asset}' is not valid UTF-8: {e}")
            });
            return None;
        }
    };
    let sub_textures = parse_subtextures(xml_text);
    if sub_textures.is_empty() {
        log::warn!("kenney_atlas: '{xml_asset}' produced no SubTexture entries");
    }

    Some(DecodedSheet {
        rgba: img.into_raw(),
        width,
        sub_textures,
    })
}

/// Suppress repeat `kenney_atlas` warnings for a key we've already logged.
/// The renderer also negative-caches per `cache_key`, but that key only
/// covers the renderer's view; this guards `load_sheet` failures (which
/// happen once per sheet, before per-sprite caching applies).
static MISS_LOG: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

fn warn_once<F: FnOnce() -> String>(key: String, msg: F) {
    let Ok(mut seen) = MISS_LOG.lock() else { return };
    if seen.insert(key) {
        log::warn!("{}", msg());
    }
}

/// Lightweight scanner for `<SubTexture name="…" x="…" y="…" width="…" height="…"/>`.
/// Kenney's TexturePacker output is one element per line with no nested content,
/// so we don't need a full XML parser.
fn parse_subtextures(xml: &str) -> HashMap<String, SubRect> {
    let mut out = HashMap::new();
    for line in xml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("<SubTexture ") else {
            continue;
        };
        let name = attr(rest, "name");
        let x: Option<u32> = attr(rest, "x").and_then(|s| s.parse().ok());
        let y: Option<u32> = attr(rest, "y").and_then(|s| s.parse().ok());
        let w: Option<u32> = attr(rest, "width").and_then(|s| s.parse().ok());
        let h: Option<u32> = attr(rest, "height").and_then(|s| s.parse().ok());
        if let (Some(name), Some(x), Some(y), Some(w), Some(h)) = (name, x, y, w, h) {
            out.insert(name.to_string(), SubRect { x, y, w, h });
        }
    }
    out
}

fn attr<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=\"");
    let i = s.find(&needle)?;
    let start = i + needle.len();
    let end = s[start..].find('"')?;
    Some(&s[start..start + end])
}

fn crop_rgba(rgba: &[u8], src_width: u32, rect: SubRect) -> Option<Vec<u8>> {
    const BPP: u32 = 4;
    let mut out = Vec::with_capacity((rect.w * rect.h * BPP) as usize);
    for row in 0..rect.h {
        let src_row = rect.y + row;
        let start = ((src_row * src_width + rect.x) * BPP) as usize;
        let end = start + (rect.w * BPP) as usize;
        if end > rgba.len() {
            return None;
        }
        out.extend_from_slice(&rgba[start..end]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constants reused by the live code so we test exactly what ships.
    const KEYBOARD_XML: &str = include_str!(
        "../../assets/kenney_input-prompts/Keyboard & Mouse/keyboard-&-mouse_sheet_double.xml"
    );

    #[test]
    fn parser_extracts_known_keyboard_entries() {
        let map = parse_subtextures(KEYBOARD_XML);
        assert!(map.contains_key("keyboard_q"));
        assert!(map.contains_key("keyboard_e"));
        assert!(map.contains_key("keyboard_space"));
        assert!(map.contains_key("keyboard_backspace"));
        let q = map["keyboard_q"];
        assert!(q.w > 0 && q.h > 0);
    }

    #[test]
    fn attr_handles_quoted_values() {
        assert_eq!(attr(r#"name="foo" x="12""#, "name"), Some("foo"));
        assert_eq!(attr(r#"name="foo" x="12""#, "x"), Some("12"));
        assert_eq!(attr(r#"name="foo""#, "missing"), None);
    }

    /// End-to-end: load the keyboard sheet via the live asset loader (packs or
    /// loose) and crop a known sprite. Catches breakage between bake_assets,
    /// the pack index, and the cropper without booting wgpu.
    #[test]
    fn live_keyboard_atlas_extracts_q() {
        let sheet = "kenney_input-prompts/Keyboard & Mouse/keyboard-&-mouse_sheet_double.png";
        let got = extract_sprite_rgba(sheet, "keyboard_q");
        assert!(got.is_some(), "extract_sprite_rgba returned None for keyboard_q");
        let (rgba, w, h) = got.unwrap();
        assert!(w > 0 && h > 0);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }
}
