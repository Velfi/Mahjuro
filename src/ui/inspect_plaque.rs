//! Multi-line focus inspect — [`push_focus_tooltip_panel_2d`] — uses [`crate::ui::tooltip`] framing.

use crate::core::consumable::Consumable;
use crate::core::debuff::TileDebuff;
use crate::core::relic::{RelicFlavorSpan, flavor_spans_plain_text};
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::render::decal::{
    DecalFonts, load_ui_font, load_ui_font_italic, measure_flavor_spans_layout,
};
use crate::render::text_shadow_lab::{
    FloatingFlavorShadowTuning, layout_floating_flavor_caption_for_spans,
};
use crate::render::theme::{color, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{
    GpuInstance, GradientQuadInstance, TextAlign, TextBlockVerticalAlign, TextLabel,
};
use crate::ui::styled_text;
use crate::ui::tooltip::push_tooltip_frame_quads;

/// Width that fits the longest `\n`-delimited hard line without wrapping.
pub fn flavor_spans_layout_width(
    spans: &[RelicFlavorSpan],
    body_px: f32,
    max_width_px: f32,
) -> f32 {
    let char_w = body_px * 0.58;
    let longest_chars = spans
        .iter()
        .flat_map(|s| s.text.split('\n'))
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    (longest_chars as f32 * char_w + body_px * 1.5).clamp(body_px * 4.0, max_width_px)
}

/// Hard lines after span concat + `\n` breaks (matches flavor raster layout).
fn flavor_hard_line_char_counts(spans: &[RelicFlavorSpan]) -> Vec<usize> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = vec![String::new()];
    for span in spans {
        let mut first = true;
        for segment in span.text.split('\n') {
            if !first {
                lines.push(String::new());
            }
            first = false;
            lines.last_mut().expect("hard line").push_str(segment);
        }
    }
    lines.into_iter().map(|s| s.chars().count()).collect()
}

/// Rough line count for relic / staircase flavor layout (matches bottom-aligned raster band).
pub fn estimated_flavor_line_count(
    spans: &[RelicFlavorSpan],
    band_w: f32,
    body_px: f32,
    max_lines: usize,
) -> usize {
    let char_w = body_px * 0.58;
    let chars_per_line = (band_w / char_w).floor().max(1.0) as usize;
    let mut lines = 0usize;
    for chars in flavor_hard_line_char_counts(spans) {
        lines += if chars == 0 {
            1
        } else {
            chars.div_ceil(chars_per_line).max(1)
        };
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
        inner_w = inner_w.max(styled_text::colored_paragraph_preferred_width(
            text,
            line_h,
            max_inner_w,
            GlossaryMode::Prose,
        ));
    }
    inner_w = inner_w.clamp(min_inner_w, max_inner_w);
    let panel_w = inner_w + pad * 2.0 + FILL_INSET_PX * 2.0;

    let block_height = |text: &str, col: [f32; 4], tier: Tier| -> f32 {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body => body_px,
        };
        styled_text::colored_multiline_text_height(text, inner_w, line_h, col, GlossaryMode::Prose)
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
        let lines = styled_text::wrap_colored_text_multiline(
            text,
            inner_w,
            line_h,
            *col,
            false,
            GlossaryMode::Prose,
        );
        styled_text::push_colored_rows_left(
            texts,
            styled_text::ColoredRowsLayout {
                text_left,
                top_y: y,
                inner_w,
                line_h,
                fallback_plain: text,
                fallback_color: *col,
                italic: false,
                glossary: GlossaryMode::Prose,
            },
            &lines,
        );
        y += h_block;
        if i + 1 < blocks.len() {
            y += section_gap;
        }
    }
}

/// Relic inspect flavor in a left-docked tooltip panel (Archive inspect).
///
/// `extra_bottom_reserve` shrinks the vertical band (e.g. leave room for footer hints).
pub fn push_relic_flavor_inspect_panel(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    window_w: f32,
    window_h: f32,
    flavor: &'static [RelicFlavorSpan],
    extra_bottom_reserve: f32,
) {
    if flavor.is_empty() {
        return;
    }

    let margin = window_w * 0.04;
    let pad = 14.0_f32.max(10.0);
    const FILL_INSET_PX: f32 = 2.0;
    let rim = crate::render::theme::metrics::tooltip_border_px(window_w, window_h);
    let body_px = typography::size(typography::H32, window_h);
    let min_font_px = typography::readable_floor_px(window_h);
    let max_inner_w = (window_w * 0.46).clamp(440.0, 720.0);
    let inner_w = flavor_spans_layout_width(flavor, body_px, max_inner_w)
        .clamp(400.0, max_inner_w);
    let inner_w_u = inner_w.max(1.0) as u32;
    let top_margin = window_h * 0.10;
    let bottom_margin = extra_bottom_reserve.max(0.0) + window_h * 0.02;
    let frame_inset = pad * 2.0 + FILL_INSET_PX * 2.0;
    let available_inner_h =
        (window_h - top_margin - bottom_margin - frame_inset).max(body_px * 2.0);
    let available_inner_h_u = available_inner_h.max(1.0) as u32;

    let (content_h, label_font_px) = if let Some(font) = load_ui_font() {
        let fonts = DecalFonts {
            regular: font,
            italic: load_ui_font_italic(),
            emoji: None,
        };
        let metrics = measure_flavor_spans_layout(
            &fonts,
            flavor,
            inner_w_u,
            u32::MAX,
            body_px,
            min_font_px,
        );
        let metrics = if metrics.text_block_h > available_inner_h {
            measure_flavor_spans_layout(
                &fonts,
                flavor,
                inner_w_u,
                available_inner_h_u,
                body_px,
                min_font_px,
            )
        } else {
            metrics
        };
        (metrics.text_block_h.ceil(), metrics.font_px)
    } else {
        let line_step = styled_text::colored_row_line_step(body_px);
        let content_lines =
            estimated_flavor_line_count(flavor, inner_w, body_px, 64);
        (line_step * content_lines as f32, body_px)
    };

    let panel_w = inner_w + frame_inset;
    let panel_h = content_h + frame_inset;
    let top = top_margin.min((window_h - bottom_margin - panel_h).max(0.0));
    let left = margin;

    push_tooltip_frame_quads(quads, left, top, panel_w, panel_h, rim);

    let text_left = left + pad + FILL_INSET_PX;
    let text_top = top + pad + FILL_INSET_PX;
    texts.push(TextLabel {
        rect: [text_left, text_top, inner_w, content_h],
        text: flavor_spans_plain_text(flavor),
        color: color::PARCHMENT,
        font_px: Some(label_font_px),
        align: TextAlign::Left,
        block_vertical_align: TextBlockVerticalAlign::Top,
        flavor_spans: Some(flavor),
        ..Default::default()
    });
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
    let tuning = FloatingFlavorShadowTuning::DEFAULT;
    let body_px = typography::size(typography::H32, window_h);
    let min_font_px = crate::render::theme::typography::readable_floor_px(window_h);
    let layout = if let Some(font) = load_ui_font() {
        let fonts = DecalFonts {
            regular: font,
            italic: load_ui_font_italic(),
            emoji: None,
        };
        layout_floating_flavor_caption_for_spans(
            window_w,
            window_h,
            &fonts,
            flavor,
            body_px,
            min_font_px,
            extra_bottom_reserve,
            &tuning,
        )
    } else {
        let line_step = styled_text::colored_row_line_step(body_px);
        let max_lines = 8usize;
        let margin_x = window_w * tuning.margin_x_frac;
        let band_w = (window_w - 2.0 * margin_x).min(tuning.band_max_w);
        let content_lines = estimated_flavor_line_count(flavor, band_w, body_px, max_lines);
        crate::render::text_shadow_lab::layout_floating_flavor_caption(
            window_w,
            window_h,
            body_px,
            line_step,
            content_lines,
            extra_bottom_reserve,
            &tuning,
        )
    };
    gradient_quads.push(layout.gradient_quad(&tuning));

    // Always rasterize via flavor_spans so copy bottom-aligns in `band_h` and
    // sits on the gradient shadow (the plain-text row path anchored at `top`).
    texts.push(TextLabel {
        rect: layout.text_rect(),
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
        block_vertical_align: Default::default(),
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
    fn flavor_line_count_honors_explicit_newlines() {
        let spans = &[
            RelicFlavorSpan {
                text: "First line",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "\n",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "Second line",
                bold: false,
                italic: false,
            },
        ];
        let body_px = 32.0;
        let wide_w = 640.0;
        let lines = estimated_flavor_line_count(spans, wide_w, body_px, 8);
        assert_eq!(lines, 2, "newline separator span should break lines");
    }

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
