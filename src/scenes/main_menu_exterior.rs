//! Waterfront façade backdrop (`assets/backgrounds/main_menu_exterior.png`) with a flat hub menu.
//! Replaces the legacy candlelit start screen (`start_screen.rs`, removed).

use std::cell::RefCell;

use crate::audio::SfxId;
use crate::core::progression::PlayerProgress;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::game::run::RunState;
use crate::persistence::{self, ResumeScene, TileMaterial};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::focus_nav::{self, FocusDir};
use crate::ui::input::UiAction;

use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::options::OptionsScene;
use super::profile_select::ProfileSelectScene;
use super::shop::ShopScene;
use super::start_game_modal::TileSelectScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HubFocus {
    Continue,
    NewGame,
    Profile,
    Collection,
    Options,
    Quit,
}

fn menu_items(in_progress: bool) -> Vec<HubFocus> {
    let mut items = Vec::with_capacity(8);
    if in_progress {
        items.push(HubFocus::Continue);
    }
    items.push(HubFocus::NewGame);
    items.push(HubFocus::Profile);
    items.push(HubFocus::Collection);
    items.push(HubFocus::Options);
    items.push(HubFocus::Quit);
    items
}

fn label_for(item: HubFocus, in_progress: bool) -> &'static str {
    match item {
        HubFocus::Continue => "Continue",
        HubFocus::NewGame => {
            if in_progress {
                "New Game"
            } else {
                "Play"
            }
        }
        HubFocus::Profile => "Profile",
        HubFocus::Collection => "Collection",
        HubFocus::Options => "Options",
        HubFocus::Quit => "Quit",
    }
}

fn default_focus(in_progress: bool) -> HubFocus {
    if in_progress {
        HubFocus::Continue
    } else {
        HubFocus::NewGame
    }
}

pub(crate) fn scene_from_resume(
    resume_scene: ResumeScene,
    run: &mut RunState,
    progress: &PlayerProgress,
) -> Scene {
    match resume_scene {
        ResumeScene::Gameplay => Scene::Gameplay(GameplayScene::new()),
        ResumeScene::Shop => {
            if GameEngine::resumes_to_tutorial_shop(run) {
                Scene::Shop(ShopScene::new_tutorial(run))
            } else {
                Scene::Shop(ShopScene::new(run, progress))
            }
        }
        ResumeScene::PickBlind => Scene::PickBlind(super::pick_blind::PickBlindScene::new()),
    }
}

pub struct MainMenuExteriorScene {
    focus: Option<HubFocus>,
    last_focus_rects: RefCell<Vec<(HubFocus, [f32; 4])>>,
    cursor_pos: (f32, f32),
    pub positions: crate::ui::scene_layout::MainMenuExteriorPositions,
}

impl MainMenuExteriorScene {
    pub fn new() -> Self {
        Self {
            focus: None,
            last_focus_rects: RefCell::new(Vec::new()),
            cursor_pos: (0.0, 0.0),
            positions: crate::ui::scene_layout::load_main_menu_exterior_positions(),
        }
    }

    fn hub_layout_rects(w: f32, h: f32, items: &[HubFocus]) -> Vec<(HubFocus, [f32; 4])> {
        let row_h = (h * 0.046).max(28.0);
        let gap = h * 0.014;
        let n = items.len() as f32;
        let stack_h = n * row_h + (n - 1.0).max(0.0) * gap;
        let mut y = (h - stack_h) * 0.5;
        let rw = (w * 0.52).min(520.0);
        let x0 = (w - rw) * 0.5;
        items
            .iter()
            .copied()
            .map(|item| {
                let r = [x0, y, rw, row_h];
                y += row_h + gap;
                (item, r)
            })
            .collect()
    }
}

impl SceneBehavior for MainMenuExteriorScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.cursor_pos = ctx.cursor_pos;

        let in_progress = GameEngine::run_in_progress(ctx.run);
        let items = menu_items(in_progress);

        if self.focus.is_none() || !items.contains(&self.focus.unwrap()) {
            self.focus = Some(default_focus(in_progress));
        }
        let prev_focus = self.focus;

        let focus_rects = self.last_focus_rects.borrow().clone();

        let pointer_pick =
            if ctx.input_mode == crate::ui::input::InputMode::Cursor && !focus_rects.is_empty() {
                focus_nav::focus_target_at_cursor(&focus_rects, ctx.cursor_pos.0, ctx.cursor_pos.1)
            } else {
                None
            };

        if ctx.input_mode == crate::ui::input::InputMode::Cursor {
            if let Some(m) = pointer_pick {
                self.focus = Some(m);
            }
        }

        let mut activated = false;
        for action in ctx.actions {
            match action {
                UiAction::FocusUp | UiAction::FocusPrev => {
                    if let Some(cur) = self.focus {
                        if let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == cur) {
                            if let Some(next) =
                                focus_nav::pick_neighbor(rect, FocusDir::Up, &focus_rects)
                            {
                                self.focus = Some(next);
                            }
                        }
                    }
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    if let Some(cur) = self.focus {
                        if let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == cur) {
                            if let Some(next) =
                                focus_nav::pick_neighbor(rect, FocusDir::Down, &focus_rects)
                            {
                                self.focus = Some(next);
                            }
                        }
                    }
                }
                UiAction::Confirm => activated = true,
                UiAction::Cancel | UiAction::Pause => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    self.focus = Some(HubFocus::Quit);
                }
                _ => {}
            }
        }

        if !ctx.button_clicks.is_empty() {
            if let Some(m) = pointer_pick {
                self.focus = Some(m);
                activated = true;
            }
        }

        if self.focus != prev_focus {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        if activated {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            match self.focus {
                Some(HubFocus::Continue) => {
                    return Some(scene_from_resume(ctx.resume_scene, ctx.run, ctx.progress));
                }
                Some(HubFocus::NewGame) => {
                    if ctx.tutorial_eligible {
                        return Some(Scene::TileSelect(TileSelectScene::new_tutorial()));
                    }
                    if ctx.multiple_materials {
                        return Some(Scene::TileSelect(TileSelectScene::new()));
                    }
                    let settings = persistence::load_settings();
                    GameEngine::start_run_with_material(
                        ctx.run,
                        TileMaterial::default(),
                        ctx.progress,
                        &settings,
                    );
                    return Some(Scene::Shop(ShopScene::new(ctx.run, ctx.progress)));
                }
                Some(HubFocus::Profile) => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()));
                }
                Some(HubFocus::Collection) => {
                    return Some(Scene::Collection(CollectionScene::new()));
                }
                Some(HubFocus::Options) => {
                    return Some(Scene::Options(OptionsScene::new()));
                }
                Some(HubFocus::Quit) => {
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

        // App modals (level-up / relic unlock) append 3D hero staging after the
        // scene, but all `Text` cmds share one post-tonemap overlay pass. Hub
        // labels would composite with modal copy and read on top of the relic.
        // Treat this like shop/collection inspect: backdrop only until the modal
        // dismisses (see `SplashScene` and `DrawCtx::modal_active`).
        if ctx.modal_active {
            let mut frame = UiFrame::new();
            frame.background(BackgroundId::MainMenuExterior);
            if ctx.effect_layers.starfield {
                frame.starfield();
            }
            frame.cursor_pos = Some(self.cursor_pos);
            frame.window_title = format!(
                "Mahjuro — {}",
                if cfg!(debug_assertions) {
                    "vNEXT"
                } else {
                    env!("CARGO_PKG_VERSION")
                }
            );
            return frame;
        }

        let in_progress = ctx.game_in_progress;
        let items = menu_items(in_progress);
        let focus_rects = Self::hub_layout_rects(w, h, &items);
        *self.last_focus_rects.borrow_mut() = focus_rects.clone();

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

        let profile_h = (typography::size(typography::CAPTION, h, ui_scale) * 1.45).max(15.0);
        let profile_y = focus_rects
            .first()
            .map(|(_, r)| r[1] - profile_h - h * 0.022)
            .unwrap_or(h * 0.12)
            .max(h * 0.06);

        let mut quads: Vec<GpuInstance> = Vec::new();
        if let Some(focus) = self.focus
            && let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == focus)
        {
            focus_nav::push_focus_ring(rect, scale, w, h, &mut quads);
        }

        let menu_font = typography::size(typography::HEADING, h, ui_scale).max(20.0);
        let label_color = color::PARCHMENT;
        let mut text_labels = vec![
            TextLabel {
                rect: [0.0, profile_y, w, profile_h],
                text: prof_text,
                color: color::UMBER,
                align: TextAlign::Center,
                ..Default::default()
            },
            TextLabel {
                rect: [0.0, h - (menu_font * 2.2).max(36.0), w, menu_font * 1.5],
                text: "Arrow keys to navigate  |  Enter/Space to select".into(),
                color: color::UMBER,
                align: TextAlign::Center,
                ..Default::default()
            },
        ];

        for &(item, rect) in &focus_rects {
            text_labels.push(TextLabel {
                rect,
                text: label_for(item, in_progress).into(),
                font_px: Some(menu_font),
                color: label_color,
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        let buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), 0)];

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::MainMenuExterior);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        frame.quads(quads);
        frame.texts(text_labels);
        frame.buttons = buttons;
        frame.cursor_pos = Some(self.cursor_pos);
        frame.window_title = format!(
            "Mahjuro — {}",
            if cfg!(debug_assertions) {
                "vNEXT"
            } else {
                env!("CARGO_PKG_VERSION")
            }
        );

        frame
    }
}
