//! Post-ordeal interstitial — [`staircase.glb`](../../assets/3d/staircase.glb) and a single
//! prompt before the between-wing shop.

use crate::render::draw_cmd::{ScenePunctualLight, UiFrame};
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::render::staircase_glb;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::PointLight;
use crate::scenes::shop::ShopScene;
use crate::ui::input::UiAction;

use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

pub struct StaircaseScene;

impl StaircaseScene {
    pub fn new() -> Self {
        Self
    }
}

impl SceneBehavior for StaircaseScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx
            .actions
            .iter()
            .any(|a| matches!(a, UiAction::Confirm | UiAction::Cancel))
        {
            return Some(Scene::Shop(ShopScene::new(ctx.run, ctx.progress)));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        if staircase_glb::staircase_glb_loaded() {
            frame.camera_override =
                Some(staircase_glb::staircase_camera(w, h, ctx.room_gltf_height_scale));
            frame.staircase_environment();
            let room_glb = staircase_glb::staircase_glb_has_embedded_lights();
            frame.scene_lighting.embedded_gltf_punctual = room_glb;
            frame.scene_lighting.room_glb_brdf = room_glb;
            frame.scene_lighting.spot_lights = if room_glb {
                staircase_glb::staircase_embedded_spot_lights_runtime(
                    w,
                    h,
                    ctx.room_gltf_height_scale,
                    &ctx.shop_env_lighting,
                )
            } else {
                Vec::new()
            };
            let inverse_punctual: Vec<ScenePunctualLight> = if room_glb {
                staircase_glb::staircase_embedded_point_lights_runtime(
                    w,
                    h,
                    ctx.room_gltf_height_scale,
                    &ctx.shop_env_lighting,
                )
                .into_iter()
                .map(ScenePunctualLight::InverseSquare)
                .collect()
            } else {
                Vec::new()
            };
            frame.scene_lighting.punctual = inverse_punctual;
            if !room_glb {
                frame.scene_lighting.set_smooth_points(vec![PointLight {
                    pos: [w * 0.5, h * 0.55, h * 0.35],
                    radius: h * 1.4,
                    color: [0.92, 0.82, 0.68],
                    intensity: 1.35,
                }]);
            }
        }

        let prompt_font = typography::size(typography::H20, h);
        let prompt_y = h * 0.78;
        frame.text(TextLabel {
            text: "Press onward, despite the danger?".into(),
            rect: [0.0, prompt_y, w, prompt_font * 2.0],
            font_px: Some(prompt_font),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            ..Default::default()
        });

        frame.buttons = vec![ButtonDef::ui((0.0, 0.0, w, h), UiAction::Confirm)];

        frame
    }
}
