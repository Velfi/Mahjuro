//! Left summary plaque — compact ledger blocks with clipped overflow.

use crate::core::tile::{Suit, Tile};
use crate::game::run::RunState;
use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{SelectedTileDetails, WallStats};
use crate::render::theme::color;
use crate::render::wgpu_renderer::TextAlign;

use super::super::layout::{WallLayout, text_line_h, wall_progress_bar_block_h};
use super::super::sidebar_scroll::{SidebarScrollDraw, measure_detail_panel_height};
use super::detail::draw_wall_detail_panel;
use super::text::{push_plaque, push_text, push_text_maybe_clip};

const VALUE_W: f32 = 38.0;

pub fn draw_wall_summary_panel(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    placements: &mut Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
    layout: &WallLayout,
    stats: &WallStats,
    details: Option<&SelectedTileDetails>,
    run: &RunState,
    representative: Option<&Tile>,
    window_w: f32,
    window_h: f32,
    scroll_y: f32,
    scroll_clip: [f32; 4],
    mode: WallLedgerMode,
) {
    let pad = layout.summary_pad();
    let rect = [
        layout.summary_x,
        layout.summary_y,
        layout.summary_w,
        layout.summary_h,
    ];
    push_plaque(frame, rect, 0.90);
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [
            layout.summary_x + layout.summary_w - 1.0,
            layout.summary_y + 4.0,
            1.0,
            layout.summary_h - 8.0,
        ],
        color: color::alpha(color::BRASS, 0.14),
        user: 0,
    });
    push_divider(
        frame,
        layout.summary_x + pad,
        layout.summary_y + layout.summary_h - 1.0,
        layout.summary_w - pad * 2.0,
    );

    let y = layout.summary_y + pad;
    let title_line = text_line_h(layout.caption_px);
    push_text(
        texts,
        [
            layout.summary_x + pad,
            y,
            layout.summary_w - pad * 2.0,
            title_line,
        ],
        "WALL SUMMARY",
        layout.caption_px,
        color::alpha(color::BRASS, 0.82),
        true,
        TextAlign::Left,
    );

    let content_top = y + title_line + 8.0;
    let scroll = SidebarScrollDraw {
        content_top,
        scroll_y,
        clip: scroll_clip,
        x: layout.summary_x,
        w: layout.summary_w,
    };
    let mut logical_y = 0.0_f32;

    draw_wall_tab_summary(
        frame,
        texts,
        placements,
        layout,
        stats,
        &scroll,
        details,
        run,
        representative,
        window_w,
        window_h,
        &mut logical_y,
        mode,
    );
}

fn push_divider(frame: &mut crate::render::draw_cmd::UiFrame, x: f32, y: f32, w: f32) {
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [x, y, w, 1.0],
        color: color::alpha(color::STONE, 0.18),
        user: 0,
    });
}

fn push_section_header(
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    scroll: &SidebarScrollDraw,
    logical_y: &mut f32,
    title: &str,
) {
    let line = text_line_h(layout.caption_px);
    let screen_y = scroll.screen_y(*logical_y);
    if scroll.visible(screen_y, line) {
        push_text_maybe_clip(
            texts,
            [
                layout.summary_x + layout.summary_pad(),
                screen_y,
                layout.summary_w - layout.summary_pad() * 2.0,
                line,
            ],
            title,
            layout.caption_px,
            color::alpha(color::CHAMPAGNE, 0.92),
            true,
            TextAlign::Left,
            Some(scroll.clip),
        );
    }
    *logical_y += line + 4.0;
}

fn draw_wall_progress_bar(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    scroll: &SidebarScrollDraw,
    logical_y: &mut f32,
    remaining: usize,
    total: usize,
) {
    let pad = layout.summary_pad();
    let inner_w = layout.summary_w - pad * 2.0;
    let end_w = 44.0;
    let gap = 6.0;
    let bar_w = (inner_w - end_w * 2.0 - gap * 2.0).max(8.0);

    let count_line = text_line_h(layout.count_px);
    let bar_h = (count_line * 0.38).max(10.0);
    let count_row_h = count_line.max(bar_h);
    let label_line = text_line_h(layout.caption_px);
    let block_h = wall_progress_bar_block_h(layout);

    let screen_y = scroll.screen_y(*logical_y);
    if !scroll.visible(screen_y, block_h) {
        *logical_y += block_h;
        return;
    }

    let x0 = layout.summary_x + pad;
    let bar_x = x0 + end_w + gap;
    let bar_y = screen_y + (count_row_h - bar_h) * 0.5;

    push_text_maybe_clip(
        texts,
        [x0, screen_y, end_w, count_line],
        format!("{remaining}"),
        layout.count_px,
        color::CHAMPAGNE,
        true,
        TextAlign::Left,
        Some(scroll.clip),
    );
    push_text_maybe_clip(
        texts,
        [x0 + inner_w - end_w, screen_y, end_w, count_line],
        format!("{total}"),
        layout.count_px,
        color::STONE,
        true,
        TextAlign::Right,
        Some(scroll.clip),
    );

    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [bar_x, bar_y, bar_w, bar_h],
        color: color::alpha(color::WALNUT_INK, 0.65),
        user: 0,
    });
    let frac = if total > 0 {
        remaining as f32 / total as f32
    } else {
        0.0
    };
    let fill_w = bar_w * frac.clamp(0.0, 1.0);
    if fill_w > 1.0 {
        frame.quad(crate::render::wgpu_renderer::GpuInstance {
            rect: [bar_x, bar_y, fill_w, bar_h],
            color: color::alpha(color::CHAMPAGNE, 0.55),
            user: 0,
        });
    }

    let label_y = screen_y + count_row_h + 2.0;
    let label_w = inner_w * 0.5;
    push_text_maybe_clip(
        texts,
        [x0, label_y, label_w, label_line],
        "remaining",
        layout.caption_px,
        color::alpha(color::STONE, 0.75),
        false,
        TextAlign::Left,
        Some(scroll.clip),
    );
    push_text_maybe_clip(
        texts,
        [x0 + inner_w - label_w, label_y, label_w, label_line],
        "total",
        layout.caption_px,
        color::alpha(color::STONE, 0.75),
        false,
        TextAlign::Right,
        Some(scroll.clip),
    );

    *logical_y += block_h;
}

fn draw_wall_tab_summary(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    placements: &mut Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
    layout: &WallLayout,
    stats: &WallStats,
    scroll: &SidebarScrollDraw,
    details: Option<&SelectedTileDetails>,
    run: &RunState,
    representative: Option<&Tile>,
    window_w: f32,
    window_h: f32,
    logical_y: &mut f32,
    mode: WallLedgerMode,
) {
    draw_wall_progress_bar(
        frame,
        texts,
        layout,
        scroll,
        logical_y,
        stats.total_remaining,
        stats.total_wall,
    );
    *logical_y += 6.0;
    let divider_y = scroll.screen_y(*logical_y);
    if scroll.visible(divider_y, 1.0) {
        push_divider(
            frame,
            layout.summary_x + layout.summary_pad(),
            divider_y,
            layout.summary_w - layout.summary_pad() * 2.0,
        );
    }
    *logical_y += 7.0;

    push_section_header(texts, layout, scroll, logical_y, "SUIT BALANCE");
    draw_suit_balance_bars(frame, texts, layout, stats, scroll, logical_y);
    *logical_y += 4.0;
    let divider_y = scroll.screen_y(*logical_y);
    if scroll.visible(divider_y, 1.0) {
        push_divider(
            frame,
            layout.summary_x + layout.summary_pad(),
            divider_y,
            layout.summary_w - layout.summary_pad() * 2.0,
        );
    }
    *logical_y += 7.0;

    if let Some(details) = details {
        let detail_h = measure_detail_panel_height(layout, scroll.w, details, mode);
        let detail_top = scroll.screen_y(*logical_y);
        let detail_rect = [scroll.x, detail_top, scroll.w, detail_h];
        draw_wall_detail_panel(
            frame,
            texts,
            placements,
            layout,
            detail_rect,
            details,
            run,
            representative,
            window_w,
            window_h,
            scroll.clip,
            mode,
        );
        *logical_y += detail_h;
    }
}

fn draw_suit_balance_bars(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    stats: &WallStats,
    scroll: &SidebarScrollDraw,
    logical_y: &mut f32,
) {
    let line = text_line_h(layout.caption_px);
    let pad = layout.summary_pad();
    let inner_w = layout.summary_w - pad * 2.0;
    let label_w = inner_w * 0.34;
    let count_w = VALUE_W;
    let bar_x = layout.summary_x + pad + label_w + 4.0;
    let bar_w = inner_w - label_w - count_w - 8.0;
    let bar_h = (line * 0.28).max(4.0);
    let total = stats.total_remaining.max(1) as f32;
    let value_x = layout.summary_x + pad + inner_w - count_w;
    let rows = [
        ("Manzu", stats.suit_summary.manzu, Suit::Manzu),
        ("Souzu", stats.suit_summary.souzu, Suit::Souzu),
        ("Pinzu", stats.suit_summary.pinzu, Suit::Pinzu),
        ("Honors", stats.suit_summary.honors, Suit::Wind),
        ("Flowers", stats.suit_summary.flowers, Suit::Flower),
    ];
    for (name, count, suit) in rows {
        let screen_y = scroll.screen_y(*logical_y);
        if !scroll.visible(screen_y, line) {
            *logical_y += line + 2.0;
            continue;
        }
        let row_y = screen_y + (line - bar_h) * 0.5;
        push_text_maybe_clip(
            texts,
            [layout.summary_x + pad, screen_y, label_w, line],
            name,
            layout.caption_px,
            suit.keyword_color(),
            false,
            TextAlign::Left,
            Some(scroll.clip),
        );
        frame.quad(crate::render::wgpu_renderer::GpuInstance {
            rect: [bar_x, row_y, bar_w.max(1.0), bar_h],
            color: color::alpha(color::WALNUT_INK, 0.65),
            user: 0,
        });
        let fill = bar_w * (count as f32 / total);
        if fill > 1.0 {
            frame.quad(crate::render::wgpu_renderer::GpuInstance {
                rect: [bar_x, row_y, fill, bar_h],
                color: color::alpha(suit.keyword_color(), 0.50),
                user: 0,
            });
        }
        push_text_maybe_clip(
            texts,
            [value_x, screen_y, VALUE_W, line],
            format!("{count}"),
            layout.caption_px,
            color::STONE,
            false,
            TextAlign::Right,
            Some(scroll.clip),
        );
        *logical_y += line + 2.0;
    }
}
