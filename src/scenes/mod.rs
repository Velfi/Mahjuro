//! Scene system: each screen in the game is a `Scene` variant.
//! Scenes transition by returning `Some(Scene)` from `update()`.

pub mod animation_lab;
pub mod archive;
pub mod archive_career;
pub mod button_aabb_lab;
pub mod cascade_lab;
mod cascade_lab_click;
pub mod celebration_overlay;
pub mod credits;
pub mod debug_visibility;
mod defeat_tableau;
pub(crate) mod flowers_intro_copy;
pub mod gameplay;
pub mod guide;
pub mod hallway;
pub mod journal_transition;
pub mod lamp_moths;
pub mod main_menu;
pub mod material_viewer;
pub(crate) mod melds_intro_copy;
pub mod object3d_inspect;
pub mod options;
pub mod pause_menu;
pub mod profile_select;
pub mod roller_lab;
pub mod rumble_lab;
pub mod run_summary;
mod run_summary_panel;
pub(crate) mod scoring_intro_copy;
pub mod shadow_ao_lab;
pub mod shop;
pub mod showcase;
pub mod showcase_stage;
pub mod splash;
pub mod stairway;
pub mod start_game_modal;
pub mod tile_anchor_lab;
pub(crate) mod tiles_intro_copy;
pub mod tixels;
pub mod transition_playground;
pub mod tutorial_campaign;
pub mod tutorial_summary;
pub mod wall_ledger;
pub mod yaku_journal;
pub use animation_lab::AnimationLabScene;
pub use archive::ArchiveScene;
pub use button_aabb_lab::ButtonAabbLabScene;
pub use cascade_lab::CascadeLabScene;
pub use credits::CreditsScene;
use enum_dispatch::enum_dispatch;
pub use gameplay::GameplayScene;
pub use guide::GuideScene;
pub use hallway::HallwayScene;
pub use main_menu::MainMenuScene;
pub use material_viewer::MaterialViewerScene;
pub use options::OptionsScene;
pub use profile_select::ProfileSelectScene;
pub use roller_lab::RollerLabScene;
pub use rumble_lab::RumbleLabScene;
pub use run_summary::{DefeatScene, RunSummaryScene, VictoryScene};
pub use shadow_ao_lab::ShadowAoLabScene;
pub use shop::ShopScene;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
pub use showcase::MetaLevelUpPresenter;
pub use showcase::{
    ArchiveInspectPresenter, ShopInspectPresenter, ShowcasePresenter, ShowcaseScene,
    TilePackPresenter, ZodiacPresenter,
};
pub use splash::SplashScene;
pub use stairway::StairwayScene;
pub use start_game_modal::TileSelectScene;
pub use tile_anchor_lab::TileAnchorLabScene;
pub use tixels::TixelsScene;
pub use transition_playground::TransitionPlaygroundScene;
pub use tutorial_campaign::TutorialCampaignScene;
pub use tutorial_summary::TutorialSummaryScene;
pub use wall_ledger::WallLedgerScene;
pub use yaku_journal::YakuJournalScene;

use crate::effect_layers::EffectLayers;
use crate::game::cascade::CascadeTuning;
use crate::game::event_bus::EventBus;
use crate::game::run::RunState;
use crate::persistence::ResumeScene;
use crate::render::animation::AnimationController;
use crate::render::draw_cmd::UiFrame;
use crate::render::scene_keys;
use crate::ui::input::{InputMode, RumbleLabOp, UiAction};
use crate::ui::layout::LayoutResult;

pub use debug_visibility::DebugVisibility;

pub use mahjuro_types::BackgroundId;

/// Everything a scene's `update()` may need.
pub struct UpdateCtx<'a> {
    pub actions: &'a [UiAction],
    /// Scene-defined button click ids fired this frame.
    /// Each entry is the `id` of a `ButtonAction::Scene(id)` whose rect was
    /// clicked. Scenes interpret these ids however they like — typically by
    /// matching against named const values local to the scene.
    pub button_clicks: &'a [u32],
    pub progress: &'a crate::core::progression::PlayerProgress,
    /// Active save profile slot (0–2).
    pub active_profile: usize,
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
    /// True while the left mouse button is held (press through release).
    /// Used for continuous gestures such as options-menu slider drags.
    pub mouse_left_down: bool,
    /// For **splash**: `true` once the window has a [`WgpuRenderer`] (hub may still be
    /// decoding façade art / relics in parallel). Other scenes: `true` when boot async
    /// loaders have finished GPU upload (see [`crate::render::wgpu_renderer::WgpuRenderer::is_loading`]).
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
    /// Whether the player should be offered the tutorial (first run, tutorial
    /// not yet completed). Read by the start screen and tile-select scenes.
    pub tutorial_eligible: bool,
    /// Whether the player has unlocked multiple tile materials (i.e., has won
    /// at least once). When false, the tile-select scene is skipped.
    pub multiple_materials: bool,
    /// Scene the app should restore when the player chooses Continue.
    pub resume_scene: ResumeScene,
    /// `true` while a scene transition is pending (fade-out in progress).
    /// Scenes should skip input processing but continue running animations.
    pub transitioning: bool,
    /// Pushdown overlay request: set by a scene's `update()` to push a new
    /// overlay scene on top of the stack, or pop the current overlay back
    /// to the scene beneath. `SceneTransition` continues to mean "replace
    /// the current top of the stack" — overlays are orthogonal to that.
    pub overlay_request: &'a mut Option<OverlayRequest>,
    /// True when running under the headless screenshot harness. Scenes
    /// should fast-forward any wall-clock-gated intro animations (candle
    /// light ramps, smoke curtains, wind gusts) to their final state so
    /// a one-shot capture renders the scene as the player would see it
    /// after settling, not as a dark mid-fade-in.
    pub headless: bool,
    /// Layer toggles (must match [`DrawCtx::effect_layers`] for the same frame).
    pub effect_layers: EffectLayers,
    /// Gamepad right stick while a showcase **orbit** presenter is active (−1..1 each axis).
    pub item_inspect_orbit_stick: (f32, f32),
    /// Showcase orbit zoom: analog triggers (`RT − LT`) plus digital bumpers as ±1.
    pub item_inspect_zoom_triggers: f32,
    /// LMB-drag pixel delta for shop storeroom turntable (consumed each frame by [`ShopScene`]).
    pub shop_storeroom_orbit_drag_px: (f32, f32),
    /// Queued by the rumble lab scene; drained into input state after `update()`.
    pub rumble_lab_ops: &'a mut Vec<RumbleLabOp>,
    /// [`ShopScene`] under shop showcase **inspect** presenter: orbit sync + focus cycling.
    pub suspended_shop: Option<&'a mut ShopScene>,
    /// [`ArchiveScene`] under archive showcase **inspect** presenter: focused artifact cycling.
    pub suspended_collection: Option<&'a mut ArchiveScene>,
    /// Same as [`DrawCtx::room_gltf_height_scale`] — vertical scale for embedded glTF rooms vs window height.
    pub room_gltf_height_scale: f32,
    /// Archive scene sets this to `Some(progress.run_history.len())` when the player
    /// leaves Archive after visiting Chronicle so the app can persist menu hints.
    pub bump_archive_chronicle_seen: &'a mut Option<u32>,
    /// Set when Archive opens on a legacy profile that needs seen-set backfill.
    pub seed_archive_seen: &'a mut bool,
    /// Per-profile chronicle cursor from settings — stable for the Archive visit.
    pub archive_chronicle_last_seen: u32,
    pub main_menu_effects: crate::render::main_menu_effects_tuning::MainMenuEffectsTuning,
    pub flame_tuning: crate::render::flame_tuning::FlameTuning,
    /// Live audio when the main app is ticking; absent in offline bake builds.
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    pub audio: Option<&'a mut crate::audio::AudioManager>,
}

/// Pushdown-stack action a scene's `update()` can request. Scenes do this
/// by writing to [`UpdateCtx::overlay_request`]; the `App` applies the
/// request after `update()` returns.
///
/// - `Push(scene)`: the current scene is suspended (its state preserved);
///   `scene` becomes the active top of stack. Only the top ticks and draws.
/// - `Pop`: the current top is discarded; the scene beneath resumes with
///   its state intact.
pub enum OverlayRequest {
    Push(Box<Scene>),
    Pop,
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
    /// Whether an app-level modal overlay is active (modal queue, debug
    /// overlays, etc). Scenes should suppress hover tooltips when true.
    pub modal_active: bool,
    /// Vertical scale for embedded glTF room scenes (`shop.glb`, `hallway.glb`, `archive.glb`, …):
    /// authored room height is multiplied by `window_h *` this. Debug tuning can override.
    pub room_gltf_height_scale: f32,
    /// Shop punctual + tonemap tuning (debug overlay / defaults from `room_glb` constants).
    pub shop_env_lighting: crate::render::room_glb::RoomEnvLightingTune,
    /// Per-scene room GLB lighting + height (shop, pick_chamber, gameplay, …).
    pub env_per_scene: &'a rustc_hash::FxHashMap<
        &'static str,
        (crate::render::room_glb::RoomEnvLightingTune, f32),
    >,
    /// Master switches for layered visuals — start from [`EffectLayers::BASELINE`]
    /// and enable fields incrementally (see `effect_layers.rs`).
    pub effect_layers: EffectLayers,
    /// Last known cursor position in window pixels (for hover chrome in flat UI scenes).
    pub cursor_pos: (f32, f32),
    /// Active input device — cursor-driven scenes use this with `cursor_pos` for hover rings.
    pub input_mode: InputMode,
    /// Resolves a controller-prompt glyph for a given [`UiAction`] (Kenney
    /// Input Prompts atlases). See [`crate::ui::glyph_source::GlyphResolver`].
    pub glyphs: crate::ui::glyph_source::GlyphResolver,
    /// Frozen [`ShopScene`] under shop showcase inspect — drawn via [`crate::scenes::shop::render_shop_frame`] with orbit dolly.
    pub suspended_shop: Option<&'a ShopScene>,
    /// Suspended collection beneath collection showcase inspect for pedestal orbit.
    pub suspended_collection: Option<&'a ArchiveScene>,
    /// Physical tile proportions for 3D layout (options / renderer settings).
    pub tile_preset: crate::persistence::TilePreset,
    /// True when Archive has unseen catalog entries or chronicle runs (this profile).
    pub archive_has_new: bool,
    /// Settings cursor for chronicle run-log NEW markers.
    pub archive_chronicle_last_seen_run_len: u32,
    /// When set (Debug → Hallway hall FX…), pick-blind hallway uses
    /// [`HallwayDistortionDebugSnapshot::resolve`] (see `hallway_glb.rs`) instead of
    /// [`HallwayDistortion::from_pick_chamber`] alone.
    pub hallway_distortion_debug:
        Option<crate::render::hallway_glb::HallwayDistortionDebugSnapshot>,
    /// Main-menu trailer mode camera override (moon close-up → glTF camera).
    pub main_menu_trailer_camera: Option<crate::render::draw_cmd::CameraParams>,
    /// Splash hub decode progress in `[0, 1]` (main-menu room, shadow, atlases).
    pub loading_hub_progress: f32,
    pub main_menu_effects: crate::render::main_menu_effects_tuning::MainMenuEffectsTuning,
    pub flame_tuning: crate::render::flame_tuning::FlameTuning,
}

impl<'a> DrawCtx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        layout: &'a LayoutResult,
        anim: &'a AnimationController,
        run: &'a RunState,
        progress: &'a crate::core::progression::PlayerProgress,
        active_profile: usize,
        game_in_progress: bool,
        proj: &'a crate::render::wgpu_renderer::ProjectionCache,
        picked_gameplay_object: Option<crate::render::wgpu_renderer::GameplayPick>,
        picked_shop_object: Option<crate::render::wgpu_renderer::ShopHit>,
        debug_visibility: DebugVisibility,
        modal_active: bool,
        room_gltf_height_scale: f32,
        shop_env_lighting: crate::render::room_glb::RoomEnvLightingTune,
        env_per_scene: &'a rustc_hash::FxHashMap<
            &'static str,
            (crate::render::room_glb::RoomEnvLightingTune, f32),
        >,
        effect_layers: EffectLayers,
        cursor_pos: (f32, f32),
        input_mode: InputMode,
        glyphs: crate::ui::glyph_source::GlyphResolver,
        suspended_shop: Option<&'a ShopScene>,
        suspended_collection: Option<&'a ArchiveScene>,
        tile_preset: crate::persistence::TilePreset,
        archive_has_new: bool,
        archive_chronicle_last_seen_run_len: u32,
        hallway_distortion_debug: Option<
            crate::render::hallway_glb::HallwayDistortionDebugSnapshot,
        >,
        main_menu_trailer_camera: Option<crate::render::draw_cmd::CameraParams>,
        loading_hub_progress: f32,
        main_menu_effects: crate::render::main_menu_effects_tuning::MainMenuEffectsTuning,
        flame_tuning: crate::render::flame_tuning::FlameTuning,
    ) -> Self {
        Self {
            layout,
            anim,
            run,
            progress,
            active_profile,
            game_in_progress,
            proj,
            picked_gameplay_object,
            picked_shop_object,
            debug_visibility,
            modal_active,
            room_gltf_height_scale,
            shop_env_lighting,
            env_per_scene,
            effect_layers,
            cursor_pos,
            input_mode,
            glyphs,
            suspended_shop,
            suspended_collection,
            tile_preset,
            archive_has_new,
            archive_chronicle_last_seen_run_len,
            hallway_distortion_debug,
            main_menu_trailer_camera,
            loading_hub_progress,
            main_menu_effects,
            flame_tuning,
        }
    }

    /// Room punctual tuning + glTF height for a specific scene bucket.
    pub fn room_env_for(
        &self,
        scene_key: &'static str,
    ) -> (crate::render::room_glb::RoomEnvLightingTune, f32) {
        *self
            .env_per_scene
            .get(scene_key)
            .unwrap_or(&(self.shop_env_lighting, self.room_gltf_height_scale))
    }
}

pub use mahjuro_types::{ButtonAction, ButtonDef};

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
///   - Large payloads (e.g. [`GameplayScene`]) live in [`Box`] inside the
///     [`Scene`] enum; the blanket [`SceneBehavior`] impl for [`Box`] in this
///     module forwards every method to the inner value. **When you add a
///     method here, add a forward in that impl too** (even if this trait
///     provides a default).
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
    /// `active_buttons` safety wipe in `main.rs` consults
    /// `App::modal_overlay_active`, so a scene that forgets to declare its
    /// overlay will leak hover/clicks through it.
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

    /// Map logical face buttons to semantic [`UiAction`]s for the active scene.
    ///
    /// The main loop skips this when an overlay stack entry or blocking in-scene
    /// modal is up. Default: no face bindings.
    fn face_button_bindings(
        &self,
        _ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        crate::ui::input::FaceButtonBindings::default()
    }
}

/// Transparent adapter so large scene payloads can live in [`Box`] without a
/// per-scene hand-maintained forward impl.
///
/// **When adding a method to [`SceneBehavior`], forward it here too** — even
/// if the trait supplies a default. Otherwise boxed variants (notably
/// [`Scene::Gameplay`]) silently inherit the default instead of delegating to
/// the inner scene.
impl<T: SceneBehavior + ?Sized> SceneBehavior for Box<T> {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        (**self).update(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        (**self).draw_frame(ctx)
    }

    fn has_blocking_overlay(&self) -> bool {
        (**self).has_blocking_overlay()
    }

    fn pause_options_overlay(&self) -> Option<&OptionsScene> {
        (**self).pause_options_overlay()
    }

    fn face_button_bindings(
        &self,
        ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        (**self).face_button_bindings(ctx)
    }
}

/// The active scene. Enum dispatch — no `Box<dyn Trait>`.
///
/// `enum_dispatch` reads this attribute and generates `impl SceneBehavior
/// for Scene` arms automatically; see [`SceneBehavior`] for the contract.
#[enum_dispatch(SceneBehavior)]
#[allow(clippy::large_enum_variant)]
pub enum Scene {
    Splash(SplashScene),
    MainMenu(MainMenuScene),
    TileSelect(TileSelectScene),
    ProfileSelect(ProfileSelectScene),
    Shop(ShopScene),
    Showcase(ShowcaseScene),
    Hallway(HallwayScene),
    Stairway(StairwayScene),
    /// Boxed to keep the enum small; [`SceneBehavior`] for [`Box`] forwards to the inner scene.
    Gameplay(Box<GameplayScene>),
    Victory(VictoryScene),
    Defeat(DefeatScene),
    Guide(GuideScene),
    MaterialViewer(MaterialViewerScene),
    TileAnchorLab(TileAnchorLabScene),
    ButtonAabbLab(ButtonAabbLabScene),
    Tixels(TixelsScene),
    Options(OptionsScene),
    Credits(CreditsScene),
    Archive(ArchiveScene),
    TutorialCampaign(TutorialCampaignScene),
    TutorialSummary(TutorialSummaryScene),
    TransitionPlayground(TransitionPlaygroundScene),
    AnimationLab(AnimationLabScene),
    RumbleLab(RumbleLabScene),
    RollerLab(RollerLabScene),
    CascadeLab(Box<CascadeLabScene>),
    ShadowAoLab(ShadowAoLabScene),
    YakuJournal(YakuJournalScene),
    WallLedger(WallLedgerScene),
}

/// Canonical renderer scene-key string for tonemap, shadows, and pick prefixes.
pub fn active_scene_key(scene: &Scene) -> Option<&'static str> {
    match scene {
        Scene::Showcase(_) => Some("showcase"),
        Scene::Shop(_) => Some(scene_keys::SHOP),
        Scene::Gameplay(_) => Some(scene_keys::GAMEPLAY),
        Scene::Archive(_) => Some(scene_keys::ARCHIVE),
        Scene::Hallway(_) => Some(scene_keys::HALLWAY),
        Scene::Stairway(_) => Some(scene_keys::STAIRWAY),
        Scene::MainMenu(_) => Some(scene_keys::MAIN_MENU),
        Scene::Options(_) => Some(scene_keys::OPTIONS),
        Scene::Victory(_) => Some(scene_keys::VICTORY),
        Scene::Defeat(_) => Some(scene_keys::DEFEAT),
        Scene::TutorialCampaign(_) => Some("tutorial"),
        Scene::Guide(_) => Some("guide"),
        Scene::YakuJournal(_) => Some("yaku_journal"),
        Scene::WallLedger(_) => Some("wall_ledger"),
        Scene::TileAnchorLab(_) => Some("tile_anchor_lab"),
        Scene::ButtonAabbLab(_) => Some("button_aabb_lab"),
        Scene::AnimationLab(_) => Some(scene_keys::SHOP),
        Scene::RollerLab(_) => Some(scene_keys::GAMEPLAY),
        Scene::CascadeLab(_) => Some(scene_keys::GAMEPLAY),
        Scene::ShadowAoLab(_) => Some(scene_keys::SHADOW_AO_LAB),
        Scene::Tixels(_) => Some("tixels"),
        Scene::TileSelect(_) => Some("tile_select"),
        _ => None,
    }
}

/// Scene directly under `top` on the overlay stack (the suspended host when `top` is an overlay).
pub fn overlay_renderer_parent<'a>(
    base: &'a Scene,
    overlay_stack: &'a [Scene],
) -> Option<&'a Scene> {
    if overlay_stack.is_empty() {
        None
    } else if overlay_stack.len() >= 2 {
        Some(&overlay_stack[overlay_stack.len() - 2])
    } else {
        Some(base)
    }
}

/// Renderer scene key for the frame being drawn. Item inspect overlays
/// ([`ShowcasePresenter::ShopInspect`], [`ShowcasePresenter::ArchiveInspect`]) inherit
/// tonemap, punctual layout, and catalog balance from the suspended parent scene.
pub fn active_scene_key_for_renderer(top: &Scene, parent: Option<&Scene>) -> Option<&'static str> {
    if let Scene::Showcase(s) = top
        && matches!(
            s.presenter,
            ShowcasePresenter::ShopInspect(_) | ShowcasePresenter::ArchiveInspect(_)
        )
        && let Some(key) = parent.and_then(active_scene_key)
    {
        return Some(key);
    }
    active_scene_key(top)
}

/// Return from profile picker to the Archive (collection) without a `profile_select` ↔ `collection` cycle.
pub(crate) fn scene_archive() -> Scene {
    Scene::Archive(ArchiveScene::new())
}

#[cfg(test)]
mod scene_behavior_tests {
    use super::*;
    use crate::ui::input::{FaceBindingCtx, UiAction};

    #[test]
    fn boxed_gameplay_forwards_face_button_bindings_through_scene_enum() {
        let scene = Scene::Gameplay(Box::default());
        let bindings = scene.face_button_bindings(FaceBindingCtx {
            xy_quick_action: true,
        });
        assert_eq!(
            bindings.west_press,
            Some(UiAction::WestFacePress),
            "boxed Gameplay must delegate face bindings, not use the trait default"
        );
        assert_eq!(bindings.north_press, Some(UiAction::NorthFacePress));
    }
}
