//! Roller Lab — debug scene for testing authored gameplay score rollers.
//!
//! Entered from Debug → Labs → Roller Lab...

use std::time::Instant;

use crate::render::draw_cmd::{CameraParams, UiFrame};
use crate::render::theme::{self, ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;

use super::{BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

const CLICK_PREV: u32 = 0xE210;
const CLICK_NEXT: u32 = 0xE211;
const CLICK_ZERO: u32 = 0xE212;
const CLICK_HUGE: u32 = 0xE213;
const CLICK_BACK: u32 = 0xE214;

const PRESETS: &[(u64, u64)] = &[
    (0, 500),
    (126, 500),
    (1440, 500),
    (500, 500),
    (9876, 12500),
    (999_999, 250_000),
    (12_345_678, 87_654_321),
];

pub struct RollerLabScene {
    has_suspended: bool,
    preset_idx: usize,
    score: u64,
    target: u64,
    last_frame: Instant,
    lamp_time: f32,
}

impl RollerLabScene {
    pub fn new(has_suspended: bool) -> Self {
        let (score, target) = PRESETS[0];
        Self {
            has_suspended,
            preset_idx: 0,
            score,
            target,
            last_frame: Instant::now(),
            lamp_time: 0.0,
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }

    fn tick_time(&mut self) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.lamp_time += dt;
    }

    fn apply_preset(&mut self, idx: usize) {
        self.preset_idx = idx % PRESETS.len();
        let (score, target) = PRESETS[self.preset_idx];
        self.score = score;
        self.target = target;
    }

    fn next_preset(&mut self) {
        self.apply_preset((self.preset_idx + 1) % PRESETS.len());
    }

    fn prev_preset(&mut self) {
        self.apply_preset((self.preset_idx + PRESETS.len() - 1) % PRESETS.len());
    }
}

impl SceneBehavior for RollerLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.tick_time();

        for &cid in ctx.button_clicks {
            match cid {
                CLICK_PREV => self.prev_preset(),
                CLICK_NEXT => self.next_preset(),
                CLICK_ZERO => {
                    self.score = 0;
                    self.target = 500;
                }
                CLICK_HUGE => {
                    self.score = 9_876_543;
                    self.target = 12_345_678;
                }
                CLICK_BACK => return self.go_back(ctx.overlay_request),
                _ => {}
            }
        }
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => return self.go_back(ctx.overlay_request),
                _ => {}
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.window_title = "Mahjuro — Roller Lab".into();

        let env_h = ctx.room_env_for("gameplay").1;
        let camera = crate::render::gameplay_glb::gameplay_camera_from_glb_if_present(h, env_h);
        frame.camera_override = camera.or(Some(CameraParams::default_table_camera(h)));

        frame.gameplay_score_roller_values = Some((self.score, self.target));
        frame.gameplay_cash_in_button_visible = false;
        frame.gameplay_environment();

        let room_glb_lights = crate::render::gameplay_glb::gameplay_glb_has_embedded_lights();
        frame.scene_lighting.embedded_gltf_punctual = room_glb_lights;
        frame.scene_lighting.room_glb_brdf = room_glb_lights;
        if room_glb_lights {
            let tune = ctx.room_env_for("gameplay").0;
            frame.scene_lighting.punctual =
                crate::render::gameplay_glb::gameplay_embedded_point_lights_runtime(
                    w,
                    h,
                    env_h,
                    &tune,
                    self.lamp_time,
                    1.0,
                    ctx.flame_tuning.candle_flicker_amp,
                )
                .into_iter()
                .map(crate::render::draw_cmd::ScenePunctualLight::InverseSquare)
                .collect();
            frame.scene_lighting.set_gltf_embedded_spot_lights(
                crate::render::gameplay_glb::gameplay_embedded_spot_lights_runtime(
                    w, h, env_h, &tune,
                ),
            );
        }

        let btn_y = h * 0.92;
        let btn_h = (42.0 * scale).max(30.0);
        let panel_h = h * 0.14;
        let panel = [w * 0.28, btn_y - panel_h - h * 0.02, w * 0.66, panel_h];
        frame.quad(GpuInstance {
            rect: panel,
            color: color::alpha(color::WALNUT_DEEP, 0.90),
            user: 0,
        });
        frame.text(TextLabel {
            rect: [
                panel[0],
                panel[1] + panel[3] * 0.10,
                panel[2],
                panel[3] * 0.28,
            ],
            text: format!("Score: {}   Target: {}", self.score, self.target),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H28, h)),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [
                panel[0],
                panel[1] + panel[3] * 0.42,
                panel[2],
                panel[3] * 0.24,
            ],
            text: "Presets: Prev/Next or confirm. Esc: exit.".into(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H42, h)),
            ..Default::default()
        });

        let btn_w = (118.0 * scale).max(84.0);
        let gap = (10.0 * scale).max(6.0);
        let total_w = btn_w * 5.0 + gap * 4.0;
        let x0 = (w - total_w) * 0.5;
        let buttons = [
            (CLICK_PREV, "Prev"),
            (CLICK_NEXT, "Next"),
            (CLICK_ZERO, "0/500"),
            (CLICK_HUGE, "Huge"),
            (CLICK_BACK, "Back"),
        ];
        for (i, (id, label)) in buttons.iter().enumerate() {
            let x = x0 + i as f32 * (btn_w + gap);
            let rect = [x, btn_y, btn_w, btn_h];
            let colors = theme::button_colors(ButtonVariant::Default, ButtonState::Rest);
            frame.quad(GpuInstance {
                rect,
                color: colors.bg,
                user: 0,
            });
            frame.text(TextLabel {
                rect,
                text: (*label).into(),
                color: colors.text,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H36, h)),
                ..Default::default()
            });
            frame
                .buttons
                .push(ButtonDef::scene((rect[0], rect[1], rect[2], rect[3]), *id));
        }

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame
    }
}
