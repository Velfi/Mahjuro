//! Floating Kenney prompts under the discard bowl, play mirror, and cash-in tablet.

use super::focus::FocusTarget;
use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{PromptIconQuad, UiFrame};
use crate::render::theme::color;
use crate::render::theme::typography;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::ui::button_prompts::PromptInputSurface;
use crate::ui::input::InputMode;
use crate::ui::kenney_prompt_paths::gameplay_keyboard_prompt_icons;

/// Whether to show the West / North (keyboard **Q** / **E**) gameplay legend for discard or play.
///
/// Hides when the action cannot run (`action_enabled` false). With a controller and
/// "X and Y quick action" off, also hides while focus is on inspect-only HUD (relics, yaku
/// tablets, pegs, etc.) so prompts match what those face buttons do from hand / action buttons.
pub fn gameplay_west_north_legend_active(
    input_mode: InputMode,
    xy_quick_action: bool,
    focus: Option<FocusTarget>,
    action_enabled: bool,
) -> bool {
    if !action_enabled {
        return false;
    }
    match input_mode {
        InputMode::Keyboard | InputMode::Cursor => true,
        InputMode::Controller => {
            if xy_quick_action {
                return true;
            }
            match focus {
                None => true,
                Some(
                    FocusTarget::Relic(_)
                    | FocusTarget::Peg(_)
                    | FocusTarget::Gold
                    | FocusTarget::YakuTablet(_)
                    | FocusTarget::Dora
                    | FocusTarget::Consumable(_),
                ) => false,
                Some(_) => true,
            }
        }
    }
}

/// Matches [`crate::scenes::gameplay::scene_behavior`] copy: draw → mirror, discard → bowl.
const GAMEPLAY_ACTION_PROMPT_LABELS: [&str; 3] = ["Discard", "Draw", "Cash in"];

/// Extra inset inside the text rect (and pill) so glyphs aren’t flush to the backer edge.
const LABEL_PAD_X: f32 = 4.0;
const LABEL_PAD_Y: f32 = 3.0;

pub fn push_gameplay_action_prompts(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    discard_btn_rect: (f32, f32, f32, f32),
    play_btn_rect: (f32, f32, f32, f32),
    trigger_btn_rect: (f32, f32, f32, f32),
    cash_in_enabled: bool,
    show_discard_legend: bool,
    show_play_legend: bool,
    discard_undo_bottom_y: Option<f32>,
    hud_text: &mut Vec<TextLabel>,
) {
    let h = ctx.layout.window_h;
    let w = ctx.layout.window_w;

    let surface = match ctx.input_mode {
        InputMode::Controller => PromptInputSurface::Controller,
        InputMode::Keyboard | InputMode::Cursor => PromptInputSurface::MouseOrKeyboard,
    };
    let keyboard_icons = gameplay_keyboard_prompt_icons();

    // Match [`crate::scenes::shop::view::render_shop_frame`] floating legend (non-inspect row).
    let font_px = typography::size(typography::H42, h);
    let legend_font_px = typography::size(typography::H20, h);
    let bar_h_ref = h * 0.056;
    let icon_cap_3x = bar_h_ref * 0.72 * 3.0;
    let icon_px = icon_cap_3x.clamp(48.0, 132.0);
    let gap_after_icon = icon_px * 0.18;
    let pill_bg = [0.06_f32, 0.055, 0.07, 0.82];
    let pill_bg_disabled = [0.045_f32, 0.042, 0.048, 0.55];
    let pill_pad_x = (icon_px * 0.10).clamp(6.0, 16.0) + (h * 0.003).clamp(4.0, 8.0);

    let ui_font = load_ui_font();
    let legend_line_h = ui_font
        .as_ref()
        .and_then(|f| f.horizontal_line_metrics(legend_font_px))
        .map(|lm| lm.new_line_size)
        .unwrap_or(legend_font_px * 1.2)
        .max(legend_font_px * 0.85);
    let legend_text_h_px = legend_line_h.max(8.0).round().max(1.0) as u32;
    let pill_pad_y = (legend_line_h * 0.14).clamp(3.0, 9.0);

    let rects: [(f32, f32, f32, f32); 3] = [discard_btn_rect, play_btn_rect, trigger_btn_rect];

    // One baseline for all three legends so cash-in stays aligned with discard/draw
    // even when the trigger rect sits above the mirror (no structure yet).
    let legend_row_top = {
        let (_ddx, ddy, ddw, ddh) = discard_btn_rect;
        let (_pdx, pdy, pdw, pdh) = play_btn_rect;
        let mut row_top = f32::NEG_INFINITY;
        if ddw > 1.0 && ddh > 1.0 {
            let mut t = ddy + ddh + h * 0.008;
            if let Some(ub) = discard_undo_bottom_y {
                t = t.max(ub + h * 0.006);
            }
            row_top = row_top.max(t);
        }
        if pdw > 1.0 && pdh > 1.0 {
            row_top = row_top.max(pdy + pdh + h * 0.008);
        }
        if row_top.is_finite() {
            row_top
        } else {
            let (_dx, dy, _dw, dh) = trigger_btn_rect;
            dy + dh + h * 0.008
        }
    };

    // Indices with valid hit targets, in Discard → Draw → Cash-in order.
    // When there is no meld yet, cash-in uses the same screen center as the mirror; centering
    // each cluster on its button stacks Draw and Cash-in. Flow into columns like the shop legend.
    let mut visible: [usize; 3] = [0; 3];
    let mut n_visible = 0usize;
    for i in 0..3 {
        let (_dx, _dy, dw, dh) = rects[i];
        if dw <= 1.0 || dh <= 1.0 {
            continue;
        }
        if i == 0 && !show_discard_legend {
            continue;
        }
        if i == 1 && !show_play_legend {
            continue;
        }
        visible[n_visible] = i;
        n_visible += 1;
    }
    if n_visible == 0 {
        return;
    }

    let mut measured_full: [f32; 3] = [1.0, 1.0, 1.0];
    for k in 0..n_visible {
        let i = visible[k];
        let m: f32 = if let Some(ref font) = ui_font {
            let (_, _, advances) = measure_label_advances(
                font,
                GAMEPLAY_ACTION_PROMPT_LABELS[i],
                8192,
                legend_text_h_px,
                Some(legend_font_px),
            );
            advances.iter().copied().sum()
        } else {
            let est_ch = GAMEPLAY_ACTION_PROMPT_LABELS[i].chars().count().max(1) as f32;
            (legend_font_px * 0.52 * est_ch).max(8.0)
        };
        measured_full[i] = m.max(1.0);
    }

    let inner_left = w * 0.05;
    let inner_right = w * 0.95;
    let inner_w = (inner_right - inner_left).max(8.0);
    let col_w = inner_w / n_visible as f32;
    let col_pad = (col_w * 0.045).min(8.0_f32).max(2.0);

    let label_block_h = legend_line_h + LABEL_PAD_Y * 2.0;
    let mut icon_px_mut = icon_px;
    let mut gap_mut = gap_after_icon;
    loop {
        let mut fits = true;
        for k in 0..n_visible {
            let i = visible[k];
            let label_inner = measured_full[i] + LABEL_PAD_X * 2.0;
            let cluster = icon_px_mut + gap_mut + label_inner;
            if cluster > col_w - col_pad * 2.0 {
                fits = false;
                break;
            }
        }
        if fits || icon_px_mut <= 18.0 {
            break;
        }
        icon_px_mut -= 1.0;
        gap_mut = icon_px_mut * 0.18;
    }
    let icon_px = icon_px_mut;
    let gap_after_icon = gap_mut;

    let primary_h = (icon_px * 1.06).max(label_block_h).max(font_px * 1.35);
    let row_top = legend_row_top;
    let iy = row_top + (primary_h - icon_px) * 0.5;
    let label_top = row_top + (primary_h - label_block_h) * 0.5;

    let mut pill_quads: Vec<GpuInstance> = Vec::with_capacity(n_visible);
    let mut icon_cmds: Vec<PromptIconQuad> = Vec::with_capacity(n_visible);

    for k in 0..n_visible {
        let i = visible[k];
        let cash_in_disabled = i == 2 && !cash_in_enabled;

        let col_x = inner_left + k as f32 * col_w;
        let ix = col_x + col_pad;
        let text_x = ix + icon_px + gap_after_icon;
        let max_text_w = (col_x + col_w - col_pad - text_x).max(10.0);
        let text_w = measured_full[i].min(max_text_w).max(1.0);
        let label_inner_w = text_w + LABEL_PAD_X * 2.0;

        let pill_left = text_x - pill_pad_x;
        let pill_w = (label_inner_w + pill_pad_x * 2.0).max(1.0);

        pill_quads.push(GpuInstance {
            rect: [
                pill_left,
                label_top - pill_pad_y,
                pill_w,
                label_block_h + pill_pad_y * 2.0,
            ],
            color: if cash_in_disabled {
                pill_bg_disabled
            } else {
                pill_bg
            },
            user: 0,
        });
        let icon_tint = if cash_in_disabled {
            color::alpha(
                color::darken(color::alpha(color::PORCELAIN_AGED, 0.96), 0.45),
                0.5,
            )
        } else {
            color::alpha(color::PORCELAIN_AGED, 0.96)
        };
        let label_color = if cash_in_disabled {
            color::alpha(color::UMBER, 0.72)
        } else {
            color::alpha(color::PORCELAIN_AGED, 0.96)
        };
        let action = match i {
            0 => crate::ui::input::UiAction::WestFacePress,
            1 => crate::ui::input::UiAction::NorthFacePress,
            _ => crate::ui::input::UiAction::TriggerStructure,
        };
        let source = match surface {
            PromptInputSurface::Controller => ctx.glyphs.glyph_for(action),
            PromptInputSurface::MouseOrKeyboard => Some(keyboard_icons[i].clone()),
        };
        if let Some(source) = source {
            icon_cmds.push(PromptIconQuad {
                inst: GpuInstance {
                    rect: [ix, iy, icon_px, icon_px],
                    color: icon_tint,
                    user: 0,
                },
                source,
            });
        }
        hud_text.push(TextLabel {
            rect: [
                text_x + LABEL_PAD_X,
                label_top + LABEL_PAD_Y,
                text_w,
                legend_line_h,
            ],
            text: GAMEPLAY_ACTION_PROMPT_LABELS[i].to_string(),
            color: label_color,
            font_px: Some(legend_font_px),
            align: TextAlign::Left,
            no_glossary: false,
            scroll_offset: 0.0,
            flavor_spans: None,
            bold: false,
            italic: false,
            underline: false,
            text_effect: crate::render::text_effect::TextEffectId::Flat,
            rotation_quarters: 0,
            baseline_shift_px: 0.0,
        });
    }

    if !pill_quads.is_empty() {
        frame.squircle_quads(pill_quads);
        frame.prompt_icon_quads(icon_cmds);
    }
}
