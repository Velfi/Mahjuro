//! Animation lab — debug overlay for scrubbing and playing embedded glTF node
//! animations on isolated room meshes (shop `Eyeball` / `eyeball_travel` by default).

use std::time::Instant;

use glam::Vec3;
use crate::render::draw_cmd::{CameraParams, UiFrame};
use crate::render::room_glb::{
    room_camera_with_room_clip_planes, room_node_mesh_center_world, with_shop_glb_cpu,
};
use crate::render::theme::{self, ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu::MainMenuScene;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const CLIP_NAME: &str = "eyeball_travel";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    Play,
    Pause,
    Restart,
    ScrubBar,
    Back,
}

impl LabAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    panel: [f32; 4],
    scrub_bar: [f32; 4],
}

pub struct AnimationLabScene {
    has_suspended: bool,
    tree: TreeState,
    scrub_secs: f32,
    duration_secs: f32,
    playing: bool,
    dragging_scrub: bool,
    age_secs: f32,
    last_frame: Instant,
}

impl AnimationLabScene {
    pub fn new(has_suspended: bool) -> Self {
        let duration_secs = with_shop_glb_cpu(|opt| {
            opt.and_then(|cpu| cpu.gltf_anim_library.clip_duration(CLIP_NAME))
        })
        .unwrap_or(0.0);
        Self {
            has_suspended,
            tree: TreeState::default(),
            scrub_secs: 0.0,
            duration_secs,
            playing: false,
            dragging_scrub: false,
            age_secs: 0.0,
            last_frame: Instant::now(),
        }
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(Scene::MainMenu(MainMenuScene::new()))
        }
    }

    fn tick_clock(&mut self) -> f32 {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        if self.playing && self.duration_secs > 0.0 {
            self.scrub_secs += dt;
            if self.scrub_secs >= self.duration_secs {
                self.scrub_secs = self.duration_secs;
                self.playing = false;
            }
        }
        dt
    }

    fn scrub_t(&self) -> f32 {
        if self.duration_secs <= 0.0 {
            0.0
        } else {
            (self.scrub_secs / self.duration_secs).clamp(0.0, 1.0)
        }
    }

    fn layout(w: f32, h: f32) -> Layout {
        let scale = theme::metrics::scene_scale(w, h);
        let panel_h = (168.0 * scale).clamp(132.0, h * 0.34);
        let panel_margin = (18.0 * scale).max(10.0);
        let panel = [
            panel_margin,
            h - panel_h - panel_margin,
            w - panel_margin * 2.0,
            panel_h,
        ];
        let scrub_bar = [
            panel[0] + panel[2] * 0.34,
            panel[1] + panel[3] * 0.38,
            panel[2] * 0.62,
            panel[3] * 0.18,
        ];
        Layout { panel, scrub_bar }
    }

    fn controls(&self, layout: &Layout) -> Vec<FlatItem<LabAction>> {
        let [x, y, w, h] = layout.panel;
        let left_x = x + w * 0.03;
        let left_w = w * 0.25;
        let btn_h = h * 0.20;
        let btn_gap = h * 0.05;
        let row_y = y + h * 0.62;
        let btn_w = (left_w - btn_gap * 2.0) / 3.0;
        vec![
            FlatItem::new(
                LabAction::Play.id(),
                [left_x, row_y, btn_w, btn_h],
                LabAction::Play,
            ),
            FlatItem::new(
                LabAction::Pause.id(),
                [left_x + btn_w + btn_gap, row_y, btn_w, btn_h],
                LabAction::Pause,
            ),
            FlatItem::new(
                LabAction::Restart.id(),
                [left_x + (btn_w + btn_gap) * 2.0, row_y, btn_w, btn_h],
                LabAction::Restart,
            ),
            FlatItem::new(
                LabAction::ScrubBar.id(),
                layout.scrub_bar,
                LabAction::ScrubBar,
            ),
            FlatItem::new(
                LabAction::Back.id(),
                [x + w * 0.72, y + h * 0.62, w * 0.24, btn_h],
                LabAction::Back,
            ),
        ]
    }

    fn set_scrub_from_cursor(&mut self, cursor_x: f32, rect: [f32; 4]) {
        let t = ((cursor_x - rect[0]) / rect[2].max(1.0)).clamp(0.0, 1.0);
        self.scrub_secs = t * self.duration_secs;
        self.playing = false;
    }

    fn cursor_on_scrub(&self, layout: &Layout, cursor: (f32, f32)) -> bool {
        let r = layout.scrub_bar;
        cursor.0 >= r[0] && cursor.0 <= r[0] + r[2] && cursor.1 >= r[1] && cursor.1 <= r[1] + r[3]
    }

    fn lab_camera(_w: f32, h: f32, env_h: f32) -> CameraParams {
        with_shop_glb_cpu(|opt| {
            let target = opt
                .and_then(|cpu| room_node_mesh_center_world(cpu, h, env_h, "Eyeball"))
                .unwrap_or(Vec3::ZERO);
            let s = crate::render::room_glb::room_env_world_scale(h, env_h);
            let radius = opt
                .and_then(|cpu| {
                    cpu.marker_mesh_bounds_doc
                        .get("Eyeball")
                        .map(|b| (b.max - b.min).length())
                })
                .unwrap_or(0.4 * s)
                .max(s * 0.08);
            let eye = target + Vec3::new(radius * 1.6, -radius * 1.8, radius * 1.1);
            let mut cam = CameraParams {
                eye: eye.to_array(),
                target: target.to_array(),
                up: [0.0, 0.0, 1.0],
                fovy_deg: 42.0,
                clip_near: Some((radius * 0.02).max(0.05)),
                clip_far: Some(radius * 40.0),
            };
            if let Some(cpu) = opt {
                cam = room_camera_with_room_clip_planes(cam, h, env_h, cpu);
            }
            cam
        })
    }
}

impl SceneBehavior for AnimationLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.tick_clock();

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

        if ctx.mouse_left_down {
            if self.dragging_scrub {
                self.set_scrub_from_cursor(ctx.cursor_pos.0, layout.scrub_bar);
            } else if self.cursor_on_scrub(&layout, ctx.cursor_pos) {
                self.dragging_scrub = true;
                self.set_scrub_from_cursor(ctx.cursor_pos.0, layout.scrub_bar);
            }
        } else {
            self.dragging_scrub = false;
        }

        match action {
            Some(LabAction::Play) => {
                if self.scrub_secs >= self.duration_secs && self.duration_secs > 0.0 {
                    self.scrub_secs = 0.0;
                }
                self.playing = true;
            }
            Some(LabAction::Pause) => self.playing = false,
            Some(LabAction::Restart) => {
                self.scrub_secs = 0.0;
                self.playing = true;
            }
            Some(LabAction::ScrubBar) => {
                self.set_scrub_from_cursor(ctx.cursor_pos.0, layout.scrub_bar);
            }
            Some(LabAction::Back) => return self.go_back(ctx.overlay_request),
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
        let env_h = ctx.room_env_for("shop").1;
        let _tune = ctx.room_env_for("shop").0;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);

        if with_shop_glb_cpu(|opt| opt.is_some()) {
            frame.shop_environment();
            frame.shop_env_eyeball_only = true;
            frame.shop_env_unlit_debug = true;
            frame.shop_gltf_anim_samples = vec![(CLIP_NAME.to_string(), self.scrub_secs)];
            frame.scene_lighting.room_glb_brdf = true;
        }

        frame.camera_override = Some(Self::lab_camera(w, h, env_h));

        draw_hud(
            &mut frame,
            &layout,
            &controls,
            focused,
            self.scrub_t(),
            self.scrub_secs,
            self.duration_secs,
            self.playing,
        );
        self.tree
            .register_flat_buttons(&controls, &mut frame.buttons);

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::archive_footer(h),
        );
        frame.window_title = "Mahjuro — Animation Lab".into();
        frame
    }
}

struct HudAnim {
    playing: bool,
}

fn draw_hud(
    frame: &mut UiFrame,
    layout: &Layout,
    controls: &[FlatItem<LabAction>],
    focused: Option<FocusId>,
    scrub_t: f32,
    scrub_secs: f32,
    duration_secs: f32,
    playing: bool,
) {
    let scale = theme::metrics::scene_scale(layout.panel[2], layout.panel[3]);
    let anim = HudAnim { playing };

    push_panel(
        frame,
        layout.panel,
        color::alpha(color::WALNUT_DEEP, 0.94),
        color::BRASS,
    );

    frame.text(TextLabel {
        rect: [
            layout.panel[0] + 14.0 * scale,
            layout.panel[1] + 10.0 * scale,
            layout.panel[2] - 28.0 * scale,
            layout.panel[3] * 0.22,
        ],
        text: format!("Animation Lab — {CLIP_NAME} (Eyeball, unlit)"),
        color: color::PARCHMENT,
        font_px: Some(typography::size(typography::H28, layout.panel[3] * 0.9)),
        align: TextAlign::Left,
        ..Default::default()
    });

    let time_label = if duration_secs > 0.0 {
        format!("{scrub_secs:.2}s / {duration_secs:.2}s")
    } else {
        "clip not loaded".to_string()
    };

    draw_bar(
        frame,
        layout.scrub_bar,
        scrub_t,
        time_label,
        focused == Some(LabAction::ScrubBar.id()),
        color::alpha(color::CHAMPAGNE, 0.88),
    );

    for item in controls {
        match item.action {
            LabAction::Play | LabAction::Pause | LabAction::Restart | LabAction::Back => {}
            LabAction::ScrubBar => continue,
        };
        let label = match item.action {
            LabAction::Play => "Play",
            LabAction::Pause => if anim.playing { "Pause" } else { "Paused" },
            LabAction::Restart => "Restart",
            LabAction::Back => "Back",
            LabAction::ScrubBar => continue,
        };
        draw_button(
            frame,
            item.rect,
            label,
            ButtonVariant::Default,
            focused == Some(item.id),
        );
    }

    let status = if duration_secs <= 0.0 {
        "Load shop.glb with eyeball_travel to preview"
    } else if anim.playing {
        "Playing"
    } else {
        "Scrub or press Play"
    };
    frame.text(TextLabel {
        rect: [
            layout.panel[0] + layout.panel[2] * 0.34,
            layout.panel[1] + layout.panel[3] * 0.62,
            layout.panel[2] * 0.34,
            layout.panel[3] * 0.18,
        ],
        text: status.into(),
        color: color::alpha(color::PARCHMENT, 0.82),
        font_px: Some(typography::size(typography::H36, layout.panel[3] * 0.5)),
        align: TextAlign::Left,
        ..Default::default()
    });
}

fn push_panel(frame: &mut UiFrame, rect: [f32; 4], bg: [f32; 4], border: [f32; 4]) {
    frame.quad(GpuInstance {
        rect,
        color: bg,
        user: 0,
    });
    let t = (rect[3] * 0.018).clamp(1.0, 2.0);
    for (dy, dh) in [(0.0, t), (rect[3] - t, t)] {
        frame.quad(GpuInstance {
            rect: [rect[0], rect[1] + dy, rect[2], dh],
            color: border,
            user: 0,
        });
    }
    for dx in [0.0, rect[2] - t] {
        frame.quad(GpuInstance {
            rect: [rect[0] + dx, rect[1] + t, t, rect[3] - t * 2.0],
            color: border,
            user: 0,
        });
    }
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
    for (dy, dh) in [(0.0, t), (outer[3] - t, t)] {
        frame.quad(GpuInstance {
            rect: [outer[0], outer[1] + dy, outer[2], dh],
            color: color::CHAMPAGNE,
            user: 0,
        });
    }
    for dx in [0.0, outer[2] - t] {
        frame.quad(GpuInstance {
            rect: [outer[0] + dx, outer[1] + t, t, outer[3] - t * 2.0],
            color: color::CHAMPAGNE,
            user: 0,
        });
    }
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
        font_px: Some(typography::size(typography::H36, rect[3] * 10.0)),
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
        font_px: Some(typography::size(typography::H36, rect[3] * 10.0)),
        ..Default::default()
    });
}
