//! Meta profile level-up (unlock carousel) on the full-screen showcase overlay.

use std::time::{Duration, Instant};

use crate::render::draw_cmd::{ShowcaseRenderHints, UiFrame, apply_modal_relic_staging};
use crate::scenes::celebration_overlay;
use crate::ui::input::UiAction;
use crate::ui::modal::{Modal, modal_paginated_unlock_layer_vecs};

use crate::scenes::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

pub struct MetaLevelUpPresenter {
    modal: Modal,
    started_at: Instant,
    intro_gate: celebration_overlay::CelebrationShowcaseIntroGate,
    last_fireworks_tick: Instant,
    dismissed: bool,
}

impl MetaLevelUpPresenter {
    pub fn new(modal: Modal) -> Self {
        Self {
            modal,
            started_at: Instant::now(),
            intro_gate: celebration_overlay::CelebrationShowcaseIntroGate::new(
                celebration_overlay::ShootingStarCelebrationIntro::new_meta_level_up(),
            ),
            last_fireworks_tick: Instant::now(),
            dismissed: false,
        }
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            layout_use_ray_plane_z: false,
            tile_pack_celebration_tonemap: false,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: true,
            modal_relic_staging: true,
        }
    }

    fn tick_modal_fireworks(&mut self, dt: f32) {
        if let Some(ref mut fw) = self.modal.fireworks {
            fw.update(dt);
        }
    }

    fn try_advance_or_dismiss(&mut self) -> bool {
        if self.modal.advance_unlock_page() {
            true
        } else {
            self.dismissed = true;
            false
        }
    }

    pub fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_fireworks_tick)
            .as_secs_f32();
        self.last_fireworks_tick = now;
        self.tick_modal_fireworks(dt);

        self.intro_gate.tick(&mut ctx);
        if ctx.headless {
            self.started_at = Instant::now() - Duration::from_secs_f32(2.0);
        }
        let has_input = ctx.actions.iter().any(|a| {
            matches!(
                a,
                UiAction::Confirm
                    | UiAction::Cancel
                    | UiAction::Pause
                    | UiAction::NorthFacePress
                    | UiAction::FocusPrev
                    | UiAction::FocusNext
                    | UiAction::CommitDiscard
            )
        }) || !ctx.button_clicks.is_empty();
        if has_input {
            self.intro_gate.skip_intro();
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusPrev => {
                    self.modal.navigate_unlock_page(-1);
                }
                UiAction::FocusNext => {
                    self.modal.navigate_unlock_page(1);
                }
                UiAction::Confirm => {
                    let _ = self.try_advance_or_dismiss();
                }
                UiAction::Cancel | UiAction::Pause | UiAction::NorthFacePress => {
                    self.dismissed = true;
                }
                _ => {}
            }
        }

        if !ctx.button_clicks.is_empty() {
            let _ = self.try_advance_or_dismiss();
        }

        if self.dismissed {
            *ctx.overlay_request = Some(OverlayRequest::Pop);
        }
        None
    }

    pub fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        let intro_a = self.intro_gate.intro.content_alpha_for(&ctx.effect_layers);
        // Meta level-up keeps the celebration dimmer + wipe; no zodiac starfield layer.
        celebration_overlay::CelebrationOverlayScratch::new(w, h)
            .push_dimmer_scaled(&mut frame, intro_a)
            .push_starfield_if(&mut frame, false)
            .push_depth_reset_for_celebration_mesh(&mut frame);

        let card_alpha = self.modal.card_fade_alpha() * intro_a;
        let (instances, labels, relic_objects, mut gradient_quads) =
            modal_paginated_unlock_layer_vecs(&self.modal, card_alpha, w, h);
        self.modal
            .append_fireworks_gradient_quads(&mut gradient_quads);

        frame.quads(instances);
        frame.texts(labels);
        if !gradient_quads.is_empty() {
            frame.gradient_quads(gradient_quads);
        }
        apply_modal_relic_staging(&mut frame, w, h, relic_objects);

        frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];

        frame.window_title = "Mahjuro".to_string();

        self.intro_gate
            .intro
            .push_shooting_star_cascade_if_active(&mut frame, &ctx.effect_layers);

        frame.showcase_render_hints = Self::render_hints();
        frame
    }
}
