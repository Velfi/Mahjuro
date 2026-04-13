//! Scene system: each screen in the game is a `Scene` variant.
//! Scenes transition by returning `Some(Scene)` from `update()`.

pub mod collection;
pub mod game_over;
pub mod gameplay;
pub mod journal;
pub mod meld_guide;
pub mod options;
pub mod pause_menu;
pub mod pick_blind;
pub mod profile_select;
pub mod shop;
pub mod solitaire;
pub mod splash;
pub mod start_game_modal;
pub mod start_screen;
pub mod tile_literacy;
pub mod tutorial_campaign;
pub mod tutorial_overlay;
pub mod tutorial_recap;
pub mod tutorial_summary;

pub use collection::CollectionScene;
pub use game_over::GameOverScene;
pub use gameplay::GameplayScene;
pub use meld_guide::MeldGuideScene;
pub use options::OptionsScene;
pub use pick_blind::PickBlindScene;
pub use profile_select::ProfileSelectScene;
pub use shop::ShopScene;
pub use solitaire::SolitaireScene;
pub use splash::SplashScene;
pub use start_game_modal::TileSelectScene;
pub use start_screen::StartScreenScene;
pub use tile_literacy::TileLiteracyScene;
pub use tutorial_campaign::TutorialCampaignScene;
pub use tutorial_recap::TutorialRecapScene;
pub use tutorial_summary::TutorialSummaryScene;

use enum_dispatch::enum_dispatch;

use crate::core::relic::{RelicId, RelicState};
use crate::game::cascade::CascadeTuning;
use crate::game::event_bus::EventBus;
use crate::game::run::RunState;
use crate::render::animation::AnimationController;
use crate::render::draw_cmd::UiFrame;
use crate::render::wgpu_renderer::GpuInstance;
use crate::ui::input::{InputMode, UiAction};
use crate::ui::layout::{LayoutResult, Rect};

/// Per-element visibility flags driven by the debug visibility modal.
/// Plumbed through `DrawCtx` so scenes can skip pushing draw cmds at the
/// call site (necessary for elements that share a `DrawCmd` variant — e.g.
/// the blind plaque vs. the scoring placard, both of which are
/// `DrawCmd::Plaque(_)` and so can't be told apart by a post-process filter).
#[derive(Clone, Copy, Debug, Default)]
pub struct DebugVisibility {
    pub hide_candles: bool,
    pub hide_blind_plaque: bool,
    #[allow(dead_code)]
    pub hide_scoring_placard: bool,
}

/// Which background image to display behind the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BackgroundId {
    /// No background image — just the clear color.
    #[default]
    None,
    /// Solid black fill. Synthesised in the renderer (1×1 black texture)
    /// rather than loaded from disk so scenes that need a true-black backdrop
    /// behind the volumetric smoke composite have a pass-A draw to bind to.
    /// Quad-based fills get reordered into the late HUD overlay pass and
    /// would paint over the smoke instead.
    Black,
    /// Main menu: scattered tiles on dark wood.
    Menu,
    /// Score/results: golden radiant center burst.
    Score,
}

impl BackgroundId {
    /// Asset path relative to the `assets/` root (embedded via rust-embed).
    pub fn asset_path(self) -> Option<&'static str> {
        match self {
            BackgroundId::None => None,
            BackgroundId::Black => None,
            BackgroundId::Menu => Some("backgrounds/menu_bg.png"),
            BackgroundId::Score => Some("backgrounds/score_bg.png"),
        }
    }
}

/// Everything a scene's `update()` may need.
pub struct UpdateCtx<'a> {
    pub actions: &'a [UiAction],
    /// Scene-defined button click ids fired this frame.
    /// Each entry is the `id` of a `ButtonAction::Scene(id)` whose rect was
    /// clicked. Scenes interpret these ids however they like — typically by
    /// matching against named const values local to the scene.
    pub button_clicks: &'a [u32],
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
    /// Set to delete a profile slot (index 0–2). Handled after scene update.
    pub delete_profile: &'a mut Option<usize>,
    /// Set to mark the onboarding tutorial as completed immediately.
    pub complete_onboarding: &'a mut bool,
    /// Current mouse cursor position in window coordinates.
    pub cursor_pos: (f32, f32),
    /// `true` once all background asset loading has completed.
    pub loading_done: bool,
    /// Cascade animation timing parameters.
    pub cascade_tuning: &'a CascadeTuning,
    /// Result of `pick_shop_object` for the cursor this frame, when the
    /// shop scene is active. Used by the shop's update() to route mouse
    /// clicks to 3D objects (relics, ribbons, dishes).
    pub picked_shop_object: Option<crate::render::wgpu_renderer::ShopHit>,
    /// Result of `pick_gameplay_object` for the cursor this frame, when
    /// the gameplay scene is active. Used by the gameplay scene's
    /// update() to route mouse clicks to 3D action objects (sort/play
    /// wood tablets and the discard bowl).
    pub picked_gameplay_object: Option<crate::render::wgpu_renderer::GameplayPick>,
    /// Which input device the player most recently used. Mirrors
    /// `DrawCtx::input_mode` so scenes can route directional actions
    /// vs. cursor activity in `update()` without re-deriving it.
    pub input_mode: InputMode,
    /// Index of the hand tile under the cursor as determined by raycasting
    /// from the camera through the cursor against each tile's OBB. Mirrors
    /// `DrawCtx::picked_hand_tile` so scenes can sync cursor → focus during
    /// `update()` without going back to the renderer.
    pub picked_hand_tile: Option<usize>,
    /// Accumulated scroll-wheel delta this frame in line units.
    /// Negative = scroll up (content moves down), positive = scroll down.
    pub scroll_lines: f32,
    /// User's UI scale preference (1.0 = default). Thread through to
    /// `typography::size()` and `metrics::scene_scale()`.
    pub ui_scale: f32,
    /// Whether the player should be offered the tutorial (first run, tutorial
    /// not yet completed). Read by the start screen and tile-select scenes.
    pub tutorial_eligible: bool,
    /// Whether the player has unlocked multiple tile materials (i.e., has won
    /// at least once). When false, the tile-select scene is skipped.
    pub multiple_materials: bool,
    /// `true` while a scene transition is pending (fade-out in progress).
    /// Scenes should skip input processing but continue running animations.
    pub transitioning: bool,
}

/// Everything a scene's `draw()` may need.
pub struct DrawCtx<'a> {
    pub layout: &'a LayoutResult,
    pub anim: &'a AnimationController,
    pub run: &'a RunState,
    pub progress: &'a crate::core::progression::PlayerProgress,
    pub active_profile: usize,
    /// Whether a game run is currently in progress (for resume/restart UI).
    pub game_in_progress: bool,
    /// All per-frame projected screen-space rects from the renderer.
    pub proj: &'a crate::render::wgpu_renderer::ProjectionCache,
    /// Result of `pick_gameplay_object` — the topmost yaku tablet, wood
    /// action tablet, or discard bowl the cursor is over this frame, if
    /// any.
    pub picked_gameplay_object: Option<crate::render::wgpu_renderer::GameplayPick>,
    /// Result of `pick_shop_object` — the topmost shop object the cursor is
    /// over this frame, if any.
    pub picked_shop_object: Option<crate::render::wgpu_renderer::ShopHit>,
    /// Debug visibility toggles set from the in-game debug visibility modal.
    pub debug_visibility: DebugVisibility,
    /// User's UI scale preference (1.0 = default).
    pub ui_scale: f32,
    /// Whether an app-level modal overlay is active (modal queue, debug
    /// overlays, etc). Scenes should suppress hover tooltips when true.
    pub modal_active: bool,
}

/// What happens when a `ButtonDef` is clicked.
///
/// `Ui(action)` enqueues `action` into the next frame's action queue, just
/// like a key/gamepad press — use this when the click is semantically the
/// same as some keyboard input the scene already handles (e.g. a "Next"
/// button that does the same thing as Enter).
///
/// `Scene(id)` enqueues `id` into `UpdateCtx::button_clicks`, where the
/// scene's `update()` can match against its own named const values. Use this
/// when N buttons each need to do something different and there is no
/// natural keyboard equivalent — it avoids hijacking unrelated `UiAction`
/// variants. See AGENTS.md "Mouse Input in Scenes".
#[derive(Clone, Copy, Debug)]
pub enum ButtonAction {
    Ui(UiAction),
    Scene(u32),
}

/// A clickable UI button: screen rect + the action it triggers.
pub struct ButtonDef {
    pub rect: (f32, f32, f32, f32),
    pub action: ButtonAction,
}

impl ButtonDef {
    /// Convenience constructor for the common `Ui(action)` case.
    pub fn ui(rect: (f32, f32, f32, f32), action: UiAction) -> Self {
        Self {
            rect,
            action: ButtonAction::Ui(action),
        }
    }

    /// Convenience constructor for scene-defined click ids.
    pub fn scene(rect: (f32, f32, f32, f32), id: u32) -> Self {
        Self {
            rect,
            action: ButtonAction::Scene(id),
        }
    }
}


/// Screen rect of relic badge slot `slot_idx` inside the relic strip.
/// Single source of truth for badge layout — used by `relic_row` and by
/// scenes that need to hit-test or highlight a specific badge.
pub fn relic_badge_rect(
    strip: &Rect,
    window_w: f32,
    max_slots: usize,
    slot_idx: usize,
    ui_scale: f32,
) -> (f32, f32, f32, f32) {
    let scale = window_w / 600.0 * ui_scale;
    let badge_w = (window_w / max_slots.max(1) as f32).min(160.0 * scale);
    let total_w = badge_w * max_slots as f32;
    let start_x = (window_w - total_w) * 0.5;
    let inset = 2.0 * scale;
    let bx = start_x + slot_idx as f32 * badge_w;
    (bx + inset, strip.y, badge_w - inset * 2.0, strip.h)
}

/// Build glow overlay quads for relics that recently activated during a
/// scoring cascade. Draws *behind* the relic row's icon so the existing badge
/// background remains the dominant color, with the glow blooming around it.
///
/// `glow_starts` maps each glowing relic id to the `Instant` it last fired.
/// The glow fades over `lifetime`, after which the entry should be evicted by
/// the caller. Returns the additive overlay quads.
///
/// The gameplay scene uses its own 3D-projected glow path (the relics live
/// on a brass dish, not in this 2D row), so this helper is here for future
/// scenes that *do* render relics via `relic_row` (shop, results).
#[allow(dead_code)]
pub fn relic_glow_overlays(
    relics: &RelicState,
    glow_starts: &std::collections::HashMap<RelicId, std::time::Instant>,
    strip: &Rect,
    window_w: f32,
    now: std::time::Instant,
    lifetime: std::time::Duration,
    ui_scale: f32,
) -> Vec<GpuInstance> {
    if glow_starts.is_empty() {
        return Vec::new();
    }
    let total_slots = relics.max_slots;
    if total_slots == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let lifetime_s = lifetime.as_secs_f32().max(0.001);
    for (slot_idx, id) in relics.active.iter().enumerate() {
        let Some(start) = glow_starts.get(id) else {
            continue;
        };
        let age = now.saturating_duration_since(*start).as_secs_f32();
        if age >= lifetime_s {
            continue;
        }
        // Quadratic falloff: bright at start, soft tail.
        let t = (1.0 - age / lifetime_s).clamp(0.0, 1.0);
        let alpha = (t * t * 0.85).clamp(0.0, 0.85);
        let (rx, ry, rw, rh) = relic_badge_rect(strip, window_w, total_slots, slot_idx, ui_scale);
        // Bloom rect: slightly larger than the badge so the glow appears to
        // spill outward. Use the gold accent so it reads as "this fired".
        let pad = (rh * 0.18).max(2.0);
        out.push(GpuInstance {
            rect: [rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0],
            color: crate::render::theme::color::alpha(
                crate::render::theme::color::CHAMPAGNE,
                alpha,
            ),
        });
    }
    out
}

/// `None` = stay in current scene; `Some(scene)` = transition.
pub type SceneTransition = Option<Scene>;

/// Behavior shared by every scene variant.
///
/// `enum_dispatch` generates the dispatch arms on `Scene` automatically: each
/// trait method becomes a `match self { Scene::X(s) => s.method(...), ... }`
/// at compile time, so calling `scene.update(ctx)` on the enum forwards to
/// the inner type with zero overhead and no `Box<dyn Trait>` indirection.
///
/// **Why a trait instead of hand-rolled match dispatch:**
///   - Adding a method here = define it once. The compiler enforces that
///     every variant implements it (or inherits a default), so there is no
///     hand-maintained dispatch table to forget.
///   - Default methods like [`Self::has_blocking_overlay`] mean scenes that
///     don't care just inherit the safe answer — no per-variant `_ => false`
///     enumeration.
///   - Adding a new scene = add it to the [`Scene`] enum and `impl
///     SceneBehavior for NewScene { ... }`. No other files change.
#[enum_dispatch]
pub trait SceneBehavior {
    /// Advance scene state by one frame. Returns `Some(next)` to transition.
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition;

    /// Build the canonical `UiFrame` for this scene's frame.
    ///
    /// Returns a single ordered `UiFrame.cmds` list where push order *is*
    /// z-order, so quads and text labels interleave correctly.
    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame;

    /// Whether the scene has a *modal-like internal overlay* currently up
    /// (pause menu, glossary, embedded options sub-screen, scoring cascade,
    /// etc). This is the scene's contribution to `App::modal_overlay_active`,
    /// which centralizes the "is anything blocking input below it" question.
    ///
    /// **Pattern contract:** any new in-scene overlay that should block
    /// input and hover for elements below it MUST be reported here. The
    /// two universal gates (`skip_tooltips` and the `active_buttons` safety
    /// wipe in `main.rs`) consult `App::modal_overlay_active`, so a scene
    /// that forgets to declare its overlay will leak hover/clicks through
    /// it.
    ///
    /// Default: `false`. Scenes without internal overlays inherit the
    /// safe answer automatically.
    fn has_blocking_overlay(&self) -> bool {
        false
    }

    /// Borrow the in-pause-menu options overlay if the player has opened
    /// it. Used by the main loop to sync live audio/graphics settings the
    /// same way it does for the standalone `OptionsScene`.
    ///
    /// Default: `None`. Scenes without an embedded pause menu inherit it.
    fn pause_options_overlay(&self) -> Option<&OptionsScene> {
        None
    }
}

/// The active scene. Enum dispatch — no `Box<dyn Trait>`.
///
/// `enum_dispatch` reads this attribute and generates `impl SceneBehavior
/// for Scene` arms automatically; see [`SceneBehavior`] for the contract.
#[enum_dispatch(SceneBehavior)]
pub enum Scene {
    Splash(SplashScene),
    StartScreen(StartScreenScene),
    TileSelect(TileSelectScene),
    ProfileSelect(ProfileSelectScene),
    Shop(ShopScene),
    PickBlind(PickBlindScene),
    Gameplay(GameplayScene),
    GameOver(GameOverScene),
    MeldGuide(MeldGuideScene),
    Options(OptionsScene),
    Collection(CollectionScene),
    Solitaire(SolitaireScene),
    TutorialRecap(TutorialRecapScene),
    TutorialCampaign(TutorialCampaignScene),
    TutorialSummary(TutorialSummaryScene),
    TileLiteracy(TileLiteracyScene),
}
