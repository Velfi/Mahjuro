use std::time::{Duration, Instant};

use crate::core::zodiac::ZodiacKind;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{Object3d, Object3dKind, UiFrame};
use crate::render::table_transform::rot_fixed_axes_deg_matrix;
use crate::render::wgpu_renderer::PointLight;
use crate::scenes::celebration_overlay;
use crate::ui::input::UiAction;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::load_shop_positions;

use super::{BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

const DISMISS_GRACE: f32 = 0.35;

pub struct ZodiacCelebrationScene {
    kind: ZodiacKind,
    yaku_name: &'static str,
    new_level: u32,
    /// Ribbon sway / entrance timing (independent of [`CelebrationStarShowerIntro`]).
    started_at: Instant,
    intro: celebration_overlay::CelebrationStarShowerIntro,
    /// Wall-clock start of the post-intro dismiss grace window.
    intro_grace_start: Option<Instant>,
    dismissed: bool,
}

impl ZodiacCelebrationScene {
    pub fn new(kind: ZodiacKind, yaku_name: &'static str, new_level: u32) -> Self {
        Self {
            kind,
            yaku_name,
            new_level,
            started_at: Instant::now(),
            intro: celebration_overlay::CelebrationStarShowerIntro::new(),
            intro_grace_start: None,
            dismissed: false,
        }
    }

    fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }
}

impl SceneBehavior for ZodiacCelebrationScene {
    fn has_blocking_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.intro
            .tick_audio(ctx.bus, ctx.headless, &ctx.effect_layers);
        if ctx.headless {
            self.intro.jump_to_done();
            self.intro_grace_start = Some(Instant::now() - Duration::from_secs_f32(1.0));
            self.started_at = Instant::now() - Duration::from_secs_f32(2.0);
        }
        if self.intro.is_done_for(&ctx.effect_layers) && self.intro_grace_start.is_none() {
            self.intro_grace_start = Some(Instant::now());
        }
        let grace_ok = self
            .intro_grace_start
            .map(|t| Instant::now().saturating_duration_since(t).as_secs_f32() >= DISMISS_GRACE)
            .unwrap_or(false);
        let has_input = ctx.actions.iter().any(|a| {
            matches!(
                a,
                UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
            )
        }) || !ctx.button_clicks.is_empty();
        if has_input && self.intro.is_done_for(&ctx.effect_layers) && grace_ok {
            self.dismissed = true;
        }
        if self.dismissed {
            ctx.bus.push(GameEvent::ZodiacLevelUp);
            GameEngine::set_finished_zodiac_celebration(ctx.run, self.yaku_name, self.new_level);
            *ctx.overlay_request = Some(super::OverlayRequest::Pop);
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        let intro_a = self.intro.content_alpha_for(&ctx.effect_layers);
        celebration_overlay::CelebrationOverlayScratch::new(w, h)
            .push_dimmer_scaled(&mut frame, intro_a)
            .push_starfield_if(&mut frame, ctx.effect_layers.starfield)
            .push_depth_reset_for_celebration_mesh(&mut frame);

        let t = self.elapsed();
        let ribbon_w = h * 0.12;
        let ribbon_l = h * 0.55;

        let sway_yaw = (t * 1.8).sin() * 12.0;
        let sway_roll = (t * 2.5 + 0.7).sin() * 6.0;
        // Base 90° pitch matches the shop's wall-hung pose so the ribbon
        // drapes along -Z; the small sin() wobble adds a gentle sway.
        let tilt = 90.0 + (t * 1.2).sin() * 3.0;
        let alpha = (t / 0.3).clamp(0.0, 1.0) * intro_a;

        let rx = tilt.to_radians();
        let ry = sway_yaw.to_radians();
        let rz = sway_roll.to_radians();
        let base_rotation = rot_fixed_axes_deg_matrix(
            glam::Mat4::from_rotation_z(rz)
                * glam::Mat4::from_rotation_y(ry)
                * glam::Mat4::from_rotation_x(rx),
        );
        // Re-read every frame so arrange-mode commits (which write to
        // shop.json immediately) take effect during a live celebration.
        let positions = load_shop_positions();
        let anchor = PlacementAnchor::new(
            [0.0, 0.0, 0.0],
            base_rotation,
            &positions.celeb_zodiac,
            "shop.celebrations.zodiac",
            ctx.layout,
        );
        let cx = anchor.pos[0];
        let cy = anchor.pos[1];
        let lift = anchor.pos[2];

        // Dramatic celestial lighting: warm key + cool fill on the hanging
        // ribbon so `scene_shaded` meshes are not rendered black against the
        // dimmer + starfield backdrop.
        frame.point_lights = vec![
            PointLight {
                pos: [cx + w * 0.18, cy - h * 0.10, lift + h * 0.35],
                radius: w.max(h) * 2.4,
                color: [1.00, 0.88, 0.62],
                intensity: 2.6,
            },
            PointLight {
                pos: [cx - w * 0.22, cy + h * 0.05, lift + h * 0.20],
                radius: w.max(h) * 2.0,
                color: [0.55, 0.70, 1.00],
                intensity: 1.4,
            },
            PointLight {
                pos: [cx, cy - h * 0.15, lift - h * 0.15],
                radius: w.max(h) * 1.6,
                color: [0.95, 0.78, 0.25],
                intensity: 1.1,
            },
        ];

        frame.object3d_batch(vec![Object3d {
            pos: anchor.pos,
            extents: [ribbon_w, ribbon_l, ribbon_w * 0.15],
            rotation: anchor.object3d_rotation(),
            color: [1.0, 1.0, 1.0, alpha],
            kind: Object3dKind::ZodiacRibbon {
                kind: Some(self.kind),
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some(anchor.arrange_name),
        }]);

        frame.text(celebration_overlay::label_zodiac_level_title(
            h,
            w,
            format!("{} Lvl.{}", self.yaku_name, self.new_level),
            alpha,
        ));
        frame.text(celebration_overlay::label_confirm_to_continue(
            h, w, t, alpha,
        ));

        if self.intro.is_done_for(&ctx.effect_layers) {
            frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];
        }
        frame.window_title = "Mahjuro".to_string();

        self.intro
            .push_shooting_star_cascade_if_active(&mut frame, &ctx.effect_layers);

        frame
    }
}
