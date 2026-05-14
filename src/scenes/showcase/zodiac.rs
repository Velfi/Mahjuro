//! Zodiac level-up ribbon on the showcase overlay.

use std::time::{Duration, Instant};

use crate::core::zodiac::ZodiacKind;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{ShowcaseRenderHints, UiFrame};
use crate::render::theme::color;
use crate::render::ribbon_mesh::{ZodiacRibbonSpec, zodiac_ribbon_object3d};
use crate::render::table_transform::rot_fixed_axes_deg_matrix;
use crate::render::world_space::pixel_to_world;
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::scenes::celebration_overlay;
use crate::ui::input::UiAction;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::load_shop_positions;

use crate::scenes::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

pub struct ZodiacPresenter {
    kind: ZodiacKind,
    yaku_name: &'static str,
    new_level: u32,
    started_at: Instant,
    intro_gate: celebration_overlay::CelebrationShowcaseIntroGate,
    dismissed: bool,
}

impl ZodiacPresenter {
    pub fn new(kind: ZodiacKind, yaku_name: &'static str, new_level: u32) -> Self {
        Self {
            kind,
            yaku_name,
            new_level,
            started_at: Instant::now(),
            intro_gate: celebration_overlay::CelebrationShowcaseIntroGate::new(
                celebration_overlay::ShootingStarCelebrationIntro::new_zodiac(),
            ),
            dismissed: false,
        }
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints::default()
    }

    fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    pub fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        self.intro_gate.tick(&mut ctx);
        if ctx.headless {
            self.started_at = Instant::now() - Duration::from_secs_f32(2.0);
        }
        let has_input = ctx.actions.iter().any(|a| {
            matches!(
                a,
                UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
            )
        }) || !ctx.button_clicks.is_empty();
        if has_input && self.intro_gate.ready_for_dismiss(&ctx.effect_layers) {
            self.dismissed = true;
        }
        if self.dismissed {
            ctx.bus.push(GameEvent::ZodiacLevelUp);
            GameEngine::set_finished_zodiac_celebration(ctx.run, self.yaku_name, self.new_level);
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
        celebration_overlay::CelebrationOverlayScratch::new(w, h)
            .push_dimmer_scaled(&mut frame, intro_a)
            .push_starfield_if(&mut frame, ctx.effect_layers.starfield)
            .push_depth_reset_for_celebration_mesh(&mut frame);

        let t = self.elapsed();
        let ribbon_l = h * 0.55;

        let sway_yaw = (t * 1.8).sin() * 12.0;
        let sway_roll = (t * 2.5 + 0.7).sin() * 6.0;
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

        frame.scene_lighting.set_smooth_points(vec![
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
                color: color::rgb(color::RELIC_GOLD),
                intensity: 1.1,
            },
        ]);

        let cos_outer = (36.0_f32).to_radians().cos();
        let cos_inner = (22.0_f32).to_radians().cos();
        let spot_lift = lift + h * 0.42;
        let spot_pos = [cx, cy - h * 0.06, spot_lift];
        let tw = pixel_to_world(w, h, cx, cy, lift);
        let lw = pixel_to_world(w, h, spot_pos[0], spot_pos[1], spot_pos[2]);
        let dir = (tw - lw).normalize_or_zero();
        let dir = if dir.length_squared() < 1e-4 {
            glam::Vec3::new(0.0, 0.4, -1.0).normalize()
        } else {
            dir
        };
        frame.scene_lighting.spot_lights = vec![SpotLight {
            pos: spot_pos,
            dir: dir.to_array(),
            radius: w.max(h) * 2.2,
            cos_outer,
            cos_inner,
            color: color::rgb(color::TALLOW),
            intensity: 6.0,
        }];

        frame.object3d_batch(vec![zodiac_ribbon_object3d(ZodiacRibbonSpec {
            pos: anchor.pos,
            length: ribbon_l,
            rotation: anchor.object3d_rotation(),
            color: [1.0, 1.0, 1.0, alpha],
            kind: Some(self.kind),
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some(anchor.arrange_name),
        })]);

        frame.text(celebration_overlay::label_zodiac_level_title(
            h,
            w,
            format!("{} Lvl.{}", self.yaku_name, self.new_level),
            alpha,
        ));
        frame.text(celebration_overlay::label_confirm_to_continue(
            h, w, t, alpha,
        ));

        if self.intro_gate.intro.is_done_for(&ctx.effect_layers) {
            frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];
        }
        frame.window_title = "Mahjuro".to_string();

        self.intro_gate
            .intro
            .push_shooting_star_cascade_if_active(&mut frame, &ctx.effect_layers);

        frame.showcase_render_hints = Self::render_hints();
        frame
    }
}
