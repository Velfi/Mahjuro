//! Scene system: each screen in the game is a `Scene` variant.
//! Scenes transition by returning `Some(Scene)` from `update()`.

pub mod game_over;
pub mod gameplay;
pub mod pick_blind;
pub mod results;
pub mod shop;
pub mod start_screen;

pub use game_over::GameOverScene;
pub use gameplay::GameplayScene;
pub use pick_blind::PickBlindScene;
pub use results::ResultsScene;
pub use shop::ShopScene;
pub use start_screen::StartScreenScene;

use crate::core::tile::Tile;
use crate::game::event_bus::EventBus;
use crate::game::run::RunState;
use crate::render::animation::AnimationController;
use crate::render::wgpu_renderer::{GpuInstance, RelicIcon, TextLabel};
use crate::ui::input::UiAction;
use crate::core::relic::{RelicId, RelicState, all_relic_defs};
use crate::ui::layout::{LayoutResult, Rect};

/// Everything a scene's `update()` may need.
pub struct UpdateCtx<'a> {
    pub actions: &'a [UiAction],
    pub run: &'a mut RunState,
    pub bus: &'a mut EventBus,
    pub anim: &'a mut AnimationController,
    /// Current focused hand-slot index (managed by `App`).
    pub focus_tile_index: usize,
}

/// Everything a scene's `draw()` may need.
pub struct DrawCtx<'a> {
    pub layout: &'a LayoutResult,
    pub anim: &'a AnimationController,
    pub run: &'a RunState,
    pub focus_tile_index: usize,
}

/// A clickable UI button: screen rect + the action it triggers.
pub struct ButtonDef {
    pub rect: (f32, f32, f32, f32),
    pub action: UiAction,
}

/// What a scene returns from `draw()`.
pub struct SceneDrawOutput {
    pub instances: Vec<GpuInstance>,
    /// Tiles to render as 3D meshes in the hand strip (empty = no hand tiles).
    pub hand_tiles: Vec<Tile>,
    /// Screen-space `(x, y, w, h)` rects for each hand tile; parallel with `hand_tiles`.
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    /// Index of the focused tile within `hand_tiles`.
    pub focus: usize,
    /// Which hand tiles are selected for discard (parallel with `hand_tiles`).
    pub selected_tiles: Vec<bool>,
    /// Text labels drawn on top of UI panels (score, relics, relic choice names, etc.).
    pub text_labels: Vec<TextLabel>,
    /// Relic icons drawn as textured quads.
    pub relic_icons: Vec<RelicIcon>,
    /// Clickable buttons overlaid on the scene.
    pub buttons: Vec<ButtonDef>,
    pub window_title: String,
}

/// Build GPU elements for a relic display row below the score panel.
/// Returns (background quads, text labels, relic icon quads).
pub fn relic_row(
    relics: &RelicState,
    panel: &Rect,
    window_w: f32,
) -> (Vec<GpuInstance>, Vec<TextLabel>, Vec<RelicIcon>) {
    let defs = all_relic_defs();
    let active: Vec<(RelicId, &str)> = relics
        .active
        .iter()
        .filter_map(|id| defs.iter().find(|d| d.id == *id).map(|d| (*id, d.name)))
        .collect();
    if active.is_empty() {
        return (vec![], vec![], vec![]);
    }
    // Place relic badges in a row below the score panel.
    let row_h = panel.h * 0.7;
    let row_y = panel.y + panel.h;
    let badge_w = (window_w / active.len().max(1) as f32).min(120.0);
    let total_w = badge_w * active.len() as f32;
    let start_x = (window_w - total_w) * 0.5;

    let mut instances = Vec::new();
    let mut labels = Vec::new();
    let mut icons = Vec::new();
    for (i, (relic_id, name)) in active.iter().enumerate() {
        let bx = start_x + i as f32 * badge_w;
        let inset = 2.0;
        let cell_w = badge_w - inset * 2.0;
        // Background quad.
        instances.push(GpuInstance {
            rect: [bx + inset, row_y, cell_w, row_h],
            color: [0.18, 0.22, 0.35, 0.85],
        });
        // Icon: square, centered horizontally in the cell, in the upper portion.
        let icon_size = row_h * 0.6;
        let icon_x = bx + inset + (cell_w - icon_size) * 0.5;
        let icon_y = row_y + row_h * 0.02;
        icons.push(RelicIcon {
            rect: [icon_x, icon_y, icon_size, icon_size],
            relic_id: *relic_id,
        });
        // Name label below the icon (smaller area).
        let label_h = row_h * 0.32;
        let label_y = icon_y + icon_size + row_h * 0.02;
        labels.push(TextLabel {
            rect: [bx + inset, label_y, cell_w, label_h],
            text: name.to_string(),
            color: [0.8, 0.75, 0.5, 1.0],
        });
    }
    (instances, labels, icons)
}

/// `None` = stay in current scene; `Some(scene)` = transition.
pub type SceneTransition = Option<Scene>;

/// The active scene. Enum dispatch — no `Box<dyn Trait>`.
pub enum Scene {
    StartScreen(StartScreenScene),
    Shop(ShopScene),
    PickBlind(PickBlindScene),
    Gameplay(GameplayScene),
    Results(ResultsScene),
    GameOver(GameOverScene),
}

impl Scene {
    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        match self {
            Scene::StartScreen(s) => s.update(ctx),
            Scene::Shop(s) => s.update(ctx),
            Scene::PickBlind(s) => s.update(ctx),
            Scene::Gameplay(s) => s.update(ctx),
            Scene::Results(s) => s.update(ctx),
            Scene::GameOver(s) => s.update(ctx),
        }
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        match self {
            Scene::StartScreen(s) => s.draw(ctx),
            Scene::Shop(s) => s.draw(ctx),
            Scene::PickBlind(s) => s.draw(ctx),
            Scene::Gameplay(s) => s.draw(ctx),
            Scene::Results(s) => s.draw(ctx),
            Scene::GameOver(s) => s.draw(ctx),
        }
    }
}
