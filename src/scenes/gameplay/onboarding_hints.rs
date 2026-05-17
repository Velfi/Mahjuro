//! Guided onboarding prompts during the Lessons blind.

use crate::game::onboarding::{finale_intro_message, lessons_affinity_indices};
use crate::game::run::RunState;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::ui::widget::{self, TextStyle};

/// Sync step 0 → 1 when the player selects tiles.
pub fn sync_onboarding_step(run: &mut RunState) {
    let lessons = run
        .onboarding
        .as_ref()
        .is_some_and(|o| o.lessons_active());
    if !lessons {
        return;
    }
    let advance = run.onboarding.as_ref().is_some_and(|o| o.step == 0)
        && crate::game::engine::GameEngine::read(run).selected_count > 0;
    if advance {
        if let Some(ref mut onboarding) = run.onboarding {
            onboarding.step = 1;
        }
    }
}

pub fn lessons_hint_indices(run: &RunState) -> Option<Vec<usize>> {
    if !run.onboarding_lessons_active() {
        return None;
    }
    let interaction = crate::game::engine::GameEngine::read_interaction(run);
    if interaction.selected_indices.is_empty() {
        return None;
    }
    Some(lessons_affinity_indices(
        &interaction.hand,
        &interaction.selected,
    ))
}

pub fn push_lessons_banner(frame: &mut UiFrame, ctx: &DrawCtx<'_>, run: &RunState) {
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

    let banner_w = (w * 0.72).min(720.0 * scale);
    let banner_h = (56.0 * scale).max(44.0);
    let banner_x = (w - banner_w) * 0.5;
    let banner_y = h * 0.055;

    let mut quads = Vec::new();
    quads.push(GpuInstance {
        rect: [banner_x, banner_y, banner_w, banner_h],
        color: color::alpha(color::WALNUT_INK, 0.92),
        user: 0,
    });
    quads.push(GpuInstance {
        rect: [banner_x, banner_y, banner_w, 3.0 * scale],
        color: color::GOLD,
        user: 0,
    });

    let mut texts = Vec::new();
    widget::push_text_block(
        &mut texts,
        [
            banner_x + 16.0 * scale,
            banner_y + 10.0 * scale,
            banner_w - 32.0 * scale,
            banner_h - 16.0 * scale,
        ],
        prompt,
        TextStyle {
            tier: typography::H36,
            color: color::CHAMPAGNE,
            padding: 0.0,
            align: TextAlign::Center,
            ..Default::default()
        },
        h,
    );

    if onboarding.step >= 3 && onboarding.discard_river_tooltip_shown {
        texts.push(TextLabel {
            rect: [
                banner_x,
                banner_y + banner_h + 6.0 * scale,
                banner_w,
                22.0 * scale,
            ],
            text: "Discarded tiles sit in the river — they don't score.".to_string(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H42, h)),
            ..Default::default()
        });
    }

    frame.quads(quads);
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

    let mut quads = Vec::new();
    quads.push(GpuInstance {
        rect: [panel_x, panel_y, panel_w, panel_h],
        color: color::alpha(color::WALNUT_DEEP, 0.94),
        user: 0,
    });
    quads.push(GpuInstance {
        rect: [panel_x, panel_y, 4.0 * scale, panel_h],
        color: color::RUBY,
        user: 0,
    });

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
            ..Default::default()
        },
        h,
    );

    frame.quads(quads);
    frame.texts(texts);
}

pub fn mark_finale_intro_seen(run: &mut RunState) {
    if let Some(ref mut onboarding) = run.onboarding {
        onboarding.finale_intro_shown = true;
    }
}
