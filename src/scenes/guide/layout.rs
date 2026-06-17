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
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
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

use super::GuideNav;
use super::{PAGE_ECONOMY, PAGE_FLOWERS, PAGE_MELDS, PAGE_SCORING, PAGE_TILES, PAGE_YAKU};

// ── Guide layout frame ────────────────────────────────────────────────────

/// Margins and reserved header / nav bands for every guide page.
pub(crate) struct GuideLayout {
    pub(crate) window_w: f32,
    pub(crate) window_h: f32,
    margin: f32,
    pub(crate) content_x: f32,
    pub(crate) content_w: f32,
    pub(crate) content_bottom: f32,
    header_btn_h: f32,
}

pub(crate) struct GuideHeaderChrome {
    pub(crate) back: [f32; 4],
    pub(crate) prev: [f32; 4],
    pub(crate) next: [f32; 4],
    pub(crate) page_counter: [f32; 4],
}

pub(crate) struct GuideNavHeader {
    pub(crate) copy_x: f32,
    pub(crate) title_y: f32,
    pub(crate) subtitle_y: f32,
    pub(crate) content_top: f32,
    pub(crate) title_font: f32,
    pub(crate) body_font: f32,
}

pub(super) fn guide_copy_inset_x(w: f32) -> f32 {
    w * 0.055
}

pub(crate) fn page_nav_subtitle(page: usize) -> Option<&'static str> {
    match page {
        PAGE_TILES => Some(tiles_intro_copy::PAGE_SUBTITLE),
        PAGE_MELDS => Some(melds_intro_copy::PAGE_SUBTITLE),
        PAGE_YAKU => Some(yaku_intro_copy::PAGE_SUBTITLE),
        PAGE_FLOWERS => Some(flowers_intro_copy::PAGE_SUBTITLE),
        PAGE_SCORING => Some(scoring_intro_copy::SUBTITLE),
        PAGE_ECONOMY => Some(economy_intro_copy::SUBTITLE),
        _ => None,
    }
}

pub(crate) fn guide_nav_header(
    w: f32,
    h: f32,
    back: [f32; 4],
    subtitle: Option<&str>,
) -> GuideNavHeader {
    let scale = metrics::scene_scale(w, h);
    let title_font = typography::size(typography::H20, h);
    let body_font = typography::size(typography::H42, h);
    let jr = (w.min(h) / 720.0).clamp(1.0, 1.38);
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        guide_copy_inset_x(w),
        (18.0 * scale).max(10.0),
        title_font,
        jr,
    );
    let subtitle_w = w * 0.72;
    let divider_y = if let Some(sub) = subtitle {
        let subtitle_line_h = styled_text::styled_line_block_height_at_font_px(
            sub,
            subtitle_w,
            body_font,
            GlossaryMode::Prose,
            color::PARCHMENT,
        );
        title.subtitle_y + subtitle_line_h
    } else {
        HeaderChromeMetrics::from_window(w, h).chrome_bottom()
    };
    GuideNavHeader {
        copy_x: title.copy_x,
        title_y: title.title_y,
        subtitle_y: title.subtitle_y,
        content_top: divider_y,
        title_font,
        body_font,
    }
}

impl GuideLayout {
    pub(crate) fn new(w: f32, h: f32) -> Self {
        let chrome = HeaderChromeMetrics::from_window(w, h);
        let content_bottom = h - screen_footer_reserve(w, h) - 12.0 * chrome.ui;
        Self {
            window_w: w,
            window_h: h,
            margin: chrome.margin,
            content_x: chrome.margin,
            content_w: w - chrome.margin * 2.0,
            content_bottom,
            header_btn_h: chrome.button_h,
        }
    }

    pub(crate) fn header_chrome(&self) -> GuideHeaderChrome {
        let chrome_metrics = HeaderChromeMetrics::from_window(self.window_w, self.window_h);
        let btn_h = self.header_btn_h;
        let btn_gap = 10.0 * (chrome_metrics.margin / 48.0);
        let row_y = self.margin;

        let back = chrome_metrics.back_rect_left();

        let arrow_w = btn_h * 1.12;
        let right_edge = self.window_w - chrome_metrics.margin;
        let next = [right_edge - arrow_w, row_y, arrow_w, btn_h];

        let counter_w = (112.0 * (chrome_metrics.margin / 48.0)).clamp(96.0, 140.0);
        let counter_x = next[0] - btn_gap - counter_w;
        let page_counter = [counter_x, row_y, counter_w, btn_h];

        let prev = [counter_x - btn_gap - arrow_w, row_y, arrow_w, btn_h];

        GuideHeaderChrome {
            back,
            prev,
            next,
            page_counter,
        }
    }
}

pub(super) fn push_guide_panel_stroke(frame: &mut UiFrame, rect: [f32; 4], color: [f32; 4]) {
    let [x, y, w, h] = rect;
    let t = 1.0;
    frame.quad(GpuInstance {
        rect: [x, y, w, t],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y + h - t, w, t],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x, y, t, h],
        color,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + w - t, y, t, h],
        color,
        user: 0,
    });
}

/// Header rule below the nav title band. Returns top of the content band.
pub(crate) fn push_guide_chrome(frame: &mut UiFrame, layout: &GuideLayout, divider_y: f32) -> f32 {
    let jr = (layout.window_w.min(layout.window_h) / 720.0).clamp(1.0, 1.38);
    frame.quad(GpuInstance {
        rect: [layout.content_x, divider_y, layout.content_w, 1.0],
        color: color::alpha(color::STONE, 0.45),
        user: 0,
    });

    divider_y + 1.0 + (18.0 * jr).max(14.0)
}

pub(crate) fn push_guide_header_nav(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    tree: &TreeState,
    page: usize,
    pages: usize,
    scale: f32,
    w: f32,
    h: f32,
    page_title: &str,
    nav_header: &GuideNavHeader,
    subtitle: Option<&str>,
) {
    let prev_enabled = page > 0;
    let next_enabled = page + 1 < pages;
    let chrome = layout.header_chrome();
    let items = [
        FlatItem::new(GuideNav::Back.id(), chrome.back, GuideNav::Back),
        FlatItem::new(GuideNav::Prev.id(), chrome.prev, GuideNav::Prev),
        FlatItem::new(GuideNav::Next.id(), chrome.next, GuideNav::Next),
    ];
    let mut nav_quads = Vec::new();
    let mut nav_texts = Vec::new();
    let mut junk_buttons = Vec::new();
    for item in &items {
        let focused = tree.focused() == Some(item.id);
        let (label, variant, state) = match item.action {
            GuideNav::Prev => {
                let state = if !prev_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("◀️", ButtonVariant::Default, state)
            }
            GuideNav::Back => (
                "Back",
                ButtonVariant::Default,
                if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                },
            ),
            GuideNav::Next => {
                let state = if !next_enabled {
                    ButtonState::Disabled
                } else if focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                };
                ("▶️", ButtonVariant::Default, state)
            }
        };
        widget::push_button(
            &mut nav_quads,
            &mut nav_texts,
            &mut junk_buttons,
            widget::ButtonSpec {
                rect: item.rect,
                label,
                variant,
                state,
                action: UiAction::Confirm,
            },
        );
        if focused {
            focus_nav::push_focus_ring(item.rect, scale, w, h, &mut nav_quads);
        }
    }
    frame.quads(nav_quads);
    for label in nav_texts {
        frame.text(label);
    }
    let counter_font = typography::size(typography::H28, h);
    let counter_line_h = styled_text::colored_row_line_step(counter_font);
    let btn_y = chrome.prev[1];
    let btn_h = chrome.prev[3];
    let counter_rect = [
        chrome.page_counter[0],
        btn_y + (btn_h - counter_line_h) * 0.5,
        chrome.page_counter[2],
        counter_line_h,
    ];
    frame.text(TextLabel {
        rect: counter_rect,
        text: format!("{} / {}", page + 1, pages),
        color: color::UMBER,
        align: TextAlign::Center,
        font_px: Some(counter_font),
        bold: true,
        ..Default::default()
    });
    junk_buttons.clear();
    tree.register_flat_buttons(&items, &mut frame.buttons);

    frame.text(TextLabel {
        rect: [
            nav_header.copy_x,
            nav_header.title_y,
            w * 0.55,
            nav_header.title_font * 1.15,
        ],
        text: page_title.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(nav_header.title_font),
        bold: true,
        ..Default::default()
    });
    if let Some(sub) = subtitle {
        let mut subtitle_labels = Vec::new();
        styled_text::push_colored_line_left(
            &mut subtitle_labels,
            nav_header.copy_x,
            nav_header.subtitle_y,
            w * 0.72,
            nav_header.body_font,
            sub,
            color::PARCHMENT,
            GlossaryMode::Prose,
        );
        frame.texts(subtitle_labels);
    }
}
