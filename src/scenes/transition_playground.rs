//! Transition playground — debug scene for scrubbing and playing scene-to-scene
//! transitions without needing to route through real game flow.

use std::time::Instant;

use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{self, ButtonState, ButtonVariant, color, typography};
use crate::render::transition_fx::{OverlayTransitionKind, push_overlay_transition};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const MIN_DURATION_SECS: f32 = 0.25;
const MAX_DURATION_SECS: f32 = 3.0;

pub struct TransitionPlaygroundScene {
    has_suspended: bool,
    tree: TreeState,
    progress: f32,
    duration_secs: f32,
    style: TransitionStyle,
    play_dir: i8,
    last_nonzero_dir: i8,
    preview_time: f32,
    last_frame: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransitionAction {
    PlayForward,
    PlayBackward,
    TogglePlayback,
    SnapA,
    SnapB,
    ProgressBar,
    DurationBar,
    StylePrev,
    StyleNext,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransitionStyle {
    Crossfade,
    SlideLeft,
    SlideUp,
    ZoomFade,
    TileTeeth,
    ForestOfTiles,
    GalaxyOfTiles,
    Maelstrom,
    TileWaterfall,
    ShufflingFan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoSceneKind {
    A,
    B,
}

#[derive(Clone, Copy, Debug)]
struct Pose {
    dx: f32,
    dy: f32,
    scale: f32,
    alpha: f32,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    viewport: [f32; 4],
    panel: [f32; 4],
    progress_bar: [f32; 4],
    duration_bar: [f32; 4],
}

impl TransitionAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

impl TransitionStyle {
    const ALL: [Self; 10] = [
        Self::Crossfade,
        Self::SlideLeft,
        Self::SlideUp,
        Self::ZoomFade,
        Self::TileTeeth,
        Self::ForestOfTiles,
        Self::GalaxyOfTiles,
        Self::Maelstrom,
        Self::TileWaterfall,
        Self::ShufflingFan,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Crossfade => "Crossfade",
            Self::SlideLeft => "Slide Left",
            Self::SlideUp => "Slide Up",
            Self::ZoomFade => "Zoom Fade",
            Self::TileTeeth => "Tile Teeth",
            Self::ForestOfTiles => "Forest of Tiles",
            Self::GalaxyOfTiles => "Galaxy of Tiles",
            Self::Maelstrom => "Maelstrom",
            Self::TileWaterfall => "Tile Waterfall",
            Self::ShufflingFan => "Shuffling Fan",
        }
    }

    fn step(self, dir: i32) -> Self {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32;
        let next = (idx + dir).rem_euclid(Self::ALL.len() as i32) as usize;
        Self::ALL[next]
    }
}

impl TransitionPlaygroundScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            tree: TreeState::default(),
            progress: 0.0,
            duration_secs: 1.1,
            style: TransitionStyle::Crossfade,
            play_dir: 0,
            last_nonzero_dir: 1,
            preview_time: 0.0,
            last_frame: Instant::now(),
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

    fn tick_preview(&mut self) -> f32 {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.preview_time += dt;
        if self.play_dir != 0 {
            let signed_dt = dt / self.duration_secs.max(0.001) * self.play_dir as f32;
            self.progress = (self.progress + signed_dt).clamp(0.0, 1.0);
            if self.progress <= 0.0 || self.progress >= 1.0 {
                self.play_dir = 0;
            }
        }
        dt
    }

    fn layout(w: f32, h: f32) -> Layout {
        let scale = theme::metrics::scene_scale(w, h);
        let panel_h = (172.0 * scale).clamp(136.0, h * 0.34);
        let panel_margin = (18.0 * scale).max(10.0);
        let panel = [
            panel_margin,
            h - panel_h - panel_margin,
            w - panel_margin * 2.0,
            panel_h,
        ];
        let viewport = [0.0, 0.0, w, (panel[1] - panel_margin).max(h * 0.5)];
        let progress_bar = [
            panel[0] + panel[2] * 0.34,
            panel[1] + panel[3] * 0.28,
            panel[2] * 0.62,
            panel[3] * 0.16,
        ];
        let duration_bar = [
            panel[0] + panel[2] * 0.34,
            panel[1] + panel[3] * 0.60,
            panel[2] * 0.62,
            panel[3] * 0.16,
        ];
        Layout {
            viewport,
            panel,
            progress_bar,
            duration_bar,
        }
    }

    fn controls(&self, layout: &Layout) -> Vec<FlatItem<TransitionAction>> {
        let [x, y, w, h] = layout.panel;
        let left_x = x + w * 0.03;
        let left_w = w * 0.25;
        let btn_h = h * 0.18;
        let btn_gap = h * 0.05;
        let row1_y = y + h * 0.24;
        let row2_y = y + h * 0.56;
        let btn_w = (left_w - btn_gap) * 0.5;
        let style_y = y + h * 0.80;
        let style_btn_w = btn_h;
        vec![
            FlatItem::new(
                TransitionAction::PlayForward.id(),
                [left_x, row1_y, btn_w, btn_h],
                TransitionAction::PlayForward,
            ),
            FlatItem::new(
                TransitionAction::PlayBackward.id(),
                [left_x + btn_w + btn_gap, row1_y, btn_w, btn_h],
                TransitionAction::PlayBackward,
            ),
            FlatItem::new(
                TransitionAction::TogglePlayback.id(),
                [left_x, row2_y, btn_w, btn_h],
                TransitionAction::TogglePlayback,
            ),
            FlatItem::new(
                TransitionAction::Back.id(),
                [left_x + btn_w + btn_gap, row2_y, btn_w, btn_h],
                TransitionAction::Back,
            ),
            FlatItem::new(
                TransitionAction::ProgressBar.id(),
                layout.progress_bar,
                TransitionAction::ProgressBar,
            ),
            FlatItem::new(
                TransitionAction::DurationBar.id(),
                layout.duration_bar,
                TransitionAction::DurationBar,
            ),
            FlatItem::new(
                TransitionAction::SnapA.id(),
                [x + w * 0.34, style_y, w * 0.14, btn_h * 0.92],
                TransitionAction::SnapA,
            ),
            FlatItem::new(
                TransitionAction::StylePrev.id(),
                [x + w * 0.50, style_y, style_btn_w, btn_h * 0.92],
                TransitionAction::StylePrev,
            ),
            FlatItem::new(
                TransitionAction::StyleNext.id(),
                [x + w * 0.83, style_y, style_btn_w, btn_h * 0.92],
                TransitionAction::StyleNext,
            ),
            FlatItem::new(
                TransitionAction::SnapB.id(),
                [x + w * 0.67, style_y, w * 0.14, btn_h * 0.92],
                TransitionAction::SnapB,
            ),
        ]
    }

    fn set_progress_from_cursor(&mut self, cursor_x: f32, rect: [f32; 4]) {
        let t = ((cursor_x - rect[0]) / rect[2]).clamp(0.0, 1.0);
        self.progress = t;
        self.play_dir = 0;
    }

    fn set_duration_from_cursor(&mut self, cursor_x: f32, rect: [f32; 4]) {
        let t = ((cursor_x - rect[0]) / rect[2]).clamp(0.0, 1.0);
        self.duration_secs = MIN_DURATION_SECS + (MAX_DURATION_SECS - MIN_DURATION_SECS) * t;
    }

    fn set_play_dir(&mut self, dir: i8) {
        self.play_dir = dir;
        self.last_nonzero_dir = dir;
    }

    fn is_playing(&self) -> bool {
        self.play_dir != 0
    }

    fn poses(&self, viewport: [f32; 4]) -> [(DemoSceneKind, Pose); 2] {
        let t = self.progress.clamp(0.0, 1.0);
        let w = viewport[2];
        let h = viewport[3];
        match self.style {
            TransitionStyle::Crossfade => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: 1.0 - t,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: t,
                    },
                ),
            ],
            TransitionStyle::SlideLeft => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: -w * t,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: 1.0,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: w * (1.0 - t),
                        dy: 0.0,
                        scale: 1.0,
                        alpha: 1.0,
                    },
                ),
            ],
            TransitionStyle::SlideUp => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: 0.0,
                        dy: -h * t,
                        scale: 1.0,
                        alpha: 1.0,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: 0.0,
                        dy: h * (1.0 - t),
                        scale: 1.0,
                        alpha: 1.0,
                    },
                ),
            ],
            TransitionStyle::ZoomFade => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0 + 0.08 * t,
                        alpha: 1.0 - t,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 0.92 + 0.08 * t,
                        alpha: t,
                    },
                ),
            ],
            TransitionStyle::TileTeeth => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: 1.0 - t,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: t,
                    },
                ),
            ],
            TransitionStyle::ForestOfTiles
            | TransitionStyle::GalaxyOfTiles
            | TransitionStyle::Maelstrom
            | TransitionStyle::TileWaterfall
            | TransitionStyle::ShufflingFan => [
                (
                    DemoSceneKind::A,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: 1.0 - t,
                    },
                ),
                (
                    DemoSceneKind::B,
                    Pose {
                        dx: 0.0,
                        dy: 0.0,
                        scale: 1.0,
                        alpha: t,
                    },
                ),
            ],
        }
    }
}

impl SceneBehavior for TransitionPlaygroundScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.tick_preview();

        if ctx.transitioning {
            return None;
        }

        let layout = Self::layout(ctx.layout.window_w, ctx.layout.window_h);
        let items = self.controls(&layout);
        let input = TreeInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window: (ctx.layout.window_w, ctx.layout.window_h),
            input_mode: ctx.input_mode,
            scroll_lines: ctx.scroll_lines,
        };
        let action = self.tree.update_flat(&items, input);

        for ui in ctx.actions {
            if matches!(ui, UiAction::Cancel | UiAction::Pause) {
                return self.go_back(ctx.overlay_request);
            }
        }

        match action {
            Some(TransitionAction::PlayForward) => self.set_play_dir(1),
            Some(TransitionAction::PlayBackward) => self.set_play_dir(-1),
            Some(TransitionAction::TogglePlayback) => {
                if self.is_playing() {
                    self.play_dir = 0;
                } else {
                    let dir = if self.progress <= 0.0 {
                        1
                    } else if self.progress >= 1.0 {
                        -1
                    } else {
                        self.last_nonzero_dir
                    };
                    self.set_play_dir(dir);
                }
            }
            Some(TransitionAction::SnapA) => {
                self.progress = 0.0;
                self.play_dir = 0;
            }
            Some(TransitionAction::SnapB) => {
                self.progress = 1.0;
                self.play_dir = 0;
            }
            Some(TransitionAction::ProgressBar) => {
                self.set_progress_from_cursor(ctx.cursor_pos.0, layout.progress_bar);
            }
            Some(TransitionAction::DurationBar) => {
                self.set_duration_from_cursor(ctx.cursor_pos.0, layout.duration_bar);
            }
            Some(TransitionAction::StylePrev) => self.style = self.style.step(-1),
            Some(TransitionAction::StyleNext) => self.style = self.style.step(1),
            Some(TransitionAction::Back) => return self.go_back(ctx.overlay_request),
            None => {}
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let layout = Self::layout(w, h);
        let controls = self.controls(&layout);
        let focused = self.tree.focused();

        let mut frame = UiFrame::new();
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });

        for (scene, pose) in self.poses(layout.viewport) {
            if pose.alpha > 0.001 {
                draw_demo_scene(&mut frame, layout.viewport, scene, pose, self.preview_time);
            }
        }
        if let Some(kind) = match self.style {
            TransitionStyle::TileTeeth => Some(OverlayTransitionKind::TileTeeth),
            TransitionStyle::ForestOfTiles => Some(OverlayTransitionKind::ForestOfTiles),
            TransitionStyle::GalaxyOfTiles => Some(OverlayTransitionKind::GalaxyOfTiles),
            TransitionStyle::Maelstrom => Some(OverlayTransitionKind::Maelstrom),
            TransitionStyle::TileWaterfall => Some(OverlayTransitionKind::TileWaterfall),
            TransitionStyle::ShufflingFan => Some(OverlayTransitionKind::ShufflingFan),
            TransitionStyle::Crossfade
            | TransitionStyle::SlideLeft
            | TransitionStyle::SlideUp
            | TransitionStyle::ZoomFade => None,
        } {
            push_overlay_transition(&mut frame, kind, self.progress, (w, layout.viewport[3]));
        }

        draw_top_banner(
            &mut frame,
            layout.viewport,
            self.progress,
            self.duration_secs,
            self.style,
            self.is_playing(),
        );

        push_panel(
            &mut frame,
            layout.panel,
            color::alpha(color::WALNUT_DEEP, 0.94),
            color::BRASS,
        );
        draw_controls(
            &mut frame,
            &layout,
            &controls,
            focused,
            PlaygroundAnim {
                progress: self.progress,
                duration_secs: self.duration_secs,
                style: self.style,
                is_playing: self.is_playing(),
            },
        );
        self.tree
            .register_flat_buttons(&controls, &mut frame.buttons);

        frame.window_title = "Mahjuro — Transition Playground".into();
        frame
    }
}

fn draw_top_banner(
    frame: &mut UiFrame,
    viewport: [f32; 4],
    progress: f32,
    duration_secs: f32,
    style: TransitionStyle,
    is_playing: bool,
) {
    let [x, y, w, _] = viewport;
    let scale = theme::metrics::scene_scale(w, viewport[3]);
    let badge = [
        x + 18.0 * scale,
        y + 18.0 * scale,
        420.0 * scale,
        76.0 * scale,
    ];
    push_panel(
        frame,
        badge,
        color::alpha(color::WALNUT_DEEP, 0.9),
        color::GOLD,
    );
    frame.text(TextLabel {
        rect: [
            badge[0] + 14.0 * scale,
            badge[1] + 10.0 * scale,
            badge[2],
            badge[3] * 0.38,
        ],
        text: format!(
            "{}  |  t = {:.2}  |  {:.2}s  |  {}",
            style.label(),
            progress,
            duration_secs,
            if is_playing { "playing" } else { "paused" }
        ),
        color: color::CHAMPAGNE,
        font_px: Some(typography::size(typography::HEADING, viewport[3])),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            badge[0] + 14.0 * scale,
            badge[1] + badge[3] * 0.48,
            badge[2],
            badge[3] * 0.28,
        ],
        text: "Scene A lives at t=0. Scene B lives at t=1. Click the bars to scrub.".into(),
        color: color::STONE,
        font_px: Some(typography::size(typography::CAPTION, viewport[3])),
        ..Default::default()
    });
}

/// Live animation state for the playground controls panel: current
/// progress (0..1), the configured duration in seconds, the selected
/// transition style, and whether the animation is currently running.
struct PlaygroundAnim {
    progress: f32,
    duration_secs: f32,
    style: TransitionStyle,
    is_playing: bool,
}

fn draw_controls(
    frame: &mut UiFrame,
    layout: &Layout,
    controls: &[FlatItem<TransitionAction>],
    focused: Option<FocusId>,
    anim: PlaygroundAnim,
) {
    let PlaygroundAnim {
        progress,
        duration_secs,
        style,
        is_playing,
    } = anim;
    let scale = theme::metrics::scene_scale(layout.panel[2], layout.panel[3]);

    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.03,
            layout.panel[1] + layout.panel[3] * 0.04,
            layout.panel[2] * 0.3,
            layout.panel[3] * 0.16,
        ],
        text: "Transition Playground".into(),
        color: color::CHAMPAGNE,
        font_px: Some(typography::size(typography::HEADING, layout.panel[3])),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.34,
            layout.panel[1] + layout.panel[3] * 0.08,
            layout.panel[2] * 0.62,
            layout.panel[3] * 0.12,
        ],
        text: "Progress".into(),
        color: color::STONE,
        font_px: Some(typography::size(typography::CAPTION, layout.panel[3])),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.34,
            layout.panel[1] + layout.panel[3] * 0.40,
            layout.panel[2] * 0.62,
            layout.panel[3] * 0.12,
        ],
        text: "Duration".into(),
        color: color::STONE,
        font_px: Some(typography::size(typography::CAPTION, layout.panel[3])),
        ..Default::default()
    });

    for item in controls {
        let is_focus = focused == Some(item.id);
        match item.action {
            TransitionAction::ProgressBar => {
                draw_bar(
                    frame,
                    item.rect,
                    progress,
                    format!("{:.0}%", progress * 100.0),
                    is_focus,
                    color::GOLD,
                );
            }
            TransitionAction::DurationBar => {
                let t = ((duration_secs - MIN_DURATION_SECS)
                    / (MAX_DURATION_SECS - MIN_DURATION_SECS))
                    .clamp(0.0, 1.0);
                draw_bar(
                    frame,
                    item.rect,
                    t,
                    format!("{duration_secs:.2}s"),
                    is_focus,
                    color::JADE,
                );
            }
            TransitionAction::StylePrev => {
                draw_button(frame, item.rect, "<", ButtonVariant::Subtle, is_focus);
            }
            TransitionAction::StyleNext => {
                draw_button(frame, item.rect, ">", ButtonVariant::Subtle, is_focus);
            }
            TransitionAction::PlayForward => {
                draw_button(
                    frame,
                    item.rect,
                    "Play A -> B",
                    ButtonVariant::Primary,
                    is_focus,
                );
            }
            TransitionAction::PlayBackward => {
                draw_button(
                    frame,
                    item.rect,
                    "Play B -> A",
                    ButtonVariant::Default,
                    is_focus,
                );
            }
            TransitionAction::TogglePlayback => {
                draw_button(
                    frame,
                    item.rect,
                    if is_playing { "Pause" } else { "Resume" },
                    ButtonVariant::Subtle,
                    is_focus,
                );
            }
            TransitionAction::Back => {
                draw_button(frame, item.rect, "Back", ButtonVariant::Subtle, is_focus);
            }
            TransitionAction::SnapA => {
                draw_button(frame, item.rect, "Jump A", ButtonVariant::Default, is_focus);
            }
            TransitionAction::SnapB => {
                draw_button(frame, item.rect, "Jump B", ButtonVariant::Default, is_focus);
            }
        }
    }

    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.55,
            layout.panel[1] + layout.panel[3] * 0.80,
            layout.panel[2] * 0.26,
            layout.panel[3] * 0.14,
        ],
        text: style.label().into(),
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(typography::size(typography::BODY, layout.panel[3])),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.82,
            layout.panel[1] + layout.panel[3] * 0.05,
            layout.panel[2] * 0.15,
            layout.panel[3] * 0.16,
        ],
        text: format!("{}x", (1.0 / duration_secs).max(0.1) * 1.1),
        color: color::UMBER,
        align: TextAlign::Right,
        font_px: Some(typography::size(typography::MICRO, layout.panel[3])),
        ..Default::default()
    });

    frame.quad(GpuInstance {
        rect: [
            layout.panel[0] + 14.0 * scale,
            layout.panel[1] + layout.panel[3] - 10.0 * scale,
            layout.panel[2] - 28.0 * scale,
            1.0,
        ],
        color: color::alpha(color::UMBER, 0.45),
        user: 0,
    });
}

fn draw_button(
    frame: &mut UiFrame,
    rect: [f32; 4],
    label: &str,
    variant: ButtonVariant,
    focused: bool,
) {
    let state = if focused {
        ButtonState::Hover
    } else {
        ButtonState::Rest
    };
    let colors = theme::button_colors(variant, state);
    push_panel(frame, rect, colors.bg, colors.border);
    if focused {
        push_focus_outline(frame, rect);
    }
    frame.text(TextLabel {
        rect,
        text: label.into(),
        color: colors.text,
        align: TextAlign::Center,
        font_px: Some(typography::size(typography::BODY, rect[3] * 10.0)),
        ..Default::default()
    });
}

fn draw_bar(
    frame: &mut UiFrame,
    rect: [f32; 4],
    t: f32,
    value_label: String,
    focused: bool,
    fill: [f32; 4],
) {
    let colors = theme::button_colors(
        ButtonVariant::Default,
        if focused {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        },
    );
    push_panel(frame, rect, colors.bg, colors.border);
    let inset = rect[3] * 0.22;
    frame.quad(GpuInstance {
        rect: [
            rect[0] + inset,
            rect[1] + inset,
            rect[2] - inset * 2.0,
            rect[3] - inset * 2.0,
        ],
        color: color::alpha(color::WALNUT_INK, 0.95),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [
            rect[0] + inset,
            rect[1] + inset,
            (rect[2] - inset * 2.0) * t.clamp(0.0, 1.0),
            rect[3] - inset * 2.0,
        ],
        color: fill,
        user: 0,
    });
    if focused {
        push_focus_outline(frame, rect);
    }
    frame.text(TextLabel {
        rect,
        text: value_label,
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(typography::size(typography::BODY, rect[3] * 10.0)),
        ..Default::default()
    });
}

fn draw_demo_scene(
    frame: &mut UiFrame,
    viewport: [f32; 4],
    kind: DemoSceneKind,
    pose: Pose,
    time: f32,
) {
    match kind {
        DemoSceneKind::A => draw_scene_a(frame, viewport, pose, time),
        DemoSceneKind::B => draw_scene_b(frame, viewport, pose, time),
    }
}

fn draw_scene_a(frame: &mut UiFrame, viewport: [f32; 4], pose: Pose, time: f32) {
    let glow = time.sin() * 0.5 + 0.5;
    push_scene_panel(
        frame,
        viewport,
        [0.09, 0.07, 0.12, pose.alpha],
        pose,
        [0.75, 0.52, 0.16, pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.05, 0.10, 0.90, 0.18],
        pose,
        [0.14, 0.11, 0.20, 0.95 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.08, 0.28, 0.52, 0.42],
        pose,
        [0.18, 0.13, 0.08, 0.92 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.64, 0.26, 0.24, 0.18],
        pose,
        [0.42, 0.24, 0.10, 0.90 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.64, 0.49, 0.24, 0.21],
        pose,
        [0.20, 0.15, 0.09, 0.95 * pose.alpha],
    );
    for i in 0..3 {
        let local_x = 0.14 + i as f32 * 0.14 + glow * 0.01 * (i as f32 - 1.0);
        push_rect(
            frame,
            viewport,
            [local_x, 0.76, 0.10, 0.10],
            pose,
            [0.85, 0.68 - i as f32 * 0.08, 0.28, 0.85 * pose.alpha],
        );
    }
    push_label(
        frame,
        viewport,
        [0.08, 0.12, 0.84, 0.08],
        "Scene A — ember hall",
        LabelStyle {
            color_rgba: color::CHAMPAGNE,
            font_px: typography::size(typography::TITLE, viewport[3]),
            align: TextAlign::Center,
        },
        pose,
    );
    push_label(
        frame,
        viewport,
        [0.11, 0.36, 0.46, 0.12],
        "Warm, chunky shapes and strong contrast are useful for testing fades.",
        LabelStyle {
            color_rgba: color::PARCHMENT,
            font_px: typography::size(typography::BODY, viewport[3]),
            align: TextAlign::Left,
        },
        pose,
    );
    push_label(
        frame,
        viewport,
        [0.67, 0.30, 0.18, 0.05],
        "Queue",
        LabelStyle {
            color_rgba: color::AMBER,
            font_px: typography::size(typography::CAPTION, viewport[3]),
            align: TextAlign::Center,
        },
        pose,
    );
    push_label(
        frame,
        viewport,
        [0.67, 0.54, 0.18, 0.05],
        "Status",
        LabelStyle {
            color_rgba: color::STONE,
            font_px: typography::size(typography::CAPTION, viewport[3]),
            align: TextAlign::Center,
        },
        pose,
    );
}

fn draw_scene_b(frame: &mut UiFrame, viewport: [f32; 4], pose: Pose, time: f32) {
    let wave = (time * 1.3).sin() * 0.5 + 0.5;
    push_scene_panel(
        frame,
        viewport,
        [0.04, 0.08, 0.12, pose.alpha],
        pose,
        [0.25, 0.66, 0.78, pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.04, 0.09, 0.92, 0.16],
        pose,
        [0.08, 0.17, 0.24, 0.96 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.08, 0.31, 0.28, 0.47],
        pose,
        [0.09, 0.24, 0.30, 0.94 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.41, 0.31, 0.47, 0.21],
        pose,
        [0.08, 0.13, 0.22, 0.94 * pose.alpha],
    );
    push_rect(
        frame,
        viewport,
        [0.41, 0.57 + wave * 0.015, 0.47, 0.13],
        pose,
        [0.14, 0.39, 0.44, 0.88 * pose.alpha],
    );
    for i in 0..4 {
        push_rect(
            frame,
            viewport,
            [0.11, 0.36 + i as f32 * 0.09, 0.21, 0.05],
            pose,
            [0.24, 0.74 - i as f32 * 0.08, 0.76, 0.80 * pose.alpha],
        );
    }
    push_label(
        frame,
        viewport,
        [0.08, 0.12, 0.84, 0.08],
        "Scene B — tide terminal",
        LabelStyle {
            color_rgba: color::PARCHMENT,
            font_px: typography::size(typography::TITLE, viewport[3]),
            align: TextAlign::Center,
        },
        pose,
    );
    push_label(
        frame,
        viewport,
        [0.45, 0.36, 0.39, 0.10],
        "Cool hues, offset cards, and animated rows help reveal sliding transforms.",
        LabelStyle {
            color_rgba: color::STONE,
            font_px: typography::size(typography::BODY, viewport[3]),
            align: TextAlign::Left,
        },
        pose,
    );
    push_label(
        frame,
        viewport,
        [0.45, 0.60, 0.39, 0.06],
        "Telemetry ribbon",
        LabelStyle {
            color_rgba: color::CHAMPAGNE,
            font_px: typography::size(typography::CAPTION, viewport[3]),
            align: TextAlign::Center,
        },
        pose,
    );
}

fn push_scene_panel(
    frame: &mut UiFrame,
    viewport: [f32; 4],
    local: [f32; 4],
    pose: Pose,
    border: [f32; 4],
) {
    push_rect(
        frame,
        viewport,
        [0.0, 0.0, 1.0, 1.0],
        pose,
        [local[0], local[1], local[2], local[3]],
    );
    let rect = transform_rect(viewport, [0.025, 0.03, 0.95, 0.92], pose);
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], 2.0],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1] + rect[3] - 2.0, rect[2], 2.0],
        color: border,
        user: 0,
    });
}

fn push_panel(frame: &mut UiFrame, rect: [f32; 4], bg: [f32; 4], border: [f32; 4]) {
    frame.quad(GpuInstance {
        rect,
        color: bg,
        user: 0,
    });
    let t = (rect[3] * 0.018).clamp(1.0, 2.0);
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], t],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1] + rect[3] - t, rect[2], t],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1] + t, t, rect[3] - t * 2.0],
        color: border,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0] + rect[2] - t, rect[1] + t, t, rect[3] - t * 2.0],
        color: border,
        user: 0,
    });
}

fn push_focus_outline(frame: &mut UiFrame, rect: [f32; 4]) {
    let pad = (rect[3] * 0.12).clamp(2.0, 5.0);
    let outer = [
        rect[0] - pad,
        rect[1] - pad,
        rect[2] + pad * 2.0,
        rect[3] + pad * 2.0,
    ];
    let t = pad.max(2.0);
    frame.quad(GpuInstance {
        rect: [outer[0], outer[1], outer[2], t],
        color: color::CHAMPAGNE,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [outer[0], outer[1] + outer[3] - t, outer[2], t],
        color: color::CHAMPAGNE,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [outer[0], outer[1] + t, t, outer[3] - t * 2.0],
        color: color::CHAMPAGNE,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [outer[0] + outer[2] - t, outer[1] + t, t, outer[3] - t * 2.0],
        color: color::CHAMPAGNE,
        user: 0,
    });
}

fn push_rect(
    frame: &mut UiFrame,
    viewport: [f32; 4],
    local: [f32; 4],
    pose: Pose,
    color_rgba: [f32; 4],
) {
    frame.quad(GpuInstance {
        rect: transform_rect(viewport, local, pose),
        color: color_rgba,
        user: 0,
    });
}

/// Visual style for a single playground `push_label` call: base RGBA
/// (alpha gets modulated by the animation pose), pixel-space font size
/// before the pose scale is applied, and text alignment.
struct LabelStyle {
    color_rgba: [f32; 4],
    font_px: f32,
    align: TextAlign,
}

fn push_label(
    frame: &mut UiFrame,
    viewport: [f32; 4],
    local: [f32; 4],
    text: &str,
    style: LabelStyle,
    pose: Pose,
) {
    let LabelStyle {
        mut color_rgba,
        font_px,
        align,
    } = style;
    color_rgba[3] *= pose.alpha;
    frame.text(TextLabel {
        rect: transform_rect(viewport, local, pose),
        text: text.into(),
        color: color_rgba,
        font_px: Some(font_px * pose.scale.max(0.7)),
        align,
        ..Default::default()
    });
}

fn transform_rect(viewport: [f32; 4], local: [f32; 4], pose: Pose) -> [f32; 4] {
    let vx = viewport[0];
    let vy = viewport[1];
    let vw = viewport[2];
    let vh = viewport[3];
    let local_px = [
        vx + local[0] * vw,
        vy + local[1] * vh,
        local[2] * vw,
        local[3] * vh,
    ];
    let cx = vx + vw * 0.5;
    let cy = vy + vh * 0.5;
    let x = cx + (local_px[0] - cx) * pose.scale + pose.dx;
    let y = cy + (local_px[1] - cy) * pose.scale + pose.dy;
    [x, y, local_px[2] * pose.scale, local_px[3] * pose.scale]
}
