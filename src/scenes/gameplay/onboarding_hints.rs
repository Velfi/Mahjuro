//! Guided onboarding prompts during the Lessons blind.

use crate::game::onboarding::finale_intro_message;
use crate::game::run::RunState;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::ui::colored_keywords;
use crate::ui::styled_text;
use crate::ui::widget::{self, TextStyle};

/// Sync step 0 → 1 when the player selects tiles.
pub fn sync_onboarding_step(run: &mut RunState) {
    let lessons = run.onboarding.as_ref().is_some_and(|o| o.lessons_active());
    if !lessons {
        return;
    }
    run.sync_onboarding_invalid_meld_hint();
    let advance = run.onboarding.as_ref().is_some_and(|o| o.step == 0)
        && crate::game::engine::GameEngine::read(run).selected_count > 0;
    if advance && let Some(ref mut onboarding) = run.onboarding {
        onboarding.step = 1;
    }
}

pub fn push_lessons_banner(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    run: &RunState,
    wiggle_x: f32,
) {
    let Some(ref onboarding) = run.onboarding else {
        return;
    };
    if !onboarding.lessons_active() {
        return;
    }

    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let scale = metrics::scene_scale(w, h);
    let prompt = onboarding.lessons_prompt(run);
    let show_river_tip = onboarding.step >= 3 && onboarding.discard_river_tooltip_shown;
    let river_tip = "Discarded tiles sit in the river — they don't score.";

    let pad = (16.0 * scale).max(12.0);
    let panel_w = (w * 0.36).min(440.0 * scale).max(240.0 * scale);
    let panel_x = (18.0 * scale).max(12.0) + wiggle_x;
    let panel_y = h * 0.12;
    let copy_w = panel_w - pad * 2.0;

    let label_font = typography::size(typography::H42, h);
    let prompt_font = typography::size(typography::H36, h);
    let tip_font = typography::size(typography::H42, h);
    let label_h = colored_keywords::colored_row_line_step(label_font);
    let prompt_h = styled_text::styled_line_block_height_at_font_px(
        prompt,
        copy_w,
        prompt_font,
        true,
        color::CHAMPAGNE,
    );
    let tip_h = if show_river_tip {
        styled_text::styled_line_block_height_at_font_px(
            river_tip,
            copy_w,
            tip_font,
            false,
            color::PARCHMENT,
        )
    } else {
        0.0
    };
    let section_gap = (10.0 * scale).max(6.0);
    let tip_gap = if show_river_tip { section_gap } else { 0.0 };
    let panel_h = pad * 2.0 + label_h + section_gap + prompt_h + tip_gap + tip_h;
    let panel_rect = [panel_x, panel_y, panel_w, panel_h];

    let mut quads = Vec::new();
    widget::push_panel_colored(
        &mut quads,
        panel_rect,
        color::WALNUT_SOFT,
        color::BRASS,
    );

    let mut texts = Vec::new();
    let mut text_y = panel_y + pad;
    texts.push(TextLabel {
        rect: [panel_x + pad, text_y, copy_w, label_h],
        text: "Tutorial".to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(label_font),
        ..Default::default()
    });
    text_y += label_h + section_gap;
    widget::push_text_block(
        &mut texts,
        [panel_x + pad, text_y, copy_w, prompt_h],
        prompt,
        TextStyle {
            tier: typography::H36,
            color: color::CHAMPAGNE,
            padding: 0.0,
            align: TextAlign::Left,
            glossary_tint: true,
        },
        h,
    );
    if show_river_tip {
        text_y += prompt_h + tip_gap;
        widget::push_text_block(
            &mut texts,
            [panel_x + pad, text_y, copy_w, tip_h],
            river_tip,
            TextStyle {
                tier: typography::H42,
                color: color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Left,
                glossary_tint: false,
            },
            h,
        );
    }

    frame.overlay_quads(quads);
    frame.texts(texts);
}

pub fn push_finale_intro_banner(frame: &mut UiFrame, ctx: &DrawCtx<'_>, run: &RunState) {
    let Some(ref onboarding) = run.onboarding else {
        return;
    };
    if onboarding.phase != crate::game::onboarding::OnboardingPhase::Finale
        || onboarding.finale_intro_shown
    {
        return;
    }

    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let scale = metrics::scene_scale(w, h);
    let panel_w = (w * 0.78).min(760.0 * scale);
    let panel_h = (150.0 * scale).min(h * 0.22);
    let panel_x = (w - panel_w) * 0.5;
    let panel_y = h * 0.12;

    let quads = vec![
        GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_DEEP, 0.94),
            user: 0,
        },
        GpuInstance {
            rect: [panel_x, panel_y, 4.0 * scale, panel_h],
            color: color::RUBY,
            user: 0,
        },
    ];

    let mut texts = Vec::new();
    widget::push_text_block(
        &mut texts,
        [
            panel_x + 20.0 * scale,
            panel_y + 14.0 * scale,
            panel_w - 36.0 * scale,
            panel_h - 24.0 * scale,
        ],
        finale_intro_message(),
        TextStyle {
            tier: typography::H36,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Left,
            glossary_tint: true,
        },
        h,
    );

    frame.overlay_quads(quads);
    frame.texts(texts);
}

pub fn mark_finale_intro_seen(run: &mut RunState) {
    if let Some(ref mut onboarding) = run.onboarding {
        onboarding.finale_intro_shown = true;
    }
}
