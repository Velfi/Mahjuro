//! Multi-line focus inspect — [`push_focus_tooltip_panel_2d`] — uses [`crate::ui::tooltip`] framing.

use crate::core::consumable::Consumable;
use crate::core::debuff::TileDebuff;
use crate::core::relic::RelicFlavorSpan;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::colored_keywords;
use crate::ui::tooltip::{self, push_tooltip_frame_quads};
use crate::ui::widget;

/// Focus inspect: title, optional accent line (price/tier/CTA), and description in a screen-space
/// panel ([`tooltip`] brass + midnight frame), anchored to `anchor_rect` when provided.
pub fn push_focus_tooltip_panel_2d(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    window_w: f32,
    window_h: f32,
    anchor_rect: Option<[f32; 4]>,
    title: &str,
    desc: &str,
    cta: &str,
    accent_color: [f32; 4],
    hover_is_owned: bool,
    skip_title_block: bool,
) {
    if title.is_empty() {
        return;
    }

    let pad = 14.0_f32.max(10.0);
    let border = tooltip::FRAME_BORDER_PX;
    let margin = window_w * 0.02;
    let panel_w = (window_w * 0.38).min(520.0);
    let inner_w = (panel_w - pad * 2.0 - border * 2.0).max(80.0);

    let heading_px = typography::size(typography::HEADING, window_h).max(15.0);
    let body_px = typography::size(typography::BODY, window_h).max(13.0);
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
        let font_px = line_h.max(8.0);
        match tier {
            Tier::Desc => {
                let lines = colored_keywords::wrap_colored_text_multiline(
                    text,
                    inner_w,
                    line_h,
                    *col,
                );
                colored_keywords::push_colored_rows_left(
                    texts,
                    text_left,
                    y,
                    inner_w,
                    &lines,
                    line_h,
                    text,
                    *col,
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
    texts: &mut Vec<TextLabel>,
    window_w: f32,
    window_h: f32,
    flavor: &'static [RelicFlavorSpan],
    extra_bottom_reserve: f32,
) {
    if flavor.is_empty() {
        return;
    }
    let margin_x = window_w * 0.06;
    let band_w = (window_w - 2.0 * margin_x).min(760.0);
    let left = (window_w - band_w) * 0.5;
    let body_px = typography::size(typography::BODY, window_h).max(13.0);
    let line_step = body_px * 1.4;
    let max_lines = 5usize;
    let band_h = (line_step * max_lines as f32 + body_px * 0.5)
        .min(window_h * 0.25)
        .max(body_px * 2.0);
    let bottom_margin = window_h * 0.055 + extra_bottom_reserve.max(0.0);
    let top = window_h - bottom_margin - band_h;
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
    });
}

/// Identity line for gameplay hover (title strip).
#[inline]
pub fn hand_tile_decal_title(tile: &Tile) -> String {
    tile.full_name()
}

/// Full gameplay hover: identity + keyword scoring lines.
pub fn hand_tile_inspect_lines(
    tile: &Tile,
    dora_faces: &[(crate::core::tile::Suit, u8)],
    boss_debuffs: &[TileDebuff],
    selected: bool,
) -> Vec<(String, [f32; 4])> {
    let mut lines = vec![(hand_tile_decal_title(tile), color::CHAMPAGNE)];
    lines.extend(hand_tile_keyword_lines(
        tile,
        dora_faces,
        boss_debuffs,
        selected,
    ));
    lines
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

/// Title, optional accent line (current faces when dora is active), and rules body for the dora plinth.
pub fn dora_focus_tooltip_strings(
    dora_enabled: bool,
    dora_faces: &[(crate::core::tile::Suit, u8)],
) -> (String, String, String) {
    use crate::core::tile::Tile;
    let title = "Dora bonus".to_string();
    let cta = if dora_enabled && !dora_faces.is_empty() {
        let s = dora_faces
            .iter()
            .map(|(suit, rank)| Tile::new(*suit, *rank, 0).full_name())
            .collect::<Vec<_>>()
            .join(", ");
        format!("Current faces: {s}")
    } else {
        String::new()
    };
    let desc = if !dora_enabled {
        "Unlocks at Ante 4. When active, the plinth shows bonus tile faces; each match in your scored hand is worth +25 chips.".to_string()
    } else {
        "Bonus faces appear on the plinth. Each tile in your hand that matches a bonus face scores +25 chips when you cash out. Relics can reveal extra faces.\n\nMatching tiles in your hand glow."
            .to_string()
    };
    (title, cta, desc)
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
    if dora_faces.iter().any(|d| *d == face) {
        lines.push(("Dora · +25 chips".into(), color::GOLD));
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
