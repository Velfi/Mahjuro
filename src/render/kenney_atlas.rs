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
/// requested name, or the PNG fails to decode.
pub fn extract_sprite_rgba(sheet_asset: &str, sprite_name: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut cache = SHEET_CACHE.lock().ok()?;
    if !cache.contains_key(sheet_asset) {
        let sheet = load_sheet(sheet_asset)?;
        cache.insert(sheet_asset.to_string(), sheet);
    }
    let sheet = cache.get(sheet_asset)?;
    let rect = sheet.sub_textures.get(sprite_name).copied()?;
    let rgba = crop_rgba(&sheet.rgba, sheet.width, rect)?;
    Some((rgba, rect.w, rect.h))
}

fn load_sheet(sheet_asset: &str) -> Option<DecodedSheet> {
    let png = crate::asset_path::get(sheet_asset)?;
    let img = image::load_from_memory(&png.data).ok()?.to_rgba8();
    let (width, _height) = img.dimensions();

    let xml_asset = format!("{}.xml", sheet_asset.strip_suffix(".png")?);
    let xml = crate::asset_path::get(&xml_asset)?;
    let xml_text = std::str::from_utf8(&xml.data).ok()?;
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
}
