//! Start screen — title screen with main menu.

use crate::game::run::RunState;
use crate::persistence;
use crate::render::theme::{ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::widget_tree::{
    self as wt, FocusId, Tree, TreeFrame, TreeInput, TreeState, noop_render_custom,
};

use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::options::OptionsScene;
use super::profile_select::ProfileSelectScene;
use super::shop::ShopScene;
use super::solitaire::SolitaireScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainAction {
    Continue,
    NewGame,
    Solitaire,
    Profile,
    Collection,
    Options,
    Quit,
}

impl MainAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub struct StartScreenScene {
    tree: TreeState,
}

impl StartScreenScene {
    pub fn new() -> Self {
        Self {
            tree: TreeState::new(),
        }
    }

    /// Anchor rect for the menu column. Pushes the menu below the title +
    /// active-profile summary so they never overlap on short windows, where
    /// the default vertically-centered anchor would otherwise put the topmost
    /// button right under the profile line.
    fn menu_anchor(w: f32, h: f32) -> [f32; 4] {
        let scale = (w.min(h)) / 600.0;
        // Mirror the header math in `draw` so the anchor tracks the real
        // bottom of the title + profile block.
        let title_h = (typography::size(typography::DISPLAY, h) * 1.6).max(48.0);
        let title_y = h * 0.08;
        let prof_h = (typography::size(typography::CAPTION, h) * 1.6).max(20.0);
        let header_bottom = title_y + title_h + h * 0.02 + prof_h;
        // Reserve room at the bottom for the hint line.
        let hint_h = (typography::size(typography::MICRO, h) * 1.7).max(16.0);
        let bottom_reserve = hint_h + 24.0 * scale;
        let cw = (260.0 * scale).min(w * 0.7);
        let cx = (w - cw) * 0.5;
        let cy = header_bottom + h * 0.02;
        let ch = (h - cy - bottom_reserve).max(0.0);
        [cx, cy, cw, ch]
    }

    fn build_tree(&self, in_progress: bool) -> Tree<MainAction> {
        let mut items: Vec<wt::Node<MainAction>> = Vec::new();
        if in_progress {
            items.push(wt::button_id(
                MainAction::Continue.id(),
                "Continue",
                MainAction::Continue,
                ButtonVariant::Primary,
            ));
            items.push(wt::button_id(
                MainAction::NewGame.id(),
                "New Game",
                MainAction::NewGame,
                ButtonVariant::Default,
            ));
        } else {
            items.push(wt::button_id(
                MainAction::NewGame.id(),
                "Play",
                MainAction::NewGame,
                ButtonVariant::Primary,
            ));
        }
        items.push(wt::button_id(
            MainAction::Solitaire.id(),
            "Solitaire",
            MainAction::Solitaire,
            ButtonVariant::Default,
        ));
        items.push(wt::button_id(
            MainAction::Profile.id(),
            "Profile",
            MainAction::Profile,
            ButtonVariant::Default,
        ));
        items.push(wt::button_id(
            MainAction::Collection.id(),
            "Collection",
            MainAction::Collection,
            ButtonVariant::Default,
        ));
        items.push(wt::button_id(
            MainAction::Options.id(),
            "Options",
            MainAction::Options,
            ButtonVariant::Default,
        ));
        items.push(wt::button_id(
            MainAction::Quit.id(),
            "Quit",
            MainAction::Quit,
            ButtonVariant::Danger,
        ));
        Tree::vertical_menu(items)
    }

    fn build_anchored_tree(&self, in_progress: bool, w: f32, h: f32) -> Tree<MainAction> {
        self.build_tree(in_progress)
            .with_anchor(Self::menu_anchor(w, h))
    }

    fn start_game(&self, run: &mut RunState) -> SceneTransition {
        *run = RunState::new_demo();
        Some(Scene::Shop(ShopScene::new(run.run_number, &run.relics)))
    }
}

impl SceneBehavior for StartScreenScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let in_progress = ctx.run.is_in_progress();
        let tree = self.build_anchored_tree(in_progress, ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update(
            &tree,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
            },
        );

        // Esc / cancel quits.
        for a in ctx.actions {
            if matches!(
                a,
                crate::ui::input::UiAction::Cancel | crate::ui::input::UiAction::Pause
            ) {
                *ctx.quit_requested = true;
            }
        }

        match action {
            Some(MainAction::Continue) => Some(Scene::Gameplay(GameplayScene::new())),
            Some(MainAction::NewGame) => self.start_game(ctx.run),
            Some(MainAction::Solitaire) => Some(Scene::Solitaire(SolitaireScene::new())),
            Some(MainAction::Profile) => {
                Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()))
            }
            Some(MainAction::Collection) => Some(Scene::Collection(CollectionScene::new())),
            Some(MainAction::Options) => Some(Scene::Options(OptionsScene::new())),
            Some(MainAction::Quit) => {
                *ctx.quit_requested = true;
                None
            }
            None => None,
        }
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let in_progress = ctx.game_in_progress;

        // The dark backdrop is drawn via `BackgroundId::Black` (a synthetic
        // 1×1 texture in the renderer) rather than a fullscreen quad. The
        // renderer reorders quad ops into a HUD overlay pass that runs
        // *after* the smoke composite, so a fullscreen quad here would paint
        // over the smoke. Background ops run in pass A, before the smoke
        // composite, which is what we want.
        let mut instances: Vec<GpuInstance> = Vec::new();
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title — gold serif display. The rect height is bumped above the
        // raw `typography::size(...)` so the rasterizer's 0.55-of-rect-height
        // font sizing lands near the nominal tier size instead of well below.
        let title_h = (typography::size(typography::DISPLAY, h) * 1.6).max(48.0);
        let title_y = h * 0.08;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "M A H J U R O".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Active profile summary below title.
        let prof_y = title_y + title_h + h * 0.02;
        let prof_h = (typography::size(typography::CAPTION, h) * 1.6).max(20.0);
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
        text_labels.push(TextLabel {
            rect: [0.0, prof_y, w, prof_h],
            text: prof_text,
            color: color::MIST,
            ..Default::default()
        });

        // Render the menu via the widget tree (single source of truth for
        // layout, hit-test, hover, focus, click registration).
        let tree = self.build_anchored_tree(in_progress, w, h);
        let mut frame = TreeFrame {
            instances: &mut instances,
            labels: &mut text_labels,
            buttons: &mut buttons,
            window: (w, h),
        };
        self.tree.draw(&tree, &mut frame, &noop_render_custom);

        // Hint text at bottom.
        let hint_h = (typography::size(typography::MICRO, h) * 1.7).max(16.0);
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Arrow keys to navigate  |  Enter/Space to select".into(),
            color: color::SLATE,
            ..Default::default()
        });

        SceneDrawOutput {
            background: super::BackgroundId::Black,
            tray_instances: vec![],
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: "Mahjuro".into(),
            departing_indices: vec![],
            hint_indices: vec![],
            flame_instances: vec![],
            // Same warm overhead key light the shop uses — front-and-above,
            // depth-aware so the smoke pools under it.
            point_lights: vec![PointLight {
                pos: [w * 0.5, h * 0.05, h * 0.55],
                radius: h * 1.20,
                color: [1.00, 0.86, 0.55],
                intensity: 1.40,
            }],
            candles: vec![],
            relic_placements: vec![],
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}
