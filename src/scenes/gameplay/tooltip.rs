use crate::game::engine::GameEngine;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

/// Plain-language hand-shape description for a yaku, mirrored from the
/// glossary so the gameplay tooltip and the help overlay agree.
pub(super) fn yaku_card_shape_text(yk: crate::core::yaku::YakuKind) -> &'static str {
    // Suit emoji match tile_suit_emoji: 🎴 Characters, 🎋 Bamboo, 🔴 Circles.
    // Honor emoji: 🐉 Dragon, 🌬 Wind.
    use crate::core::yaku::YakuKind;
    match yk {
        YakuKind::Tanyao => {
            "All tiles 2\u{2013}8, no honors or terminals (e.g. \u{1f3b4}234 \u{1f38b}567 \u{1f534}88)"
        }
        YakuKind::Toitoi => {
            "All triplets and kongs, no sequences (e.g. \u{1f3b4}222 \u{1f38b}555 \u{1f534}999)"
        }
        YakuKind::FullHand => "Complete 14-tile hand: 4+4+4+4+2 (4 melds + 1 pair), not 2x7",
        YakuKind::Yakuhai => {
            "Triplet of any dragon or round wind (e.g. \u{1f409}\u{1f409}\u{1f409})"
        }
        YakuKind::Iipeikou => {
            "Two identical sequences in one suit (e.g. \u{1f38b}123 \u{1f38b}123)"
        }
        YakuKind::SanshokuDoujun => {
            "Same sequence in all 3 suits (e.g. \u{1f3b4}456 \u{1f38b}456 \u{1f534}456)"
        }
        YakuKind::Ittsu => {
            "1\u{2013}9 straight in one suit (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789)"
        }
        YakuKind::Honitsu => {
            "One number suit + honors only (e.g. \u{1f38b}234 \u{1f38b}678 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chinitsu => {
            "All one number suit, no honors (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789 \u{1f38b}11)"
        }
        YakuKind::Junchan => {
            "Every meld has a 1 or 9 (e.g. \u{1f38b}123 \u{1f3b4}789 \u{1f534}111 \u{1f38b}99)"
        }
        YakuKind::Honroutou => {
            "Only 1s, 9s, and honors (e.g. \u{1f38b}111 \u{1f3b4}999 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chiitoitsu => {
            "Seven distinct pairs (e.g. \u{1f3b4}11 \u{1f3b4}33 \u{1f38b}55 \u{1f38b}77 \u{1f534}22 \u{1f534}44 \u{1f32c}\u{1f32c})"
        }
        YakuKind::ChickenHand => {
            "Valid hand with no yaku \u{2014} scores base chips \u{00d7} 1 mult"
        }
    }
}

/// Render a small tooltip panel anchored above-left of `(anchor_x, anchor_y)`.
/// Used by the gameplay HUD's hover-tooltip pass for zodiac slots and yaku
/// progress cards. Mirrors the styling of the existing relic tooltip block
/// (dark midnight panel + brass border + champagne title + parchment body).
///
/// Title pins its font_px so even long names like "Sanshoku Doujun (+4 mult,
/// +50 chips)" don't get auto-shrunk into illegibility, and the body uses
/// `push_text_block` with a pinned CAPTION-tier font so multi-line wrapping
/// renders at a single readable size instead of squeezing to the 8px floor.
pub(super) fn push_tooltip(
    instances: &mut Vec<GpuInstance>,
    text_labels: &mut Vec<TextLabel>,
    anchor: (f32, f32),
    viewport: crate::ui::layout::ViewportCtx,
    title: &str,
    body: &str,
) {
    use crate::render::theme::{color, typography};
    use crate::ui::widget::{self, TextStyle};
    let (anchor_x, anchor_y) = anchor;
    let crate::ui::layout::ViewportCtx {
        window_w,
        window_h,
        ui_scale,
    } = viewport;

    let pad = 14.0_f32;
    let tip_w = (window_w * 0.34).clamp(300.0, 500.0);

    // Pin font sizes — never let the rasterizer auto-shrink below readable.
    let title_font = typography::size(typography::BODY, window_h, ui_scale).max(15.0);
    let body_font = typography::size(typography::CAPTION, window_h, ui_scale).max(13.0);
    let title_h = title_font * 1.6;
    let body_line_step = body_font * 1.4;

    // Estimate body line count from text length / approx chars per line at the
    // pinned body font size. Each glyph ~ body_font * 0.55 wide on average.
    let inner_w = tip_w - pad * 2.0;
    let chars_per_line = (inner_w / (body_font * 0.55)).max(10.0) as usize;
    let est_lines = (body.len() / chars_per_line + 1).max(2) as f32;
    let body_h = (est_lines * body_line_step).min(window_h * 0.5);

    let tip_h = pad * 2.0 + title_h + 4.0 + body_h;
    let mut tip_x = anchor_x - tip_w * 0.5;
    let mut tip_y = anchor_y - tip_h - 8.0;
    tip_x = tip_x.clamp(8.0, window_w - tip_w - 8.0);
    if tip_y < 8.0 {
        tip_y = anchor_y + 8.0;
    }
    if tip_y + tip_h > window_h - 8.0 {
        tip_y = (window_h - tip_h - 8.0).max(8.0);
    }
    let bg = color::alpha(color::MIDNIGHT, 0.96);
    instances.push(GpuInstance {
        rect: [tip_x, tip_y, tip_w, tip_h],
        color: bg,
    });
    let bt = 1.5_f32;
    let border = color::BRASS;
    instances.push(GpuInstance {
        rect: [tip_x, tip_y, tip_w, bt],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x, tip_y + tip_h - bt, tip_w, bt],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x, tip_y + bt, bt, tip_h - bt * 2.0],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x + tip_w - bt, tip_y + bt, bt, tip_h - bt * 2.0],
        color: border,
    });
    text_labels.push(TextLabel {
        rect: [tip_x + pad, tip_y + pad, inner_w, title_h],
        text: title.into(),
        color: color::CHAMPAGNE,
        font_px: Some(title_font),
        ..Default::default()
    });
    widget::push_text_block(
        text_labels,
        [tip_x + pad, tip_y + pad + title_h + 4.0, inner_w, body_h],
        body,
        TextStyle {
            tier: typography::CAPTION,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Left,
        },
        window_h,
        ui_scale,
    );
}

pub(super) fn relic_tooltip_copy_detail(
    relic_id: crate::core::relic::RelicId,
    relic_index: usize,
    run: &crate::game::run::RunState,
) -> Option<String> {
    GameEngine::relic_tooltip_copy_detail(run, relic_id, relic_index)
}
