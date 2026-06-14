//! Sidebar scroll layout and content-height measurement.

use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{ModifierBreakdown, SelectedTileDetails, WallStats};
use crate::render::theme::color;
use crate::render::vocabulary_colors::GlossaryMode;
use crate::ui::styled_text::styled_line_block_height_at_font_px;

use super::draw::copies_panel_height;
use super::layout::{WallLayout, text_line_h, wall_progress_bar_block_h};

pub struct SidebarScrollLayout {
    pub max_scroll_px: f32,
    pub wheel_step_px: f32,
    pub clip: [f32; 4],
    pub content_h: f32,
    /// Scrollable panel width — [`WallLayout::summary_w`] minus scrollbar gutter when overflow.
    pub content_w: f32,
    pub scrollbar_gutter: f32,
}

/// Horizontal reserve when the sidebar scrolls: gap + track + outer inset.
pub fn sidebar_scrollbar_gutter(layout: &WallLayout) -> f32 {
    let track_w = (7.0 * layout.jr).max(5.0);
    let gap = (10.0 * layout.jr).max(8.0);
    let outer = (6.0 * layout.jr).max(4.0);
    track_w + gap + outer
}

#[derive(Clone, Copy)]
pub struct SidebarScrollbar {
    pub track: [f32; 4],
    pub thumb: [f32; 4],
    pub hit_track: [f32; 4],
    pub thumb_travel: f32,
}

#[inline]
pub fn point_in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3]
}

pub fn sidebar_scrollbar(
    layout: &WallLayout,
    clip: [f32; 4],
    content_h: f32,
    scroll_y: f32,
    max_scroll_px: f32,
) -> Option<SidebarScrollbar> {
    if max_scroll_px <= 0.0 {
        return None;
    }
    let track_w = (7.0 * layout.jr).max(5.0);
    let track_pad = (6.0 * layout.jr).max(4.0);
    let track_x = clip[0] + clip[2] - track_pad - track_w;
    let track = [track_x, clip[1], track_w, clip[3]];
    let viewport_h = clip[3];
    let thumb_h = (track[3] * (viewport_h / content_h.max(1.0)))
        .clamp((18.0 * layout.jr).max(14.0), track[3]);
    let thumb_travel = (track[3] - thumb_h).max(0.0);
    let thumb_t = scroll_y / max_scroll_px;
    let thumb_y = track[1] + thumb_travel * thumb_t;
    let hit_pad_x = (8.0 * layout.jr).max(6.0);
    Some(SidebarScrollbar {
        track,
        thumb: [track_x, thumb_y, track_w, thumb_h],
        hit_track: [
            track_x - hit_pad_x,
            track[1],
            track_w + hit_pad_x * 2.0,
            track[3],
        ],
        thumb_travel,
    })
}

pub fn sidebar_scroll_y_from_cursor(
    my: f32,
    grab_y: f32,
    sb: &SidebarScrollbar,
    max_scroll_px: f32,
) -> f32 {
    if sb.thumb_travel <= 0.0 {
        return 0.0;
    }
    let thumb_top = (my - grab_y - sb.track[1]).clamp(0.0, sb.thumb_travel);
    (thumb_top / sb.thumb_travel) * max_scroll_px
}

pub fn push_sidebar_scrollbar(
    frame: &mut crate::render::draw_cmd::UiFrame,
    sb: &SidebarScrollbar,
    dragging: bool,
) {
    use crate::render::wgpu_renderer::GpuInstance;

    frame.quad(GpuInstance {
        rect: sb.track,
        color: color::alpha(color::WALNUT_RAISED, 0.88),
        user: 0,
    });
    let thumb_alpha = if dragging { 1.0 } else { 0.9 };
    frame.quad(GpuInstance {
        rect: sb.thumb,
        color: color::alpha(color::BRASS, thumb_alpha),
        user: 0,
    });
}

pub struct SidebarScrollDraw {
    pub content_top: f32,
    pub scroll_y: f32,
    pub clip: [f32; 4],
    pub x: f32,
    pub w: f32,
    pub pad: f32,
}

impl SidebarScrollDraw {
    pub fn screen_y(&self, logical_y: f32) -> f32 {
        self.content_top + logical_y - self.scroll_y
    }

    pub fn visible(&self, screen_y: f32, h: f32) -> bool {
        screen_y + h > self.clip[1] && screen_y < self.clip[1] + self.clip[3]
    }

    pub fn content_x(&self) -> f32 {
        self.x + self.pad
    }

    pub fn inner_w(&self) -> f32 {
        (self.w - self.pad * 2.0).max(1.0)
    }
}

pub fn sidebar_scroll_layout(
    layout: &WallLayout,
    _stats: &WallStats,
    details: Option<&SelectedTileDetails>,
    mode: WallLedgerMode,
) -> SidebarScrollLayout {
    let pad = layout.summary_pad();
    let title_line = text_line_h(layout.caption_px);
    let content_top = layout.summary_y + pad + title_line + layout.section_inner_gap() + 4.0;
    let content_bottom = layout.summary_y + layout.summary_h - pad;
    let viewport_h = (content_bottom - content_top).max(0.0);
    let probe_h =
        measure_sidebar_content_height(layout, viewport_h, details, mode, layout.summary_w);
    let scrollbar_gutter = if probe_h > viewport_h {
        sidebar_scrollbar_gutter(layout)
    } else {
        0.0
    };
    let content_w = layout.summary_w - scrollbar_gutter;
    let content_h = measure_sidebar_content_height(layout, viewport_h, details, mode, content_w);
    let max_scroll_px = (content_h - viewport_h).max(0.0);
    let wheel_step_px = (text_line_h(layout.caption_px) * 2.4).clamp(36.0, 80.0);
    SidebarScrollLayout {
        max_scroll_px,
        wheel_step_px,
        clip: [layout.summary_x, content_top, layout.summary_w, viewport_h],
        content_h,
        content_w,
        scrollbar_gutter,
    }
}

pub fn measure_sidebar_content_height(
    layout: &WallLayout,
    viewport_h: f32,
    details: Option<&SelectedTileDetails>,
    mode: WallLedgerMode,
    panel_w: f32,
) -> f32 {
    let lh = text_line_h(layout.caption_px);
    let count_lh = text_line_h(layout.count_px);
    let suit_row = count_lh.max(lh) + 2.0;
    let mut y = 0.0_f32;

    // Remaining / total progress bar
    y += wall_progress_bar_block_h(layout) + layout.section_divider_gap();

    // Suit balance
    y += lh + layout.section_inner_gap();
    y += suit_row * 5.0;
    y += layout.section_inner_gap() + layout.section_divider_gap();

    if let Some(details) = details {
        y += measure_detail_panel_height(layout, panel_w, viewport_h, details, mode);
    }

    y + layout.scroll_bottom_pad()
}

pub fn measure_detail_panel_height(
    layout: &WallLayout,
    panel_w: f32,
    viewport_h: f32,
    details: &SelectedTileDetails,
    mode: WallLedgerMode,
) -> f32 {
    let pad = layout.detail_pad();
    let inner_w = panel_w - pad * 2.0;
    let caption_lh = text_line_h(layout.caption_px);
    let exhausted = details.remaining == 0;
    let top_pad = (8.0 * layout.jr).max(6.0);

    let mut y = top_pad;
    y += caption_lh + layout.section_inner_gap() + 4.0;

    let preview_size = layout.detail_preview_size(inner_w, viewport_h);
    y += preview_size * 1.08 + layout.detail_pad();

    y += text_line_h(layout.body_px) + layout.section_inner_gap() + 4.0;
    y += caption_lh + layout.section_inner_gap() + 2.0;
    y += copies_panel_height(layout, mode);
    y += 4.0;

    if modifier_summary(&details.modifiers).is_some() {
        y += 2.0 + caption_lh + 2.0;
    }

    y += layout.section_inner_gap() + layout.section_divider_gap();
    y += caption_lh + layout.section_inner_gap();

    let about_color = color::alpha(color::UMBER, if exhausted { 0.72 } else { 0.86 });
    y += styled_line_block_height_at_font_px(
        &details.about,
        inner_w,
        layout.caption_px,
        GlossaryMode::Prose,
        about_color,
    )
    .max(text_line_h(layout.caption_px));

    y + pad
}

fn modifier_summary(m: &ModifierBreakdown) -> Option<String> {
    if m.debuffed > 0 {
        Some(format!("Debuff ×{}", m.debuffed))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Suit;
    use crate::game::wall_stats::{
        AbundanceState, FaceKey, GRID_FACE_ORDER, ModifierBreakdown, SelectedTileDetails,
        TileLedgerEntry, TileLocationCounts, WallStats,
    };

    fn stub_details() -> SelectedTileDetails {
        SelectedTileDetails {
            face: FaceKey {
                suit: Suit::Souzu,
                rank: 5,
            },
            name: "5 Souzu".into(),
            remaining: 4,
            total: 4,
            locations: TileLocationCounts {
                in_wall: 4,
                in_hand: 0,
                played: 0,
                discarded: 0,
            },
            draw_probability: 0.0,
            wall_share: 0.0,
            modifiers: ModifierBreakdown::default(),
            about: "A long explanatory paragraph about this tile that should wrap across multiple lines in the sidebar detail panel.".into(),
        }
    }

    fn stub_stats() -> WallStats {
        let entries = GRID_FACE_ORDER
            .iter()
            .map(|&(suit, rank)| TileLedgerEntry {
                suit,
                rank,
                remaining: 4,
                seen: 0,
                total: 4,
                locations: TileLocationCounts {
                    in_wall: 4,
                    in_hand: 0,
                    played: 0,
                    discarded: 0,
                },
                draw_probability: 0.0,
                wall_share: 0.0,
                abundance: AbundanceState::Normal,
                modifiers: ModifierBreakdown::default(),
            })
            .collect();
        WallStats {
            entries,
            suit_summary: Default::default(),
            total_remaining: 136,
            total_wall: 136,
            most_common: Vec::new(),
            thin_exhausted: Vec::new(),
            abundant: Vec::new(),
            best_draws: Vec::new(),
            yaku_hints: Vec::new(),
            global_modifiers: ModifierBreakdown::default(),
        }
    }

    #[test]
    fn sidebar_content_exceeds_viewport_at_1080p() {
        let layout = super::super::layout::wall_layout(1920.0, 1080.0, 1.0);
        let stats = stub_stats();
        let details = stub_details();
        let scroll = sidebar_scroll_layout(&layout, &stats, Some(&details), WallLedgerMode::Live);
        assert!(scroll.clip[3] > 0.0);
        assert!(
            scroll.max_scroll_px > 0.0,
            "detail stack should overflow sidebar viewport"
        );
        let sb = sidebar_scrollbar(
            &layout,
            scroll.clip,
            scroll.content_h,
            0.0,
            scroll.max_scroll_px,
        );
        assert!(sb.is_some());
        let sb = sb.unwrap();
        assert!(sb.track[0] + sb.track[2] <= scroll.clip[0] + scroll.clip[2]);
        assert!(
            scroll.content_w + scroll.scrollbar_gutter <= layout.summary_w + 0.5,
            "gutter should not exceed panel width"
        );
    }
}
