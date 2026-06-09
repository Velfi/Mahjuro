//! Left summary plaque — compact ledger blocks with clipped overflow.

use crate::core::tile::Suit;
use crate::game::wall_stats::{face_short_name, WallCountView, WallStats};
use crate::render::theme::color;
use crate::render::wgpu_renderer::TextAlign;

use super::super::focus::LedgerNav;
use super::super::layout::{text_line_h, WallLayout};
use super::text::{push_plaque, push_text, push_text_maybe_clip};

const VALUE_W: f32 = 38.0;

pub fn draw_wall_summary_panel(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    stats: &WallStats,
    view: WallCountView,
    focus: Option<LedgerNav>,
) {
    let pad = layout.summary_pad();
    let rect = [
        layout.summary_x,
        layout.summary_y,
        layout.summary_w,
        layout.summary_h,
    ];
    let summary_clip = Some(rect);
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

    let mut y = layout.summary_y + pad;
    let title_line = text_line_h(layout.caption_px * 1.02);
    push_text(
        texts,
        [layout.summary_x + pad, y, layout.summary_w - pad * 2.0, title_line],
        "WALL SUMMARY",
        layout.caption_px * 1.02,
        color::alpha(color::BRASS, 0.82),
        true,
        TextAlign::Left,
    );
    y += title_line + 8.0;

    draw_wall_tab_summary(frame, texts, layout, stats, view, focus, summary_clip, &mut y);
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
    y: &mut f32,
    title: &str,
) {
    let line = text_line_h(layout.caption_px * 0.94);
    push_text(
        texts,
        [
            layout.summary_x + layout.summary_pad(),
            *y,
            layout.summary_w - layout.summary_pad() * 2.0,
            line,
        ],
        title,
        layout.caption_px * 0.94,
        color::alpha(color::CHAMPAGNE, 0.92),
        true,
        TextAlign::Left,
    );
    *y += line + 4.0;
}

fn push_stat_row(
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    y: &mut f32,
    label: &str,
    value: impl std::fmt::Display,
    value_color: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if clip.is_some_and(|c| *y > c[1] + c[3] - text_line_h(layout.caption_px)) {
        return;
    }
    let line = text_line_h(layout.caption_px);
    let pad = layout.summary_pad();
    let value_x = layout.summary_value_x() - VALUE_W;
    push_text_maybe_clip(
        texts,
        [layout.summary_x + pad, *y, value_x - layout.summary_x - pad - 4.0, line],
        label,
        layout.caption_px,
        color::STONE,
        false,
        TextAlign::Left,
        clip,
    );
    push_text_maybe_clip(
        texts,
        [value_x, *y, VALUE_W, line],
        format!("{value}"),
        layout.caption_px,
        value_color,
        true,
        TextAlign::Right,
        clip,
    );
    *y += line + 2.0;
}

fn push_best_draw_row(
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    y: &mut f32,
    name: &str,
    reason: &str,
    focused: bool,
    clip: Option<[f32; 4]>,
) {
    if clip.is_some_and(|c| *y > c[1] + c[3] - text_line_h(layout.caption_px) * 2.1) {
        return;
    }
    let line = text_line_h(layout.caption_px);
    let pad = layout.summary_pad();
    let reason = if reason.len() > 32 {
        format!("{}…", &reason[..31])
    } else {
        reason.to_string()
    };
    let tint = if focused {
        color::BRASS
    } else {
        color::alpha(color::JADE, 0.88)
    };
    push_text_maybe_clip(
        texts,
        [layout.summary_x + pad, *y, layout.summary_w - pad * 2.0, line],
        name,
        layout.caption_px,
        tint,
        focused,
        TextAlign::Left,
        clip,
    );
    push_text_maybe_clip(
        texts,
        [layout.summary_x + pad + 6.0, *y + line, layout.summary_w - pad * 2.0 - 6.0, line],
        reason,
        layout.caption_px * 0.88,
        color::alpha(color::UMBER, if focused { 0.78 } else { 0.62 }),
        false,
        TextAlign::Left,
        clip,
    );
    *y += line * 2.0 + 2.0;
}

fn draw_wall_tab_summary(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    stats: &WallStats,
    _view: WallCountView,
    focus: Option<LedgerNav>,
    summary_clip: Option<[f32; 4]>,
    y: &mut f32,
) {
    push_stat_row(
        texts,
        layout,
        y,
        "Total Tiles",
        stats.total_wall,
        color::STONE,
        summary_clip,
    );
    push_stat_row(
        texts,
        layout,
        y,
        "Remaining",
        stats.total_remaining,
        color::CHAMPAGNE,
        summary_clip,
    );
    *y += 6.0;
    push_divider(
        frame,
        layout.summary_x + layout.summary_pad(),
        *y,
        layout.summary_w - layout.summary_pad() * 2.0,
    );
    *y += 7.0;

    push_section_header(texts, layout, y, "SUIT BALANCE");
    draw_suit_balance_bars(frame, texts, layout, stats, y, summary_clip);
    *y += 4.0;
    push_divider(
        frame,
        layout.summary_x + layout.summary_pad(),
        *y,
        layout.summary_w - layout.summary_pad() * 2.0,
    );
    *y += 7.0;

    if let Some(c) = summary_clip {
        let min_room = text_line_h(layout.caption_px) * 2.0 + 2.0;
        let max_top = c[1] + c[3] - min_room;
        if *y > max_top {
            *y = max_top;
        }
    }
    push_section_header(texts, layout, y, "BEST DRAWS");
    if stats.best_draws.is_empty() {
        push_text_maybe_clip(
            texts,
            [
                layout.summary_x + layout.summary_pad(),
                *y,
                layout.summary_w - layout.summary_pad() * 2.0,
                text_line_h(layout.caption_px),
            ],
            "No strong draw hints",
            layout.caption_px * 0.9,
            color::alpha(color::UMBER, 0.66),
            false,
            TextAlign::Left,
            summary_clip,
        );
        *y += text_line_h(layout.caption_px) + 2.0;
    } else {
        for (i, hint) in stats.best_draws.iter().enumerate().take(3) {
            let focused = focus == Some(LedgerNav::Summary(i));
            push_best_draw_row(
                texts,
                layout,
                y,
                &face_short_name(hint.face.suit, hint.face.rank),
                &hint.reason,
                focused,
                summary_clip,
            );
        }
    }
}

fn draw_suit_balance_bars(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    stats: &WallStats,
    y: &mut f32,
    clip: Option<[f32; 4]>,
) {
    let line = text_line_h(layout.caption_px);
    let pad = layout.summary_pad();
    let label_w = layout.summary_w * 0.36;
    let bar_x = layout.summary_x + pad + label_w;
    let bar_w = layout.summary_value_x() - VALUE_W - bar_x - 8.0;
    let bar_h = (line * 0.24).max(3.0);
    let total = stats.total_remaining.max(1) as f32;
    let value_x = layout.summary_value_x() - VALUE_W;
    let rows = [
        ("Manzu", stats.suit_summary.manzu, Suit::Manzu),
        ("Souzu", stats.suit_summary.souzu, Suit::Souzu),
        ("Pinzu", stats.suit_summary.pinzu, Suit::Pinzu),
        ("Honors", stats.suit_summary.honors, Suit::Wind),
        ("Flowers", stats.suit_summary.flowers, Suit::Flower),
    ];
    for (name, count, suit) in rows {
        if clip.is_some_and(|c| *y > c[1] + c[3] - line) {
            break;
        }
        let row_y = *y + (line - bar_h) * 0.5;
        push_text(
            texts,
            [layout.summary_x + pad, *y, label_w, line],
            name,
            layout.caption_px,
            suit.keyword_color(),
            false,
            TextAlign::Left,
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
        push_text(
            texts,
            [value_x, *y, VALUE_W, line],
            format!("{count}"),
            layout.caption_px,
            color::STONE,
            false,
            TextAlign::Right,
        );
        *y += line + 2.0;
    }
}

