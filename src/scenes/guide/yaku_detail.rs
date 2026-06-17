#![allow(unused_imports)]
use crate::core::hand::{MeldKind, validate_selection};
use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::core::progression::PlayerProgress;
use crate::core::relic::{RelicId, all_relic_defs, relic_visual};
use crate::core::tag::TagKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{PACK_ASPECT_W_OVER_H, TilePackKind};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
use crate::core::zodiac::ZodiacKind;
use crate::persistence::TilePreset;
use crate::render::consumable_prop_scale::for_sale_talisman_tablet_extent;
use crate::render::decal::load_ui_font;
use crate::render::doc_tile_camera::{DOC_TILE_ROTATION, doc_tile_camera};
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, ImageQuad, ImageQuadSource, Object3d, Object3dKind, SceneLighting,
    ShowcaseTilePlacement, UiFrame, camera_facing_euler_xyz_rad,
};
use crate::render::gameplay_glb;
use crate::render::scene_keys;
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::table_transform::{
    compose_rotation_euler, mat4_to_euler_xyz_rad, rot_euler_xyz_rad,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::vocabulary_colors::{GlossaryMode, text_effect_for_glossary_tint};
use crate::render::wgpu_renderer::{
    GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel,
};
use crate::render::world_space::{
    object3d_pos_triple_for_world_center, world_on_camera_ray_plane_z,
};
use crate::ui::chart_primitives::{ChartClip, push_yaku_pill, yaku_pill_width};
use crate::ui::clip::intersect_rect;
use crate::ui::controller_hints::screen_footer_reserve;
use crate::ui::styled_text;
use crate::ui::styled_text::push_keyword_label;
use crate::ui::temptation_icons::temptation_icon_source;
use crate::ui::widget::{self, wrap_text};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeState};

use crate::scenes::archive_career::{yaku_pill_face, yaku_pill_ink, yaku_pill_rim};
use crate::scenes::economy_intro_copy;
use crate::scenes::flowers_intro_copy;
use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};
use crate::scenes::melds_intro_copy;
use crate::scenes::scoring_intro_copy;
use crate::scenes::tanuki_tips_intro_copy;
use crate::scenes::tiles_intro_copy;
use crate::scenes::yaku_intro_copy;
use crate::scenes::{BackgroundId, DrawCtx};

use glam::{Mat4, Quat, Vec3};

use super::example_grid::push_tile_group_labels;
use super::tile_layout::layout_tile_groups_with_max;
use super::TileGroup;
use super::yaku_page::yaku_page;
/// Long-form copy for the in-game guide. Journal and shop use [`yaku_shape_text`].
pub(crate) struct YakuGuideDetail {
    pub rule: &'static str,
    pub requires: &'static str,
    pub breaks_if: &'static str,
}

pub(crate) fn yaku_guide_detail(yk: YakuKind) -> YakuGuideDetail {
    match yk {
        YakuKind::ChickenHand => YakuGuideDetail {
            rule: "Fallback score when the hand wins but no other pattern applies.",
            requires: "Standard winning shape",
            breaks_if: "Another yaku is present.",
        },
        YakuKind::Tanyao => YakuGuideDetail {
            rule: "Every tile is a simple — a number ranked 2–8.",
            requires: "Simples only",
            breaks_if: "Any terminal, wind, or dragon.",
        },
        YakuKind::Yakuhai => YakuGuideDetail {
            rule: "Each dragon triplet/kong scores; so does a wind triplet/kong matching round or bonus round winds. Stacks.",
            requires: "Honor triplet or kong per Yakuhai",
            breaks_if: "Honor is only a pair, or wind doesn't match round/bonus round.",
        },
        YakuKind::Toitoi => YakuGuideDetail {
            rule: "At least two triplets or kongs; sequences don't count.",
            requires: "2+ triplets/kongs",
            breaks_if: "Any sequence appears.",
        },
        YakuKind::Honitsu => YakuGuideDetail {
            rule: "Tiles from one number suit, optionally mixed with honors.",
            requires: "Single number suit",
            breaks_if: "A second number suit appears.",
        },
        YakuKind::Iipeikou => YakuGuideDetail {
            rule: "Two identical sequences in the same suit. Ryanpeikou replaces this when both qualify.",
            requires: "Two matching sequences · one suit",
            breaks_if: "Sequences differ or span suits.",
        },
        YakuKind::Junchan => YakuGuideDetail {
            rule: "Every group contains a 1 or 9; needs at least one sequence. No honors. Beats Honroutou and Chanta when multiple qualify.",
            requires: "1 or 9 in every group · one sequence",
            breaks_if: "All groups are terminal-only triplets/pairs (Honroutou instead).",
        },
        YakuKind::Honroutou => YakuGuideDetail {
            rule: "Only terminals (1, 9) and honors — no simples. Beats Chanta when both qualify.",
            requires: "Terminals and honors only",
            breaks_if: "Any number tile 2–8 appears.",
        },
        YakuKind::Shousangen => YakuGuideDetail {
            rule: "Two dragon triplets/kongs plus a pair of the third dragon on a full hand. Replaces dragon Yakuhai.",
            requires: "2 dragon melds · 1 dragon pair · full hand",
            breaks_if: "All three dragons are triplets (Daisangen instead).",
        },
        YakuKind::Daisangen => YakuGuideDetail {
            rule: "All three dragon triplets/kongs on a full hand. Replaces dragon Yakuhai.",
            requires: "Red · green · white dragon melds · full hand",
            breaks_if: "Only two dragon triplets (Shousangen instead).",
        },
        YakuKind::Chinitsu => YakuGuideDetail {
            rule: "Pure suit — every tile from one number suit.",
            requires: "Single number suit · no honors",
            breaks_if: "Any honor or second number suit appears.",
        },
        YakuKind::SanshokuDoujun => YakuGuideDetail {
            rule: "The same sequence (e.g. 4-5-6) appears once in each number suit.",
            requires: "Matching sequence · Manzu · Souzu · Pinzu",
            breaks_if: "Ranks differ or a suit is missing.",
        },
        YakuKind::Ittsu => YakuGuideDetail {
            rule: "Three sequences forming 1–9 in a single suit (1-2-3, 4-5-6, 7-8-9).",
            requires: "All three ranges · same suit",
            breaks_if: "Sequences split across suits.",
        },
        YakuKind::Chiitoitsu => YakuGuideDetail {
            rule: "Alternate shape: seven distinct pairs instead of four melds.",
            requires: "7 different pair types",
            breaks_if: "Any pair type repeats.",
        },
        YakuKind::KokushiMusou => YakuGuideDetail {
            rule: "One of each terminal and honor, plus one duplicate among them.",
            requires: "All 13 types · one duplicate",
            breaks_if: "A type is missing or duplicate isn't a terminal/honor.",
        },
        YakuKind::Chanta => YakuGuideDetail {
            rule: "Every group touches a terminal or honor, with at least one honor, one simple, and one sequence. Weakest of the terminal-family yaku — Junchan or Honroutou replace it when they qualify.",
            requires: "Terminal/honor in every group · honor · simple · sequence",
            breaks_if: "A group is all simples (middle-only), or the pair lacks a terminal/honor.",
        },
        YakuKind::Ryanpeikou => YakuGuideDetail {
            rule: "Two different sequences, each appearing twice, all in one suit. Full hand; replaces Iipeikou.",
            requires: "Two duplicated sequences · same suit · full hand",
            breaks_if: "Only one sequence is duplicated, or suits differ.",
        },
        YakuKind::SanshokuDoukou => YakuGuideDetail {
            rule: "The same rank triplet (or kong) in all three number suits.",
            requires: "Same rank · triplet/kong · all three suits",
            breaks_if: "Groups are sequences, not triplets.",
        },
        YakuKind::Pinfu => YakuGuideDetail {
            rule: "All sequences, no triplets — pair is a simple (2–8) in a number suit. Full hand.",
            requires: "4 sequences · simple number pair",
            breaks_if: "Triplets/kongs or honor pair.",
        },
    }
}

pub(super) fn draw_yaku_guide_page(
    frame: &mut UiFrame,
    _progress: &PlayerProgress,
    w: f32,
    h: f32,
    scale: f32,
    yaku: &[YakuKind],
    body_top: f32,
    content_floor: f32,
    cam: &CameraParams,
) {
    if yaku.is_empty() {
        return;
    }
    let n = yaku.len();
    let band_gap = h * 0.014;
    let band_h = (content_floor - body_top - band_gap * (n.saturating_sub(1)) as f32) / n as f32;

    for (i, &yk) in yaku.iter().enumerate() {
        let band_top = body_top + i as f32 * (band_h + band_gap);
        let band_bottom = band_top + band_h;
        let detail = yaku_guide_detail(yk);
        let (_, groups) = yaku_page(yk);
        draw_yaku_entry(
            frame,
            w,
            h,
            scale,
            yk,
            &detail,
            &groups,
            band_top,
            band_bottom,
            cam,
            n > 1,
        );
        if i + 1 < n {
            let div_y = band_bottom + band_gap * 0.35;
            frame.quad(GpuInstance {
                rect: [w * 0.06, div_y, w * 0.88, (1.5 * scale).max(1.0)],
                color: color::alpha(color::STONE, 0.35),
                user: 0,
            });
        }
    }
}

pub(super) fn draw_yaku_entry(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    scale: f32,
    yk: YakuKind,
    detail: &YakuGuideDetail,
    groups: &[TileGroup],
    band_top: f32,
    band_bottom: f32,
    cam: &CameraParams,
    compact: bool,
) {
    let name_font = typography::size(
        if compact {
            typography::H24
        } else {
            typography::H20
        },
        h,
    );
    let stats_font = typography::size(typography::H42, h);
    let body_font = typography::size(typography::H36, h);
    let label_font = typography::size(typography::H28, h);
    let pad = w * 0.05;
    let inner_w = w - pad * 2.0;

    let name_h = name_font * (if compact { 1.12 } else { 1.22 });
    frame.text(TextLabel {
        rect: [pad, band_top, inner_w, name_h],
        text: yk.name().into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(name_font),
        bold: true,
        ..Default::default()
    });

    let stats = format!("+{} Han · +{} Fu", yk.han_bonus(), yk.fu_bonus());
    let stats_y = band_top + name_h + h * 0.002;
    let stats_h = push_dense_text_lines(
        frame,
        [pad, stats_y, inner_w, 0.0],
        &stats,
        stats_font,
        color::alpha(color::CHAMPAGNE, 0.82),
        1.18,
    );

    let rule_y = stats_y + stats_h + h * 0.004;
    let rule_h = push_dense_text(
        frame,
        [pad, rule_y, inner_w, 0.0],
        detail.rule,
        body_font,
        color::PARCHMENT,
    );

    let cols_top = rule_y + rule_h + h * (if compact { 0.006 } else { 0.012 });
    let col_gap = w * 0.02;
    let text_col_w = inner_w * 0.32;
    let tile_col_w = inner_w - text_col_w - col_gap;
    let breaks_reserve = body_font * (if compact { 1.35 } else { 1.55 });
    let col_h = (band_bottom - cols_top - breaks_reserve - h * 0.008).max(h * 0.12);

    let requires_label = format!("Requires: {}", detail.requires);
    push_dense_text(
        frame,
        [pad, cols_top, text_col_w, col_h],
        &requires_label,
        label_font,
        color::STONE,
    );

    let tile_x = pad + text_col_w + col_gap;
    let tile_center_y = cols_top + col_h * 0.48;
    let group_refs: Vec<&TileGroup> = groups.iter().collect();
    let max_tile = if compact { h * 0.17 } else { h * 0.24 };
    let (placements, labels) = layout_tile_groups_with_max(
        cam,
        &group_refs,
        w,
        h,
        tile_center_y,
        Some([tile_x, tile_x + tile_col_w]),
        max_tile,
        0.98,
    );
    if !placements.is_empty() {
        frame
            .cmds
            .push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tile_group_labels(frame, &labels, h, scale, false);

    let breaks_y = band_bottom - breaks_reserve;
    let breaks_text = format!("Breaks if: {}", detail.breaks_if);
    push_dense_text(
        frame,
        [pad, breaks_y, inner_w, 0.0],
        &breaks_text,
        body_font,
        color::STONE,
    );
}

pub(crate) fn push_dense_text(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
) -> f32 {
    push_dense_text_lines(
        frame,
        rect,
        text,
        font_px,
        color,
        widget::PLAIN_TEXT_LINE_STEP_MUL,
    )
}

pub(crate) fn dense_text_block_height(text: &str, width: f32, font_px: f32) -> f32 {
    let line_mul = widget::PLAIN_TEXT_LINE_STEP_MUL;
    let wrapped = styled_text::wrap_colored_text_multiline(
        text,
        width,
        font_px / 0.99,
        color::PARCHMENT,
        false,
        GlossaryMode::Prose,
    );
    font_px * line_mul * wrapped.len().max(1) as f32
}

pub(super) fn push_dense_text_lines(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
    line_mul: f32,
) -> f32 {
    push_dense_text_lines_aligned(frame, rect, text, font_px, color, line_mul, TextAlign::Left)
}

pub(super) fn push_dense_text_lines_aligned(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
    line_mul: f32,
    align: TextAlign,
) -> f32 {
    let line_h = font_px * line_mul;
    let wrapped = styled_text::wrap_colored_text_multiline(
        text,
        rect[2],
        font_px / 0.99,
        color,
        false,
        GlossaryMode::Prose,
    );
    let block_h = line_h * wrapped.len().max(1) as f32;
    let Some(font) = load_ui_font() else {
        let wrapped = wrap_text(text, rect[2], font_px / 0.99);
        frame.text(TextLabel {
            rect: [rect[0], rect[1], rect[2], block_h],
            text: wrapped.join("\n"),
            color,
            align,
            font_px: Some(font_px),
            ..Default::default()
        });
        return block_h;
    };

    for (row, chunks) in wrapped.iter().enumerate() {
        let line_y = rect[1] + row as f32 * line_h;
        let measured: f32 = chunks
            .iter()
            .map(|(s, _)| {
                s.chars()
                    .map(|ch| font.metrics(ch, font_px).advance_width)
                    .sum::<f32>()
            })
            .sum();
        let mut cx = match align {
            TextAlign::Left => rect[0],
            TextAlign::Center => rect[0] + (rect[2] - measured) * 0.5,
            TextAlign::Right => rect[0] + rect[2] - measured,
        };
        for (s, c) in chunks {
            let piece_w = s
                .chars()
                .map(|ch| font.metrics(ch, font_px).advance_width)
                .sum::<f32>()
                .max(1.0);
            let mut chunk_labels = Vec::new();
            push_keyword_label(
                &mut chunk_labels,
                TextLabel {
                    rect: [cx, line_y, piece_w, line_h],
                    text: s.clone(),
                    color: *c,
                    font_px: Some(font_px),
                    align: TextAlign::Left,
                    text_effect: text_effect_for_glossary_tint(*c),
                    ..Default::default()
                },
                color,
                true,
            );
            for lbl in chunk_labels {
                frame.text(lbl);
            }
            cx += piece_w;
        }
    }
    block_h
}
