//! Multi-line focus inspect — [`push_focus_tooltip_panel_2d`] — uses [`crate::ui::tooltip`] framing.

use crate::core::consumable::Consumable;
use crate::core::debuff::TileDebuff;
use crate::core::relic::RelicFlavorSpan;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::ui::colored_keywords;
use crate::ui::tooltip::push_tooltip_frame_quads;

/// Width that fits the longest `\n`-delimited hard line without wrapping.
pub fn flavor_spans_layout_width(
    spans: &[RelicFlavorSpan],
    body_px: f32,
    max_width_px: f32,
) -> f32 {
    let char_w = body_px * 0.52;
    let longest_chars = spans
        .iter()
        .flat_map(|s| s.text.split('\n'))
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    (longest_chars as f32 * char_w + body_px * 1.5).clamp(body_px * 4.0, max_width_px)
}

/// Rough line count for relic / staircase flavor layout (matches bottom-aligned raster band).
pub fn estimated_flavor_line_count(
    spans: &[RelicFlavorSpan],
    band_w: f32,
    body_px: f32,
    max_lines: usize,
) -> usize {
    let char_w = body_px * 0.52;
    let chars_per_line = (band_w / char_w).floor().max(1.0) as usize;
    let mut lines = 0usize;
    for span in spans {
        for segment in span.text.split('\n') {
            let chars = segment.chars().count();
            lines += if chars == 0 {
                1
            } else {
                chars.div_ceil(chars_per_line).max(1)
            };
        }
    }
    lines.clamp(1, max_lines)
}

/// Focus inspect: title, optional accent line (price/tier/CTA), and description in a screen-space
/// panel ([`tooltip`] brass + midnight frame), anchored to `anchor_rect` when provided.
///
/// `avoid_rect`: an optional screen-space rect the panel should not overlap (e.g. the gold label
/// in the shop). When a horizontal shift can resolve the collision the panel is moved; otherwise
/// it stays at its computed position.
pub struct FocusTooltipPanelParams<'a> {
    pub window_w: f32,
    pub window_h: f32,
    pub anchor_rect: Option<[f32; 4]>,
    pub title: &'a str,
    pub desc: &'a str,
    pub cta: &'a str,
    pub accent_color: [f32; 4],
    pub hover_is_owned: bool,
    pub skip_title_block: bool,
    pub avoid_rect: Option<[f32; 4]>,
}

pub fn push_focus_tooltip_panel_2d(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    params: FocusTooltipPanelParams<'_>,
) {
    let FocusTooltipPanelParams {
        window_w,
        window_h,
        anchor_rect,
        title,
        desc,
        cta,
        accent_color,
        hover_is_owned,
        skip_title_block,
        avoid_rect,
    } = params;
    if title.is_empty() {
        return;
    }

    let margin = window_w * 0.02;
    let pad = 14.0_f32.max(10.0);
    // Inset from the walnut fill edge to copy; independent of the brass rim draw width.
    const FILL_INSET_PX: f32 = 2.0;
    let rim = crate::render::theme::metrics::tooltip_border_px(window_w, window_h);
    let max_panel_w = crate::render::theme::metrics::tooltip_max_panel_px(window_w, window_h);
    let min_inner_w = 80.0_f32;
    let max_inner_w = (max_panel_w - pad * 2.0 - FILL_INSET_PX * 2.0).max(min_inner_w);

    let heading_px = typography::size(typography::H28, window_h);
    let body_px = typography::size(typography::H36, window_h);
    let section_gap = 6.0_f32;

    let desc_trim = truncate_inspect_text(desc, 400);

    #[derive(Clone, Copy)]
    enum Tier {
        Heading,
        Body,
    }

    let mut blocks: Vec<(&str, [f32; 4], Tier)> = Vec::new();
    if hover_is_owned || !skip_title_block {
        blocks.push((title, color::CHAMPAGNE, Tier::Heading));
        if !cta.is_empty() {
            blocks.push((cta, accent_color, Tier::Body));
        }
    }
    if !desc_trim.is_empty() {
        blocks.push((desc_trim.as_str(), color::PARCHMENT, Tier::Body));
    }

    if blocks.is_empty() {
        return;
    }

    let mut inner_w = min_inner_w;
    for (text, _col, tier) in &blocks {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body => body_px,
        };
        inner_w = inner_w.max(colored_keywords::colored_paragraph_preferred_width(
            text,
            line_h,
            max_inner_w,
        ));
    }
    inner_w = inner_w.clamp(min_inner_w, max_inner_w);
    let panel_w = inner_w + pad * 2.0 + FILL_INSET_PX * 2.0;

    let block_height = |text: &str, col: [f32; 4], tier: Tier| -> f32 {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body => body_px,
        };
        colored_keywords::colored_multiline_text_height(text, inner_w, line_h, col)
    };

    let mut total_h = pad * 2.0 + FILL_INSET_PX * 2.0;
    for (i, (text, col, tier)) in blocks.iter().enumerate() {
        total_h += block_height(text, *col, *tier);
        if i + 1 < blocks.len() {
            total_h += section_gap;
        }
    }

    let (cx, ay, ah) = anchor_rect
        .map(|r| (r[0] + r[2] * 0.5, r[1], r[3]))
        .unwrap_or((window_w * 0.5, window_h * 0.62, 40.0));

    let mut left = cx - panel_w * 0.5;
    left = left.clamp(margin, window_w - panel_w - margin);

    let gap = 12.0_f32;
    let mut top = ay - gap - total_h;
    if top < margin {
        top = ay + ah + gap;
    }
    top = top.clamp(margin, window_h - margin - total_h);

    // Shift the panel horizontally when it would overlap the avoid_rect (e.g. the gold label).
    if let Some(ar) = avoid_rect {
        let panel_right = left + panel_w;
        let panel_bottom = top + total_h;
        let ar_right = ar[0] + ar[2];
        let ar_bottom = ar[1] + ar[3];
        let overlaps =
            left < ar_right && panel_right > ar[0] && top < ar_bottom && panel_bottom > ar[1];
        if overlaps {
            let left_candidate = ar[0] - panel_w - margin;
            let right_candidate = ar_right + margin;
            if left_candidate >= margin {
                left = left_candidate;
            } else if right_candidate + panel_w <= window_w - margin {
                left = right_candidate;
            }
        }
    }

    push_tooltip_frame_quads(quads, left, top, panel_w, total_h, rim);

    let text_left = left + pad + FILL_INSET_PX;
    let mut y = top + pad + FILL_INSET_PX;

    for (i, (text, col, tier)) in blocks.iter().enumerate() {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body => body_px,
        };
        let h_block = block_height(text, *col, *tier);
        let lines = colored_keywords::wrap_colored_text_multiline(text, inner_w, line_h, *col, false);
        colored_keywords::push_colored_rows_left(
            texts,
            colored_keywords::ColoredRowsLayout {
                text_left,
                top_y: y,
                inner_w,
                line_h,
                fallback_plain: text,
                fallback_color: *col,
                italic: false,
            },
            &lines,
        );
        y += h_block;
        if i + 1 < blocks.len() {
            y += section_gap;
        }
    }
}

/// Relic inspect flavor only: no tooltip frame; draw as a bottom-centered band
/// (readable on a dark / black inspect backdrop).
///
/// `extra_bottom_reserve` lifts the band upward (e.g. leave room for shop inspect hints).
pub fn push_floating_relic_flavor_labels(
    gradient_quads: &mut Vec<GradientQuadInstance>,
    texts: &mut Vec<TextLabel>,
    window_w: f32,
    window_h: f32,
    flavor: &'static [RelicFlavorSpan],
    extra_bottom_reserve: f32,
) {
    if flavor.is_empty() {
        return;
    }
    let margin_x = window_w * 0.035;
    let band_w = (window_w - 2.0 * margin_x).min(1040.0);
    let left = (window_w - band_w) * 0.5;
    let body_px = typography::size(typography::H32, window_h);
    let line_step = colored_keywords::colored_row_line_step(body_px);
    let max_lines = 5usize;
    let band_h = (line_step * max_lines as f32 + body_px * 0.5)
        .min(window_h * 0.25)
        .max(body_px * 2.0);
    let bottom_margin = window_h * 0.055 + extra_bottom_reserve.max(0.0);
    let top = window_h - bottom_margin - band_h;

    // Flavor rasterizes bottom-aligned in `band_h`; size the shadow to the copy,
    // not the full band, so diffuse ink does not float above short quotes.
    let content_lines = estimated_flavor_line_count(flavor, band_w, body_px, max_lines);
    let content_h = line_step * content_lines as f32 + body_px * 0.2;
    let pad_x = band_w * 0.10;
    let pad_top = body_px * 0.45;
    let pad_bottom = band_h * 0.22;
    let shadow_h = content_h + pad_top + pad_bottom;
    let shadow_bottom = top + band_h + pad_bottom * 0.25 + line_step;
    let shadow_top = shadow_bottom - shadow_h;
    gradient_quads.push(GradientQuadInstance {
        rect: [left - pad_x, shadow_top, band_w + 2.0 * pad_x, shadow_h],
        color: color::alpha(color::WALNUT_INK, 0.82),
        feather: [0.48, 0.12, 0.0, 0.0],
    });

    texts.push(TextLabel {
        rect: [left, top, band_w, band_h],
        text: String::new(),
        color: color::CHAMPAGNE,
        font_px: Some(body_px),
        align: TextAlign::Center,
        scroll_offset: 0.0,
        flavor_spans: Some(flavor),
        bold: false,
        italic: false,
        underline: false,
        text_effect: crate::render::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: None,
        mono: false,
    });
}

/// Identity line for gameplay hover (title strip).
#[inline]
pub fn hand_tile_decal_title(tile: &Tile) -> String {
    tile.full_name()
}

/// Gameplay hand-tile focus panel: full name title and scoring keyword body.
pub fn hand_tile_focus_tooltip(
    tile: &Tile,
    dora_faces: &[(crate::core::tile::Suit, u8)],
    boss_debuffs: &[TileDebuff],
    selected: bool,
) -> (String, String) {
    let title = hand_tile_decal_title(tile);
    let desc: String = hand_tile_keyword_lines(tile, dora_faces, boss_debuffs, selected)
        .into_iter()
        .map(|(s, _)| s)
        .collect::<Vec<_>>()
        .join("\n");
    (title, desc)
}

/// Full ribbon/talisman body for gameplay focus panel (aligned with collection catalog tone).
pub fn gameplay_consumable_description_full(c: Consumable) -> String {
    match c {
        Consumable::Zodiac(z) => format!(
            "Levelled by the {} zodiac ribbon (+{:.2} mult, +{} chips per level).",
            z.name(),
            z.level_up_mult_per_level(),
            z.level_up_chips_per_level(),
        ),
        Consumable::Talisman(t) => t.description().to_string(),
        Consumable::Memorial(m) => m.description().to_string(),
    }
}

/// Title and rules body for the dora plinth (cta is always empty).
pub fn dora_focus_tooltip_strings(
    dora_enabled: bool,
    dora_faces: &[(crate::core::tile::Suit, u8)],
) -> (String, String, String) {
    use crate::core::scoring::DORA_CHIPS_PER_TILE;
    use crate::core::tile::Tile;

    let desc = format!("Dora tiles score +{DORA_CHIPS_PER_TILE} extra.");
    if !dora_enabled {
        return (
            "Dora".to_string(),
            String::new(),
            format!("Unlocks at Wing 4. {desc}"),
        );
    }
    if dora_faces.is_empty() {
        return ("Dora".to_string(), String::new(), desc);
    }
    let names: Vec<String> = dora_faces
        .iter()
        .map(|(suit, rank)| Tile::new(*suit, *rank, 0).full_name())
        .collect();
    let title = match names.as_slice() {
        [one] => format!("{one} is dora!"),
        [a, b] => format!("{a} and {b} are dora!"),
        _ => {
            let last = names.last().expect("non-empty");
            let head = names[..names.len() - 1].join(", ");
            format!("{head}, and {last} are dora!")
        }
    };
    (title, String::new(), desc)
}

/// Returns "a" or "an" based on the leading sound of `word`, optionally Title-cased.
fn indefinite_article(word: &str, capitalize: bool) -> &'static str {
    let leads_with_vowel = word
        .chars()
        .next()
        .map(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'))
        .unwrap_or(false);
    match (leads_with_vowel, capitalize) {
        (true, true) => "An",
        (true, false) => "an",
        (false, true) => "A",
        (false, false) => "a",
    }
}

/// Title and rules body for the round-wind plinth (cta is always empty).
pub fn round_wind_focus_tooltip_strings(
    primary: u8,
    bonus: Option<u8>,
) -> (String, String, String) {
    use crate::core::tile::{Suit, Tile};

    let desc = "Triplets and kongs score Yakuhai.".to_string();
    let mut ranks = vec![primary];
    if let Some(b) = bonus {
        ranks.push(b);
    }
    let names: Vec<String> = ranks
        .iter()
        .map(|rank| Tile::new(Suit::Wind, *rank, 0).full_name())
        .collect();
    let title = match names.as_slice() {
        [one] => format!("{} {one} is blowing!", indefinite_article(one, true)),
        [a, b] => format!(
            "{} {a} and {} {b} are blowing!",
            indefinite_article(a, true),
            indefinite_article(b, false),
        ),
        _ => {
            let last = names.last().expect("non-empty");
            let head = names[..names.len() - 1]
                .iter()
                .enumerate()
                .map(|(i, n)| format!("{} {n}", indefinite_article(n, i == 0)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{head}, and {} {last} are blowing!",
                indefinite_article(last, false),
            )
        }
    };
    (title, String::new(), desc)
}

/// Scoring and status keywords for hand tiles — joined into the focus panel body.
pub fn hand_tile_keyword_lines(
    tile: &Tile,
    dora_faces: &[(crate::core::tile::Suit, u8)],
    boss_debuffs: &[TileDebuff],
    selected: bool,
) -> Vec<(String, [f32; 4])> {
    let mut lines: Vec<(String, [f32; 4])> = Vec::with_capacity(6);

    match tile.suit {
        Suit::Flower => {
            lines.push(("Flower · wildcard in one meld".into(), color::STONE));
            lines.push(("Face · no base chips".into(), color::UMBER));
        }
        Suit::Season => {
            lines.push(("Season · bonus tile".into(), color::STONE));
        }
        _ => {
            lines.push((
                format!("Base · {} chips", tile.point_value()),
                color::CHAMPAGNE,
            ));
        }
    }

    let face = (tile.suit, tile.rank);
    if dora_faces.contains(&face) {
        use crate::core::scoring::DORA_CHIPS_PER_TILE;
        lines.push((format!("Dora · +{DORA_CHIPS_PER_TILE} chips"), color::GOLD));
    }

    if let Some(e) = tile.enhancement {
        lines.push(match e {
            TileEnhancement::Pearl => ("Stamp · Pearl +100/meld".into(), color::JADE),
            TileEnhancement::Gilded => ("Stamp · Gilded +$1/meld".into(), color::GOLD),
            TileEnhancement::Polychrome => {
                ("Stamp · Polychrome ×1.25/meld".into(), color::WALNUT_BRIGHT)
            }
        });
    }

    if boss_debuffs.iter().any(|d| d.matches(tile)) {
        lines.push(("Boss · this tile is debuffed".into(), color::RUBY));
    }

    if selected {
        lines.push(("Play · selected".into(), color::BRASS));
    }

    lines
}

/// Trim long inspect strings (plaques & overlays).
pub fn truncate_inspect_text(s: &str, max_chars: usize) -> String {
    let t = s.trim();
    let count = t.chars().count();
    if count <= max_chars {
        return t.to_string();
    }
    let take = max_chars.saturating_sub(1);
    t.chars().take(take).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::relic::RelicFlavorSpan;

    #[test]
    fn flavor_line_count_wraps_each_hard_line_at_narrow_width() {
        let spans = &[RelicFlavorSpan {
            text: "The storeroom remembers\nits losers\ndisplayed and priced.\nYou browse\nyour predecessors.",
            bold: false,
            italic: false,
        }];
        let body_px = 32.0;
        let narrow_w = 168.0;
        let lines = estimated_flavor_line_count(spans, narrow_w, body_px, 16);
        assert!(
            lines >= 7,
            "narrow band should wrap long hard lines, got {lines}"
        );

        let wide_w = flavor_spans_layout_width(spans, body_px, 560.0);
        let wide_lines = estimated_flavor_line_count(spans, wide_w, body_px, 16);
        assert_eq!(wide_lines, 5, "wide band should honor explicit newlines only");
    }
}
