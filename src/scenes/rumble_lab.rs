//! Rumble lab — debug overlay for iterating dual-motor force feedback (SDL weak/strong rumble).
//!
//! Enter from **Debug → Scene Jumps → Rumble Lab…** or **Ctrl+Shift+H**.

use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{self, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::{InputState, RumbleLabOp, UiAction};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    CascadeStep,
    CascadeFinal,
    TripleStagger,
    WeakOnly,
    StrongOnly,
    AlternatingMotors,
    Heartbeat,
    EnvelopeSwell,
    Back,
}

impl LabAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }

    fn label(self) -> &'static str {
        match self {
            LabAction::CascadeStep => "Cascade step tick",
            LabAction::CascadeFinal => "Cascade final (medium)",
            LabAction::TripleStagger => "Triple stagger",
            LabAction::WeakOnly => "Weak motor only",
            LabAction::StrongOnly => "Strong motor only",
            LabAction::AlternatingMotors => "Alternating motors",
            LabAction::Heartbeat => "Heartbeat double-tap",
            LabAction::EnvelopeSwell => "Envelope swell",
            LabAction::Back => "Back",
        }
    }

    fn queue_rumble(self, ops: &mut Vec<RumbleLabOp>) {
        match self {
            LabAction::Back => {}
            LabAction::CascadeStep => {
                let (w, s, d, g) = InputState::cascade_step_rumble_params();
                ops.push(RumbleLabOp::Pulse {
                    weak: w,
                    strong: s,
                    duration_ms: d,
                    gain: g,
                });
            }
            LabAction::CascadeFinal => {
                let (w, s, d, g) = InputState::cascade_final_rumble_params(128);
                ops.push(RumbleLabOp::Pulse {
                    weak: w,
                    strong: s,
                    duration_ms: d,
                    gain: g,
                });
            }
            LabAction::TripleStagger => {
                ops.push(RumbleLabOp::Composite {
                    gain: 0.55,
                    segments: vec![
                        (0, 12_000, 4_000, 38),
                        (58, 10_000, 3_500, 38),
                        (116, 8_000, 2_800, 42),
                    ],
                });
            }
            LabAction::WeakOnly => {
                ops.push(RumbleLabOp::Pulse {
                    weak: 20_000,
                    strong: 0,
                    duration_ms: 120,
                    gain: 0.58,
                });
            }
            LabAction::StrongOnly => {
                ops.push(RumbleLabOp::Pulse {
                    weak: 0,
                    strong: 14_000,
                    duration_ms: 120,
                    gain: 0.52,
                });
            }
            LabAction::AlternatingMotors => {
                ops.push(RumbleLabOp::Composite {
                    gain: 0.52,
                    segments: vec![(0, 17_000, 800, 45), (52, 600, 15_000, 48)],
                });
            }
            LabAction::Heartbeat => {
                ops.push(RumbleLabOp::Composite {
                    gain: 0.5,
                    segments: vec![(0, 14_000, 5_000, 58), (125, 11_000, 4_200, 58)],
                });
            }
            LabAction::EnvelopeSwell => {
                ops.push(RumbleLabOp::Envelope {
                    gain: 0.48,
                    weak: 16_000,
                    strong: 5_500,
                    duration_ms: 260,
                    attack_ms: 55,
                    fade_ms: 85,
                });
            }
        }
    }
}

pub struct RumbleLabScene {
    has_suspended: bool,
    tree: TreeState,
}

impl RumbleLabScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            tree: TreeState::default(),
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()))
        }
    }

    fn layout_items(w: f32, h: f32, ui_scale: f32) -> Vec<FlatItem<LabAction>> {
        let scale = theme::metrics::scene_scale(w, h, ui_scale);
        let margin_x = (28.0 * scale).max(16.0);
        let grid_top = h * 0.22 + (12.0 * scale);
        let back_h = (44.0 * scale).max(36.0);
        let grid_bottom = h - back_h - (22.0 * scale).max(14.0);
        let grid_h = (grid_bottom - grid_top).max(120.0);

        let presets = [
            LabAction::CascadeStep,
            LabAction::CascadeFinal,
            LabAction::TripleStagger,
            LabAction::WeakOnly,
            LabAction::StrongOnly,
            LabAction::AlternatingMotors,
            LabAction::Heartbeat,
            LabAction::EnvelopeSwell,
        ];

        let cols = 2usize;
        let rows = presets.len().div_ceil(cols);
        let gap = (10.0 * scale).max(6.0);
        let btn_w = ((w - margin_x * 2.0 - gap) * 0.5).max(120.0);
        let btn_h = ((grid_h - gap * (rows as f32 - 1.0)) / rows as f32).max(34.0);

        let mut items = Vec::with_capacity(presets.len() + 1);
        for (i, &preset) in presets.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;
            let x = margin_x + col as f32 * (btn_w + gap);
            let y = grid_top + row as f32 * (btn_h + gap);
            items.push(FlatItem::new(preset.id(), [x, y, btn_w, btn_h], preset));
        }

        let back_y = h - back_h - (16.0 * scale).max(10.0);
        items.push(FlatItem::new(
            LabAction::Back.id(),
            [margin_x, back_y, w - margin_x * 2.0, back_h],
            LabAction::Back,
        ));

        items
    }
}

impl SceneBehavior for RumbleLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = Self::layout_items(ctx.layout.window_w, ctx.layout.window_h, ctx.ui_scale);
        let input = TreeInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (ctx.layout.window_w, ctx.layout.window_h),
            ui_scale: ctx.ui_scale,
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
            Some(other) => {
                other.queue_rumble(ctx.rumble_lab_ops);
                None
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let items = Self::layout_items(w, h, ui_scale);
        let focused = self.tree.focused();

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        let title_font = typography::size(typography::TITLE, h, ui_scale).max(22.0);
        let body_font = typography::size(typography::BODY, h, ui_scale).max(14.0);
        let hint_font = typography::size(typography::CAPTION, h, ui_scale).max(12.0);

        frame.text(TextLabel {
            rect: [0.0, h * 0.03, w, title_font * 1.5],
            text: "Rumble Lab".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        #[cfg(target_os = "macos")]
        let body_lines = "Fire preset patterns on connected FF-capable gamepads.\n\
            macOS: Xbox/Series rumble over USB is often unavailable to apps; Bluetooth may work. Check the log line SDL_PROP_GAMEPAD_CAP_RUMBLE when the pad connects.";
        #[cfg(not(target_os = "macos"))]
        let body_lines = "Fire preset patterns on connected FF-capable gamepads.";
        #[cfg(target_os = "macos")]
        let body_h = body_font * 2.85;
        #[cfg(not(target_os = "macos"))]
        let body_h = body_font * 1.45;

        frame.text(TextLabel {
            rect: [w * 0.06, h * 0.03 + title_font * 1.55, w * 0.88, body_h],
            text: body_lines.into(),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(body_font),
            ..Default::default()
        });

        frame.text(TextLabel {
            rect: [
                w * 0.06,
                h * 0.03 + title_font * 1.55 + body_h + body_font * 0.15,
                w * 0.88,
                hint_font * 2.2,
            ],
            text: "Ctrl+Shift+H opens this overlay.".into(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(hint_font),
            ..Default::default()
        });

        for it in &items {
            let is_focused = focused == Some(it.id);
            let bg = if is_focused {
                color::alpha(color::BRASS, 0.92)
            } else {
                color::alpha(color::WALNUT_INK, 0.94)
            };
            let fg = if is_focused {
                color::WALNUT_DEEP
            } else {
                color::CHAMPAGNE
            };
            frame.quad(GpuInstance {
                rect: it.rect,
                color: bg,
            });
            frame.text(TextLabel {
                rect: it.rect,
                text: it.action.label().into(),
                color: fg,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::BODY, h, ui_scale).max(14.0)),
                ..Default::default()
            });
        }

        self.tree.register_flat_buttons(&items, &mut frame.buttons);

        frame.window_title = "Mahjuro — Rumble Lab".into();
        frame
    }
}
