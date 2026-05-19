//! Multi-line focus inspect — [`push_focus_tooltip_panel_2d`] — uses [`crate::ui::tooltip`] framing.

use crate::core::consumable::Consumable;
use crate::core::debuff::TileDebuff;
use crate::core::relic::RelicFlavorSpan;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::ui::colored_keywords;
use crate::ui::tooltip::{self, push_tooltip_frame_quads};
use crate::ui::widget;

/// Rough line count for relic flavor layout (matches bottom-aligned raster band).
fn estimated_flavor_line_count(
    spans: &[RelicFlavorSpan],
    band_w: f32,
    body_px: f32,
    max_lines: usize,
) -> usize {
    let char_count: usize = spans.iter().map(|s| s.text.chars().count()).sum();
    let explicit_lines: usize = spans.iter().map(|s| s.text.matches('\n').count() + 1).sum();
    let chars_per_line = (band_w / (body_px * 0.52)).max(18.0) as usize;
    let wrapped_lines = char_count.div_ceil(chars_per_line).max(1);
    explicit_lines.max(wrapped_lines).clamp(1, max_lines)
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

    let pad = 14.0_f32.max(10.0);
    let border = tooltip::FRAME_BORDER_PX;
    let margin = window_w * 0.02;
    let panel_w = (window_w * 0.45).min(640.0);
    let inner_w = (panel_w - pad * 2.0 - border * 2.0).max(80.0);

    let heading_px = typography::size(typography::H28, window_h);
    let body_px = typography::size(typography::H36, window_h);
    let section_gap = 6.0_f32;

    let desc_trim = truncate_inspect_text(desc, 400);

    #[derive(Clone, Copy)]
    enum Tier {
        Heading,
        Body,
        /// Keyword-tinted description (matches [`colored_keywords`]).
        Desc,
    }

    let mut blocks: Vec<(&str, [f32; 4], Tier)> = Vec::new();
    if hover_is_owned || !skip_title_block {
        blocks.push((title, color::CHAMPAGNE, Tier::Heading));
        if !cta.is_empty() {
            blocks.push((cta, accent_color, Tier::Body));
        }
    }
    if !desc_trim.is_empty() {
        blocks.push((desc_trim.as_str(), color::PARCHMENT, Tier::Desc));
    }

    if blocks.is_empty() {
        return;
    }

    let block_height = |text: &str, tier: Tier| -> f32 {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body | Tier::Desc => body_px,
        };
        let line_step = line_h * 1.4;
        let n = match tier {
            Tier::Desc => colored_keywords::wrap_colored_text_multiline(
                text,
                inner_w,
                line_h,
                color::PARCHMENT,
            )
            .len(),
            _ => widget::wrap_text(text, inner_w, line_h).len(),
        };
        n as f32 * line_step
    };

    let mut total_h = pad * 2.0 + border * 2.0;
    for (i, (text, _, tier)) in blocks.iter().enumerate() {
        total_h += block_height(text, *tier);
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

    push_tooltip_frame_quads(quads, left, top, panel_w, total_h);

    let text_left = left + pad + border;
    let mut y = top + pad + border;
    let text_w = inner_w;

    for (i, (text, col, tier)) in blocks.iter().enumerate() {
        let line_h = match tier {
            Tier::Heading => heading_px,
            Tier::Body | Tier::Desc => body_px,
        };
        let h_block = block_height(text, *tier);
        let font_px = typography::tier_at_most(line_h, window_h);
        match tier {
            Tier::Desc => {
                let lines =
                    colored_keywords::wrap_colored_text_multiline(text, inner_w, line_h, *col);
                colored_keywords::push_colored_rows_left(
                    texts,
                    colored_keywords::ColoredRowsLayout {
                        text_left,
                        top_y: y,
                        inner_w,
                        line_h,
                        fallback_plain: text,
                        fallback_color: *col,
                    },
                    &lines,
                );
            }
            _ => {
                let lines = widget::wrap_text(text, inner_w, line_h);
                let joined = lines.join("\n");
                texts.push(TextLabel {
                    rect: [text_left, y, text_w, h_block],
                    text: joined,
                    color: *col,
                    font_px: Some(font_px),
                    align: TextAlign::Left,
                    no_glossary: true,
                    ..Default::default()
                });
            }
        }
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
    let line_step = body_px * 1.4;
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
        color: color::alpha(color::LACQUER, 0.82),
        feather: [0.48, 0.12, 0.0, 0.0],
    });

    texts.push(TextLabel {
        rect: [left, top, band_w, band_h],
        text: String::new(),
        color: color::CHAMPAGNE,
        font_px: Some(body_px),
        align: TextAlign::Center,
        no_glossary: true,
        scroll_offset: 0.0,
        flavor_spans: Some(flavor),
        bold: false,
        italic: false,
        underline: false,
        text_effect: crate::render::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: None,
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
            "Levelled by the {} zodiac ribbon (+0.5 mult, +20 chips per level).",
            z.name()
        ),
        Consumable::Talisman(t) => t.description().to_string(),
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
            format!("Unlocks at Ante 4. {desc}"),
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
                ("Stamp · Polychrome ×1.2/meld".into(), color::WALNUT_BRIGHT)
            }
        });
    }

    if boss_debuffs.iter().any(|d| d.matches(tile)) {
        lines.push(("Boss · scoring penalized".into(), color::RUBY));
    }

    if selected {
        lines.push(("Play · queued".into(), color::BRASS));
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
