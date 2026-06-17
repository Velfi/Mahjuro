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

use super::layout::push_guide_panel_stroke;
use super::layout::{guide_nav_header, GuideLayout};

// ── Tanuki's Tips page (page 6) ───────────────────────────────────────────

pub(super) struct TanukiTipsScrollLayout {
    pub(super) viewport: [f32; 4],
    pub(super) cell_w: f32,
    pub(super) cell_h: f32,
    pub(super) gap: f32,
    pub(super) pad: f32,
    pub(super) rows: usize,
    pub(super) max_scroll_px: f32,
    pub(super) wheel_step_px: f32,
}

/// Content band top/bottom for a guide page (matches [`push_guide_chrome`] without drawing).
pub(super) fn guide_content_band(w: f32, h: f32, back: [f32; 4], subtitle: Option<&str>) -> (f32, f32) {
    let layout = GuideLayout::new(w, h);
    let nav_header = guide_nav_header(w, h, back, subtitle);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let content_top = nav_header.content_top + 1.0 + (18.0 * jr).max(14.0);
    (content_top, layout.content_bottom)
}

pub(super) fn tanuki_tips_scroll_layout(
    layout: &GuideLayout,
    content_top: f32,
    content_floor: f32,
) -> TanukiTipsScrollLayout {
    const ROWS: usize = 2;
    const VISIBLE_COLS: f32 = 2.35;
    let scale = metrics::scene_scale(layout.window_w, layout.window_h);
    let gap = (16.0 * scale).max(12.0);
    let pad = (14.0 * scale).max(10.0);
    let scroll_track_reserve = (14.0 * scale).max(10.0);
    let x = layout.content_x;
    let full_w = layout.content_w;
    let usable_h = (content_floor - content_top).max(1.0);
    let grid_h = (usable_h - scroll_track_reserve).max(1.0);
    let cell_h = ((grid_h - gap) / ROWS as f32).max(1.0);
    let tip_count = tanuki_tips_intro_copy::TIPS.len();
    let cols = tip_count.div_ceil(ROWS).max(1);
    let min_cell_w = (260.0 * scale).min(full_w * 0.24);
    let fill_cell_w = (full_w - gap * (cols.saturating_sub(1)) as f32) / cols as f32;
    let scroll_cell_w = (full_w - gap * (VISIBLE_COLS - 1.0)) / VISIBLE_COLS;
    let total_fill_w = cols as f32 * fill_cell_w + cols.saturating_sub(1) as f32 * gap;
    let cell_w = if total_fill_w <= full_w {
        fill_cell_w.max(min_cell_w)
    } else {
        scroll_cell_w.max(min_cell_w)
    };
    let total_w = cols as f32 * cell_w + cols.saturating_sub(1) as f32 * gap;
    let max_scroll_px = (total_w - full_w).max(0.0);
    TanukiTipsScrollLayout {
        viewport: [x, content_top, full_w, usable_h],
        cell_w,
        cell_h,
        gap,
        pad,
        rows: ROWS,
        max_scroll_px,
        wheel_step_px: (cell_w * 0.22).clamp(48.0 * scale, 120.0 * scale),
    }
}

pub(super) fn draw_tanuki_tips_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    h: f32,
    tips_layout: &TanukiTipsScrollLayout,
    scroll_px: f32,
) {
    let TanukiTipsScrollLayout {
        viewport,
        cell_w,
        cell_h,
        gap,
        pad,
        rows,
        ..
    } = *tips_layout;
    let quote_color = color::CHAMPAGNE;
    let stroke = color::alpha(color::STONE, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.22);
    let [vx, vy, vw, vh] = viewport;
    let content_x = layout.content_x;

    for (i, tip) in tanuki_tips_intro_copy::TIPS.iter().enumerate() {
        let col = i / rows;
        let row = i % rows;
        let cx = content_x + col as f32 * (cell_w + gap) - scroll_px;
        let cy = vy + row as f32 * (cell_h + gap);
        let rect = [cx, cy, cell_w, cell_h];
        let Some(clipped_panel) = intersect_rect(rect, viewport) else {
            continue;
        };

        let inner_w = (cell_w - pad * 2.0).max(1.0);
        let inner_h = (cell_h - pad * 2.0).max(1.0);
        let text_clip =
            intersect_rect([cx + pad, cy + pad, inner_w, inner_h], viewport).unwrap_or(viewport);
        let quote_text = tanuki_tips_intro_copy::quoted(tip);

        frame.quad(GpuInstance {
            rect: clipped_panel,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, clipped_panel, stroke);

        let quote_area_h = inner_h;
        let mut font = typography::size(typography::H36, h);
        let min_font = typography::size(typography::H42, h);
        let wrapped = loop {
            let lines = styled_text::wrap_colored_text_multiline(
                &quote_text,
                inner_w,
                font / 0.99,
                quote_color,
                true,
                GlossaryMode::Prose,
            );
            let block_h = styled_text::colored_wrapped_rows_height(&lines, font);
            if block_h <= quote_area_h || font <= min_font {
                break lines;
            }
            font *= 0.94;
        };
        let quote_top = cy + pad;

        let mut labels = Vec::new();
        styled_text::push_colored_rows_left(
            &mut labels,
            styled_text::ColoredRowsLayout {
                text_left: text_clip[0],
                top_y: quote_top,
                inner_w: text_clip[2],
                line_h: font,
                fallback_plain: &quote_text,
                fallback_color: quote_color,
                italic: true,
                glossary: GlossaryMode::Prose,
            },
            &wrapped,
        );
        for label in &mut labels {
            label.clip_rect = Some(text_clip);
        }
        frame.texts(labels);
    }

    if tips_layout.max_scroll_px > 0.5 {
        let track_h = 4.0;
        let track_y = vy + vh - track_h - 6.0;
        let track = [vx, track_y, vw, track_h];
        frame.quad(GpuInstance {
            rect: track,
            color: color::alpha(color::STONE, 0.28),
            user: 0,
        });
        let thumb_w = (vw * (vw / (vw + tips_layout.max_scroll_px))).clamp(48.0, vw);
        let thumb_travel = (vw - thumb_w).max(0.0);
        let thumb_x = vx + thumb_travel * (scroll_px / tips_layout.max_scroll_px);
        frame.quad(GpuInstance {
            rect: [thumb_x, track_y, thumb_w, track_h],
            color: color::alpha(color::BRASS, 0.72),
            user: 0,
        });
    }
}

