//! CPU rasterisation of per-tile Unicode decals.
//!
//! Each Mahjong tile maps to a codepoint in the Unicode Mahjong Tile block
//! (U+1F000–U+1F021).  We try to rasterise that glyph from a system or
//! project-local font; if the font doesn't carry those emoji outlines we fall
//! back to a short ASCII notation ("1m", "5p", "E", …).
//!
//! To get the real Unicode tile glyphs, drop any TTF/OTF that covers the
//! Mahjong block (e.g. Noto Emoji) at `assets/font.ttf` in the project root.

use crate::core::tile::{Suit, Tile, TileEnhancement};

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
        Suit::Flower => match tile.rank {
            1 => "梅", // plum
            2 => "蘭", // orchid
            3 => "菊", // chrysanthemum
            4 => "竹", // bamboo
            _ => "✿",
        }
        .to_string(),
        Suit::Season => match tile.rank {
            1 => "春", // spring
            2 => "夏", // summer
            3 => "秋", // autumn
            4 => "冬", // winter
            _ => "☀",
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
        Suit::Flower => "\u{1F33A}",     // 🌺 hibiscus
        Suit::Season => "\u{1F342}",     // 🍂 fallen leaf
    }
}

/// Return the asset filename stem for a tile inside a tileset directory.
///
/// Maps `(Suit, rank)` to the naming convention used in `assets/sets/`:
///   Bamboos 1–9 → B1..B9, Characters → C1..C9, Circles → D1..D9,
///   Winds → EWind/SWind/WWind/NWind, Dragons → DRed/DGreen/DWhite,
///   Flowers → Flower1..Flower4, Seasons → Season1..Season4.
fn tile_set_filename(tile: &Tile) -> Option<String> {
    match tile.suit {
        Suit::Bamboos => Some(format!("B{}", tile.rank)),
        Suit::Characters => Some(format!("C{}", tile.rank)),
        Suit::Circles => Some(format!("D{}", tile.rank)),
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

/// Try to load and alpha-blend a tileset PNG decal onto `dst`.
///
/// Returns `true` if the asset was found and blitted, `false` otherwise so the
/// caller can fall back to font rasterization.
fn blit_set_decal(dst: &mut [u8], dst_w: u32, dst_h: u32, tile: &Tile, tile_set: &str) -> bool {
    let Some(stem) = tile_set_filename(tile) else {
        return false;
    };
    let path = format!("sets/{tile_set}/{stem}.png");
    let Some(file) = crate::asset_path::get(&path) else {
        log::debug!("tileset decal not found: {path}; falling back to font rasterization");
        return false;
    };

    let Ok(decoder) =
        image::ImageReader::new(std::io::Cursor::new(file.data.as_ref())).with_guessed_format()
    else {
        return false;
    };
    let Ok(img) = decoder.decode() else {
        return false;
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
    true
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
/// loads a pre-made PNG from `assets/sets/<name>/` instead of rasterizing
/// glyphs. Falls back to font rasterization if the asset is missing.
///
/// `flip_decal_h`: set `true` for **hand** tiles (table rack). The mesh → world
/// pose makes the procedural UV map read backwards unless the atlas is mirrored
/// horizontally once. Showcase / pack tiles use `false` with their own pose.
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
    if let Some(set_name) = tile_set {
        if blit_set_decal(&mut rgba, width, height, tile, set_name) {
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
    let label = tile_short_label(tile);
    let emoji = tile_suit_emoji(tile);

    // Top half: suit-coloured short label.
    if let Some(font) = ui_font {
        let top_h = (height as f32 * 0.60) as u32;
        let band_top = (height as f32 * 0.02) as u32;
        let band = rasterize_label(font, &label, width, top_h);
        blit_tinted(&band, width, top_h, &mut rgba, width, 0, band_top, color);
    }

    // Bottom half: suit-coloured emoji.
    if let Some(font) = emoji_font {
        let bot_h = (height as f32 * 0.50) as u32;
        let band_top = (height as f32 * 0.50) as u32;
        let band = rasterize_label(font, emoji, width, bot_h);
        blit_tinted(&band, width, bot_h, &mut rgba, width, 0, band_top, color);
    }

    if tile.debuffed_visual {
        draw_debuff_marker(&mut rgba, width, height);
    }

    finish_tile_decal_rgba(rgba, width, height, flip_decal_h)
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
/// The PNGs are generated by `tools/gen_flower_decals.py` and embedded via
/// `rust-embed`. If the asset isn't found, falls back to a simple tinted "F"
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
        image::ImageReader::new(std::io::Cursor::new(file.data.as_ref())).with_guessed_format()
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
    {
        if let Ok(font) =
            fontdue::Font::from_bytes(font_data.data.as_ref(), fontdue::FontSettings::default())
        {
            let band = rasterize_label(&font, label, dst_w, dst_h);
            blit_tinted(&band, dst_w, dst_h, dst, dst_w, 0, 0, color);
        }
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
        None,
        LabelAlign::Center,
        0.0,
    );

    // Soft carved-shadow pass: a slightly darker tint, offset 1px down/right,
    // so the engraving keeps shape under directional lighting.
    let shadow = [ink[0] * 0.25, ink[1] * 0.22, ink[2] * 0.20, ink[3] * 0.85];
    blit_tinted(
        &band,
        inner_w,
        inner_h,
        &mut rgba,
        width,
        pad_x + 1,
        pad_y + 1,
        shadow,
    );
    // Main ink pass.
    blit_tinted(&band, inner_w, inner_h, &mut rgba, width, pad_x, pad_y, ink);
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
        &band,
        inner_w,
        inner_h,
        &mut rgba,
        width,
        pad_x + 2,
        pad_y + 2,
        gold_shadow,
    );
    // Gold body.
    blit_tinted(
        &band, inner_w, inner_h, &mut rgba, width, pad_x, pad_y, gold_base,
    );
    // Bright highlight offset up-left so the leaf catches the light.
    blit_tinted(
        &band,
        inner_w,
        inner_h,
        &mut rgba,
        width,
        pad_x.saturating_sub(1),
        pad_y.saturating_sub(1),
        gold_highlight,
    );
    rgba
}

/// Two-line engraved decal for the gameplay scene's hanging score plaque.
/// `top` is the blind/round/score line (champagne ink, slightly larger);
/// `bot` is the gold/wall/wind/shanten status line (parchment ink, slightly
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
pub fn rasterize_plaque_decal(
    text: &str,
    ui_font: Option<&fontdue::Font>,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let Some(font) = ui_font else {
        return rgba;
    };
    if text.trim().is_empty() {
        return rgba;
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
        fit_plaque_text(font, text, inner_w as f32, inner_h as f32);

    // Rasterize the wrapped block at the chosen size. `rasterize_block`
    // handles multi-line layout (stable line-height, per-line alignment).
    let line_refs: Vec<&str> = chosen_lines.iter().map(|s| s.as_str()).collect();
    let block = rasterize_block(
        font,
        None,
        &line_refs,
        inner_w,
        inner_h,
        Some(chosen_px),
        LabelAlign::Center,
    );

    // Gilded letters: a deep umber drop-shadow under a rich gold body, topped
    // by a bright pale-gold highlight offset up-left. Three passes give the
    // engraving a metallic, leafed-gold read at any size.
    let gold_base = [0.92_f32, 0.74, 0.28, 1.0]; // rich antique gold
    let gold_highlight = [1.00_f32, 0.96, 0.74, 1.0]; // pale champagne sheen
    let gold_shadow = [0.18_f32, 0.12, 0.04, 0.92]; // burnt umber recess

    blit_tinted(
        &block,
        inner_w,
        inner_h,
        &mut rgba,
        w,
        pad_x + 3,
        pad_y + 3,
        gold_shadow,
    );
    blit_tinted(
        &block, inner_w, inner_h, &mut rgba, w, pad_x, pad_y, gold_base,
    );
    blit_tinted(
        &block,
        inner_w,
        inner_h,
        &mut rgba,
        w,
        pad_x.saturating_sub(1),
        pad_y.saturating_sub(1),
        gold_highlight,
    );
    rgba
}

/// Fit `text` into a `(w, h)` area by searching for the largest font size
/// whose word-wrapped layout fits vertically and horizontally. Returns the
/// chosen `font_px` and the wrapped lines at that size. Honors explicit
/// `\n` as hard line breaks.
fn fit_plaque_text(
    font: &fontdue::Font,
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
        let lines = wrap_lines(font, text, candidate, w);
        let line_metrics = font.horizontal_line_metrics(candidate);
        let line_h = line_metrics
            .map(|lm| lm.new_line_size)
            .unwrap_or(candidate * 1.2);
        let total_h = line_h * lines.len() as f32;
        // Widest line's advance at this size.
        let widest = lines
            .iter()
            .map(|s| advance_width(font, s, candidate))
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
/// `\n` as hard line breaks and otherwise greedy-wraps on whitespace. If a
/// single word is wider than `max_w`, it's placed on its own line (we don't
/// break inside words — the size-fit loop will shrink further instead).
fn wrap_lines(font: &fontdue::Font, text: &str, font_px: f32, max_w: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for hard in text.split('\n') {
        let mut line = String::new();
        for word in hard.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", line, word)
            };
            if advance_width(font, &candidate, font_px) <= max_w || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        } else if hard.is_empty() {
            // Preserve blank lines from explicit `\n\n` runs.
            out.push(String::new());
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn advance_width(font: &fontdue::Font, text: &str, font_px: f32) -> f32 {
    text.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

/// Reference height (in texels) for the plaque decal texture. The actual
/// width is computed at draw time from the plaque face's world-space aspect
/// ratio so glyphs don't get stretched by the bilinear sampler when the face
/// isn't ~landscape — see the call site in `wgpu_renderer.rs`.
pub const PLAQUE_DECAL_HEIGHT: u32 = 320;

/// Measure how tall a plaque needs to be to fit `text` at `font_px`, wrapping
/// at `inner_w` pixels. Returns `(line_count, line_height_px)` — multiply and
/// add the scene's chosen vertical padding to get the full plaque pixel height.
/// Lets shop/gameplay scenes size the description plaque from the wrapped
/// line count instead of shrinking the font to fit a fixed box.
pub fn measure_plaque_wrap(
    ui_font: Option<&fontdue::Font>,
    text: &str,
    inner_w: f32,
    font_px: f32,
) -> (usize, f32) {
    let Some(font) = ui_font else {
        return (1, font_px * 1.2);
    };
    let line_h = font
        .horizontal_line_metrics(font_px)
        .map(|lm| lm.new_line_size)
        .unwrap_or(font_px * 1.2);
    let lines = wrap_lines(font, text, font_px, inner_w);
    (lines.len().max(1), line_h)
}

/// Reference long-edge size (in texels) for the ofuda decal texture. The
/// other dimension is computed at draw time from the paper face's world-space
/// aspect ratio so the bilinear sampler doesn't stretch glyphs — see the
/// call site in `wgpu_renderer.rs`.
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
    let pad_x = (w as f32 * 0.05) as u32;
    let pad_y = (h as f32 * 0.06) as u32;
    let inner_w = w.saturating_sub(pad_x * 2).max(1);
    let inner_h = h.saturating_sub(pad_y * 2).max(1);

    // Sumi-ink calligraphy: warm-black body with a soft brown drop shadow so
    // the strokes lift off the parchment background even when the candle
    // light desaturates the paper.
    let ink = [0.12_f32, 0.08, 0.05, 1.0];
    let shadow = [0.55_f32, 0.30, 0.10, 0.55];

    let stamp = |band: &[u8], band_h: u32, y_off: u32, rgba: &mut Vec<u8>| {
        blit_tinted(band, inner_w, band_h, rgba, w, pad_x + 2, y_off + 2, shadow);
        blit_tinted(band, inner_w, band_h, rgba, w, pad_x, y_off, ink);
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
        let rule_px = fit_multiline_font_px(
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
fn fit_multiline_font_px(
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

/// Paint a soft inner border + corner gem onto an RGBA8 decal to mark a tile
/// that has been enhanced by a talisman. The border is a 4-px-thick frame
/// with a softer outer glow that fades to transparent, and a small inlay
/// "gem" in the upper-right corner.
///
/// Each enhancement gets its own visual treatment:
/// - **Jade** — two-tone imperial green, lighter on top edges, deeper on
///   bottom (suggests carved stone catching light from above).
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

        TileEnhancement::Jade => {
            // Two-tone carved stone: cosine over t puts the highlight along
            // the top of the frame and the deepest green along the bottom.
            // top_w ≈ 1 at the top edge, ≈ 0 at the bottom.
            let top_w = ((t * std::f32::consts::TAU).cos() * 0.5 + 0.5).clamp(0.0, 1.0);
            let r = ((0.10 + 0.30 * top_w) * 255.0) as u8;
            let g = ((0.50 + 0.40 * top_w) * 255.0) as u8;
            let b = ((0.30 + 0.25 * top_w) * 255.0) as u8;
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

        TileEnhancement::Jade => {
            // Carved jade orb: brighter highlight in the upper-left, deeper
            // green in the lower-right. Same axis convention as the frame.
            let highlight = 0.875_f32;
            let mut d_angle = (angle_t - highlight).abs();
            if d_angle > 0.5 {
                d_angle = 1.0 - d_angle;
            }
            let lit = (1.0 - d_angle * 2.0).clamp(0.0, 1.0) * (1.0 - 0.4 * r_norm);
            let r = ((0.10 + 0.35 * lit) * 255.0) as u8;
            let g = ((0.50 + 0.45 * lit) * 255.0) as u8;
            let b = ((0.28 + 0.30 * lit) * 255.0) as u8;
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
    rasterize_label_styled_with_fallback(font, None, text, width, height, font_px, align, 0.0)
}

/// Like [`rasterize_label_styled`] but with an optional emoji fallback font.
/// Characters missing from the primary font are rasterized with the fallback.
pub fn rasterize_label_styled_with_fallback(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    text: &str,
    width: u32,
    height: u32,
    font_px: Option<f32>,
    align: LabelAlign,
    scroll_offset: f32,
) -> Vec<u8> {
    // Multi-line: lay out each line at the same font size, stacked vertically.
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > 1 {
        return rasterize_block(font, emoji_font, &lines, width, height, font_px, align);
    }

    // Single-line fast path retains the historical centring behaviour.
    let font_px = match font_px {
        Some(px) => px.max(8.0),
        None => (height as f32 * 0.55)
            .min(width as f32 * 1.5 / text.chars().count().max(1) as f32)
            .max(8.0),
    };

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

    // Horizontal start depends on alignment, then shifted by scroll_offset.
    let start_x = match align {
        LabelAlign::Left => 0.0,
        LabelAlign::Center => (width as f32 - total_advance) * 0.5,
        LabelAlign::Right => width as f32 - total_advance,
    } - scroll_offset;

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    blit_line(&glyphs, &mut rgba, width, height, start_x, baseline_y);
    rgba
}

/// Multi-line block layout. Lines are stacked vertically, all rendered at the
/// same `font_px` so the paragraph reads as a coherent block.
fn rasterize_block(
    font: &fontdue::Font,
    emoji_font: Option<&fontdue::Font>,
    lines: &[&str],
    width: u32,
    height: u32,
    font_px: Option<f32>,
    align: LabelAlign,
) -> Vec<u8> {
    // For multi-line blocks the font size MUST be pinned — auto-shrink would
    // produce different sizes per line which defeats the purpose. If the
    // caller didn't pin one, derive it from height / line_count.
    let font_px = font_px
        .unwrap_or_else(|| (height as f32 * 0.55 / lines.len() as f32).max(8.0))
        .max(8.0);

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
                .map(|ch| pick_font(font, emoji_font, ch).rasterize(ch, font_px))
                .collect();
            let advance: f32 = glyphs.iter().map(|(m, _)| m.advance_width).sum();
            LineGlyphs { glyphs, advance }
        })
        .collect();

    // Use font.line_metrics for a stable line height across lines.
    let line_metrics = font.horizontal_line_metrics(font_px);
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
    }

    rgba
}

/// Pick the best font for `ch`: use the primary font if it has the glyph,
/// otherwise fall back to the emoji font (if provided).
fn pick_font<'a>(
    primary: &'a fontdue::Font,
    fallback: Option<&'a fontdue::Font>,
    ch: char,
) -> &'a fontdue::Font {
    if primary.has_glyph(ch) {
        primary
    } else if let Some(fb) = fallback {
        fb
    } else {
        primary
    }
}

struct GlyphData {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
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
                &g.bitmap,
                g.metrics.width,
                g.metrics.height,
                glyph_left,
                glyph_top,
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
                g.bitmap,
                g.metrics.width,
                g.metrics.height,
                glyph_left,
                glyph_top,
                rgba,
                width,
                height,
            );
        }
        cx += g.metrics.advance_width;
    }
}

fn blit_glyph(
    bitmap: &[u8],
    gw: usize,
    gh: usize,
    glyph_left: i32,
    glyph_top: i32,
    rgba: &mut [u8],
    width: u32,
    height: u32,
) {
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
