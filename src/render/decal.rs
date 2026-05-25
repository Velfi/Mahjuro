//! CPU rasterisation of per-tile Unicode decals.
//!
//! Each Mahjong tile maps to a codepoint in the Unicode Mahjong Tile block
//! (U+1F000–U+1F021).  We try to rasterise that glyph from a system or
//! project-local font; if the font doesn't carry those emoji outlines we fall
//! back to plain-language text (rank digits, wind names, etc.).
//!
//! To get the real Unicode tile glyphs, drop any TTF/OTF that covers the
//! Mahjong block (e.g. Noto Emoji) at `assets/font.ttf` in the project root.

use crate::core::relic::RelicFlavorSpan;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Mirror RGBA image horizontally in place (fixes decal vs mesh +Z chirality).
fn flip_rgba_h_in_place(rgba: &mut [u8], width: u32, height: u32) {
    let bpp = 4usize;
    let w = width as usize;
    let h = height as usize;
    let row_stride = w * bpp;
    for y in 0..h {
        let row = y * row_stride;
        for x in 0..w / 2 {
            let a = row + x * bpp;
            let b = row + (w - 1 - x) * bpp;
            for i in 0..bpp {
                rgba.swap(a + i, b + i);
            }
        }
    }
}

#[inline]
fn finish_tile_decal_rgba(mut rgba: Vec<u8>, width: u32, height: u32, flip_h: bool) -> Vec<u8> {
    if flip_h {
        flip_rgba_h_in_place(&mut rgba, width, height);
    }
    rgba
}

/// Readable label for fallback tile-face rasterization and 2D tile overlays when the
/// Mahjong Unicode glyph is unavailable — no `m`/`s`/`p`, wind initials, or kanji.
pub fn tile_face_display_label(tile: &Tile) -> String {
    match tile.suit {
        Suit::Manzu | Suit::Souzu | Suit::Pinzu => format!("{}", tile.rank),
        Suit::Wind => match tile.rank {
            1 => "East".into(),
            2 => "South".into(),
            3 => "West".into(),
            4 => "North".into(),
            _ => "?".into(),
        },
        Suit::Dragon => match tile.rank {
            1 => "Red".into(),
            2 => "Green".into(),
            3 => "White".into(),
            _ => "?".into(),
        },
        Suit::Flower | Suit::Season => tile.full_name(),
    }
}

/// Emoji indicator for each suit, rendered below the main label.
pub fn tile_suit_emoji(tile: &Tile) -> &'static str {
    match tile.suit {
        Suit::Manzu => "\u{1F3B4}",  // 🎴 flower card
        Suit::Souzu => "\u{1F38B}",  // 🎋 tanabata tree / bamboo
        Suit::Pinzu => "\u{1F534}",  // 🔴 red circle / disc
        Suit::Wind => "\u{1F32C}",   // 🌬 wind face
        Suit::Dragon => "\u{1F409}", // 🐉 dragon
        Suit::Flower => "\u{1F33A}", // 🌺 hibiscus
        Suit::Season => "\u{1F342}", // 🍂 fallen leaf
    }
}

/// Return the asset filename stem for a tile inside a tileset directory.
///
/// Maps `(Suit, rank)` to the naming convention used in `assets/textures/tile_sets/`:
///   Souzu 1–9 → B1..B9, Manzu → C1..C9, Pinzu → D1..D9,
///   Winds → EWind/SWind/WWind/NWind, Dragons → DRed/DGreen/DWhite,
///   Flowers → Flower1..Flower4, Seasons → Season1..Season4.
fn tile_set_filename(tile: &Tile) -> Option<String> {
    match tile.suit {
        Suit::Souzu => Some(format!("B{}", tile.rank)),
        Suit::Manzu => Some(format!("C{}", tile.rank)),
        Suit::Pinzu => Some(format!("D{}", tile.rank)),
        Suit::Wind => {
            let prefix = match tile.rank {
                1 => "E",
                2 => "S",
                3 => "W",
                4 => "N",
                _ => return None,
            };
            Some(format!("{prefix}Wind"))
        }
        Suit::Dragon => {
            let name = match tile.rank {
                1 => "Red",
                2 => "Green",
                3 => "White",
                _ => return None,
            };
            Some(format!("D{name}"))
        }
        Suit::Flower => Some(format!("Flower{}", tile.rank)),
        Suit::Season => Some(format!("Season{}", tile.rank)),
    }
}

/// Cached, decoded atlas for a tileset: the full RGBA8 image plus a lookup
/// from tile code ("B1", "EWind", …) to its (x, y) origin in atlas pixels.
struct Atlas {
    rgba: image::RgbaImage,
    tile_w: u32,
    tile_h: u32,
    columns: u32,
    origins: FxHashMap<String, (u32, u32)>,
}

fn atlas_cache() -> &'static Mutex<FxHashMap<String, Option<std::sync::Arc<Atlas>>>> {
    static CACHE: OnceLock<Mutex<FxHashMap<String, Option<std::sync::Arc<Atlas>>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// Minimal parser for the fixed-schema atlas.toml our packer emits.
/// Recognises: `image = "..."`, `tile_width = N`, `tile_height = N`,
/// `columns = N`, and a `layout = [ "CODE", "CODE", … ]` block that may span
/// multiple lines. Values outside this schema are ignored.
/// Extract quoted tokens from one line of a layout block.
///
/// A quoted empty string (`""`) represents an intentional empty layout slot —
/// the cell exists in the grid but has no tile content. These must be
/// preserved in the output vector so subsequent tile codes land at the
/// correct row/col index. Bare commas, brackets, and whitespace are skipped;
/// unquoted tokens are ignored.
fn push_layout_tokens(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
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
                    // codes may sit on this same line after '['
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

/// Load+decode an atlas for `tile_set`, caching the result. Returns `None` if
/// the set has no atlas.toml, the atlas PNG is missing, or either fails to
/// parse. Subsequent calls for the same set are O(1).
fn load_atlas(tile_set: &str) -> Option<std::sync::Arc<Atlas>> {
    let mut cache = atlas_cache().lock().ok()?;
    if let Some(slot) = cache.get(tile_set) {
        return slot.clone();
    }
    let result = decode_atlas(tile_set).map(std::sync::Arc::new);
    if result.is_none() {
        log::warn!(
            "tileset '{tile_set}' has no loadable atlas (expected sets/{tile_set}/atlas.toml + atlas.png)"
        );
    }
    cache.insert(tile_set.to_string(), result.clone());
    result
}

fn decode_atlas(tile_set: &str) -> Option<Atlas> {
    let toml_path = format!("textures/tile_sets/{tile_set}/atlas.toml");
    let toml_file = crate::asset_path::get(&toml_path)?;
    let toml_src = std::str::from_utf8(toml_file.data.as_ref()).ok()?;
    let (tile_w, tile_h, columns, layout) = parse_atlas_toml(toml_src)?;
    if tile_w == 0 || tile_h == 0 || columns == 0 {
        return None;
    }

    let png_path = format!("textures/tile_sets/{tile_set}/atlas.png");
    let png_file = crate::asset_path::get(&png_path)?;
    let decoder = image::ImageReader::new(std::io::Cursor::new(png_file.data.as_slice()))
        .with_guessed_format()
        .ok()?;
    let img = decoder.decode().ok()?.to_rgba8();

    let mut origins: FxHashMap<String, (u32, u32)> =
        FxHashMap::with_capacity_and_hasher(layout.len(), Default::default());
    for (i, code) in layout.into_iter().enumerate() {
        // Empty layout slots exist as row-padding ("" entries in atlas.toml).
        // They consume a grid cell but never get looked up, so skip indexing.
        if code.is_empty() {
            continue;
        }
        let col = (i as u32) % columns;
        let row = (i as u32) / columns;
        origins.insert(code, (col * tile_w, row * tile_h));
    }
    Some(Atlas {
        rgba: img,
        tile_w,
        tile_h,
        columns,
        origins,
    })
}

/// Try to blit a tile from the tileset's atlas onto `dst`, resizing to
/// `dst_w × dst_h`. Returns `true` on success, `false` if the set has no atlas
/// or this tile code isn't in the layout (caller falls back to font rasterization).
fn blit_set_decal(dst: &mut [u8], dst_w: u32, dst_h: u32, tile: &Tile, tile_set: &str) -> bool {
    let Some(code) = tile_set_filename(tile) else {
        return false;
    };
    let Some(atlas) = load_atlas(tile_set) else {
        return false;
    };
    let Some(&(ox, oy)) = atlas.origins.get(&code) else {
        log_missing_tile_code_once(tile_set, &code);
        return false;
    };
    let _ = atlas.columns; // kept in the struct for debugging / future packing

    // Crop the tile from the atlas, then resize to the target decal dimensions.
    let sub = image::imageops::crop_imm(&atlas.rgba, ox, oy, atlas.tile_w, atlas.tile_h).to_image();
    let sub = image::imageops::resize(&sub, dst_w, dst_h, image::imageops::FilterType::Lanczos3);

    for (i, src_px) in sub.pixels().enumerate() {
        let di = i * 4;
        if di + 3 >= dst.len() {
            break;
        }
        let sa = src_px[3] as f32 / 255.0;
        let da = dst[di + 3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        if out_a > 0.0 {
            for c in 0..3 {
                dst[di + c] =
                    ((src_px[c] as f32 * sa + dst[di + c] as f32 * da * (1.0 - sa)) / out_a) as u8;
            }
            dst[di + 3] = (out_a * 255.0) as u8;
        }
    }
    true
}

#[inline]
fn log_missing_tile_code_once(tile_set: &str, code: &str) {
    static MISSING_TILE_CODE_LOGGED: OnceLock<Mutex<FxHashSet<String>>> = OnceLock::new();
    let key = format!("{tile_set}|{code}");
    let cache = MISSING_TILE_CODE_LOGGED.get_or_init(|| Mutex::new(FxHashSet::default()));
    let mut logged = cache.lock().unwrap_or_else(|e| e.into_inner());
    if logged.insert(key) {
        log::debug!("tile code '{code}' missing from atlas '{tile_set}'");
    }
}

/// Rasterise a `width × height` RGBA8 face decal that mirrors the 2D tile layout:
///   * suit-coloured short label (number / wind / dragon) on the upper portion
///   * suit-coloured emoji indicator on the lower portion
///   * transparent background — the 3D tile shader composites this over the
///     wood albedo from the GLB.
///
/// `width` and `height` should match the tile face's world aspect ratio
/// (long axis vertical, short axis horizontal) so glyph pixels stay square
/// after the shader stretches the texture across the face.
///
/// When `tile_set` is `Some("original")` (or another set name), the function
/// crops the tile from that set's `assets/textures/tile_sets/<name>/atlas.png` (using the
/// `atlas.toml` layout) instead of rasterizing glyphs. Falls back to font
/// rasterization if the atlas is missing or doesn't list this tile code.
///
/// `flip_decal_h`: horizontally mirror the rasterised atlas before upload. Face UVs
/// in `tile_3d.wgsl` handle orientation relative to the mesh; callers normally use
/// `false`. Keep `true` only if a specific layout still needs an extra horizontal flip.
pub fn rasterize_tile_face_decal(
    tile: &Tile,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    width: u32,
    height: u32,
    tile_set: Option<&str>,
    flip_decal_h: bool,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    // If a tileset is configured, try to load the PNG decal for this tile.
    if let Some(set_name) = tile_set
        && blit_set_decal(&mut rgba, width, height, tile, set_name)
    {
        // Talisman accent border drawn *after* the set decal so it
        // composites on top and stays visible.
        if let Some(enh) = tile.enhancement {
            draw_enhancement_border(&mut rgba, width, height, enh);
        }
        if tile.debuffed_visual {
            draw_debuff_marker(&mut rgba, width, height);
        }
        return finish_tile_decal_rgba(rgba, width, height, flip_decal_h);
    }

    // Talisman accent border. Drawn first so the symbol/emoji blits sit on
    // top of it. Each enhancement gets a distinct hue (see
    // `TileEnhancement::accent_color`) so the player can read the talisman
    // identity at a glance even from across the table. Polychrome takes a
    // special rainbow path inside `draw_enhancement_border`.
    if let Some(enh) = tile.enhancement {
        draw_enhancement_border(&mut rgba, width, height, enh);
    }

    // Flower tiles use pre-generated decal PNGs instead of font rasterization.
    if tile.suit == Suit::Flower {
        blit_flower_decal(&mut rgba, width, height, tile.rank);
        if tile.debuffed_visual {
            draw_debuff_marker(&mut rgba, width, height);
        }
        return finish_tile_decal_rgba(rgba, width, height, flip_decal_h);
    }

    let color = tile.suit_color();
    let label = tile_face_display_label(tile);
    let emoji = tile_suit_emoji(tile);

    // Top half: suit-coloured short label.
    if let Some(font) = ui_font {
        let top_h = (height as f32 * 0.60) as u32;
        let band_top = (height as f32 * 0.02) as u32;
        let band = rasterize_label(font, &label, width, top_h);
        blit_tinted(
            TintedSrc {
                pixels: &band,
                width,
                height: top_h,
            },
            TintedDst {
                pixels: &mut rgba,
                width,
                x: 0,
                y: band_top,
            },
            color,
        );
    }

    // Bottom half: suit-coloured emoji.
    if let Some(font) = emoji_font {
        let bot_h = (height as f32 * 0.50) as u32;
        let band_top = (height as f32 * 0.50) as u32;
        let band = rasterize_label(font, emoji, width, bot_h);
        blit_tinted(
            TintedSrc {
                pixels: &band,
                width,
                height: bot_h,
            },
            TintedDst {
                pixels: &mut rgba,
                width,
                x: 0,
                y: band_top,
            },
            color,
        );
    }

    if tile.debuffed_visual {
        draw_debuff_marker(&mut rgba, width, height);
    }

    finish_tile_decal_rgba(rgba, width, height, flip_decal_h)
}

/// Transparent RGBA containing only the debuff X (matches [`draw_debuff_marker`] on tiles).
pub fn rasterize_debuff_marker_overlay(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    draw_debuff_marker(&mut rgba, width, height);
    rgba
}

fn draw_debuff_marker(rgba: &mut [u8], width: u32, height: u32) {
    let w = width as i32;
    let h = height as i32;
    let marker_half = ((width.min(height) as f32) * 0.28).round().max(18.0);
    let cx = (width as f32 * 0.5).round() as i32;
    let cy = (height as f32 * 0.48).round() as i32;
    let slash_len = marker_half * 1.22;
    let white_half = (marker_half * 0.24).max(4.0);
    let red_half = (marker_half * 0.15).max(2.5);

    let blend = |rgba: &mut [u8], x: i32, y: i32, rgb: (u8, u8, u8), alpha: f32| {
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        if a == 0 {
            return;
        }
        let di = ((y as u32 * width + x as u32) * 4) as usize;
        rgba[di] = rgb.0;
        rgba[di + 1] = rgb.1;
        rgba[di + 2] = rgb.2;
        rgba[di + 3] = rgba[di + 3].saturating_add(a);
    };

    let outer = slash_len + white_half + 2.0;
    for y in (cy - outer as i32)..=(cy + outer as i32) {
        for x in (cx - outer as i32)..=(cx + outer as i32) {
            let dx = x - cx;
            let dy = y - cy;
            let strokes = [
                (
                    (dy + dx).abs() as f32 / std::f32::consts::SQRT_2,
                    (dy - dx).abs() as f32 / std::f32::consts::SQRT_2,
                ),
                (
                    (dy - dx).abs() as f32 / std::f32::consts::SQRT_2,
                    (dy + dx).abs() as f32 / std::f32::consts::SQRT_2,
                ),
            ];

            for (line_dist, along) in strokes {
                if along > slash_len {
                    continue;
                }

                // White underpaint keeps the decal legible over suit glyphs.
                if line_dist <= white_half {
                    let edge = 1.0 - line_dist / (white_half + 0.01);
                    let taper = 1.0 - (along / (slash_len + 0.01)).powf(1.35);
                    blend(rgba, x, y, (255, 248, 238), 0.24 + edge * taper * 0.76);
                }

                // Red center stroke so the mark reads as a bold X.
                if line_dist <= red_half {
                    let edge = 1.0 - line_dist / (red_half + 0.01);
                    let taper = 1.0 - (along / (slash_len + 0.01)).powf(1.15);
                    blend(rgba, x, y, (204, 24, 24), 0.40 + edge * taper * 0.60);
                }
            }
        }
    }
}

/// Load and blit a pre-generated flower decal PNG onto `dst`.
///
/// The PNGs are generated by `tools/gen_flower_decals.py` and shipped in asset
/// packs (or loose `assets/` in dev). If the asset isn't found, falls back to a simple tinted "F"
/// glyph so the tile still renders.
fn blit_flower_decal(dst: &mut [u8], dst_w: u32, dst_h: u32, rank: u8) {
    let name = match rank {
        1 => "plum",
        2 => "orchid",
        3 => "chrysanthemum",
        4 => "bamboo",
        _ => return,
    };
    let path = format!("textures/flower_{rank}_{name}.png");
    let Some(file) = crate::asset_path::get(&path) else {
        // Asset not generated yet — fall back to a tinted placeholder.
        log::debug!("flower decal not found: {path}; using fallback");
        blit_flower_fallback(dst, dst_w, dst_h, rank);
        return;
    };

    // Decode PNG and alpha-blend onto the destination buffer.
    let Ok(decoder) =
        image::ImageReader::new(std::io::Cursor::new(file.data.as_slice())).with_guessed_format()
    else {
        blit_flower_fallback(dst, dst_w, dst_h, rank);
        return;
    };
    let Ok(img) = decoder.decode() else {
        blit_flower_fallback(dst, dst_w, dst_h, rank);
        return;
    };
    let img = img.resize_exact(dst_w, dst_h, image::imageops::FilterType::Lanczos3);
    let img = img.to_rgba8();

    for (i, src_px) in img.pixels().enumerate() {
        let di = i * 4;
        if di + 3 >= dst.len() {
            break;
        }
        let sa = src_px[3] as f32 / 255.0;
        let da = dst[di + 3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        if out_a > 0.0 {
            for c in 0..3 {
                dst[di + c] =
                    ((src_px[c] as f32 * sa + dst[di + c] as f32 * da * (1.0 - sa)) / out_a) as u8;
            }
            dst[di + 3] = (out_a * 255.0) as u8;
        }
    }
}

/// Simple tinted fallback when the AI-generated decal PNGs aren't available.
/// Draws the Chinese character for the flower in the suit's pink accent.
fn blit_flower_fallback(dst: &mut [u8], dst_w: u32, dst_h: u32, rank: u8) {
    let label = match rank {
        1 => "梅",
        2 => "蘭",
        3 => "菊",
        4 => "竹",
        _ => "✿",
    };
    let color = [0.90f32, 0.45, 0.55, 1.0]; // Suit::Flower accent
    // Use a simple centered label if we can find the embedded UI font.
    if let Some(font_data) = crate::asset_path::get("NotoSansSC-Regular.ttf")
        .or_else(|| crate::asset_path::get("font.ttf"))
        && let Ok(font) =
            fontdue::Font::from_bytes(font_data.data.as_ref(), fontdue::FontSettings::default())
    {
        let band = rasterize_label(&font, label, dst_w, dst_h);
        blit_tinted(
            TintedSrc {
                pixels: &band,
                width: dst_w,
                height: dst_h,
            },
            TintedDst {
                pixels: dst,
                width: dst_w,
                x: 0,
                y: 0,
            },
            color,
        );
    }
}

/// Rasterise a single-line engraved label onto a transparent RGBA8 buffer at
/// `width × height`, ready to be uploaded as a per-instance decal texture for
/// the bone yaku tablets and lacquered wood action tablets.
///
/// The text is laid out by [`rasterize_label`] (which auto-shrinks to fit
/// width), and then blitted twice: once one pixel down/right in a darker
/// "carved shadow" colour so the engraving reads at oblique camera angles, and
/// once at the requested `ink` colour. Background is fully transparent so the
/// shader can `mix` the procedural base material with the decal alpha.
pub fn rasterize_tablet_label_decal(
    text: &str,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    width: u32,
    height: u32,
    ink: [f32; 4],
) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let Some(font) = ui_font else {
        return rgba;
    };
    // The label fills most of the tablet face but leaves a small margin so
    // the glyphs don't run into the silhouette edge from the side.
    let pad_x = (width as f32 * 0.08) as u32;
    let pad_y = (height as f32 * 0.18) as u32;
    let inner_w = width.saturating_sub(pad_x * 2).max(1);
    let inner_h = height.saturating_sub(pad_y * 2).max(1);

    let band = rasterize_label_styled_with_fallback(
        font,
        emoji_font,
        text,
        inner_w,
        inner_h,
        LabelStyle {
            font_px: None,
            align: LabelAlign::Center,
            scroll_offset: 0.0,
            underline: false,
            baseline_shift_px: 0.0,
        },
    );

    // Soft carved-shadow pass: a slightly darker tint, offset 1px down/right,
    // so the engraving keeps shape under directional lighting.
    let shadow = [ink[0] * 0.25, ink[1] * 0.22, ink[2] * 0.20, ink[3] * 0.85];
    blit_tinted(
        TintedSrc {
            pixels: &band,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width,
            x: pad_x + 1,
            y: pad_y + 1,
        },
        shadow,
    );
    // Main ink pass.
    blit_tinted(
        TintedSrc {
            pixels: &band,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width,
            x: pad_x,
            y: pad_y,
        },
        ink,
    );
    rgba
}

/// Convenience wrapper: warm bone-coloured engraving for the yaku tablets.
/// The default extents of the bone tablets in the gameplay scene give a
/// roughly portrait-ish aspect, but the tablet text reads as a single short
/// line (e.g. "Tanyao"), so we keep the decal landscape and let the shader
/// stretch it across the face.
pub fn rasterize_yaku_tablet_decal(
    name: &str,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
) -> Vec<u8> {
    rasterize_tablet_label_decal(name, ui_font, emoji_font, 256, 96, [0.42, 0.32, 0.18, 1.0])
}

/// Gilded engraving for the wood action tablets. Uses the same three-pass
/// treatment as the hanging plaques — burnt-umber recess, rich gold body,
/// pale champagne highlight — so "Sort by Suit" etc. read as carved and
/// gold-painted lettering rather than flat ink.
pub fn rasterize_wood_tablet_decal(label: &str, ui_font: Option<&fontdue::Font>) -> Vec<u8> {
    let width: u32 = 512;
    let height: u32 = 192;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let Some(font) = ui_font else {
        return rgba;
    };
    let pad_x = (width as f32 * 0.03) as u32;
    let pad_y = (height as f32 * 0.06) as u32;
    let inner_w = width.saturating_sub(pad_x * 2).max(1);
    let inner_h = height.saturating_sub(pad_y * 2).max(1);

    let band = rasterize_label(font, label, inner_w, inner_h);

    // Three-pass gilded letters — same palette as the hanging score plaques.
    let gold_shadow = [0.18_f32, 0.12, 0.04, 0.92];
    let gold_base = [0.92_f32, 0.74, 0.28, 1.0];
    let gold_highlight = [1.00_f32, 0.96, 0.74, 1.0];

    // Drop shadow (offset down-right so the recess reads from above).
    blit_tinted(
        TintedSrc {
            pixels: &band,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width,
            x: pad_x + 2,
            y: pad_y + 2,
        },
        gold_shadow,
    );
    // Gold body.
    blit_tinted(
        TintedSrc {
            pixels: &band,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width,
            x: pad_x,
            y: pad_y,
        },
        gold_base,
    );
    // Bright highlight offset up-left so the leaf catches the light.
    blit_tinted(
        TintedSrc {
            pixels: &band,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width,
            x: pad_x.saturating_sub(1),
            y: pad_y.saturating_sub(1),
        },
        gold_highlight,
    );
    rgba
}

/// Two-line engraved decal for the gameplay scene's hanging score plaque.
/// `top` is the blind/round/score line (champagne ink, slightly larger);
/// `bot` is the gold/wall/wind status line (parchment ink, slightly
/// smaller). Both are baked into a 1024×384 transparent RGBA texture so the
/// lit-mesh shader can composite them onto the +Z face of the plaque mesh.
///
/// Each line is rasterised independently with `font_px = None` so the
/// auto-shrink path picks a size that fits both the available height
/// **and** the line length — long score strings shrink horizontally
/// instead of overflowing the inner band and getting clipped.
///
/// The plaque mesh is set up so only the front face samples this decal —
/// chain nubs and side strips collapse to the (0,0) corner which we leave
/// fully transparent. A 2-px-offset carved-shadow pass renders behind each
/// line so the engraving still reads after the texture is bilinear-stretched
/// across the on-screen plaque face.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaqueDecalStyle {
    /// Gilded three-pass engraving for lacquered wood plaques (default).
    GildedEngraving,
    /// Dark warm brown ink for light grounds (Archive `sign_description_*` boards).
    WalnutInkOnLight,
}

/// Shadow / body / highlight tints and per-pass pixel offsets for plaque engraving.
#[derive(Clone, Copy, Debug)]
pub struct LabelStyleColors {
    pub shadow: [f32; 4],
    pub base: [f32; 4],
    pub highlight: [f32; 4],
    pub shadow_off: (u32, u32),
    pub highlight_off: (i32, i32),
}

/// Same layout pipeline as [`rasterize_plaque_decal`] with a selectable ink palette.
pub fn rasterize_plaque_decal_styled(
    text: &str,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    w: u32,
    h: u32,
    style: PlaqueDecalStyle,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let Some(font) = ui_font else {
        return rgba;
    };
    if text.trim().is_empty() {
        return rgba;
    }
    if matches!(style, PlaqueDecalStyle::WalnutInkOnLight) {
        return rasterize_plaque_walnut_ink_colored_keywords(text, font, emoji_font, w, h);
    }

    // Horizontal padding so glyphs don't run into the engraved-edge silhouette
    // of the plaque face. Vertical padding is kept tight so the text fills
    // most of the texture height — the face is upright (no foreshortening
    // tilt) so every vertical pixel helps it read after the bilinear stretch
    // onto the wood.
    let pad_x = (w as f32 * 0.05) as u32;
    let pad_y = (h as f32 * 0.05) as u32;
    let inner_w = w.saturating_sub(pad_x * 2).max(1);
    let inner_h = h.saturating_sub(pad_y * 2).max(1);

    // Dynamically-sized text container: try a sequence of candidate font
    // sizes (largest first), word-wrapping at each size to determine how many
    // lines the text actually occupies. Pick the biggest size where both:
    //   line_count * line_height ≤ inner_h, and
    //   no single line's advance exceeds inner_w.
    // Forced newlines in the input (`\n`) are honored as hard breaks.
    let (chosen_px, chosen_lines) =
        fit_plaque_text(font, emoji_font, text, inner_w as f32, inner_h as f32);

    // Rasterize the wrapped block at the chosen size. `rasterize_block`
    // handles multi-line layout (stable line-height, per-line alignment).
    let line_refs: Vec<&str> = chosen_lines.iter().map(|s| s.as_str()).collect();
    let block = rasterize_block(
        &DecalFonts {
            regular: font,
            italic: None,
            emoji: emoji_font,
        },
        &line_refs,
        &LabelBlockRasterParams {
            width: inner_w,
            height: inner_h,
            font_px: Some(chosen_px),
            align: LabelAlign::Center,
            underline: false,
        },
    );

    // Shadow / body / highlight tints + per-pass pixel offset (relative to the body pass).
    // Tighter offsets read as ink on a flat ground; larger offsets read as carved engraving.
    let LabelStyleColors {
        shadow,
        base,
        highlight,
        shadow_off,
        highlight_off,
    } = match style {
        PlaqueDecalStyle::GildedEngraving => {
            // Gilded letters: a deep umber drop-shadow under a rich gold body, topped
            // by a bright pale-gold highlight offset up-left. Three passes give the
            // engraving a metallic, leafed-gold read at any size.
            LabelStyleColors {
                shadow: [0.18_f32, 0.12, 0.04, 0.92],   // burnt umber recess
                base: [0.92_f32, 0.74, 0.28, 1.0],      // rich antique gold
                highlight: [1.00_f32, 0.96, 0.74, 1.0], // pale champagne sheen
                shadow_off: (3, 3),
                highlight_off: (-1, -1),
            }
        }
        PlaqueDecalStyle::WalnutInkOnLight => {
            // Walnut / iron-gall ink on off-white board: keep the shadow tight and
            // soft so it reads as paper bleed instead of a second silhouette. No
            // bright top-pass — on white grounds a champagne lift would wash the
            // body out.
            LabelStyleColors {
                shadow: [0.10_f32, 0.06, 0.04, 0.32],
                base: [0.22_f32, 0.13, 0.09, 1.0],
                highlight: [0.0_f32, 0.0, 0.0, 0.0],
                shadow_off: (1, 1),
                highlight_off: (0, 0),
            }
        }
    };

    if shadow[3] > 0.0 {
        blit_tinted(
            TintedSrc {
                pixels: &block,
                width: inner_w,
                height: inner_h,
            },
            TintedDst {
                pixels: &mut rgba,
                width: w,
                x: pad_x + shadow_off.0,
                y: pad_y + shadow_off.1,
            },
            shadow,
        );
    }
    blit_tinted(
        TintedSrc {
            pixels: &block,
            width: inner_w,
            height: inner_h,
        },
        TintedDst {
            pixels: &mut rgba,
            width: w,
            x: pad_x,
            y: pad_y,
        },
        base,
    );
    if highlight[3] > 0.0 {
        let hx = if highlight_off.0 >= 0 {
            pad_x + highlight_off.0 as u32
        } else {
            pad_x.saturating_sub(highlight_off.0.unsigned_abs())
        };
        let hy = if highlight_off.1 >= 0 {
            pad_y + highlight_off.1 as u32
        } else {
            pad_y.saturating_sub(highlight_off.1.unsigned_abs())
        };
        blit_tinted(
            TintedSrc {
                pixels: &block,
                width: inner_w,
                height: inner_h,
            },
            TintedDst {
                pixels: &mut rgba,
                width: w,
                x: hx,
                y: hy,
            },
            highlight,
        );
    }
    rgba
}

pub fn rasterize_plaque_decal(
    text: &str,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    w: u32,
    h: u32,
) -> Vec<u8> {
    rasterize_plaque_decal_styled(
        text,
        ui_font,
        emoji_font,
        w,
        h,
        PlaqueDecalStyle::GildedEngraving,
    )
}

/// Fit `text` into a `(w, h)` area by searching for the largest font size
/// whose word-wrapped layout fits vertically and horizontally. Returns the
/// chosen `font_px` and the wrapped lines at that size. Honors explicit
/// `\n` as hard line breaks.
fn fit_plaque_text(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    w: f32,
    h: f32,
) -> (f32, Vec<String>) {
    // Candidate sizes sweep downward from a height-based ceiling. We let
    // word-wrap take up vertical space so short strings grow as large as the
    // box height allows, rather than being pinned by their single-line width.
    let max_by_h = h * 0.85;
    let mut candidate = max_by_h.max(12.0);

    let min_px = 10.0_f32;
    loop {
        let lines = wrap_lines(font, emoji_font, text, candidate, w);
        let line_metrics = font.horizontal_line_metrics(candidate);
        let line_h = line_metrics
            .map(|lm| lm.new_line_size)
            .unwrap_or(candidate * 1.2);
        let total_h = line_h * lines.len() as f32;
        // Widest line's advance at this size.
        let widest = lines
            .iter()
            .map(|s| advance_width(font, emoji_font, s, candidate))
            .fold(0.0_f32, f32::max);
        if total_h <= h && widest <= w {
            return (candidate, lines);
        }
        if candidate <= min_px {
            // Floor: accept the smallest legal size even if it overflows a
            // little rather than producing unreadable glyphs.
            return (min_px, lines);
        }
        // Shrink by 7% per iteration — fast enough to converge, fine enough
        // to land on a visually distinct size per step.
        candidate = (candidate * 0.93).max(min_px);
    }
}

/// Word-wrap `text` to the given pixel width at `font_px`. Treats explicit
/// `\n` as hard line breaks and otherwise wraps on whitespace with TeX-style
/// demerit minimization ([`crate::ui::text_wrap::break_units_kp`]). Words are
/// atomic — we don't break inside words; the size-fit loop shrinks instead.
fn wrap_lines(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    font_px: f32,
    max_w: f32,
) -> Vec<String> {
    use crate::ui::text_wrap::{TextBreakUnit, break_units_kp};

    let space_w = advance_width(font, emoji_font, " ", font_px);
    let mut out: Vec<String> = Vec::new();
    for hard in text.split('\n') {
        let words: Vec<&str> = hard.split_whitespace().collect();
        if words.is_empty() {
            out.push(String::new());
            continue;
        }
        let units: Vec<TextBreakUnit<String>> = words
            .iter()
            .map(|w| TextBreakUnit {
                width: advance_width(font, emoji_font, w, font_px),
                payload: (*w).to_string(),
            })
            .collect();
        for line in break_units_kp(&units, max_w, space_w) {
            out.push(line.join(" "));
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn advance_width(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    font_px: f32,
) -> f32 {
    text.chars()
        .map(|ch| {
            pick_font(font, emoji_font, ch)
                .metrics(ch, font_px)
                .advance_width
        })
        .sum()
}

/// Archive description plaque: same auto-fit + wrap as [`rasterize_plaque_decal_styled`]
/// ([`PlaqueDecalStyle::WalnutInkOnLight`]), with per-token keyword tints.
fn rasterize_plaque_walnut_ink_colored_keywords(
    text: &str,
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    const DEFAULT_INK: [f32; 4] = [0.22_f32, 0.13, 0.09, 1.0];
    const SHADOW_TINT: [f32; 4] = [0.10_f32, 0.06, 0.04, 0.32];

    let pad_x = (w as f32 * 0.05) as u32;
    let pad_y = (h as f32 * 0.05) as u32;
    let inner_w = w.saturating_sub(pad_x * 2).max(1);
    let inner_h = h.saturating_sub(pad_y * 2).max(1);

    let (chosen_px, chosen_lines) =
        fit_plaque_text(font, emoji_font, text, inner_w as f32, inner_h as f32);

    let line_metrics = font.horizontal_line_metrics(chosen_px);
    let line_h = line_metrics
        .map(|lm| lm.new_line_size)
        .unwrap_or(chosen_px * 1.2);
    let ascender_px = line_metrics.map(|m| m.ascent).unwrap_or(chosen_px * 0.8);
    let line_cell_h = line_h.ceil() as u32 + 2;
    let baseline_in_band = ((line_cell_h as f32 - line_h) * 0.5).max(0.0) + ascender_px;

    let n_lines = chosen_lines.len().max(1);
    let total_text_h = line_h * n_lines as f32;
    let block_top = ((inner_h as f32 - total_text_h) * 0.5).max(0.0);

    for (i, line) in chosen_lines.iter().enumerate() {
        let mut chunks: Vec<(String, [f32; 4])> = Vec::new();
        for (idx, word) in line.split_whitespace().enumerate() {
            if idx > 0 {
                chunks.push((" ".to_string(), DEFAULT_INK));
            }
            chunks.extend(super::vocabulary_colors::colored_token_segments(
                word,
                DEFAULT_INK,
            ));
        }
        if chunks.is_empty() {
            continue;
        }

        let total_adv: f32 = chunks
            .iter()
            .map(|(s, _)| advance_width(font, emoji_font, s, chosen_px))
            .sum();
        let mut cx = (inner_w as f32 - total_adv) * 0.5;
        let line_baseline_global = pad_y as f32 + block_top + i as f32 * line_h + ascender_px;
        let dst_y = (line_baseline_global - baseline_in_band).floor().max(0.0) as u32;

        // One soft shadow for the whole line. Per-token shadows used to call `blit_tinted`
        // for every word; destination alpha is accumulated with `saturating_add`, so
        // overlapping +1/+1 shadow passes stacked into opaque black slabs on dense copy.
        let rh = line_cell_h.max(1);
        let line_shadow_band = rasterize_label_styled_with_fallback(
            font,
            emoji_font,
            line,
            inner_w,
            rh,
            LabelStyle {
                font_px: Some(chosen_px),
                align: LabelAlign::Center,
                scroll_offset: 0.0,
                underline: false,
                baseline_shift_px: 0.0,
            },
        );
        blit_tinted(
            TintedSrc {
                pixels: &line_shadow_band,
                width: inner_w,
                height: rh,
            },
            TintedDst {
                pixels: &mut rgba,
                width: w,
                x: pad_x.saturating_add(1),
                y: dst_y.saturating_add(1),
            },
            SHADOW_TINT,
        );

        for (s, tint) in &chunks {
            let aw = advance_width(font, emoji_font, s, chosen_px);
            if aw <= 0.0 {
                continue;
            }
            let rw = aw.ceil() as u32 + 3;
            let band = rasterize_label_styled_with_fallback(
                font,
                emoji_font,
                s,
                rw.max(1),
                rh,
                LabelStyle {
                    font_px: Some(chosen_px),
                    align: LabelAlign::Left,
                    scroll_offset: 0.0,
                    underline: false,
                    baseline_shift_px: 0.0,
                },
            );
            let dst_x = pad_x + cx.floor() as u32;
            blit_tinted(
                TintedSrc {
                    pixels: &band,
                    width: rw.max(1),
                    height: rh,
                },
                TintedDst {
                    pixels: &mut rgba,
                    width: w,
                    x: dst_x,
                    y: dst_y,
                },
                *tint,
            );
            cx += aw;
        }
    }

    rgba
}

/// Reference height (in texels) for the plaque decal texture. The actual
/// width is computed at draw time from the plaque face's world-space aspect
/// ratio so glyphs don't get stretched by the bilinear sampler when the face
/// isn't ~landscape — see the call site in `wgpu_renderer.rs`.
pub const PLAQUE_DECAL_HEIGHT: u32 = 320;

/// Reference long-edge size (in texels) for the ofuda decal texture. The
/// other dimension is computed at draw time from the paper face's world-space
/// aspect ratio so the bilinear sampler doesn't stretch glyphs — see the
/// call site in `wgpu_renderer.rs`.
#[allow(dead_code)]
pub const OFUDA_DECAL_LONG_EDGE: u32 = 1024;

/// Paint a boss-rule ofuda paper face: large title (boss name) at the top,
/// wrapped rule description below in a smaller hand. Both passes use a deep
/// sumi-ink tint with a soft drop shadow so the calligraphy reads off the
/// warm paper background.
pub fn rasterize_ofuda_decal(
    title: &str,
    rule: &str,
    ui_font: Option<&fontdue::Font>,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let Some(font) = ui_font else {
        return rgba;
    };
    // Keep the paper margins visibly present, but let the calligraphy occupy
    // more of the sheet. The previous 8% inset left the body copy floating in
    // an overly narrow center column on portrait boss ofuda.
    let pad_x = (w as f32 * 0.025) as u32;
    let pad_y = (h as f32 * 0.03) as u32;
    let inner_w = w.saturating_sub(pad_x * 2).max(1);
    let inner_h = h.saturating_sub(pad_y * 2).max(1);

    // Sumi-ink calligraphy: warm-black body with a soft brown drop shadow so
    // the strokes lift off the parchment background even when the candle
    // light desaturates the paper.
    let ink = [0.12_f32, 0.08, 0.05, 1.0];
    let shadow = [0.55_f32, 0.30, 0.10, 0.55];

    let stamp = |band: &[u8], band_h: u32, y_off: u32, rgba: &mut Vec<u8>| {
        blit_tinted(
            TintedSrc {
                pixels: band,
                width: inner_w,
                height: band_h,
            },
            TintedDst {
                pixels: rgba,
                width: w,
                x: pad_x + 2,
                y: y_off + 2,
            },
            shadow,
        );
        blit_tinted(
            TintedSrc {
                pixels: band,
                width: inner_w,
                height: band_h,
            },
            TintedDst {
                pixels: rgba,
                width: w,
                x: pad_x,
                y: y_off,
            },
            ink,
        );
    };

    if rule.is_empty() {
        // Title-only mode: fill the full inner height.
        let title_chars = title.chars().count().max(1) as f32;
        let title_px = (inner_h as f32 * 0.78)
            .min(inner_w as f32 / (title_chars * 0.55))
            .max(14.0);
        let title_h = ((title_px * 1.15) as u32).min(inner_h).max(1);
        let title_band = rasterize_label_styled(
            font,
            title,
            inner_w,
            title_h,
            Some(title_px),
            LabelAlign::Center,
        );
        let y_off = pad_y + inner_h.saturating_sub(title_h) / 2;
        stamp(&title_band, title_h, y_off, &mut rgba);
    } else {
        // Pick a title font size from a *nominal* title band (~35% of inner_h),
        // then build a tight title band sized just to the chosen glyphs. The
        // wrapped rule body gets whatever's left after a small inter-block gap,
        // and the whole stack is centred vertically.
        let nominal_title_h = (inner_h as f32 * 0.35) as u32;
        let title_chars = title.chars().count().max(1) as f32;
        let title_px = (nominal_title_h as f32 * 0.78)
            .min(inner_w as f32 * 1.5 / title_chars)
            .max(14.0);
        let title_h = ((title_px * 1.15) as u32).min(inner_h).max(1);

        // Tiny inter-block gap.
        let gap_h: u32 = 6;

        // Rule: word-wrap on a rough character budget per line so the body
        // reads as 2–3 stacked lines. The wrapped block gets the remaining
        // vertical room after the title and gap. Target the larger of "rule_h
        // / 3" and "inner_w / 7" so narrow portrait papers (in-round HUD
        // ofuda) still get tall glyphs even when there's lots of vertical
        // room and very little horizontal room — width is the binding
        // constraint there, not height.
        let rule_h = inner_h.saturating_sub(title_h + gap_h).max(1);
        let rule_px_target = (rule_h as f32 / 3.0)
            .max(inner_w as f32 / 7.0)
            .clamp(72.0, 160.0);
        let approx_glyph_w = rule_px_target * 0.50;
        let chars_per_line = ((inner_w as f32 / approx_glyph_w).floor() as usize).max(8);
        let wrapped_rule = wrap_text(rule, chars_per_line);
        let wrapped_rule_lines: Vec<&str> = wrapped_rule.lines().collect();
        let rule_px = fit_label_block_font_px(
            font,
            None,
            &wrapped_rule_lines,
            inner_w,
            rule_h,
            rule_px_target,
            44.0,
        );

        let title_band = rasterize_label_styled(
            font,
            title,
            inner_w,
            title_h,
            Some(title_px),
            LabelAlign::Center,
        );
        let rule_band = rasterize_label_styled(
            font,
            &wrapped_rule,
            inner_w,
            rule_h,
            Some(rule_px),
            LabelAlign::Center,
        );

        // Centre the (title + gap + rule) stack vertically inside `inner_h`.
        let stack_h = title_h + gap_h + rule_h;
        let stack_top = pad_y + inner_h.saturating_sub(stack_h) / 2;
        stamp(&title_band, title_h, stack_top, &mut rgba);
        stamp(&rule_band, rule_h, stack_top + title_h + gap_h, &mut rgba);
    }
    rgba
}

/// Greedy word-wrap by character budget. Returns lines joined with `\n` so
/// the result can flow straight into `rasterize_label_styled`'s multi-line
/// path. Falls back to splitting an oversized single word at the budget so a
/// runaway token can't blow past the line cap.
fn wrap_text(text: &str, max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            if word.chars().count() > max_chars {
                // Hard split a single oversized word.
                let mut buf = String::new();
                for ch in word.chars() {
                    buf.push(ch);
                    if buf.chars().count() == max_chars {
                        lines.push(std::mem::take(&mut buf));
                    }
                }
                current = buf;
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}

/// Pick the largest font size up to `target_px` that keeps a multi-line block
/// within both the supplied width and height.
pub fn fit_label_block_font_px(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    lines: &[&str],
    width: u32,
    height: u32,
    target_px: f32,
    min_px: f32,
) -> f32 {
    fn fits(
        font: &fontdue::Font,
        emoji_font: Option<&fontdue::Font>,
        lines: &[&str],
        width: u32,
        height: u32,
        font_px: f32,
    ) -> bool {
        let line_h = font
            .horizontal_line_metrics(font_px)
            .map(|lm| lm.new_line_size)
            .unwrap_or(font_px * 1.2);
        if line_h * lines.len() as f32 > height as f32 {
            return false;
        }

        lines.iter().all(|line| {
            let advance: f32 = line
                .chars()
                .map(|ch| {
                    pick_font(font, emoji_font, ch)
                        .metrics(ch, font_px)
                        .advance_width
                })
                .sum();
            advance <= width as f32
        })
    }

    let min_px = min_px.max(8.0).min(target_px.max(8.0));
    if lines.is_empty() || fits(font, emoji_font, lines, width, height, target_px) {
        return target_px.max(8.0);
    }

    let mut lo = min_px;
    let mut hi = target_px.max(min_px);
    for _ in 0..10 {
        let mid = (lo + hi) * 0.5;
        if fits(font, emoji_font, lines, width, height, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Largest font size ≤ `target_px` so a single line fits `width` × `height`.
pub fn fit_single_line_font_px(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    width: u32,
    height: u32,
    target_px: f32,
    min_px: f32,
) -> f32 {
    fit_label_block_font_px(font, emoji_font, &[text], width, height, target_px, min_px)
}

/// Resolve a pinned or auto font size for UI labels before rasterization.
pub fn resolve_label_font_px(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    width: u32,
    height: u32,
    font_px: Option<f32>,
    min_px: f32,
) -> f32 {
    let min_px = min_px.max(8.0);
    let lines: Vec<&str> = text.split('\n').collect();
    match font_px {
        None => (height as f32 * 0.55)
            .min(width as f32 * 1.5 / text.chars().count().max(1) as f32)
            .max(min_px),
        Some(target) => {
            let target = target.max(min_px);
            if lines.len() > 1 {
                fit_label_block_font_px(font, emoji_font, &lines, width, height, target, min_px)
            } else {
                fit_single_line_font_px(font, emoji_font, text, width, height, target, min_px)
            }
        }
    }
}

/// Paint a soft inner border + corner gem onto an RGBA8 decal to mark a tile
/// that has been enhanced by a talisman. The border is a 4-px-thick frame
/// with a softer outer glow that fades to transparent, and a small inlay
/// "gem" in the upper-right corner.
///
/// Each enhancement gets its own visual treatment:
/// - **Pearl** — soft, low-saturation iridescence cycling around the frame
///   in cool/warm pastels.
/// - **Gilded** — polished gold with a sine-wave brightness highlight that
///   runs around the perimeter so the frame reads as a reflective metal rim.
/// - **Polychrome** — full-saturation rainbow flowing clockwise around the
///   frame; gem is a polar rainbow.
fn draw_enhancement_border(rgba: &mut [u8], width: u32, height: u32, enh: TileEnhancement) {
    let w = width as i32;
    let h = height as i32;

    // Frame parameters in pixels. Tuned for the 192×256 face decal — chunky
    // enough to read across the table without crowding the rank/emoji glyphs.
    let inset: i32 = 6;
    let thickness: i32 = 9;
    let glow: i32 = 11;

    // Perimeter parameter t∈[0,1). We walk the inset rect clockwise from
    // the top-left so styling flows naturally around the tile.
    let inner_w = (w - 2 * inset).max(1) as f32;
    let inner_h = (h - 2 * inset).max(1) as f32;
    let perimeter = 2.0 * (inner_w + inner_h);
    let perimeter_t = |x: i32, y: i32| -> f32 {
        let xc = (x as f32).clamp(inset as f32, (w - 1 - inset) as f32);
        let yc = (y as f32).clamp(inset as f32, (h - 1 - inset) as f32);
        let lx = xc - inset as f32;
        let ly = yc - inset as f32;
        let d_top = ly;
        let d_bot = inner_h - ly;
        let d_lft = lx;
        let d_rgt = inner_w - lx;
        let m = d_top.min(d_bot).min(d_lft).min(d_rgt);
        let s = if m == d_top {
            lx
        } else if m == d_rgt {
            inner_w + ly
        } else if m == d_bot {
            inner_w + inner_h + (inner_w - lx)
        } else {
            2.0 * inner_w + inner_h + (inner_h - ly)
        };
        (s / perimeter).rem_euclid(1.0)
    };

    let put = |rgba: &mut [u8], x: i32, y: i32, rgb: (u8, u8, u8), alpha: f32| {
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        if a == 0 {
            return;
        }
        let di = ((y as u32 * width + x as u32) * 4) as usize;
        rgba[di] = rgb.0;
        rgba[di + 1] = rgb.1;
        rgba[di + 2] = rgb.2;
        rgba[di + 3] = rgba[di + 3].saturating_add(a);
    };

    // Distance from a point to the inner-frame outline (axis-aligned ring).
    let frame_dist = |x: i32, y: i32| -> i32 {
        let dx = (inset - x).max(x - (w - 1 - inset)).max(0);
        let dy = (inset - y).max(y - (h - 1 - inset)).max(0);
        if dx == 0 && dy == 0 {
            let inner_dx = (x - inset).min(w - 1 - inset - x);
            let inner_dy = (y - inset).min(h - 1 - inset - y);
            -inner_dx.min(inner_dy)
        } else {
            dx.max(dy)
        }
    };

    for y in 0..h {
        for x in 0..w {
            let d = frame_dist(x, y);
            let in_band = d <= 0 && d >= -thickness;
            let in_glow = d > 0 && d <= glow;
            if !in_band && !in_glow {
                continue;
            }
            let t = perimeter_t(x, y);
            let rgb = enhancement_border_color(enh, t);
            if in_band {
                put(rgba, x, y, rgb, 0.85);
            } else {
                let g = 1.0 - (d as f32 / (glow as f32 + 1.0));
                put(rgba, x, y, rgb, 0.55 * g * g);
            }
        }
    }

    // Corner gem in the upper-right: a small filled disc, brighter than the
    // border so it reads as an inlay. The gem styling matches the border
    // (polar coords drive the per-pixel colour for directional materials).
    let cx = w - inset - thickness - 10;
    let cy = inset + thickness + 10;
    let radius: i32 = 10;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let r2 = (dx * dx + dy * dy) as f32;
            let edge = (radius * radius) as f32;
            if r2 > edge {
                continue;
            }
            // Normalised radius [0,1] from centre and angle [0,1) clockwise
            // from the top — same convention as the border perimeter so the
            // gem feels visually consistent with the frame.
            let r_norm = (r2 / edge).sqrt();
            let theta = (dx as f32).atan2(-dy as f32);
            let angle_t = (theta / std::f32::consts::TAU + 1.0).rem_euclid(1.0);
            let rgb = enhancement_gem_color(enh, angle_t, r_norm);
            let alpha = 0.6 + 0.4 * (1.0 - r_norm);
            put(rgba, cx + dx, cy + dy, rgb, alpha);
        }
    }
}

/// Per-pixel colour of the enhancement border at perimeter parameter `t`.
fn enhancement_border_color(enh: TileEnhancement, t: f32) -> (u8, u8, u8) {
    match enh {
        TileEnhancement::Polychrome => hsv_to_rgb_u8(t, 0.85, 1.0),

        TileEnhancement::Pearl => {
            // Iridescent: low-saturation hue drift. Adjacent pixels share
            // similar tints so the rim looks like nacre catching the light
            // rather than a candy-coloured rainbow.
            let hue = (t + 0.55).rem_euclid(1.0);
            hsv_to_rgb_u8(hue, 0.22, 1.0)
        }

        TileEnhancement::Gilded => {
            // Polished gold: a sine-wave brightness pulse runs around the
            // perimeter so one side of the frame catches light strongly
            // while the opposite side falls into warm shadow.
            let phase = (t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2).sin();
            let bright = 0.55 + 0.45 * (phase * 0.5 + 0.5);
            let r = (1.00 * bright * 255.0).clamp(0.0, 255.0) as u8;
            let g = (0.80 * bright * 255.0).clamp(0.0, 255.0) as u8;
            let b = (0.25 * bright * bright * 255.0).clamp(0.0, 255.0) as u8;
            (r, g, b)
        }
    }
}

/// Per-pixel colour of the corner gem. `angle_t ∈ [0,1)` is the clockwise
/// angle from the top of the gem; `r_norm ∈ [0,1]` is normalised distance
/// from the gem centre (0 = centre, 1 = rim).
fn enhancement_gem_color(enh: TileEnhancement, angle_t: f32, r_norm: f32) -> (u8, u8, u8) {
    match enh {
        TileEnhancement::Polychrome => hsv_to_rgb_u8(angle_t, 0.9, 1.0),

        TileEnhancement::Pearl => {
            // Radial pearl: hue drifts around the gem at low saturation,
            // brighter at the centre to suggest a polished orb catching
            // a specular highlight.
            let hue = (angle_t + 0.6).rem_euclid(1.0);
            let v = (1.0 - 0.25 * r_norm).clamp(0.0, 1.0);
            hsv_to_rgb_u8(hue, 0.18, v)
        }

        TileEnhancement::Gilded => {
            // Polished gold orb: bright specular near a fixed angle (top-
            // left of the gem) falling off into warmer amber elsewhere.
            // Distance from the highlight angle in [0, 0.5].
            let highlight = 0.875_f32; // ~10 o'clock
            let mut d_angle = (angle_t - highlight).abs();
            if d_angle > 0.5 {
                d_angle = 1.0 - d_angle;
            }
            let bright = 1.0 - 0.55 * (d_angle * 2.0) - 0.20 * r_norm;
            let bright = bright.clamp(0.35, 1.0);
            let r = (1.00 * bright * 255.0) as u8;
            let g = (0.82 * bright * 255.0) as u8;
            let b = (0.28 * bright * bright * 255.0) as u8;
            (r, g, b)
        }
    }
}

/// Convert HSV (h,s,v in [0,1]) to an 8-bit RGB triple. Used by the
/// polychrome border / gem so we don't need a colour-space dependency.
fn hsv_to_rgb_u8(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (
        (r * 255.0).clamp(0.0, 255.0) as u8,
        (g * 255.0).clamp(0.0, 255.0) as u8,
        (b * 255.0).clamp(0.0, 255.0) as u8,
    )
}

/// RGBA source buffer + dimensions for `blit_tinted`.
struct TintedSrc<'a> {
    pixels: &'a [u8],
    width: u32,
    height: u32,
}

/// RGBA destination buffer + row width + top-left blit offset.
struct TintedDst<'a> {
    pixels: &'a mut [u8],
    width: u32,
    x: u32,
    y: u32,
}

/// Blit a single-channel-in-alpha RGBA source onto an RGBA destination at
/// `(dst.x, dst.y)`, compositing **straight-alpha** ink (`tint.rgb` with coverage
/// from the source alpha channel) over existing pixels with Porter–Duff "over".
///
/// Historically this used `dst.a += src.a`, which stacks overlapping glyph passes
/// (shadow + body, adjacent lines, per-token blits) into opaque black slabs on
/// archive plaques and other dense decals.
fn blit_tinted(src: TintedSrc<'_>, dst: TintedDst<'_>, tint: [f32; 4]) {
    let TintedSrc {
        pixels: src,
        width: sw,
        height: sh,
    } = src;
    let dw = dst.width.max(1);
    let dst_x = dst.x;
    let dst_y = dst.y;
    let dst = &mut *dst.pixels;
    let dst_h = (dst.len() / (dw as usize * 4)) as u32;

    let sr = tint[0].clamp(0.0, 1.0);
    let sg = tint[1].clamp(0.0, 1.0);
    let sb = tint[2].clamp(0.0, 1.0);
    let tint_a = tint[3].clamp(0.0, 1.0);

    for y in 0..sh {
        for x in 0..sw {
            let dx = dst_x + x;
            let dy = dst_y + y;
            if dx >= dw || dy >= dst_h {
                continue;
            }
            let si = ((y * sw + x) * 4) as usize;
            let di = ((dy * dw + dx) * 4) as usize;
            if di + 3 >= dst.len() {
                continue;
            }
            let a_src = (src[si + 3] as f32 / 255.0) * tint_a;
            if a_src <= 1e-5 {
                continue;
            }
            let da = dst[di + 3] as f32 / 255.0;
            let dr = dst[di] as f32 / 255.0;
            let dg = dst[di + 1] as f32 / 255.0;
            let db = dst[di + 2] as f32 / 255.0;

            let out_a = a_src + da * (1.0 - a_src);
            if out_a <= 1e-5 {
                continue;
            }
            let out_r = (sr * a_src + dr * da * (1.0 - a_src)) / out_a;
            let out_g = (sg * a_src + dg * da * (1.0 - a_src)) / out_a;
            let out_b = (sb * a_src + db * da * (1.0 - a_src)) / out_a;

            dst[di] = (out_r * 255.0).clamp(0.0, 255.0) as u8;
            dst[di + 1] = (out_g * 255.0).clamp(0.0, 255.0) as u8;
            dst[di + 2] = (out_b * 255.0).clamp(0.0, 255.0) as u8;
            dst[di + 3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
}

/// Load Noto Emoji for tile decals (covers U+1F000–U+1F02B as outline glyphs).
/// Cached; loaded from asset packs or loose `assets/`.
fn load_noto_emoji_bytes() -> Option<Vec<u8>> {
    static CACHE: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let candidates = [
                "fonts/Noto_Emoji/NotoEmoji-VariableFont_wght.ttf",
                "fonts/Noto_Emoji/static/NotoEmoji-Regular.ttf",
            ];
            for path in candidates {
                if let Some(file) = crate::asset_path::get(path) {
                    log::debug!("decal: loaded Noto Emoji from embedded {path}");
                    return Some(file.data.to_vec());
                }
            }
            log::warn!("decal: Noto Emoji not found in embedded assets.");
            None
        })
        .clone()
}

/// Load Noto Emoji as a ready-to-use `fontdue::Font` (for tile symbols).
/// Cached so the font is only parsed once.
pub fn load_noto_emoji_font() -> Option<fontdue::Font> {
    static CACHE: std::sync::OnceLock<Option<fontdue::Font>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            load_noto_emoji_bytes().and_then(|b| {
                fontdue::Font::from_bytes(b.as_slice(), fontdue::FontSettings::default()).ok()
            })
        })
        .clone()
}

/// Static-lifetime symbols / math / Greek fallback consulted by [`pick_font`] before the emoji
/// font. **Noto Sans Math** covers Greek (π), math operators (≈, ≠, ≤, ≥, ×, ÷, …), arrows,
/// and most BMP punctuation — superset of what plaque copy actually requests.
///
/// Loaded lazily from `assets/fonts/Noto_Sans_Math/NotoSansMath-Regular.ttf`. Returns `None`
/// (and caches that) when the asset is unavailable, so the cost amortizes to one lookup per process.
fn noto_sans_symbols_font() -> Option<&'static fontdue::Font> {
    static CELL: std::sync::OnceLock<Option<fontdue::Font>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        let candidates = ["fonts/Noto_Sans_Math/NotoSansMath-Regular.ttf"];
        for path in candidates {
            if let Some(file) = crate::asset_path::get(path)
                && let Ok(f) =
                    fontdue::Font::from_bytes(file.data.as_ref(), fontdue::FontSettings::default())
            {
                log::debug!("decal: loaded Noto Sans Math (symbol fallback) from {path}");
                return Some(f);
            }
        }
        log::debug!("decal: Noto Sans Math symbol fallback not found in embedded assets.");
        None
    })
    .as_ref()
}

/// Load the UI font and return a ready-to-use `fontdue::Font`.
/// Cached so the font is only parsed once.
pub fn load_ui_font() -> Option<&'static fontdue::Font> {
    static CACHE: std::sync::OnceLock<Option<fontdue::Font>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            load_ui_font_bytes().and_then(|b| {
                fontdue::Font::from_bytes(b.as_slice(), fontdue::FontSettings::default()).ok()
            })
        })
        .as_ref()
}

/// Xanh Mono for tabular Chronicle copy (run receipt columns, KPI values, chart ticks).
pub fn load_mono_font() -> Option<&'static fontdue::Font> {
    static CACHE: std::sync::OnceLock<Option<fontdue::Font>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let path = "fonts/Xanh_Mono/XanhMono-Regular.ttf";
            crate::asset_path::get(path).and_then(|file| {
                fontdue::Font::from_bytes(file.data.as_ref(), fontdue::FontSettings::default()).ok()
            })
        })
        .as_ref()
}

/// Instrument Serif italic for UI (relic inspect flavor, etc.). Falls back to
/// [`load_ui_font`] when the italic file is missing.
pub fn load_ui_font_italic() -> Option<&'static fontdue::Font> {
    static CACHE: std::sync::OnceLock<Option<fontdue::Font>> = OnceLock::new();
    match CACHE.get_or_init(|| {
        let path = "fonts/Instrument_Serif/InstrumentSerif-Italic.ttf";
        if let Some(file) = crate::asset_path::get(path)
            && let Ok(f) =
                fontdue::Font::from_bytes(file.data.as_ref(), fontdue::FontSettings::default())
        {
            return Some(f);
        }
        log::debug!("decal: italic UI font missing at {path}, using regular");
        None
    }) {
        Some(font) => Some(font),
        None => load_ui_font(),
    }
}

/// Horizontal alignment hint for [`rasterize_label_styled`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelAlign {
    Left,
    Center,
    Right,
}

/// Legacy single-line rasterise — keeps the old auto-shrink behaviour so
/// existing call sites (tile-face decals, single-line labels with no explicit
/// `font_px` set) render exactly as before.
pub fn rasterize_label(font: &fontdue::Font, text: &str, width: u32, height: u32) -> Vec<u8> {
    rasterize_label_styled(font, text, width, height, None, LabelAlign::Center)
}

/// Rasterise `text` into a `width × height` RGBA8 bitmap.
///
/// - `font_px = Some(px)` pins the font size — every line in the (possibly
///   multi-line) text is laid out at exactly that pixel size, so a wrapped
///   paragraph reads as a uniform block instead of each line shrink-fitting
///   independently. `None` falls back to the legacy auto-sizing formula
///   `min(height * 0.55, width * 1.5 / chars).max(8.0)`.
/// - `text` may contain `\n` to indicate hard line breaks. Lines stack
///   vertically and `align` controls horizontal placement of each line
///   within the rect.
///
/// Uses fontdue's per-glyph metrics for correct baseline and bearing placement so
/// that descenders (g, p, y …) sit below the baseline while ascenders (h, f, k …)
/// rise above it.  Horizontal advance widths are used for spacing (fontdue does not
/// expose kerning tables, so letter-spacing is at the "natural" advance level).
pub fn rasterize_label_styled(
    font: &fontdue::Font,
    text: &str,
    width: u32,
    height: u32,
    font_px: Option<f32>,
    align: LabelAlign,
) -> Vec<u8> {
    rasterize_label_styled_with_fallback(
        font,
        None,
        text,
        width,
        height,
        LabelStyle {
            font_px,
            align,
            scroll_offset: 0.0,
            underline: false,
            baseline_shift_px: 0.0,
        },
    )
}

/// Layout options for `rasterize_label_styled_with_fallback`: explicit font
/// size (or auto-fit when `None`), alignment, horizontal scroll offset, and
/// optional underline for the plain (single-face) raster path.
pub struct LabelStyle {
    pub font_px: Option<f32>,
    pub align: LabelAlign,
    pub scroll_offset: f32,
    pub underline: bool,
    /// Single-line path: added to baseline_y (negative moves ink up in bitmap).
    pub baseline_shift_px: f32,
}

/// Like [`rasterize_label_styled`] but with an optional emoji fallback font.
/// Characters missing from the primary font are rasterized with the fallback.
pub fn rasterize_label_styled_with_fallback(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    width: u32,
    height: u32,
    style: LabelStyle,
) -> Vec<u8> {
    let LabelStyle {
        font_px: font_px_opt,
        align,
        scroll_offset,
        underline,
        baseline_shift_px,
    } = style;
    // Multi-line: lay out each line at the same font size, stacked vertically.
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > 1 {
        return rasterize_block(
            &DecalFonts {
                regular: font,
                italic: None,
                emoji: emoji_font,
            },
            &lines,
            &LabelBlockRasterParams {
                width,
                height,
                font_px: font_px_opt,
                align,
                underline,
            },
        );
    }

    // Single-line fast path retains the historical centring behaviour.
    let font_px = resolve_label_font_px(font, emoji_font, text, width, height, font_px_opt, 8.0);

    let chars: Vec<char> = text.chars().collect();

    // Measure every glyph once to figure out the total advance and the maximum
    // ascender height (for vertical centring of the baseline).
    let glyphs: Vec<GlyphData> = chars
        .iter()
        .map(|&ch| {
            let use_font = pick_font(font, emoji_font, ch);
            let (metrics, bitmap) = use_font.rasterize(ch, font_px);
            GlyphData { metrics, bitmap }
        })
        .collect();

    let total_advance: f32 = glyphs.iter().map(|g| g.metrics.advance_width).sum();

    // When `font_px` is pinned, centre using the font's line box (same idea as
    // [`rasterize_block`]) so every substring shares one baseline. Per-string
    // glyph-extent centring would shift short runs (coloured keyword splits)
    // vertically relative to their neighbours.
    let baseline_y = (if font_px_opt.is_some() {
        if let Some(lm) = font.horizontal_line_metrics(font_px) {
            let line_h = lm.new_line_size.max(1.0);
            ((height as f32 - line_h) * 0.5).max(0.0) + lm.ascent
        } else {
            single_line_baseline_from_glyphs(&glyphs, height)
        }
    } else {
        single_line_baseline_from_glyphs(&glyphs, height)
    }) + baseline_shift_px;

    // Horizontal start depends on alignment, then shifted by scroll_offset.
    let start_x = match align {
        LabelAlign::Left => 0.0,
        LabelAlign::Center => (width as f32 - total_advance) * 0.5,
        LabelAlign::Right => width as f32 - total_advance,
    } - scroll_offset;

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    blit_line(&glyphs, &mut rgba, width, height, start_x, baseline_y);
    if underline {
        draw_underline_span(
            &mut rgba,
            width,
            height,
            start_x,
            start_x + total_advance,
            baseline_y,
            font_px,
        );
    }
    rgba
}

/// Primary / optional italic / emoji faces used by CPU label raster paths.
pub struct DecalFonts<'a> {
    pub regular: &'a fontdue::Font,
    pub italic: Option<&'a fontdue::Font>,
    pub emoji: Option<&'a fontdue::Font>,
}

/// Bitmap size and layout for [`rasterize_block`].
pub struct LabelBlockRasterParams {
    pub width: u32,
    pub height: u32,
    pub font_px: Option<f32>,
    pub align: LabelAlign,
    pub underline: bool,
}

/// Bitmap size and layout for mixed-style span rasterizers.
pub struct LabelRasterParams {
    pub width: u32,
    pub height: u32,
    pub font_px: f32,
    pub align: LabelAlign,
}

/// Multi-line block layout. Lines are stacked vertically, all rendered at the
/// same `font_px` so the paragraph reads as a coherent block.
fn rasterize_block(
    fonts: &DecalFonts<'_>,
    lines: &[&str],
    params: &LabelBlockRasterParams,
) -> Vec<u8> {
    let LabelBlockRasterParams {
        width,
        height,
        font_px,
        align,
        underline,
    } = *params;
    // Pinned sizes shrink uniformly when the block would overflow; unpinned
    // sizes derive from height / line_count (legacy).
    let derived = font_px
        .unwrap_or_else(|| (height as f32 * 0.55 / lines.len() as f32).max(8.0))
        .max(8.0);
    let font_px = if font_px.is_some() {
        fit_label_block_font_px(
            fonts.regular,
            fonts.emoji,
            lines,
            width,
            height,
            derived,
            8.0,
        )
    } else {
        derived
    };

    // Per-line layout: rasterise glyphs once, measure the widest, compute a
    // shared baseline step.
    struct LineGlyphs {
        glyphs: Vec<(fontdue::Metrics, Vec<u8>)>,
        advance: f32,
    }
    let measured: Vec<LineGlyphs> = lines
        .iter()
        .map(|line| {
            let glyphs: Vec<(fontdue::Metrics, Vec<u8>)> = line
                .chars()
                .map(|ch| pick_font(fonts.regular, fonts.emoji, ch).rasterize(ch, font_px))
                .collect();
            let advance: f32 = glyphs.iter().map(|(m, _)| m.advance_width).sum();
            LineGlyphs { glyphs, advance }
        })
        .collect();

    // Use font.line_metrics for a stable line height across lines.
    let line_metrics = fonts.regular.horizontal_line_metrics(font_px);
    let (line_h, ascender_px) = if let Some(lm) = line_metrics {
        (lm.new_line_size, lm.ascent)
    } else {
        (font_px * 1.2, font_px * 0.8)
    };

    let total_h = line_h * lines.len() as f32;
    let block_top = ((height as f32 - total_h) * 0.5).max(0.0);

    let mut rgba = vec![0u8; (width * height * 4) as usize];

    for (i, line) in measured.iter().enumerate() {
        let baseline_y = block_top + i as f32 * line_h + ascender_px;
        let start_x = match align {
            LabelAlign::Left => 0.0,
            LabelAlign::Center => (width as f32 - line.advance) * 0.5,
            LabelAlign::Right => width as f32 - line.advance,
        };
        // Convert to the (metrics, Vec<u8>) shape blit_line expects via a
        // temporary GlyphData wrapper.
        let glyph_view: Vec<GlyphRef> = line
            .glyphs
            .iter()
            .map(|(m, b)| GlyphRef {
                metrics: *m,
                bitmap: b,
            })
            .collect();
        blit_line_refs(&glyph_view, &mut rgba, width, height, start_x, baseline_y);
        if underline {
            draw_underline_span(
                &mut rgba,
                width,
                height,
                start_x,
                start_x + line.advance,
                baseline_y,
                font_px,
            );
        }
    }

    rgba
}

/// Pick the best font for `ch`: use the primary font if it has the glyph,
/// otherwise fall back to the emoji font (if provided).
/// Three-tier glyph fallback used by every decal text path:
/// 1. `primary` — the caller's font (Instrument Serif for plaque copy).
/// 2. **Noto Sans** (auto-loaded via [`noto_sans_symbols_font`]) — covers Greek (π),
///    Cyrillic, plus most BMP punctuation / math operators (≈, ×, ÷, …).
/// 3. `fallback` — Noto Emoji for emoji codepoints (passed by the caller as today).
///
/// `&'static` references coerce to the caller's `'a` automatically, so adding the symbols tier
/// keeps the existing `pick_font(primary, emoji, ch)` call signature intact at every call site.
fn pick_font<'a>(
    primary: &'a fontdue::Font,
    fallback: Option<&'a fontdue::Font>,
    ch: char,
) -> &'a fontdue::Font {
    if primary.has_glyph(ch) {
        return primary;
    }
    if let Some(symbols) = noto_sans_symbols_font()
        && symbols.has_glyph(ch)
    {
        return symbols;
    }
    if let Some(fb) = fallback {
        return fb;
    }
    primary
}

struct GlyphData {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

/// Vertical centre for a single raster line from measured glyph extents
/// (auto-shrink labels and pinned-size fallback when line metrics are missing).
fn single_line_baseline_from_glyphs(glyphs: &[GlyphData], height: u32) -> f32 {
    let ascender_px: f32 = glyphs
        .iter()
        .map(|g| g.metrics.height as f32 + g.metrics.ymin as f32)
        .fold(0.0_f32, f32::max);
    let descender_px: f32 = glyphs
        .iter()
        .map(|g| (-g.metrics.ymin as f32).max(0.0))
        .fold(0.0_f32, f32::max);
    let text_block_h = ascender_px + descender_px;
    (height as f32 - text_block_h) * 0.5 + ascender_px
}

/// 1px-thick faux underline under `[x0, x1)` at a small offset below the baseline.
fn draw_underline_span(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    x0: f32,
    x1: f32,
    baseline_y: f32,
    font_px: f32,
) {
    let iy = (baseline_y + (font_px * 0.12).clamp(1.5, 6.0)) as i32;
    if iy < 0 || iy >= height as i32 {
        return;
    }
    let row = iy as u32;
    let x_start = x0.floor().max(0.0) as i32;
    let x_end = x1.ceil().min(width as f32) as i32;
    for px in x_start..x_end {
        if px < 0 || px >= width as i32 {
            continue;
        }
        let di = ((row * width + px as u32) * 4) as usize;
        if di + 3 >= rgba.len() {
            continue;
        }
        rgba[di] = rgba[di].max(240);
        rgba[di + 1] = rgba[di + 1].max(240);
        rgba[di + 2] = rgba[di + 2].max(240);
        rgba[di + 3] = rgba[di + 3].saturating_add(220);
    }
}

struct GlyphRef<'a> {
    metrics: fontdue::Metrics,
    bitmap: &'a [u8],
}

fn blit_line(
    glyphs: &[GlyphData],
    rgba: &mut [u8],
    width: u32,
    height: u32,
    start_x: f32,
    baseline_y: f32,
) {
    let mut cx = start_x;
    for g in glyphs {
        if !g.bitmap.is_empty() {
            let glyph_left = (cx + g.metrics.xmin as f32) as i32;
            let glyph_top = (baseline_y - (g.metrics.ymin as f32 + g.metrics.height as f32)) as i32;
            blit_glyph(
                GlyphSrc {
                    bitmap: &g.bitmap,
                    gw: g.metrics.width,
                    gh: g.metrics.height,
                    left: glyph_left,
                    top: glyph_top,
                },
                rgba,
                width,
                height,
            );
        }
        cx += g.metrics.advance_width;
    }
}

fn blit_line_refs(
    glyphs: &[GlyphRef<'_>],
    rgba: &mut [u8],
    width: u32,
    height: u32,
    start_x: f32,
    baseline_y: f32,
) {
    let mut cx = start_x;
    for g in glyphs {
        if !g.bitmap.is_empty() {
            let glyph_left = (cx + g.metrics.xmin as f32) as i32;
            let glyph_top = (baseline_y - (g.metrics.ymin as f32 + g.metrics.height as f32)) as i32;
            blit_glyph(
                GlyphSrc {
                    bitmap: g.bitmap,
                    gw: g.metrics.width,
                    gh: g.metrics.height,
                    left: glyph_left,
                    top: glyph_top,
                },
                rgba,
                width,
                height,
            );
        }
        cx += g.metrics.advance_width;
    }
}

/// Source glyph for `blit_glyph`: coverage bitmap plus its pixel extent.
struct GlyphSrc<'a> {
    bitmap: &'a [u8],
    gw: usize,
    gh: usize,
    left: i32,
    top: i32,
}

fn blit_glyph(src: GlyphSrc<'_>, rgba: &mut [u8], width: u32, height: u32) {
    let GlyphSrc {
        bitmap,
        gw,
        gh,
        left: glyph_left,
        top: glyph_top,
    } = src;
    for row in 0..gh as i32 {
        for col in 0..gw as i32 {
            let px = glyph_left + col;
            let py = glyph_top + row;
            if px < 0 || px >= width as i32 || py < 0 || py >= height as i32 {
                continue;
            }
            let src = (row as usize) * gw + col as usize;
            let dst = ((py as u32 * width + px as u32) * 4) as usize;
            let v = bitmap[src];
            rgba[dst] = 255;
            rgba[dst + 1] = 255;
            rgba[dst + 2] = 255;
            rgba[dst + 3] = rgba[dst + 3].saturating_add(v);
        }
    }
}

// --- Mixed-style text (bold / italic / underline, bottom-aligned block) ---

/// One text span for CPU raster (UI styled labels or relic flavor).
#[derive(Clone, Copy, Debug)]
pub struct RasterStyleSpan<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Horizontal offset (px) for the second faux-bold pass; layout advances use
/// [`fontdue::Metrics::advance_width`] only — wrap width must not add this.
pub const FAUX_BOLD_OVERLAY_OFFSET_PX: f32 = 0.65;

#[derive(Clone, Copy)]
struct FlavorCell {
    ch: char,
    bold: bool,
    italic: bool,
    underline: bool,
}

fn pick_face_for_flavor<'a>(
    regular: &'a fontdue::Font,
    italic: &'a fontdue::Font,
    emoji: Option<&'a fontdue::Font>,
    ch: char,
    use_italic: bool,
) -> &'a fontdue::Font {
    if let Some(e) = emoji
        && !regular.has_glyph(ch)
        && e.has_glyph(ch)
    {
        return e;
    }
    if use_italic && italic.has_glyph(ch) {
        return italic;
    }
    regular
}

fn flavor_italic_face<'a>(fonts: &DecalFonts<'a>) -> &'a fontdue::Font {
    fonts.italic.unwrap_or(fonts.regular)
}

fn flavor_cell_advance(fonts: &DecalFonts<'_>, c: FlavorCell, font_px: f32) -> f32 {
    let italic = flavor_italic_face(fonts);
    let face = pick_face_for_flavor(fonts.regular, italic, fonts.emoji, c.ch, c.italic);
    face.metrics(c.ch, font_px).advance_width
}

fn flavor_line_advance(fonts: &DecalFonts<'_>, line: &[FlavorCell], font_px: f32) -> f32 {
    line.iter()
        .map(|c| flavor_cell_advance(fonts, *c, font_px))
        .sum()
}

fn tokenize_flavor_cells(cells: &[FlavorCell]) -> Vec<&[FlavorCell]> {
    if cells.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<&[FlavorCell]> = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        let ws = cells[i].ch.is_whitespace();
        let mut j = i + 1;
        while j < cells.len() && cells[j].ch.is_whitespace() == ws {
            j += 1;
        }
        out.push(&cells[i..j]);
        i = j;
    }
    out
}

fn trim_trailing_flavor_ws(line: &mut Vec<FlavorCell>) {
    while line.last().is_some_and(|c| c.ch.is_whitespace()) {
        line.pop();
    }
}

fn wrap_flavor_hard_line(
    fonts: &DecalFonts<'_>,
    cells: &[FlavorCell],
    max_w: f32,
    font_px: f32,
) -> Vec<Vec<FlavorCell>> {
    let space_w = fonts.regular.metrics(' ', font_px).advance_width;
    let tokens = tokenize_flavor_cells(cells);
    if tokens.is_empty() {
        return vec![Vec::new()];
    }
    let mut lines: Vec<Vec<FlavorCell>> = Vec::new();
    let mut line: Vec<FlavorCell> = Vec::new();
    let mut line_w = 0.0_f32;

    for tok in &tokens {
        let is_ws = tok[0].ch.is_whitespace();
        let tw = flavor_line_advance(fonts, tok, font_px);
        if is_ws {
            if line.is_empty() {
                continue;
            }
            if line_w + tw <= max_w {
                line.extend_from_slice(tok);
                line_w += tw;
            }
            continue;
        }
        if line.is_empty() {
            line.extend_from_slice(tok);
            line_w = tw;
            continue;
        }
        let gap = if line.last().is_some_and(|c| c.ch.is_whitespace()) {
            0.0
        } else {
            space_w
        };
        if line_w + gap + tw <= max_w {
            if gap > 0.0 {
                line.push(FlavorCell {
                    ch: ' ',
                    bold: false,
                    italic: false,
                    underline: false,
                });
                line_w += space_w;
            }
            line.extend_from_slice(tok);
            line_w += tw;
        } else {
            trim_trailing_flavor_ws(&mut line);
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line.extend_from_slice(tok);
            line_w = tw;
        }
    }
    trim_trailing_flavor_ws(&mut line);
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn flatten_raster_style_to_hard_lines(spans: &[RasterStyleSpan<'_>]) -> Vec<Vec<FlavorCell>> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<Vec<FlavorCell>> = vec![Vec::new()];
    for sp in spans {
        let mut first = true;
        for segment in sp.text.split('\n') {
            if !first {
                lines.push(Vec::new());
            }
            first = false;
            let cur = lines.last_mut().unwrap();
            for ch in segment.chars() {
                cur.push(FlavorCell {
                    ch,
                    bold: sp.bold,
                    italic: sp.italic,
                    underline: sp.underline,
                });
            }
        }
    }
    lines
}

/// Destination bitmap + faces for mixed-style flavor glyph blits.
struct FlavorBlitCtx<'a> {
    rgba: &'a mut [u8],
    width: u32,
    height: u32,
    fonts: &'a DecalFonts<'a>,
    font_px: f32,
}

fn blit_one_flavor_glyph(
    ctx: &mut FlavorBlitCtx<'_>,
    cx: f32,
    baseline_y: f32,
    m: fontdue::Metrics,
    bitmap: &[u8],
    bold: bool,
) {
    let mut draw = |ox: f32| {
        if bitmap.is_empty() {
            return;
        }
        let glyph_left = (cx + ox + m.xmin as f32) as i32;
        let glyph_top = (baseline_y - (m.ymin as f32 + m.height as f32)) as i32;
        blit_glyph(
            GlyphSrc {
                bitmap,
                gw: m.width,
                gh: m.height,
                left: glyph_left,
                top: glyph_top,
            },
            ctx.rgba,
            ctx.width,
            ctx.height,
        );
    };
    draw(0.0);
    if bold {
        draw(FAUX_BOLD_OVERLAY_OFFSET_PX);
    }
}

fn blit_flavor_line(
    ctx: &mut FlavorBlitCtx<'_>,
    line: &[FlavorCell],
    start_x: f32,
    baseline_y: f32,
) {
    let italic = flavor_italic_face(ctx.fonts);
    let mut cx = start_x;
    let mut u_start: Option<f32> = None;
    for c in line {
        if c.underline {
            if u_start.is_none() {
                u_start = Some(cx);
            }
        } else if let Some(sx) = u_start.take() {
            draw_underline_span(
                ctx.rgba,
                ctx.width,
                ctx.height,
                sx,
                cx,
                baseline_y,
                ctx.font_px,
            );
        }
        let face = pick_face_for_flavor(ctx.fonts.regular, italic, ctx.fonts.emoji, c.ch, c.italic);
        let (m, bmp) = face.rasterize(c.ch, ctx.font_px);
        blit_one_flavor_glyph(ctx, cx, baseline_y, m, &bmp, c.bold);
        cx += m.advance_width;
    }
    if let Some(sx) = u_start {
        draw_underline_span(
            ctx.rgba,
            ctx.width,
            ctx.height,
            sx,
            cx,
            baseline_y,
            ctx.font_px,
        );
    }
}

/// Multi-line mixed-style text (bold / italic / underline) at a fixed `font_px`,
/// bottom-aligned in `height`. Used by relic flavor and UI styled labels.
pub fn rasterize_label_raster_spans(
    fonts: &DecalFonts<'_>,
    spans: &[RasterStyleSpan<'_>],
    params: &LabelRasterParams,
) -> Vec<u8> {
    let LabelRasterParams {
        width,
        height,
        font_px,
        align,
    } = *params;
    if spans.is_empty() {
        return vec![0u8; (width * height * 4) as usize];
    }
    let font_px = font_px.max(8.0);
    let hard_lines = flatten_raster_style_to_hard_lines(spans);
    let max_w = width as f32;
    let mut soft_lines: Vec<Vec<FlavorCell>> = Vec::new();
    for hl in &hard_lines {
        if hl.is_empty() {
            soft_lines.push(Vec::new());
            continue;
        }
        let mut wrapped = wrap_flavor_hard_line(fonts, hl, max_w, font_px);
        soft_lines.append(&mut wrapped);
    }
    if soft_lines.is_empty() {
        return vec![0u8; (width * height * 4) as usize];
    }
    let line_metrics = fonts.regular.horizontal_line_metrics(font_px);
    let (line_h, ascender_px) = if let Some(lm) = line_metrics {
        (lm.new_line_size, lm.ascent)
    } else {
        (font_px * 1.2, font_px * 0.8)
    };
    let total_h = line_h * soft_lines.len() as f32;
    let block_top = (height as f32 - total_h).max(0.0);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (i, line) in soft_lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let baseline_y = block_top + i as f32 * line_h + ascender_px;
        let adv = flavor_line_advance(fonts, line, font_px);
        let start_x = match align {
            LabelAlign::Left => 0.0,
            LabelAlign::Center => (width as f32 - adv) * 0.5,
            LabelAlign::Right => width as f32 - adv,
        };
        let mut ctx = FlavorBlitCtx {
            rgba: &mut rgba,
            width,
            height,
            fonts,
            font_px,
        };
        blit_flavor_line(&mut ctx, line, start_x, baseline_y);
    }
    rgba
}

/// Multi-line relic inspect flavor: mixed regular/italic and faux-bold at a
/// fixed `font_px`, bottom-aligned in `height`.
pub fn rasterize_label_flavor_spans(
    fonts: &DecalFonts<'_>,
    spans: &[RelicFlavorSpan],
    params: &LabelRasterParams,
) -> Vec<u8> {
    if spans.is_empty() {
        let width = params.width;
        let height = params.height;
        return vec![0u8; (width * height * 4) as usize];
    }
    let mapped: Vec<RasterStyleSpan> = spans
        .iter()
        .map(|s| RasterStyleSpan {
            text: s.text,
            bold: s.bold,
            italic: s.italic,
            underline: false,
        })
        .collect();
    rasterize_label_raster_spans(fonts, &mapped, params)
}

/// Compute per-character advance widths for text rendered in a rect.
///
/// Returns `(font_px, start_x_offset, per_char_advances)` using the same
/// font sizing logic as [`rasterize_label`], so positions match the rendered
/// output exactly.
pub fn measure_label_advances(
    font: &fontdue::Font,
    text: &str,
    width: u32,
    height: u32,
    font_px: Option<f32>,
) -> (f32, f32, Vec<f32>) {
    let char_count = text.chars().count().max(1) as f32;
    // Mirror `rasterize_label_styled`: honour a pinned font size when the
    // caller supplied one, otherwise use the auto-shrink formula. Without
    // this, glossary-term underlines on labels that pin `font_px` (e.g.
    // the gameplay score header) get computed at the wrong glyph size and
    // drift away from the rendered text.
    let font_px = match font_px {
        Some(px) => px.max(8.0),
        None => (height as f32 * 0.55)
            .min(width as f32 * 1.5 / char_count)
            .max(8.0),
    };

    let advances: Vec<f32> = text
        .chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .collect();

    let total: f32 = advances.iter().sum();
    let start_x = (width as f32 - total) * 0.5;

    (font_px, start_x, advances)
}

/// Load Instrument Serif for game UI text.
///
/// Resolution order:
/// 1. Embedded Instrument Serif (the primary serif used everywhere).
/// 2. Embedded Noto Sans (a fallback for missing serif glyphs — only used
///    if the user has dropped a Noto Sans TTF into `assets/fonts/Noto_Sans/`).
/// 3. System fonts (last-ditch fallback for unbundled dev builds).
///
/// Cached so the bytes are only resolved once.
pub fn load_ui_font_bytes() -> Option<Vec<u8>> {
    static CACHE: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            // Primary: Instrument Serif.
            let primary = ["fonts/Instrument_Serif/InstrumentSerif-Regular.ttf"];
            for path in primary {
                if let Some(file) = crate::asset_path::get(path) {
                    log::debug!("decal: loaded UI font from embedded {path}");
                    return Some(file.data.to_vec());
                }
            }
            // Embedded Noto Sans fallback — only present if a TTF has been
            // dropped into assets/fonts/Noto_Sans/. Tries the variable-font name
            // first, then a static Regular.
            let noto_sans = [
                "fonts/Noto_Sans/NotoSans-VariableFont_wdth,wght.ttf",
                "fonts/Noto_Sans/NotoSans-Regular.ttf",
                "fonts/Noto_Sans/static/NotoSans-Regular.ttf",
            ];
            for path in noto_sans {
                if let Some(file) = crate::asset_path::get(path) {
                    log::debug!("decal: loaded UI font from embedded {path} (Noto Sans fallback)");
                    return Some(file.data.to_vec());
                }
            }
            // System fallbacks for non-bundled environments.
            let system = [
                "/System/Library/Fonts/Helvetica.ttc",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "C:\\Windows\\Fonts\\segoeui.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
            ];
            for path in system {
                if let Ok(bytes) = std::fs::read(path) {
                    log::debug!("decal: loaded UI font from system {path}");
                    return Some(bytes);
                }
            }
            log::warn!("decal: UI font not found.");
            None
        })
        .clone()
}

/// Unified entry point for decal rasterization used by the generic
/// [`crate::render::draw_cmd::Object3dKind::Primitive`] dispatch.
/// Dispatches to the existing layout-specialized rasterizers below.
pub fn rasterize_decal(
    spec: &crate::render::primitive::DecalSpec,
    width: u32,
    height: u32,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
) -> Vec<u8> {
    use crate::render::primitive::DecalLayout;
    match &spec.layout {
        DecalLayout::Fit { .. } => {
            rasterize_plaque_decal(&spec.text, ui_font, emoji_font, width, height)
        }
        DecalLayout::TitleRule { .. } => {
            // Ofuda split: text is "title\nrule" by convention.
            let (title, rule) = match spec.text.split_once('\n') {
                Some((t, r)) => (t, r),
                None => (spec.text.as_str(), ""),
            };
            let _ = emoji_font;
            rasterize_ofuda_decal(title, rule, ui_font, width, height)
        }
    }
}

/// Compute the decal texture dimensions for a [`DecalLayout`] given the
/// host object's world-space extents. Mirrors the per-kind sizing math
/// in the legacy dispatch arms ([wgpu_renderer.rs:7868-7872] for
/// plaque, [`CABINET_DECAL_CELL_W`] × 6 for cabinet, etc.).
pub fn decal_dimensions(
    layout: &crate::render::primitive::DecalLayout,
    extents: [f32; 3],
) -> (u32, u32) {
    use crate::render::primitive::DecalLayout;
    match layout {
        DecalLayout::Fit { target_short_edge } => {
            let h = *target_short_edge;
            let face_aspect = (extents[0] / extents[1].max(1.0)).clamp(0.5, 12.0);
            let w = ((h as f32 * face_aspect).round() as u32).clamp(256, 4096);
            (w, h)
        }
        DecalLayout::TitleRule { target_short_edge } => {
            // Ofuda is authored landscape; short edge is height.
            let h = *target_short_edge;
            let face_aspect = (extents[0] / extents[1].max(1.0)).clamp(0.5, 12.0);
            let w = ((h as f32 * face_aspect).round() as u32).clamp(256, 4096);
            (w, h)
        }
    }
}

#[cfg(test)]
mod atlas_parser_tests {
    use super::parse_atlas_toml;

    #[test]
    fn preserves_empty_layout_slots_so_indices_align() {
        // Two empty slots after DWhite push the flowers onto the next row.
        // If the parser dropped them, Flower1 would sit at index 7 instead
        // of 9, and the atlas crop would pull the wrong cell.
        let src = r#"
image = "atlas.png"
tile_width = 256
tile_height = 384
columns = 9

layout = [
    "EWind","SWind","WWind","NWind","DRed","DGreen","DWhite","","",
    "Flower1","Flower2",
]
"#;
        let (tw, th, cols, layout) = parse_atlas_toml(src).unwrap();
        assert_eq!((tw, th, cols), (256, 384, 9));
        assert_eq!(layout.len(), 11);
        assert_eq!(layout[6], "DWhite");
        assert_eq!(layout[7], "");
        assert_eq!(layout[8], "");
        assert_eq!(layout[9], "Flower1");
        assert_eq!(layout[10], "Flower2");
    }

    #[test]
    fn handles_single_line_layout() {
        let src = r#"tile_width = 10
tile_height = 20
columns = 3
layout = ["A","B","","C"]
"#;
        let (_, _, _, layout) = parse_atlas_toml(src).unwrap();
        assert_eq!(layout, vec!["A", "B", "", "C"]);
    }
}
