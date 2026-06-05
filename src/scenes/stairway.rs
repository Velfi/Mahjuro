//! Post-ordeal interstitial — [`staircase.glb`](../../assets/3d/staircase.glb) and a single
//! prompt before the between-wing shop.

use crate::core::relic::RelicFlavorSpan;
use crate::render::draw_cmd::UiFrame;
use crate::render::scene_keys;
use crate::render::staircase_glb;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::PointLight;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::colored_keywords;
use crate::ui::controller_hints::{
    HintStyle, confirm_continue_footer_row, hint_style_with_alpha, push_inline_hint_rows,
};
use crate::ui::input::UiAction;
use crate::ui::inspect_plaque::{estimated_flavor_line_count, flavor_spans_layout_width};

use super::{BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

pub struct StairwayScene {
    flavor: &'static [RelicFlavorSpan],
}

impl Default for StairwayScene {
    fn default() -> Self {
        Self::new()
    }
}

impl StairwayScene {
    pub fn new() -> Self {
        Self {
            flavor: crate::core::staircase_flavor::random_entry_flavor(),
        }
    }
}

fn push_flavor_text(frame: &mut UiFrame, w: f32, h: f32, flavor: &'static [RelicFlavorSpan]) {
    if flavor.is_empty() {
        return;
    }
    let margin_x = w * 0.045;
    let margin_y = h * 0.11;
    let max_inner_w = (w * 0.44).min(560.0).max(w - 2.0 * margin_x);
    let body_px = typography::size(typography::H32, h);
    let line_step = colored_keywords::colored_row_line_step(body_px);
    let content_w = flavor_spans_layout_width(flavor, body_px, max_inner_w);
    let content_lines = estimated_flavor_line_count(flavor, content_w, body_px, 16);
    let content_h = line_step * content_lines as f32 + body_px * 0.35;
    let left = w - margin_x - content_w;
    let top = margin_y;

    frame.text(TextLabel {
        rect: [left, top, content_w, content_h],
        text: String::new(),
        color: color::CHAMPAGNE,
        font_px: Some(body_px),
        align: TextAlign::Center,
        flavor_spans: Some(flavor),
        ..Default::default()
    });
}

impl SceneBehavior for StairwayScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx
            .actions
            .iter()
            .any(|a| matches!(a, UiAction::Confirm | UiAction::Cancel))
        {
            return Some(SceneIntent::ShopFromRun);
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let flavor = self.flavor;
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        if staircase_glb::staircase_glb_loaded() {
            frame.camera_override = Some(staircase_glb::staircase_camera(
                w,
                h,
                ctx.room_gltf_height_scale,
            ));
            frame.staircase_environment();
            let room_glb = staircase_glb::staircase_glb_has_embedded_lights();
            frame.scene_lighting.embedded_gltf_punctual = room_glb;
            frame.scene_lighting.room_glb_brdf = room_glb;
            frame
                .scene_lighting
                .set_gltf_embedded_spot_lights(if room_glb {
                    staircase_glb::staircase_embedded_spot_lights_runtime(
                        w,
                        h,
                        ctx.room_gltf_height_scale,
                        &ctx.room_env_for(scene_keys::STAIRWAY).0,
                    )
                } else {
                    Vec::new()
                });
            let (inverse_punctual, punctual_gltf_nodes) = if room_glb {
                crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                    staircase_glb::staircase_embedded_point_lights_runtime_tagged(
                        w,
                        h,
                        ctx.room_gltf_height_scale,
                        &ctx.room_env_for(scene_keys::STAIRWAY).0,
                    ),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            frame.scene_lighting.punctual = inverse_punctual;
            frame.scene_lighting.punctual_gltf_nodes = punctual_gltf_nodes;
            if !room_glb {
                frame.scene_lighting.set_smooth_points(vec![PointLight {
                    pos: [w * 0.5, h * 0.55, h * 0.35],
                    radius: h * 1.4,
                    color: [0.92, 0.82, 0.68],
                    intensity: 1.35,
                }]);
            }
        }

        push_flavor_text(&mut frame, w, h, flavor);

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

        let hint_style = hint_style_with_alpha(HintStyle::standard(h), 0.92);
        let hint_line_h = hint_style.line_h;
        let hint_y = prompt_y + prompt_font * 2.0 + prompt_font * 0.55;
        push_inline_hint_rows(
            &mut frame,
            &ctx,
            &[[0.0, hint_y, w, hint_line_h]],
            &[confirm_continue_footer_row(ctx.input_mode, "")],
            hint_style,
        );

        frame.buttons = vec![ButtonDef::ui((0.0, 0.0, w, h), UiAction::Confirm)];

        frame
    }
}
