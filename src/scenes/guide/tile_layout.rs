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

use super::content::MeldLabel;
use super::TileGroup;

// ── Tile layout ───────────────────────────────────────────────────────────

pub(super) fn layout_tile_groups_with_max(
    cam: &CameraParams,
    groups: &[&TileGroup],
    window_w: f32,
    window_h: f32,
    center_y: f32,
    x_span: Option<[f32; 2]>,
    max_tile: f32,
    width_fill: f32,
) -> (Vec<ShowcaseTilePlacement>, Vec<MeldLabel>) {
    if groups.is_empty() {
        return (vec![], vec![]);
    }

    let total_tiles: usize = groups.iter().map(|g| g.tiles.len()).sum();
    let num_gaps = groups.len().saturating_sub(1);

    let (layout_w, layout_origin) = match x_span {
        Some([x0, x1]) => (x1 - x0, x0),
        None => (window_w, 0.0),
    };

    // Compute tile size to fill `width_fill` of the layout span, capped for readability.
    let gap_equiv = num_gaps as f32 * 0.6; // gap = 0.6 tile widths
    let tile_size = ((layout_w * width_fill) / (total_tiles as f32 + gap_equiv))
        .min(max_tile)
        .max(30.0);
    let gap = tile_size * 0.6;

    let total_w = total_tiles as f32 * tile_size + num_gaps as f32 * gap;
    let start_x = layout_origin + (layout_w - total_w) * 0.5;

    let scale = (window_w.min(window_h)) / 600.0;
    let label_gaps = ShowcaseTileLabelGaps {
        underline_gap: (8.0 * scale).max(5.0),
        underline_h: (3.0 * scale).max(2.0),
        label_text_gap: (5.0 * scale).max(3.0),
    };

    let mut placements = Vec::with_capacity(total_tiles);
    let mut labels = Vec::new();
    let mut cursor_x = start_x;

    for group in groups {
        let group_start_x = cursor_x;
        let mut centers_xy = Vec::with_capacity(group.tiles.len());

        for tile in &group.tiles {
            let px = cursor_x + tile_size * 0.5;
            centers_xy.push([px, center_y]);
            placements.push(ShowcaseTilePlacement {
                tile: *tile,
                center_pos: [px, center_y, 0.0],
                rotation: DOC_TILE_ROTATION,
                scale: 1.0,
                size_px: tile_size,
                brightness: 1.0,
                opacity: 1.0,
                selected: false,
                hovered: false,
                outline: false,
                glow: false,
                glow_color: None,
                outline_sel: None,
                pick_id: None,
                overlay_rect_group: None,
            });
            cursor_x += tile_size;
        }

        let group_w = cursor_x - group_start_x;
        let bounds = showcase_tile_merge_projected_group(
            cam,
            window_w,
            window_h,
            TilePreset::Chinese,
            DOC_TILE_ROTATION,
            1.0,
            tile_size,
            0.0,
            &centers_xy,
        );
        let anchor = showcase_tile_group_label_anchor(bounds, label_gaps);
        labels.push(MeldLabel {
            x: group_start_x,
            y: anchor.label_y,
            w: group_w,
            underline_y: anchor.underline_y,
            text: group.label.to_string(),
            color: group.accent,
        });

        cursor_x += gap;
    }

    (placements, labels)
}

/// Short, journal-friendly rule text for each yaku.
pub(crate) fn yaku_shape_text(yk: YakuKind) -> &'static str {
    match yk {
        YakuKind::ChickenHand => "Valid hand with no other yaku",
        YakuKind::Tanyao => "All tiles ranked 2\u{2013}8",
        YakuKind::Toitoi => "At least two triplets/kongs, no sequences",
        YakuKind::Shousangen => {
            "Two dragon triplets/kongs plus a pair of the third dragon (full hand)"
        }
        YakuKind::Daisangen => "All three dragon triplets/kongs (full hand)",
        YakuKind::Yakuhai => "Each dragon or matching wind triplet/kong; stacks",
        YakuKind::Iipeikou => "Two identical sequences in the same suit",
        YakuKind::Ryanpeikou => "Two doubled sequences in one suit (full hand)",
        YakuKind::SanshokuDoujun => "Same sequence in all three suits",
        YakuKind::SanshokuDoukou => "Same-rank triplet in all three suits",
        YakuKind::Ittsu => "1\u{2013}9 straight in one suit",
        YakuKind::Honitsu => "One number suit plus honors only",
        YakuKind::Chinitsu => "Single number suit, no honors",
        YakuKind::Junchan => "Every group has a 1 or 9; needs a sequence, no honors",
        YakuKind::Honroutou => "Only terminals and honors (1, 9, winds, dragons)",
        YakuKind::Chanta => {
            "Every group has a terminal or honor; needs honor, simple, and a sequence"
        }
        YakuKind::Chiitoitsu => "Seven distinct pairs",
        YakuKind::KokushiMusou => "One of each terminal and honor, plus one duplicate",
        YakuKind::Pinfu => "Four sequences plus a 2\u{2013}8 pair (full hand)",
    }
}
