//! CPU rasterisation of per-tile Unicode decals.
//!
//! Each Mahjong tile maps to a codepoint in the Unicode Mahjong Tile block
//! (U+1F000–U+1F021).  We try to rasterise that glyph from a system or
//! project-local font; if the font doesn't carry those emoji outlines we fall
//! back to a short ASCII notation ("1m", "5p", "E", …).
//!
//! To get the real Unicode tile glyphs, drop any TTF/OTF that covers the
//! Mahjong block (e.g. Noto Emoji) at `assets/font.ttf` in the project root.

use crate::core::tile::{Suit, Tile};

/// Unicode codepoint for a tile's Mahjong Tile block character.
///
/// Block layout:
///   Winds  East–North : U+1F000–U+1F003  (rank 1–4)
///   Dragons Chun/Hatsu/Haku: U+1F004–U+1F006 (rank 1–3)
///   Man 1–9           : U+1F007–U+1F00F  (rank 1–9)
///   Sou 1–9           : U+1F010–U+1F018  (rank 1–9)
///   Pin 1–9           : U+1F019–U+1F021  (rank 1–9)
/// Public so the renderer can access it for 2D tile labels.
pub fn tile_codepoint(tile: &Tile) -> u32 {
    match tile.suit {
        Suit::Wind => 0x1F000 + (tile.rank as u32 - 1),
        Suit::Dragon => 0x1F004 + (tile.rank as u32 - 1),
        Suit::Characters => 0x1F006 + tile.rank as u32,
        Suit::Bamboos => 0x1F00F + tile.rank as u32,
        Suit::Circles => 0x1F018 + tile.rank as u32,
    }
}

/// Rasterise a `size×size` RGBA8 decal for `tile`.
///
/// Prefers the full Unicode tile glyph if the loaded font supports it,
/// then falls back to a short ASCII label.
pub fn rasterize_tile_decal(tile: &Tile, size: u32) -> Vec<u8> {
    let font_bytes = load_noto_emoji_bytes();
    let font = font_bytes
        .as_deref()
        .and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok());

    if let Some(ref f) = font {
        // Try the actual Unicode Mahjong codepoint first.
        let cp = tile_codepoint(tile);
        if let Some(ch) = char::from_u32(cp) {
            let (metrics, bitmap) = f.rasterize(ch, size as f32 * 0.82);
            if !bitmap.is_empty() && metrics.width > 0 {
                return bitmap_to_rgba_centered(&bitmap, metrics.width, metrics.height, size);
            }
        }
    }

    // Fallback: ASCII short-name using the UI font (Noto Emoji has no ASCII outlines).
    let label = tile_short_label(tile);
    if let Some(ref f) = load_ui_font() {
        return rasterize_text(f, &label, size);
    }
    // No font found at all: transparent placeholder.
    vec![0u8; (size * size * 4) as usize]
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Short ASCII label used when the Unicode glyph isn't available.
pub fn tile_short_label(tile: &Tile) -> String {
    match tile.suit {
        // 🎴 Characters (man) — use the rank number
        Suit::Characters => format!("{}", tile.rank),
        // 🎋 Bamboos (sou) — use the rank number
        Suit::Bamboos => format!("{}", tile.rank),
        // ⭕ Circles (pin) — use the rank number
        Suit::Circles => format!("{}", tile.rank),
        Suit::Wind => match tile.rank {
            1 => "E",
            2 => "S",
            3 => "W",
            4 => "N",
            _ => "?",
        }
        .to_string(),
        Suit::Dragon => match tile.rank {
            1 => "Chun",
            2 => "Hatsu",
            3 => "Haku",
            _ => "?",
        }
        .to_string(),
    }
}

/// Emoji indicator for each suit, rendered below the main label.
pub fn tile_suit_emoji(tile: &Tile) -> &'static str {
    match tile.suit {
        Suit::Characters => "\u{1F3B4}",  // 🎴 flower card
        Suit::Bamboos => "\u{1F38B}",     // 🎋 tanabata tree / bamboo
        Suit::Circles => "\u{1F534}",     // 🔴 red circle / disc
        Suit::Wind => "\u{1F32C}",        // 🌬 wind face
        Suit::Dragon => "\u{1F409}",      // 🐉 dragon
    }
}

/// Convert a fontdue greyscale alpha bitmap to a centred white RGBA8 image.
fn bitmap_to_rgba_centered(bitmap: &[u8], gw: usize, gh: usize, size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let ox = ((size as i32 - gw as i32) / 2).max(0) as u32;
    let oy = ((size as i32 - gh as i32) / 2).max(0) as u32;
    for y in 0..gh as u32 {
        for x in 0..gw as u32 {
            let px = ox + x;
            let py = oy + y;
            if px < size && py < size {
                let src = (y as usize * gw + x as usize) as usize;
                let dst = ((py * size + px) * 4) as usize;
                let a = bitmap[src];
                rgba[dst] = 0;
                rgba[dst + 1] = 0;
                rgba[dst + 2] = 0;
                rgba[dst + 3] = a;
            }
        }
    }
    rgba
}

/// Render `text` (left-to-right) centred in a `size×size` white RGBA8 image.
fn rasterize_text(font: &fontdue::Font, text: &str, size: u32) -> Vec<u8> {
    let font_px = size as f32 * 0.45;
    let chars: Vec<char> = text.chars().collect();

    // Measure total advance width.
    let advances: Vec<f32> = chars
        .iter()
        .map(|&ch| {
            let (m, _) = font.rasterize(ch, font_px);
            m.advance_width
        })
        .collect();
    let total_w: f32 = advances.iter().sum();

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let mut cx = (size as f32 - total_w) * 0.5;
    let base_y = size as f32 * 0.5;

    for (&ch, &adv) in chars.iter().zip(advances.iter()) {
        let (metrics, bitmap) = font.rasterize(ch, font_px);
        if bitmap.is_empty() {
            cx += adv;
            continue;
        }
        let glyph_top = base_y - metrics.height as f32 * 0.5;
        for y in 0..metrics.height as u32 {
            for x in 0..metrics.width as u32 {
                let px = (cx + x as f32) as i32;
                let py = (glyph_top + y as f32) as i32;
                if px >= 0 && px < size as i32 && py >= 0 && py < size as i32 {
                    let src = y as usize * metrics.width + x as usize;
                    let dst = ((py as u32 * size + px as u32) * 4) as usize;
                    let v = bitmap[src];
                    rgba[dst] = 0;
                    rgba[dst + 1] = 0;
                    rgba[dst + 2] = 0;
                    // Accumulate so overlapping glyphs blend nicely.
                    rgba[dst + 3] = rgba[dst + 3].saturating_add(v);
                }
            }
        }
        cx += adv;
    }
    rgba
}

/// Load Noto Emoji for tile decals (covers U+1F000–U+1F02B as outline glyphs).
fn load_noto_emoji_bytes() -> Option<Vec<u8>> {
    let assets = crate::asset_path::assets_dir();
    let variable = assets.join("Noto_Emoji/NotoEmoji-VariableFont_wght.ttf");
    let regular = assets.join("Noto_Emoji/static/NotoEmoji-Regular.ttf");
    let candidates: Vec<String> = vec![
        variable.to_string_lossy().into_owned(),
        regular.to_string_lossy().into_owned(),
    ];
    let refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    load_first(&refs, "Noto Emoji")
}

/// Load Noto Emoji as a ready-to-use `fontdue::Font` (for tile symbols).
pub fn load_noto_emoji_font() -> Option<fontdue::Font> {
    load_noto_emoji_bytes()
        .as_ref()
        .and_then(|b| fontdue::Font::from_bytes(b.as_slice(), fontdue::FontSettings::default()).ok())
}

/// Load the UI font and return a ready-to-use `fontdue::Font`.
pub fn load_ui_font() -> Option<fontdue::Font> {
    load_ui_font_bytes()
        .as_ref()
        .and_then(|b| fontdue::Font::from_bytes(b.as_slice(), fontdue::FontSettings::default()).ok())
}

/// Rasterise `text` into a `width × height` RGBA8 bitmap, centred on the baseline.
///
/// Uses fontdue's per-glyph metrics for correct baseline and bearing placement so
/// that descenders (g, p, y …) sit below the baseline while ascenders (h, f, k …)
/// rise above it.  Horizontal advance widths are used for spacing (fontdue does not
/// expose kerning tables, so letter-spacing is at the "natural" advance level).
pub fn rasterize_label(font: &fontdue::Font, text: &str, width: u32, height: u32) -> Vec<u8> {
    // Target font size: 55 % of the rect height, clamped so the text fits horizontally.
    let font_px = (height as f32 * 0.55)
        .min(width as f32 * 1.5 / text.chars().count().max(1) as f32)
        .max(8.0);

    let chars: Vec<char> = text.chars().collect();

    // Measure every glyph once to figure out the total advance and the maximum
    // ascender height (for vertical centring of the baseline).
    struct GlyphData {
        metrics: fontdue::Metrics,
        bitmap: Vec<u8>,
    }
    let glyphs: Vec<GlyphData> = chars
        .iter()
        .map(|&ch| {
            let (metrics, bitmap) = font.rasterize(ch, font_px);
            GlyphData { metrics, bitmap }
        })
        .collect();

    let total_advance: f32 = glyphs.iter().map(|g| g.metrics.advance_width).sum();

    // The typographic ascender is the highest point above the baseline.
    // Use the tallest glyph's (height + ymin) as a proxy.
    let ascender_px: f32 = glyphs
        .iter()
        .map(|g| g.metrics.height as f32 + g.metrics.ymin as f32)
        .fold(0.0_f32, f32::max);
    let descender_px: f32 = glyphs
        .iter()
        .map(|g| (-g.metrics.ymin as f32).max(0.0))
        .fold(0.0_f32, f32::max);
    let text_block_h = ascender_px + descender_px;

    // Place the baseline so the text block is vertically centred.
    // In the pixel buffer Y increases downward, while fontdue uses Y-up from baseline.
    let baseline_y = (height as f32 - text_block_h) * 0.5 + ascender_px;

    // Horizontal start: centre the full advance width.
    let start_x = (width as f32 - total_advance) * 0.5;

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let mut cx = start_x;

    for g in &glyphs {
        if !g.bitmap.is_empty() {
            // Top-left corner of this glyph in buffer coordinates.
            // fontdue: origin is the baseline; ymin is signed pixels from baseline to
            // bottom of the glyph bounding box (positive = above baseline).
            let glyph_left = (cx + g.metrics.xmin as f32) as i32;
            let glyph_top = (baseline_y - (g.metrics.ymin as f32 + g.metrics.height as f32)) as i32;

            for row in 0..g.metrics.height as i32 {
                for col in 0..g.metrics.width as i32 {
                    let px = glyph_left + col;
                    let py = glyph_top + row;
                    if px < 0 || px >= width as i32 || py < 0 || py >= height as i32 {
                        continue;
                    }
                    let src = (row as usize) * g.metrics.width + col as usize;
                    let dst = ((py as u32 * width + px as u32) * 4) as usize;
                    let v = g.bitmap[src];
                    rgba[dst] = 255;
                    rgba[dst + 1] = 255;
                    rgba[dst + 2] = 255;
                    rgba[dst + 3] = rgba[dst + 3].saturating_add(v);
                }
            }
        }
        cx += g.metrics.advance_width;
    }
    rgba
}

/// Load Cormorant Garamond for game UI text.
pub fn load_ui_font_bytes() -> Option<Vec<u8>> {
    let assets = crate::asset_path::assets_dir();
    let variable = assets.join("Cormorant_Garamond/CormorantGaramond-VariableFont_wght.ttf");
    let regular = assets.join("Cormorant_Garamond/static/CormorantGaramond-Regular.ttf");
    let mut candidates: Vec<String> = vec![
        variable.to_string_lossy().into_owned(),
        regular.to_string_lossy().into_owned(),
    ];
    // System fallbacks for non-bundled environments.
    candidates.extend([
        "/System/Library/Fonts/Helvetica.ttc".to_owned(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".to_owned(),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_owned(),
        "/usr/share/fonts/TTF/DejaVuSans.ttf".to_owned(),
        "C:\\Windows\\Fonts\\segoeui.ttf".to_owned(),
        "C:\\Windows\\Fonts\\arial.ttf".to_owned(),
    ]);
    let refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    load_first(&refs, "UI font")
}

fn load_first(candidates: &[&str], label: &str) -> Option<Vec<u8>> {
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            log::debug!("decal: loaded {label} from {path}");
            return Some(bytes);
        }
    }
    log::warn!("decal: {label} not found.");
    None
}

