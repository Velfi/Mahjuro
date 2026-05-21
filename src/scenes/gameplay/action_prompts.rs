//! Floating Kenney prompts along the bottom edge (Discard / Draw / Cash in).

use super::focus::FocusTarget;
use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{ImageQuad, UiFrame};
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
                    | FocusTarget::Boss
                    | FocusTarget::RoundWind
                    | FocusTarget::Consumable(_),
                ) => false,
                Some(_) => true,
            }
        }
    }
}

/// Matches [`crate::scenes::gameplay::scene_behavior`] copy: discard → bowl, play → mirror, cash in → trigger.
const GAMEPLAY_ACTION_PROMPT_LABELS: [&str; 3] = ["Discard", "Play", "Cash in"];

/// Extra inset inside the text rect (and pill) so glyphs aren’t flush to the backer edge.
const LABEL_PAD_X: f32 = 4.0;
const LABEL_PAD_Y: f32 = 3.0;

pub struct GameplayActionPromptInput<'a> {
    pub discard_btn_rect: (f32, f32, f32, f32),
    pub play_btn_rect: (f32, f32, f32, f32),
    pub trigger_btn_rect: (f32, f32, f32, f32),
    pub cash_in_enabled: bool,
    pub show_discard_legend: bool,
    pub show_play_legend: bool,
    pub hud_text: &'a mut Vec<TextLabel>,
}

pub fn push_gameplay_action_prompts(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    input: GameplayActionPromptInput<'_>,
) {
    let GameplayActionPromptInput {
        discard_btn_rect,
        play_btn_rect,
        trigger_btn_rect,
        cash_in_enabled,
        show_discard_legend,
        show_play_legend,
        hud_text,
    } = input;
    let _ = (show_discard_legend, show_play_legend);
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

    // Context-only prompt row: show actionable cash-in only. Discard / Play are
    // baseline gameplay interactions and stay unlabeled to reduce hint noise.
    // Indices remain Discard → Draw → Cash-in to preserve icon/label mapping.
    let mut visible: [usize; 3] = [0; 3];
    let mut n_visible = 0usize;
    for (i, rect) in rects.iter().enumerate() {
        let (_dx, _dy, dw, dh) = *rect;
        if dw <= 1.0 || dh <= 1.0 {
            continue;
        }
        if i != 2 {
            continue;
        }
        if i == 2 && !cash_in_enabled {
            continue;
        }
        visible[n_visible] = i;
        n_visible += 1;
    }
    if n_visible == 0 {
        return;
    }

    let mut measured_full: [f32; 3] = [1.0, 1.0, 1.0];
    for &i in visible.iter().take(n_visible) {
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

    let x = w * 0.05;
    let bw = w * 0.90;
    let inner_left = x + bw * 0.02;
    let inner_right = x + bw * 0.98;
    let inner_w = (inner_right - inner_left).max(8.0);
    let col_w = inner_w / n_visible as f32;
    let col_pad = (col_w * 0.045).clamp(2.0, 8.0);

    let label_block_h = legend_line_h + LABEL_PAD_Y * 2.0;
    let mut icon_px_mut = icon_px;
    let mut gap_mut = gap_after_icon;
    loop {
        let mut fits = true;
        for &i in visible.iter().take(n_visible) {
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
    let pad_bottom = h * 0.014;
    let block_h = primary_h;
    let row_top = h - pad_bottom - block_h;
    let iy = row_top + (primary_h - icon_px) * 0.5;
    let label_top = row_top + (primary_h - label_block_h) * 0.5;

    let mut pill_quads: Vec<GpuInstance> = Vec::with_capacity(n_visible);
    let mut icon_cmds: Vec<ImageQuad> = Vec::with_capacity(n_visible);

    for (k, &i) in visible.iter().take(n_visible).enumerate() {
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
            icon_cmds.push(ImageQuad {
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
            clip_rect: None,
        });
    }

    if !pill_quads.is_empty() {
        frame.squircle_quads(pill_quads);
        frame.image_quads(icon_cmds);
    }
}
