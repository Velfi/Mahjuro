//! Start screen — candlelit 3D menu with physical wood tablets on a table.

use std::cell::RefCell;
use std::time::Instant;

use crate::game::run::RunState;
use crate::persistence::{self, TileMaterial};
use crate::render::candle_mesh::{CandlePlacement, WICK_TIP_Y};
use crate::render::draw_cmd::{CameraParams, PlaquePlacement, UiFrame, WoodTabletPlacement};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GameplayPick, GpuInstance, PointLight, TextLabel};
use crate::ui::focus_nav::{self, FocusDir};
use crate::ui::input::UiAction;

use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::meld_guide::MeldGuideScene;
use super::options::OptionsScene;
use super::profile_select::ProfileSelectScene;
use super::shop::ShopScene;
use super::solitaire::SolitaireScene;
use super::start_game_modal::TileSelectScene;
use super::{BackgroundId, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

// ── Menu items ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuFocus {
    Continue,
    NewGame,
    Solitaire,
    MeldGuide,
    Profile,
    Collection,
    Options,
    Quit,
}

fn menu_items(in_progress: bool) -> Vec<MenuFocus> {
    let mut items = Vec::with_capacity(8);
    if in_progress {
        items.push(MenuFocus::Continue);
    }
    items.push(MenuFocus::NewGame);
    items.push(MenuFocus::Solitaire);
    items.push(MenuFocus::MeldGuide);
    items.push(MenuFocus::Profile);
    items.push(MenuFocus::Collection);
    items.push(MenuFocus::Options);
    items.push(MenuFocus::Quit);
    items
}

fn label_for(item: MenuFocus, in_progress: bool) -> &'static str {
    match item {
        MenuFocus::Continue => "Continue",
        MenuFocus::NewGame => {
            if in_progress {
                "New Game"
            } else {
                "Play"
            }
        }
        MenuFocus::Solitaire => "Solitaire",
        MenuFocus::MeldGuide => "Meld Guide",
        MenuFocus::Profile => "Profile",
        MenuFocus::Collection => "Collection",
        MenuFocus::Options => "Options",
        MenuFocus::Quit => "Quit",
    }
}

fn default_focus(in_progress: bool) -> MenuFocus {
    if in_progress {
        MenuFocus::Continue
    } else {
        MenuFocus::NewGame
    }
}

// ── Scene ───────────────────────────────────────────────────────────────

pub struct StartScreenScene {
    focus: Option<MenuFocus>,
    hover_anims: [f32; 8],
    last_frame: Instant,
    /// Focus rect graph from the previous `draw_frame`, used by `update()`
    /// for spatial navigation (one-frame-stale snapshot pattern).
    last_focus_rects: RefCell<Vec<(MenuFocus, [f32; 4])>>,
    /// Cached cursor position from the most recent `update()` for the
    /// starfield parallax effect in `draw_frame()`.
    cursor_pos: (f32, f32),
}

impl StartScreenScene {
    pub fn new() -> Self {
        Self {
            focus: None,
            hover_anims: [0.0; 8],
            last_frame: Instant::now(),
            last_focus_rects: RefCell::new(Vec::new()),
            cursor_pos: (0.0, 0.0),
        }
    }
}

impl SceneBehavior for StartScreenScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.cursor_pos = ctx.cursor_pos;

        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        let in_progress = ctx.run.is_in_progress();
        let items = menu_items(in_progress);

        // Ensure we have a valid focus.
        if self.focus.is_none() || !items.contains(&self.focus.unwrap()) {
            self.focus = Some(default_focus(in_progress));
        }

        let focus_rects = self.last_focus_rects.borrow().clone();

        // ── Mouse hover via 3D picking ──────────────────────────────────
        if ctx.input_mode == crate::ui::input::InputMode::Cursor {
            if let Some(GameplayPick::WoodTablet(idx)) = ctx.picked_gameplay_object {
                if let Some(&item) = items.get(idx) {
                    self.focus = Some(item);
                }
            }
        }

        // ── Keyboard / gamepad navigation ───────────────────────────────
        let mut activated = false;
        for action in ctx.actions {
            match action {
                UiAction::FocusUp | UiAction::FocusPrev => {
                    if let Some(cur) = self
                        .focus
                        .and_then(|f| focus_rects.iter().find(|(t, _)| *t == f).map(|(_, r)| *r))
                    {
                        if let Some(next) =
                            focus_nav::pick_neighbor(cur, FocusDir::Up, &focus_rects)
                        {
                            self.focus = Some(next);
                        }
                    }
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    if let Some(cur) = self
                        .focus
                        .and_then(|f| focus_rects.iter().find(|(t, _)| *t == f).map(|(_, r)| *r))
                    {
                        if let Some(next) =
                            focus_nav::pick_neighbor(cur, FocusDir::Down, &focus_rects)
                        {
                            self.focus = Some(next);
                        }
                    }
                }
                UiAction::Confirm => activated = true,
                UiAction::Cancel | UiAction::Pause => {
                    *ctx.quit_requested = true;
                }
                _ => {}
            }
        }

        // ── Mouse click on a wood tablet ────────────────────────────────
        if !ctx.button_clicks.is_empty() {
            if let Some(GameplayPick::WoodTablet(idx)) = ctx.picked_gameplay_object {
                if let Some(&item) = items.get(idx) {
                    self.focus = Some(item);
                    activated = true;
                }
            }
        }

        // ── Animate hover envelopes ─────────────────────────────────────
        let speed = 8.0;
        for (i, item) in items.iter().enumerate() {
            if i >= self.hover_anims.len() {
                break;
            }
            let target = if self.focus == Some(*item) { 1.0 } else { 0.0 };
            let anim = &mut self.hover_anims[i];
            *anim += (target - *anim) * (speed * dt).min(1.0);
        }

        // ── Activate focused item ───────────────────────────────────────
        if activated {
            match self.focus {
                Some(MenuFocus::Continue) => return Some(Scene::Gameplay(GameplayScene::new())),
                Some(MenuFocus::NewGame) => {
                    if ctx.tutorial_eligible {
                        return Some(Scene::TileSelect(TileSelectScene::new_tutorial()));
                    }
                    if ctx.multiple_materials {
                        return Some(Scene::TileSelect(TileSelectScene::new()));
                    }
                    // Only one material available — skip tile select.
                    *ctx.run = RunState::new_with_material(TileMaterial::default());
                    return Some(Scene::Shop(ShopScene::new(ctx.run.run_number, ctx.run)));
                }
                Some(MenuFocus::Solitaire) => return Some(Scene::Solitaire(SolitaireScene::new())),
                Some(MenuFocus::MeldGuide) => {
                    return Some(Scene::MeldGuide(MeldGuideScene::new(false)));
                }
                Some(MenuFocus::Profile) => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()));
                }
                Some(MenuFocus::Collection) => {
                    return Some(Scene::Collection(CollectionScene::new()));
                }
                Some(MenuFocus::Options) => return Some(Scene::Options(OptionsScene::new())),
                Some(MenuFocus::Quit) => {
                    *ctx.quit_requested = true;
                }
                None => {}
            }
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let layout = ctx.layout;
        let w = layout.window_w;
        let h = layout.window_h;
        let ui_scale = ctx.ui_scale;
        let scale = metrics::scene_scale(w, h, ui_scale);
        let in_progress = ctx.game_in_progress;

        let items = menu_items(in_progress);

        // Scale the user's reference camera proportionally with window
        // height so the perspective framing stays identical at every
        // resolution.  The reference was tuned at h ≈ 800.
        let cs = h / 800.0;
        let camera = CameraParams {
            eye: [0.0, 290.0 * cs, 220.0 * cs],
            target: [0.0, 0.0, -130.0 * cs],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 45.0,
        };

        // ── Wood tablet layout: centered vertical column ────────────────
        // Everything is h-relative so the 3D layout scales with the
        // camera (both use the same `cs` factor via `h`).
        let tablet_w = h * 0.14;
        let tablet_h = h * 0.045;
        let tablet_depth = h * 0.015;
        let gap = h * 0.012;
        let n = items.len() as f32;
        let total_h = n * tablet_h + (n - 1.0) * gap;
        let center_y = h * 0.38;
        let start_y = center_y - total_h * 0.5;
        let cx = w * 0.5;

        let mut tablets: Vec<WoodTabletPlacement> = Vec::new();
        for (i, &item) in items.iter().enumerate() {
            let ty = start_y + i as f32 * (tablet_h + gap);
            tablets.push(WoodTabletPlacement {
                world_pos: [cx, ty, 0.0],
                extents: [tablet_w, tablet_depth, tablet_h],
                label: label_for(item, in_progress).to_string(),
                pressed: 0.0,
                hover: if i < self.hover_anims.len() {
                    self.hover_anims[i]
                } else {
                    0.0
                },
                disabled: false,
            });
        }

        // ── Candles: two flanking the menu column ───────────────────────
        let candle_h = h * 0.08;
        let candle_offset_x = tablet_w * 0.5 + h * 0.09;
        let candle_y = center_y;

        let candle_placements = vec![
            CandlePlacement {
                world_pos: [cx - candle_offset_x, candle_y, 0.0],
                scale: candle_h,
                flicker: 1.0,
                height_scale: 1.0,
            },
            CandlePlacement {
                world_pos: [cx + candle_offset_x, candle_y, 0.0],
                scale: candle_h,
                flicker: 1.0,
                height_scale: 0.92,
            },
        ];

        // ── Point lights at wick tips ───────────────────────────────────
        let radius_px = h * 0.5;
        let mut point_lights: Vec<PointLight> = Vec::new();
        for candle in &candle_placements {
            let wick_y = WICK_TIP_Y * candle.scale * candle.height_scale;
            point_lights.push(PointLight {
                pos: [candle.world_pos[0], candle.world_pos[1], wick_y],
                radius: radius_px,
                color: [1.0, 0.55, 0.22],
                intensity: 2.0,
            });
        }

        // ── Flame sprites ───────────────────────────────────────────────
        let flame_w = h * 0.025;
        let flame_h = h * 0.042;
        let mut flame_instances: Vec<GpuInstance> = Vec::new();
        for (i, candle) in candle_placements.iter().enumerate() {
            let fx = candle.world_pos[0] - flame_w * 0.5;
            let fy = candle.world_pos[1] - flame_h * 1.2;
            let phase = i as f32 * 0.37;
            flame_instances.push(GpuInstance {
                rect: [fx, fy, flame_w, flame_h],
                color: [0.0, 0.0, 1.0, phase],
            });
        }

        // ── Title plaque ────────────────────────────────────────────────
        let plaque_y = start_y - h * 0.06;
        let plaque_w = h * 0.40;
        let plaque_h = h * 0.1375;
        let plaque_depth = h * 0.012;

        let summaries = persistence::all_profile_summaries();
        let active = ctx.active_profile;
        let summary = &summaries[active];
        let prof_text = if summary.exists {
            format!(
                "Profile {}  —  Level {} ({} runs)",
                active + 1,
                summary.level,
                summary.runs_completed,
            )
        } else {
            format!("Profile {}  —  New", active + 1)
        };

        let plaque = PlaquePlacement {
            center_pos: [cx, plaque_y, plaque_h * 0.5],
            extents: [plaque_w, plaque_h, plaque_depth],
            rotation_y_deg: 0.0,
            top_text: "M A H J U R O".into(),
            bot_text: prof_text,
        };

        // ── Update focus rect graph for next frame's update() ───────────
        let mut focus_rects = Vec::new();
        for (i, &item) in items.iter().enumerate() {
            if let Some(&rect) = ctx.proj.wood_tablet_rects.get(i) {
                focus_rects.push((item, rect));
            }
        }
        *self.last_focus_rects.borrow_mut() = focus_rects;

        // ── Focus ring (2D overlay on the focused tablet's projected rect)
        let mut quads: Vec<GpuInstance> = Vec::new();
        if let Some(focus) = self.focus {
            if let Some(idx) = items.iter().position(|&item| item == focus) {
                if let Some(&rect) = ctx.proj.wood_tablet_rects.get(idx) {
                    focus_nav::push_focus_ring(rect, scale, &mut quads);
                }
            }
        }

        // ── Navigation hint text ────────────────────────────────────────
        let hint_h = (typography::size(typography::MICRO, h, ui_scale) * 1.7).max(16.0);
        let hint_y = h - hint_h - (12.0 * scale);
        let text_labels = vec![TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Arrow keys to navigate  |  Enter/Space to select".into(),
            color: color::SLATE,
            ..Default::default()
        }];

        // ── Catch-all click target (full-screen invisible button) ───────
        let buttons = vec![super::ButtonDef {
            rect: (0.0, 0.0, w, h),
            action: super::ButtonAction::Scene(0),
        }];

        // ── Assemble the frame ──────────────────────────────────────────
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.table();
        frame.candles(candle_placements);
        frame.plaque(plaque);
        frame.wood_tablet_batch(tablets);
        frame.starfield();
        frame.flames(flame_instances);
        frame.fluid_smoke();
        frame.quads(quads);
        frame.texts(text_labels);

        frame.point_lights = point_lights;
        frame.candle_light_count = 2;
        frame.flame_height_world = h * 0.04;
        frame.cursor_pos = Some(self.cursor_pos);
        frame.camera_override = Some(camera);
        frame.buttons = buttons;
        frame.window_title = "Mahjuro".into();

        frame
    }
}
