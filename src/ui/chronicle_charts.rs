//! Ledger-specific chart and receipt primitives for the Chronicle dashboard.
//!
//! See [`chart_primitives`](crate::ui::chart_primitives) and
//! [chart guidelines](https://guides.library.duke.edu/datavis/topten).

use crate::core::OrdealKindExt;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::archive_career::{
    self, CareerKpi, OrdealRecordRow, ScoreHistoryPoint, WingOutcomeCell,
};
use crate::ui::chart_primitives::{
    self, ChartClip, LedgerPanelStyle, chart_y_axis_width_for_max, pill_label_width,
    push_chart_plot_baseline, push_chart_time_axis_labels, push_chart_y_axis,
    push_colored_label_clipped, push_label_clipped, push_ledger_panel_clipped, push_quad_clipped,
};

pub fn push_outcome_strip(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x: f32,
    y: f32,
    h: f32,
    victory: bool,
) {
    let c = if victory {
        color::alpha(color::chart::POSITIVE, 0.88)
    } else {
        color::alpha(color::chart::NEGATIVE, 0.82)
    };
    push_quad_clipped(quads, [x, y, 3.0, h], clip, c);
}

pub fn push_discovery_stamp(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    size: f32,
    micro_px: f32,
) {
    push_quad_clipped(
        quads,
        [x, y + 1.0, size, size - 2.0],
        clip,
        color::alpha(color::chart::NEGATIVE, 0.22),
    );
    push_quad_clipped(
        quads,
        [x, y + 1.0, size, 1.0],
        clip,
        color::alpha(color::chart::ACCENT, 0.65),
    );
    push_label_clipped(
        labels,
        [x, y, size, size],
        clip,
        TextLabel {
            rect: [x, y, size, size],
            text: "◆".into(),
            color: color::alpha(color::chart::ACCENT, 0.95),
            font_px: Some(micro_px),
            align: TextAlign::Center,
            ..Default::default()
        },
    );
}

/// Victory / defeat swatches for bar charts (≤2 categorical chart colors).
pub fn push_outcome_chart_legend(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    caption_px: f32,
) {
    let swatch = (row_h * 0.42).max(6.0);
    let gap = 6.0;
    let item_w = w * 0.5;
    let text_color = color::alpha(color::STONE, 0.88);
    for (col, victory, label) in [(0, true, "Victory"), (1, false, "Defeat")] {
        let ix = x + col as f32 * item_w;
        let sy = y + (row_h - swatch) * 0.5;
        push_outcome_strip(quads, clip, ix, sy, swatch, victory);
        push_label_clipped(
            labels,
            [ix + swatch + gap, y, item_w - swatch - gap, row_h],
            clip,
            TextLabel {
                rect: [ix + swatch + gap, y, item_w - swatch - gap, row_h],
                text: label.into(),
                color: text_color,
                font_px: Some(caption_px * 0.88),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
    }
}

pub fn push_kpi_card(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    inset: f32,
    stack: f32,
    cap_h: f32,
    val_h: f32,
    caption_px: f32,
    body_px: f32,
    kpi: &CareerKpi,
) {
    push_ledger_panel_clipped(quads, clip, [x, y, w, h], LedgerPanelStyle::KPI);
    let text_w = (w - inset * 2.0).max(1.0);
    let mut ly = y + inset;
    push_label_clipped(
        labels,
        [x + inset, ly, text_w, cap_h],
        clip,
        TextLabel {
            rect: [x + inset, ly, text_w, cap_h],
            text: kpi.label.into(),
            color: color::STONE,
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    ly += cap_h + stack;
    let value_default = match kpi.label {
        "Peak wing" => archive_career::chronicle_wing_color(),
        "Win rate" => color::CHAMPAGNE,
        _ => archive_career::chronicle_chips_color(),
    };
    let value_rect = [x + inset, ly, text_w, val_h];
    push_colored_label_clipped(
        labels,
        value_rect,
        clip,
        &kpi.value,
        value_default,
        body_px,
        TextAlign::Left,
        true,
    );
}

/// Compact single-line stat panel (`label` left, value right) for score-history KPIs.
fn push_score_stat_panel(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    inset: f32,
    inline_gap: f32,
    row_h: f32,
    caption_px: f32,
    body_px: f32,
    label: &str,
    value: &str,
) -> f32 {
    let panel_h = inset * 2.0 + row_h;
    push_ledger_panel_clipped(quads, clip, [x, y, w, panel_h], LedgerPanelStyle::KPI);
    let text_w = (w - inset * 2.0).max(1.0);
    let label_w =
        (pill_label_width(label, caption_px) + inline_gap * 0.5).clamp(1.0, text_w * 0.42);
    let value_w = (text_w - label_w - inline_gap).max(1.0);
    let value_x = x + inset + text_w - value_w;
    let line_y = y + inset;
    push_label_clipped(
        labels,
        [x + inset, line_y, label_w, row_h],
        clip,
        TextLabel {
            rect: [x + inset, line_y, label_w, row_h],
            text: label.into(),
            color: color::STONE,
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    let value_rect = [value_x, line_y, value_w, row_h];
    push_colored_label_clipped(
        labels,
        value_rect,
        clip,
        value,
        archive_career::chronicle_chips_color(),
        body_px,
        TextAlign::Right,
        true,
    );
    panel_h
}

fn score_history_stat_panel_width(
    label: &str,
    value: &str,
    caption_px: f32,
    body_px: f32,
    inset: f32,
    inline_gap: f32,
) -> f32 {
    let inner = pill_label_width(label, caption_px) + inline_gap + pill_label_width(value, body_px);
    (inner + inset * 2.0).max(1.0)
}

fn score_history_stat_row_height(row_h: f32, inset: f32) -> f32 {
    inset * 2.0 + row_h
}

pub fn push_score_history_ledger(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    points: &[ScoreHistoryPoint],
    avg_score: u64,
    personal_best: u64,
    caption_px: f32,
    body_px: f32,
    dense: bool,
    show_stat_panels: bool,
) {
    if points.is_empty() {
        return;
    }
    let max_v = points
        .iter()
        .map(|p| p.score)
        .max()
        .unwrap_or(1)
        .max(avg_score)
        .max(personal_best)
        .max(1);
    let cap_h = (caption_px / 0.55).ceil();
    let val_h = (body_px / 0.55).ceil();
    let inset = 4.0;
    let inline_gap = 6.0;
    let panel_gap = 4.0;
    let axis_label_h = cap_h + 2.0;
    let edge_pad = 2.0;

    let mut peak_i = 0usize;
    for (i, p) in points.iter().enumerate() {
        if p.score >= points[peak_i].score {
            peak_i = i;
        }
    }
    let peak_score = points[peak_i].score;
    let peak_text = archive_career::format_chips_compact(peak_score);
    let avg_text = (avg_score > 0).then(|| archive_career::format_chips_compact(avg_score));
    let stat_row_h = if show_stat_panels {
        score_history_stat_row_height(cap_h.max(val_h), inset)
    } else {
        0.0
    };
    let stat_row_gap = if show_stat_panels { 4.0 } else { 0.0 };
    let chart_x = x;
    let chart_w = w;
    let axis_w = chart_y_axis_width_for_max(max_v, caption_px);
    let content_top = y + stat_row_h + stat_row_gap;
    let (legend_h, plot_x, plot_w, plot_y, plot_h) = if dense {
        let plot_x = chart_x + axis_w;
        let plot_w = (chart_w - axis_w - edge_pad).max(24.0);
        let plot_h = (h - stat_row_h - stat_row_gap - axis_label_h).max(20.0);
        (0.0, plot_x, plot_w, content_top, plot_h)
    } else {
        let legend_h = cap_h + 2.0;
        let plot_x = chart_x + axis_w;
        let plot_w = (chart_w - axis_w).max(24.0);
        let plot_y = content_top + legend_h;
        let plot_h = (h - stat_row_h - stat_row_gap - legend_h - axis_label_h).max(20.0);
        (legend_h, plot_x, plot_w, plot_y, plot_h)
    };
    let bn = points.len().max(1);
    let slot = plot_w / bn as f32;
    let col_w = (slot - 2.0).max(2.0).min(slot * 0.88);
    let grid_color = color::alpha(color::ANTIQUE, 0.22);

    if !dense {
        push_outcome_chart_legend(
            quads,
            labels,
            clip,
            plot_x,
            content_top,
            plot_w,
            legend_h,
            caption_px,
        );
    }
    push_chart_y_axis(
        quads,
        labels,
        clip,
        chart_x,
        axis_w,
        plot_x,
        plot_w,
        plot_y,
        plot_h,
        max_v,
        caption_px,
        grid_color,
        false,
        TextAlign::Left,
    );

    if show_stat_panels {
        let mut stat_x = plot_x;
        let stat_row_h_inner = cap_h.max(val_h);
        if let Some(avg) = avg_text {
            let panel_w = score_history_stat_panel_width(
                "Avg.",
                &avg,
                caption_px,
                body_px,
                inset,
                inline_gap,
            );
            push_score_stat_panel(
                quads,
                labels,
                clip,
                stat_x,
                y,
                panel_w,
                inset,
                inline_gap,
                stat_row_h_inner,
                caption_px,
                body_px,
                "Avg.",
                &avg,
            );
            stat_x += panel_w + panel_gap;
        }
        let peak_panel_w = score_history_stat_panel_width(
            "Peak",
            &peak_text,
            caption_px,
            body_px,
            inset,
            inline_gap,
        );
        push_score_stat_panel(
            quads,
            labels,
            clip,
            stat_x,
            y,
            peak_panel_w,
            inset,
            inline_gap,
            stat_row_h_inner,
            caption_px,
            body_px,
            "Peak",
            &peak_text,
        );
    }

    if avg_score > 0 {
        let avg_frac = (avg_score as f32 / max_v as f32).clamp(0.0, 1.0);
        let ay = plot_y + plot_h - plot_h * avg_frac;
        for dx in (0..(plot_w as i32)).step_by(6) {
            push_quad_clipped(
                quads,
                [plot_x + dx as f32, ay, 3.0, 1.0],
                clip,
                color::alpha(color::chart::POSITIVE, 0.55),
            );
        }
    }

    for (i, p) in points.iter().enumerate() {
        let bx = plot_x + i as f32 * slot + (slot - col_w) * 0.5;
        let frac = (p.score as f32 / max_v as f32).clamp(0.0, 1.0);
        let bar_h = if frac > 0.0 {
            (plot_h * frac).max(2.0)
        } else {
            0.0
        };
        let y0 = plot_y + plot_h - bar_h;
        if bar_h < 1.0 {
            continue;
        }
        let bar_c = if p.victory {
            color::alpha(
                color::chart::POSITIVE,
                if i == peak_i { 0.95 } else { 0.72 },
            )
        } else {
            color::alpha(color::chart::NEGATIVE, 0.68)
        };
        push_quad_clipped(quads, [bx, y0, col_w, bar_h], clip, bar_c);
    }

    push_chart_plot_baseline(quads, clip, plot_x, plot_y, plot_w, plot_h);
    push_chart_time_axis_labels(
        labels,
        clip,
        plot_x,
        plot_y,
        plot_w,
        plot_h,
        cap_h,
        caption_px * 0.88,
    );
}

pub fn push_tile_bucket_hbar(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    label: &str,
    count: u32,
    max_count: u32,
    pct: u32,
    label_w: f32,
    caption_px: f32,
    body_px: f32,
) {
    chart_primitives::push_hbar_row(
        quads,
        labels,
        clip,
        x,
        y,
        w,
        row_h,
        label,
        count,
        max_count,
        label_w,
        color::alpha(color::STONE, 0.92),
        color::alpha(color::chart::FILL, 0.82),
        color::alpha(color::PARCHMENT, 0.94),
        caption_px,
        body_px,
        Some(&format!(" ({pct}%)")),
    );
}

pub fn push_ante_outcome_matrix(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    cells: &[WingOutcomeCell],
    caption_px: f32,
) {
    if cells.is_empty() {
        return;
    }
    let cols = cells.len().max(1) as f32;
    let cell_w = (w / cols).max(18.0);
    let cell_h = (h * 0.55).max(14.0);
    let label_h = (caption_px / 0.55).ceil();
    for (i, cell) in cells.iter().enumerate() {
        let cx = x + i as f32 * cell_w;
        let total = (cell.wins + cell.losses).max(1) as f32;
        let win_frac = cell.wins as f32 / total;
        push_quad_clipped(
            quads,
            [cx + 1.0, y + label_h + 2.0, cell_w - 2.0, cell_h],
            clip,
            color::alpha(color::WALNUT_INK, 0.5),
        );
        push_quad_clipped(
            quads,
            [
                cx + 1.0,
                y + label_h + 2.0 + cell_h * (1.0 - win_frac),
                cell_w - 2.0,
                cell_h * win_frac,
            ],
            clip,
            color::alpha(color::chart::POSITIVE, 0.55 + win_frac * 0.35),
        );
        push_label_clipped(
            labels,
            [cx, y, cell_w, label_h],
            clip,
            TextLabel {
                rect: [cx, y, cell_w, label_h],
                text: format!("W{}", cell.wing),
                color: archive_career::chronicle_wing_color(),
                font_px: Some(caption_px * 0.82),
                align: TextAlign::Center,
                mono: true,
                ..Default::default()
            },
        );
    }
}

pub fn push_ordeal_record_rows(
    labels: &mut Vec<TextLabel>,
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    rows: &[OrdealRecordRow],
    caption_px: f32,
    body_px: f32,
) {
    let gutter = 6.0;
    let score_w = (w * 0.22).clamp(56.0, 88.0);
    let wl_w = (caption_px * 5.5).max(48.0).min(w * 0.22);
    let name_w = (w - score_w - wl_w - gutter * 2.0).max(48.0);
    let wl_x = x + name_w + gutter;
    let score_x = x + w - score_w;
    for (i, row) in rows.iter().take(5).enumerate() {
        let ry = y + i as f32 * row_h;
        push_quad_clipped(
            quads,
            [x, ry + row_h - 1.0, w, 1.0],
            clip,
            color::alpha(color::BRASS, 0.12),
        );
        push_colored_label_clipped(
            labels,
            [x, ry, name_w, row_h],
            clip,
            row.ordeal.name(),
            color::alpha(color::PARCHMENT, 0.92),
            caption_px,
            TextAlign::Left,
            false,
        );
        let wl = format!("{}W · {}L", row.wins, row.losses);
        push_label_clipped(
            labels,
            [wl_x, ry, wl_w, row_h],
            clip,
            TextLabel {
                rect: [wl_x, ry, wl_w, row_h],
                text: wl,
                color: color::alpha(color::STONE, 0.88),
                font_px: Some(caption_px * 0.9),
                align: TextAlign::Left,
                mono: true,
                ..Default::default()
            },
        );
        let score_text = archive_career::format_chips_compact(row.best_score);
        let score_rect = [score_x, ry, score_w, row_h];
        push_colored_label_clipped(
            labels,
            score_rect,
            clip,
            &score_text,
            archive_career::chronicle_chips_color(),
            body_px * 0.92,
            TextAlign::Right,
            true,
        );
    }
}

pub fn push_yaku_fingerprint_rows(
    squircle_quads: &mut Vec<GpuInstance>,
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    rows: &[(crate::core::yaku::YakuKind, u32)],
    max_count: u32,
    label_w: f32,
    caption_px: f32,
    body_px: f32,
) {
    for (i, (yk, count)) in rows.iter().take(6).enumerate() {
        let row_top = y + i as f32 * row_h;
        chart_primitives::push_yaku_hbar_row(
            squircle_quads,
            quads,
            labels,
            clip,
            x,
            row_top,
            w,
            row_h,
            yk.name(),
            *count,
            max_count,
            label_w,
            archive_career::yaku_pill_face(),
            archive_career::yaku_pill_ink(),
            archive_career::yaku_pill_rim(),
            color::alpha(color::chart::FILL, 0.82),
            color::alpha(color::PARCHMENT, 0.94),
            caption_px,
            body_px,
            Some("×"),
        );
    }
}
