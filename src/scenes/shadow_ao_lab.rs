//! Shadow & AO lab — programmatic box corridor for measuring punctual shadows
//! and synthetic contact AO (see [`crate::render::shadow_ao_lab`]).

use crate::render::draw_cmd::UiFrame;
use crate::render::scene_keys;
use crate::render::shadow_ao_lab::{
    ShadowAoLabCamera, ShadowAoLabLayout, ShadowAoLabProbe, build_object3ds, build_point_lights,
    camera, probe_layout,
};
use crate::render::shadow_test_room_glb;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};
use mahjuro_gfx_types::ShadowQuality;

use super::{BackgroundId, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    CycleScene,
    CycleLayout,
    CycleCamera,
    CycleShadows,
    Back,
}

impl LabAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabScene {
    Synthetic,
    ShadowTestRoom,
}

impl LabScene {
    fn label(self) -> &'static str {
        match self {
            Self::Synthetic => "Synthetic corridor",
            Self::ShadowTestRoom => "Shadow test room",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Synthetic => Self::ShadowTestRoom,
            Self::ShadowTestRoom => Self::Synthetic,
        }
    }
}

pub struct ShadowAoLabScene {
    has_suspended: bool,
    tree: TreeState,
    scene: LabScene,
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
            scene: LabScene::ShadowTestRoom,
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
            Some(SceneIntent::MainMenu)
        }
    }

    fn layout_items(w: f32, h: f32) -> Vec<FlatItem<LabAction>> {
        let scale = metrics::scene_scale(w, h);
        let margin = (14.0 * scale).max(8.0);
        let row_h = (36.0 * scale).max(28.0);
        let row_y = h - row_h - margin;
        let gap = 8.0;
        let btn_w = ((w - margin * 2.0) - gap * 4.0) / 5.0;
        [
            LabAction::CycleScene,
            LabAction::CycleLayout,
            LabAction::CycleCamera,
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
            LabAction::CycleScene => format!("Scene: {} v", self.scene.label()),
            LabAction::CycleLayout => {
                if self.scene == LabScene::Synthetic {
                    format!("Layout: {}", self.layout.label())
                } else {
                    "Layout: n/a".into()
                }
            }
            LabAction::CycleCamera => {
                if self.scene == LabScene::Synthetic {
                    format!("Cam: {}", self.camera.label())
                } else {
                    "Cam: GLB".into()
                }
            }
            LabAction::CycleShadows => format!("Shadows: {}", self.shadow_quality.label()),
            LabAction::Back => "Back".into(),
        }
    }

    fn push_shadow_test_room(&self, frame: &mut UiFrame, ctx: &DrawCtx<'_>, w: f32, h: f32) {
        let (tune, env_h) = ctx.room_env_for(scene_keys::SHADOW_AO_LAB);
        frame.shadow_test_environment();
        frame.camera_override = Some(shadow_test_room_glb::shadow_test_room_camera(w, h, env_h));
        let room_glb = shadow_test_room_glb::shadow_test_room_glb_has_embedded_lights();
        frame.scene_lighting.embedded_gltf_punctual = room_glb;
        frame.scene_lighting.room_glb_brdf = room_glb;
        let (punctual, nodes) = if room_glb {
            crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                shadow_test_room_glb::shadow_test_room_embedded_point_lights_runtime_tagged(
                    w, h, env_h, &tune,
                ),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        frame.scene_lighting.punctual = punctual;
        frame.scene_lighting.punctual_gltf_nodes = nodes;
        if !room_glb {
            frame.scene_lighting.push_smooth(PointLight {
                pos: [w * 0.5, h * 0.42, h * 1.15],
                radius: h * 3.0,
                color: [1.0, 0.92, 0.78],
                intensity: 5.0,
            });
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
            Some(LabAction::CycleScene) => {
                self.scene = self.scene.next();
                None
            }
            Some(LabAction::CycleLayout) => {
                self.layout = self.layout.next();
                None
            }
            Some(LabAction::CycleCamera) => {
                self.camera = self.camera.next();
                None
            }
            Some(LabAction::CycleShadows) => {
                self.shadow_quality = self.shadow_quality.next();
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let items = Self::layout_items(w, h);
        let focused = self.tree.focused();

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.shadow_quality_override = Some(self.shadow_quality);
        let probes = if self.scene == LabScene::Synthetic {
            frame.shadow_ao_lab_layout = Some(self.layout);
            frame.camera_override = Some(camera(self.camera, self.orbit_yaw));
            frame.object3d_batch(build_object3ds(self.layout, w, h));
            for light in build_point_lights(w, h) {
                frame.scene_lighting.push_smooth(light);
            }
            probe_layout(self.layout, h)
        } else {
            self.push_shadow_test_room(&mut frame, &ctx, w, h);
            Vec::new()
        };

        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H42, h);
        let margin = (14.0 * scale).max(8.0);

        frame.text(TextLabel {
            rect: [margin, margin, w - margin * 2.0, title_font * 1.4],
            text: "Shadow & AO Lab".into(),
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
                "scene={}  layout={}  camera={}  shadows={}",
                self.scene.label(),
                if self.scene == LabScene::Synthetic {
                    self.layout.label()
                } else {
                    "glb"
                },
                if self.scene == LabScene::Synthetic {
                    self.camera.label()
                } else {
                    "GLB"
                },
                self.shadow_quality.label(),
            ),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });

        let probe_text = if self.scene == LabScene::Synthetic {
            probe_lines(&probes)
        } else {
            vec![
                "GLB fixture - compare roof occlusion, receiver bands, and grounded-vs-raised contact"
                    .into(),
                "Use shadow quality changes here to compare leakage, acne, and map resolution"
                    .into(),
            ]
        };
        let line_h = body_font * 1.12;
        for (i, line) in probe_text.iter().enumerate() {
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
            HintStyle::standard(w, h),
        );
        frame.window_title = "Mahjuro — Shadow & AO Lab".into();
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| self.button_label(a));
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
