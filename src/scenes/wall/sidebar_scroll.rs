//! Sidebar scroll layout and content-height measurement.

use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{ModifierBreakdown, SelectedTileDetails, WallStats};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::theme::color;
use crate::ui::styled_text::styled_line_block_height_at_font_px;

use super::layout::{WallLayout, text_line_h, wall_progress_bar_block_h};

pub struct SidebarScrollLayout {
    pub content_top: f32,
    pub viewport_h: f32,
    pub max_scroll_px: f32,
    pub wheel_step_px: f32,
    pub clip: [f32; 4],
}

pub struct SidebarScrollDraw {
    pub content_top: f32,
    pub scroll_y: f32,
    pub clip: [f32; 4],
    pub x: f32,
    pub w: f32,
}

impl SidebarScrollDraw {
    pub fn screen_y(&self, logical_y: f32) -> f32 {
        self.content_top + logical_y - self.scroll_y
    }

    pub fn visible(&self, screen_y: f32, h: f32) -> bool {
        screen_y + h > self.clip[1] && screen_y < self.clip[1] + self.clip[3]
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
    let content_top = layout.summary_y + pad + title_line + 8.0;
    let content_bottom = layout.summary_y + layout.summary_h - pad;
    let viewport_h = (content_bottom - content_top).max(0.0);
    let content_h = measure_sidebar_content_height(layout, details, mode);
    let max_scroll_px = (content_h - viewport_h).max(0.0);
    let wheel_step_px = (text_line_h(layout.caption_px) * 2.4).clamp(36.0, 80.0);
    SidebarScrollLayout {
        content_top,
        viewport_h,
        max_scroll_px,
        wheel_step_px,
        clip: [layout.summary_x, content_top, layout.summary_w, viewport_h],
    }
}

pub fn measure_sidebar_content_height(
    layout: &WallLayout,
    details: Option<&SelectedTileDetails>,
    mode: WallLedgerMode,
) -> f32 {
    let lh = text_line_h(layout.caption_px);
    let mut y = 0.0_f32;

    // Remaining / total progress bar
    y += wall_progress_bar_block_h(layout) + 6.0 + 7.0;

    // Suit balance
    y += lh + 4.0;
    y += (lh + 2.0) * 5.0;
    y += 4.0 + 7.0;

    if let Some(details) = details {
        y += measure_detail_panel_height(layout, layout.summary_w, details, mode);
    }

    y
}

pub fn measure_detail_panel_height(
    layout: &WallLayout,
    panel_w: f32,
    details: &SelectedTileDetails,
    mode: WallLedgerMode,
) -> f32 {
    let pad = 10.0;
    let inner_w = panel_w - pad * 2.0;
    let caption_lh = text_line_h(layout.caption_px);
    let exhausted = details.remaining == 0;

    let mut y = 8.0_f32;
    y += caption_lh + 8.0;

    let preview_size = (inner_w * 0.78).clamp(80.0, 168.0);
    y += preview_size * 1.08 + 10.0;

    y += text_line_h(layout.body_px) + 8.0;
    y += caption_lh + 4.0;
    let copy_rows = if mode.shows_round_locations() { 4 } else { 1 };
    y += (caption_lh + 2.0) * copy_rows as f32;
    y += 4.0;

    if modifier_summary(&details.modifiers).is_some() {
        y += 2.0 + caption_lh + 2.0;
    }

    y += 6.0 + 8.0;
    y += caption_lh + 4.0;

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
    let mut parts = Vec::new();
    if m.pearl > 0 {
        parts.push(format!("Pearl ×{}", m.pearl));
    }
    if m.gilded > 0 {
        parts.push(format!("Gilded ×{}", m.gilded));
    }
    if m.polychrome > 0 {
        parts.push(format!("Poly ×{}", m.polychrome));
    }
    if m.debuffed > 0 {
        parts.push(format!("Debuff ×{}", m.debuffed));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Suit;
    use crate::game::wall_stats::{
        AbundanceState, FaceKey, ModifierBreakdown, SelectedTileDetails, TileLedgerEntry,
        TileLocationCounts, WallStats, GRID_FACE_ORDER,
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
        assert!(scroll.viewport_h > 0.0);
        assert!(
            scroll.max_scroll_px > 0.0,
            "detail stack should overflow sidebar viewport"
        );
    }
}
