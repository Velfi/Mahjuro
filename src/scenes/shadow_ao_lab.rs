//! Shadow & AO lab — programmatic box corridor for measuring punctual shadows
//! and synthetic contact AO (see [`crate::render::shadow_ao_lab`]).

use crate::render::draw_cmd::UiFrame;
use crate::render::scene_keys;
use crate::render::shadow_ao_lab::{
    ShadowAoLabCamera, ShadowAoLabLayout, ShadowAoLabProbe, build_object3ds, build_point_lights,
    camera, probe_layout,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};
use mahjuro_gfx_types::ShadowQuality;

use super::main_menu::MainMenuScene;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    CycleLayout,
    CycleCamera,
    OrbitLeft,
    OrbitRight,
    CycleShadows,
    Back,
}

impl LabAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub struct ShadowAoLabScene {
    has_suspended: bool,
    tree: TreeState,
    layout: ShadowAoLabLayout,
    camera: ShadowAoLabCamera,
    orbit_yaw: f32,
    shadow_quality: ShadowQuality,
}

impl ShadowAoLabScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            tree: TreeState::default(),
            layout: ShadowAoLabLayout::HorizontalBand,
            camera: ShadowAoLabCamera::Corridor,
            orbit_yaw: 0.0,
            shadow_quality: ShadowQuality::High,
        }
    }

    pub fn renderer_scene_key(&self) -> &'static str {
        scene_keys::SHADOW_AO_LAB
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::MainMenu(MainMenuScene::new()))
        }
    }

    fn layout_items(w: f32, h: f32) -> Vec<FlatItem<LabAction>> {
        let scale = metrics::scene_scale(w, h);
        let margin = (14.0 * scale).max(8.0);
        let row_h = (36.0 * scale).max(28.0);
        let row_y = h - row_h - margin;
        let gap = 8.0;
        let btn_w = ((w - margin * 2.0) - gap * 5.0) / 6.0;
        [
            LabAction::CycleLayout,
            LabAction::CycleCamera,
            LabAction::OrbitLeft,
            LabAction::OrbitRight,
            LabAction::CycleShadows,
            LabAction::Back,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, action)| {
            FlatItem::new(
                action.id(),
                [margin + (btn_w + gap) * i as f32, row_y, btn_w, row_h],
                action,
            )
        })
        .collect()
    }

    fn button_label(&self, action: LabAction) -> String {
        match action {
            LabAction::CycleLayout => format!("Layout: {}", self.layout.label()),
            LabAction::CycleCamera => format!("Cam: {}", self.camera.label()),
            LabAction::CycleShadows => format!("Shadows: {}", self.shadow_quality.label()),
            LabAction::OrbitLeft => "◀ orbit".into(),
            LabAction::OrbitRight => "orbit ▶".into(),
            LabAction::Back => "Back".into(),
        }
    }
}

impl SceneBehavior for ShadowAoLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = Self::layout_items(ctx.layout.window_w, ctx.layout.window_h);
        let input = TreeInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (ctx.layout.window_w, ctx.layout.window_h),
            input_mode: ctx.input_mode,
            scroll_lines: ctx.scroll_lines,
        };
        let fired = self.tree.update_flat(&items, input);
        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                return self.go_back(ctx.overlay_request);
            }
        }
        match fired {
            Some(LabAction::Back) => self.go_back(ctx.overlay_request),
            Some(LabAction::CycleLayout) => {
                self.layout = self.layout.next();
                None
            }
            Some(LabAction::CycleCamera) => {
                self.camera = self.camera.next();
                None
            }
            Some(LabAction::OrbitLeft) => {
                self.orbit_yaw -= 0.12;
                None
            }
            Some(LabAction::OrbitRight) => {
                self.orbit_yaw += 0.12;
                None
            }
            Some(LabAction::CycleShadows) => {
                self.shadow_quality = self.shadow_quality.next();
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let items = Self::layout_items(w, h);
        let focused = self.tree.focused();
        let probes = probe_layout(self.layout, h);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.shadow_ao_lab_layout = Some(self.layout);
        frame.shadow_quality_override = Some(self.shadow_quality);
        frame.camera_override = Some(camera(self.camera, self.orbit_yaw));
        frame.object3d_batch(build_object3ds(self.layout, w, h));
        for light in build_point_lights(w, h) {
            frame.scene_lighting.push_smooth(light);
        }

        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H42, h);
        let margin = (14.0 * scale).max(8.0);

        frame.text(TextLabel {
            rect: [margin, margin, w - margin * 2.0, title_font * 1.4],
            text: "Shadow & AO Lab (synthetic)".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });

        frame.text(TextLabel {
            rect: [
                margin,
                margin + title_font * 1.5,
                w - margin * 2.0,
                body_font * 1.2,
            ],
            text: format!(
                "layout={}  camera={}  shadows={}  orbit={:.2}",
                self.layout.label(),
                self.camera.label(),
                self.shadow_quality.label(),
                self.orbit_yaw,
            ),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });

        let line_h = body_font * 1.12;
        for (i, line) in probe_lines(&probes).iter().enumerate() {
            frame.text(TextLabel {
                rect: [
                    margin,
                    margin + title_font * 2.7 + line_h * i as f32,
                    w - margin * 2.0,
                    line_h,
                ],
                text: line.clone(),
                color: color::PARCHMENT,
                align: TextAlign::Left,
                font_px: Some(body_font),
                ..Default::default()
            });
        }

        for it in &items {
            let is_focused = focused == Some(it.id);
            frame.quad(GpuInstance {
                rect: it.rect,
                color: if is_focused {
                    color::alpha(color::BRASS, 0.92)
                } else {
                    color::alpha(color::WALNUT_INK, 0.94)
                },
                user: 0,
            });
            frame.text(TextLabel {
                rect: it.rect,
                text: self.button_label(it.action),
                color: if is_focused {
                    color::WALNUT_DEEP
                } else {
                    color::CHAMPAGNE
                },
                align: TextAlign::Center,
                font_px: Some((body_font * 0.9).max(11.0)),
                ..Default::default()
            });
        }
        self.tree.register_flat_buttons(&items, &mut frame.buttons);

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::standard(h),
        );
        frame.window_title = "Mahjuro — Shadow & AO Lab".into();
        frame
    }

    fn has_blocking_overlay(&self) -> bool {
        true
    }
}

fn probe_lines(probes: &[ShadowAoLabProbe]) -> Vec<String> {
    let mut lines = vec![
        "CPU probes — analytic=punctual ray test; AO=synthetic bake; applies=depth-coherence gate"
            .into(),
    ];
    for p in probes {
        let ao = p
            .contact_ao
            .map(|c| {
                format!(
                    "ao={} Δ={} applies={}",
                    c.ao,
                    c.depth_delta
                        .map(|d| format!("{d:.3}"))
                        .unwrap_or_else(|| "—".into()),
                    c.applies
                )
            })
            .unwrap_or_else(|| "ao=—".into());
        lines.push(format!(
            "{:<10} shadow={:.0}  {ao}",
            p.label, p.analytic_shadow
        ));
    }
    lines
}
