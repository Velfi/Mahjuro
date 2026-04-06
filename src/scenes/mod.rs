//! Scene system: each screen in the game is a `Scene` variant.
//! Scenes transition by returning `Some(Scene)` from `update()`.

pub mod collection;
pub mod game_over;
pub mod gameplay;
pub mod options;
pub mod pause_menu;
pub mod pick_blind;
pub mod profile_select;
pub mod results;
pub mod shop;
pub mod start_screen;

pub use collection::CollectionScene;
pub use game_over::GameOverScene;
pub use gameplay::GameplayScene;
pub use options::OptionsScene;
pub use pick_blind::PickBlindScene;
pub use profile_select::ProfileSelectScene;
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
    pub layout: &'a LayoutResult,
    /// Current focused hand-slot index (managed by `App`).
    pub focus_tile_index: usize,
    /// Set to `true` to request the application to quit.
    pub quit_requested: &'a mut bool,
    /// Set to switch the active profile (index 0–2).
    pub switch_profile: &'a mut Option<usize>,
    /// Current mouse cursor position in window coordinates.
    pub cursor_pos: (f32, f32),
}

/// Everything a scene's `draw()` may need.
pub struct DrawCtx<'a> {
    pub layout: &'a LayoutResult,
    pub anim: &'a AnimationController,
    pub run: &'a RunState,
    pub focus_tile_index: usize,
    pub progress: &'a crate::core::progression::PlayerProgress,
    pub active_profile: usize,
    /// Whether a game run is currently in progress (for resume/restart UI).
    pub game_in_progress: bool,
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

/// Build GPU elements for a relic display row inside the relic strip.
/// Returns (background quads, text labels, relic icon quads).
pub fn relic_row(
    relics: &RelicState,
    strip: &Rect,
    window_w: f32,
) -> (Vec<GpuInstance>, Vec<TextLabel>, Vec<RelicIcon>) {
    let defs = all_relic_defs();
    let active: Vec<(RelicId, &str)> = relics
        .active
        .iter()
        .filter_map(|id| defs.iter().find(|d| d.id == *id).map(|d| (*id, d.name)))
        .collect();
    let total_slots = relics.max_slots;
    if total_slots == 0 {
        return (vec![], vec![], vec![]);
    }
    let row_h = strip.h;
    let row_y = strip.y;
    // Scale badge width with window size, cap at a reasonable max.
    let scale = window_w / 600.0;
    let badge_w = (window_w / total_slots.max(1) as f32).min(160.0 * scale);
    let total_w = badge_w * total_slots as f32;
    let start_x = (window_w - total_w) * 0.5;

    let mut instances = Vec::new();
    let mut labels = Vec::new();
    let mut icons = Vec::new();
    for i in 0..total_slots {
        let bx = start_x + i as f32 * badge_w;
        let inset = 2.0 * scale;
        let cell_w = badge_w - inset * 2.0;

        if let Some((relic_id, name)) = active.get(i) {
            // Filled slot background.
            instances.push(GpuInstance {
                rect: [bx + inset, row_y, cell_w, row_h],
                color: [0.18, 0.22, 0.35, 0.85],
            });
            // Icon: square, centered horizontally, in the upper portion.
            let icon_size = row_h * 0.65;
            let icon_x = bx + inset + (cell_w - icon_size) * 0.5;
            let icon_y = row_y + row_h * 0.02;
            icons.push(RelicIcon {
                rect: [icon_x, icon_y, icon_size, icon_size],
                relic_id: *relic_id,
            });
            // Name label below the icon.
            let label_h = row_h * 0.28;
            let label_y = icon_y + icon_size + row_h * 0.02;
            labels.push(TextLabel {
                rect: [bx + inset, label_y, cell_w, label_h],
                text: name.to_string(),
                color: [0.8, 0.75, 0.5, 1.0],
            });
        } else {
            // Empty slot: dim outline.
            instances.push(GpuInstance {
                rect: [bx + inset, row_y, cell_w, row_h],
                color: [0.12, 0.14, 0.22, 0.5],
            });
        }
    }
    (instances, labels, icons)
}

/// `None` = stay in current scene; `Some(scene)` = transition.
pub type SceneTransition = Option<Scene>;

/// The active scene. Enum dispatch — no `Box<dyn Trait>`.
pub enum Scene {
    StartScreen(StartScreenScene),
    ProfileSelect(ProfileSelectScene),
    Shop(ShopScene),
    PickBlind(PickBlindScene),
    Gameplay(GameplayScene),
    Results(ResultsScene),
    GameOver(GameOverScene),
    Options(OptionsScene),
    Collection(CollectionScene),
}

impl Scene {
    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        match self {
            Scene::StartScreen(s) => s.update(ctx),
            Scene::ProfileSelect(s) => s.update(ctx),
            Scene::Shop(s) => s.update(ctx),
            Scene::PickBlind(s) => s.update(ctx),
            Scene::Gameplay(s) => s.update(ctx),
            Scene::Results(s) => s.update(ctx),
            Scene::GameOver(s) => s.update(ctx),
            Scene::Options(s) => s.update(ctx),
            Scene::Collection(s) => s.update(ctx),
        }
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        match self {
            Scene::StartScreen(s) => s.draw(ctx),
            Scene::ProfileSelect(s) => s.draw(ctx),
            Scene::Shop(s) => s.draw(ctx),
            Scene::PickBlind(s) => s.draw(ctx),
            Scene::Gameplay(s) => s.draw(ctx),
            Scene::Results(s) => s.draw(ctx),
            Scene::GameOver(s) => s.draw(ctx),
            Scene::Options(s) => s.draw(ctx),
            Scene::Collection(s) => s.draw(ctx),
        }
    }
}
