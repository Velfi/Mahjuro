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
        Suit::Characters => "\u{1F3B4}", // 🎴 flower card
        Suit::Bamboos => "\u{1F38B}",    // 🎋 tanabata tree / bamboo
        Suit::Circles => "\u{1F534}",    // 🔴 red circle / disc
        Suit::Wind => "\u{1F32C}",       // 🌬 wind face
        Suit::Dragon => "\u{1F409}",     // 🐉 dragon
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
pub fn rasterize_tile_face_decal(
    tile: &Tile,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let color = tile.suit_color();
    let label = tile_short_label(tile);
    let emoji = tile_suit_emoji(tile);

    // Top half: suit-coloured short label.
    if let Some(font) = ui_font {
        let top_h = (height as f32 * 0.50) as u32;
        let band_top = (height as f32 * 0.05) as u32;
        let band = rasterize_label(font, &label, width, top_h);
        blit_tinted(&band, width, top_h, &mut rgba, width, 0, band_top, color);
    }

    // Bottom half: suit-coloured emoji.
    if let Some(font) = emoji_font {
        let bot_h = (height as f32 * 0.40) as u32;
        let band_top = (height as f32 * 0.55) as u32;
        let band = rasterize_label(font, emoji, width, bot_h);
        blit_tinted(&band, width, bot_h, &mut rgba, width, 0, band_top, color);
    }

    rgba
}

/// Blit a single-channel-in-alpha RGBA source onto an RGBA destination at
/// `(dst_x, dst_y)`, replacing every pixel's RGB with `tint` weighted by source
/// alpha. Used to recolour `rasterize_label`'s white-on-transparent output.
fn blit_tinted(
    src: &[u8],
    sw: u32,
    sh: u32,
    dst: &mut [u8],
    dw: u32,
    dst_x: u32,
    dst_y: u32,
    tint: [f32; 4],
) {
    let r = (tint[0] * 255.0) as u8;
    let g = (tint[1] * 255.0) as u8;
    let b = (tint[2] * 255.0) as u8;
    for y in 0..sh {
        for x in 0..sw {
            let dx = dst_x + x;
            let dy = dst_y + y;
            if dx >= dw {
                continue;
            }
            let si = ((y * sw + x) * 4) as usize;
            let di = ((dy * dw + dx) * 4) as usize;
            if di + 3 >= dst.len() {
                continue;
            }
            let a = src[si + 3];
            if a == 0 {
                continue;
            }
            // Source-over with the new pixel's tint.
            dst[di] = r;
            dst[di + 1] = g;
            dst[di + 2] = b;
            dst[di + 3] = dst[di + 3].saturating_add(a);
        }
    }
}

/// Load Noto Emoji for tile decals (covers U+1F000–U+1F02B as outline glyphs).
/// Cached; uses the compile-time embedded asset.
fn load_noto_emoji_bytes() -> Option<Vec<u8>> {
    static CACHE: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let candidates = [
                "Noto_Emoji/NotoEmoji-VariableFont_wght.ttf",
                "Noto_Emoji/static/NotoEmoji-Regular.ttf",
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

/// Load the UI font and return a ready-to-use `fontdue::Font`.
/// Cached so the font is only parsed once.
pub fn load_ui_font() -> Option<fontdue::Font> {
    static CACHE: std::sync::OnceLock<Option<fontdue::Font>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            load_ui_font_bytes().and_then(|b| {
                fontdue::Font::from_bytes(b.as_slice(), fontdue::FontSettings::default()).ok()
            })
        })
        .clone()
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
) -> (f32, f32, Vec<f32>) {
    let char_count = text.chars().count().max(1) as f32;
    let font_px = (height as f32 * 0.55)
        .min(width as f32 * 1.5 / char_count)
        .max(8.0);

    let advances: Vec<f32> = text
        .chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .collect();

    let total: f32 = advances.iter().sum();
    let start_x = (width as f32 - total) * 0.5;

    (font_px, start_x, advances)
}

/// Load Cormorant Garamond for game UI text.
///
/// Resolution order:
/// 1. Embedded Cormorant Garamond (the primary serif used everywhere).
/// 2. Embedded Noto Sans (a fallback for missing Garamond glyphs — only used
///    if the user has dropped a Noto Sans TTF into `assets/Noto_Sans/`).
/// 3. System fonts (last-ditch fallback for unbundled dev builds).
///
/// Cached so the bytes are only resolved once.
pub fn load_ui_font_bytes() -> Option<Vec<u8>> {
    static CACHE: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            // Primary: Cormorant Garamond.
            let primary = [
                "Cormorant_Garamond/CormorantGaramond-VariableFont_wght.ttf",
                "Cormorant_Garamond/static/CormorantGaramond-Regular.ttf",
            ];
            for path in primary {
                if let Some(file) = crate::asset_path::get(path) {
                    log::debug!("decal: loaded UI font from embedded {path}");
                    return Some(file.data.to_vec());
                }
            }
            // Embedded Noto Sans fallback — only present if a TTF has been
            // dropped into assets/Noto_Sans/. Tries the variable-font name
            // first, then a static Regular.
            let noto_sans = [
                "Noto_Sans/NotoSans-VariableFont_wdth,wght.ttf",
                "Noto_Sans/NotoSans-Regular.ttf",
                "Noto_Sans/static/NotoSans-Regular.ttf",
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
