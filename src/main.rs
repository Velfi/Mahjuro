//! Mahjuro — UI-first shell: winit + wgpu + cassowary + input + scene system.

// Release builds on Windows: detach from the console so launching the .exe
// doesn't pop a black terminal behind the game window. Debug builds keep the
// console so `log::info!` output is visible during development.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

pub mod asset_path;
mod audio;
mod bot;
mod core;
pub mod crash_guard;
mod debug_menu;
mod debug_overlays;
mod effect_layers;
mod game;
#[cfg(target_os = "macos")]
mod macos_updater;
#[path = "main/arrange.rs"]
mod main_arrange;
#[path = "main/bot_graph.rs"]
mod main_bot_graph;
#[path = "main/cli.rs"]
mod main_cli;
#[path = "main/commands.rs"]
mod main_commands;
#[path = "main/debug_actions.rs"]
mod main_debug_actions;
#[path = "main/debug_state.rs"]
mod main_debug_state;
#[path = "main/draw.rs"]
mod main_draw;
#[path = "main/event_loop.rs"]
mod main_event_loop;
#[path = "main/headless.rs"]
mod main_headless;
#[path = "main/render_settings.rs"]
mod main_render_settings;
mod persistence;
mod render;
mod scenes;
mod steam;
mod ui;
mod update_check;

use std::sync::Arc;
use std::time::Instant;

use clap::{ArgAction, Args, Parser, Subcommand};
use debug_menu::DebugAction;
#[cfg(debug_menu_enabled)]
use debug_menu::DebugMenuBar;
use debug_overlays::{
    CameraDebugOverlay, DebugVisResult, DebugVisibilityOverlay, SfxTestOverlay, TuningOverlay,
    TuningResult, VolumetricDebugOverlay, VolumetricDebugResult,
};
use game::cascade::CascadeTuning;
use game::event_bus::{EventBus, GameEvent};
use game::run::RunState;
use game::volumetric_tuning::VolumetricTuning;
use render::animation::AnimationController;
use render::draw_cmd::{CameraParams, UiFrame};
use render::wgpu_renderer::{DebugArrangeOverride, GpuInstance, ShopHit, TextLabel, WgpuRenderer};
use scenes::game_over::GameOverScene;
use scenes::gameplay::GameplayScene;
use scenes::material_viewer::MaterialViewerScene;
use scenes::rumble_lab::RumbleLabScene;
use scenes::shop::SHOP_DRAG_DROP_ID;
use scenes::splash::SplashScene;
use scenes::transition_playground::TransitionPlaygroundScene;
use scenes::tutorial_recap::TutorialRecapScene;
use scenes::tutorial_summary::TutorialSummaryScene;
use scenes::{ButtonAction, ButtonDef, DrawCtx, Scene, SceneBehavior, UpdateCtx};
use serde::{Deserialize, Serialize};
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
use ui::modal::{Modal, ModalQueue, ModalTheme, UnlockPage};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use main_arrange::{
    ArrangeInput, apply_arrange_to_layout, arrange_hierarchy_flat, collect_committed_rotations,
    reset_arrange_to_default, sample_arrange_placement,
};
use main_cli::Cli;
use main_debug_state::{ArrangeModeState, DebugState};
use main_render_settings::RenderSettings;

// Debug overlays (visibility toggles, cascade tuning, SFX test, camera
// params) live in `debug_overlays.rs`.  See `mod debug_overlays` above.
// `DebugState`, `ArrangeModeState`, and `RenderSettings` live in
// `main/debug_state.rs` and `main/render_settings.rs`.

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransitionKind {
    /// Default fast fade (~0.2 s).
    Quick,
    /// Dramatic shooting-star cascade (~1.7 s total before extra score steps).
    ShootingStarCascade,
    ForestOfTiles,
    GalaxyOfTiles,
    Maelstrom,
    TileWaterfall,
    ShufflingFan,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<WgpuRenderer>,
    layout_engine: UiLayout,
    input: Option<InputState>,
    run: RunState,
    bus: EventBus,
    anim: AnimationController,
    last_frame: Instant,
    last_frame_dt: f32,
    mouse_actions: Vec<UiAction>,
    mouse_button_clicks: Vec<u32>,
    /// True on the frame a left mouse button press landed. Consumed by
    /// overlays that need raw click detection (e.g. the SFX test board).
    mouse_clicked: bool,
    /// True while the left mouse button is held (for slider drag).
    mouse_left_down: bool,
    scroll_delta: f32,
    active_buttons: Vec<ButtonDef>,
    scene: Scene,
    resume_scene: persistence::ResumeScene,
    progress: crate::core::progression::PlayerProgress,
    active_profile: usize,
    audio: audio::AudioManager,
    transition_alpha: f32,
    transition_speed: f32,
    transition_timer: f32,
    transition_kind: TransitionKind,
    pending_scene: Option<Scene>,
    /// Captured when `pending_scene` was set: true if the transition was
    /// initiated by an overlay (replace overlay top at fade-end), false if
    /// by the base scene (replace `self.scene`). Decoupling this from the
    /// live stack depth prevents overlays pushed mid-transition (e.g. a
    /// zodiac celebration during a skip flow) from being clobbered.
    pending_scene_targets_overlay: bool,
    /// Pushdown stack for overlay scenes (e.g. zodiac/pack celebrations,
    /// meld guide from in-game).
    /// When non-empty, the top of the stack is the active scene: only it
    /// ticks and draws. `self.scene` is the root; overlays stack above.
    overlay_stack: Vec<Scene>,
    quit_requested: bool,
    close_saved: bool,
    modals: ModalQueue,
    pending_post_game_over_modals: Vec<Modal>,
    gfx: RenderSettings,
    effect_layers: crate::effect_layers::EffectLayers,
    debug: DebugState,
    cascade_tuning: CascadeTuning,
    volumetric_tuning: VolumetricTuning,
    deferred_round_end: Option<GameEvent>,
    /// `None` when the Steam build is hosting us — Steam handles its own
    /// update pipeline and self-replacing the binary fights the Steam
    /// installer. Otherwise `Some` and polled every frame.
    update_checker: Option<update_check::UpdateChecker>,
    /// `Some(version)` while the "update available" prompt modal is on
    /// screen. Confirming the modal triggers the install; cancelling
    /// clears this.
    pending_update_prompt: Option<String>,
    /// macOS-only: Sparkle's updater controller. Holds the framework alive
    /// for the lifetime of the app and keeps Sparkle's background scheduler
    /// running. `None` on dev builds where `Sparkle.framework` isn't
    /// embedded yet (the legacy `update_checker` then takes over) or when
    /// Steam is hosting us (Steam owns updates).
    #[cfg(target_os = "macos")]
    sparkle: Option<macos_updater::SparkleUpdater>,
    modifiers: ModifiersState,
    /// Shop drag-to-sell: the 3D hit that started a mouse-drag over an owned
    /// shop item, plus the cursor position at drag start.  Set on mouse-down;
    /// if the cursor moves far enough and is over the sell tray on mouse-up,
    /// a `SHOP_DRAG_DROP_ID` click is injected.
    shop_drag_start: Option<(crate::render::wgpu_renderer::ShopHit, (f32, f32))>,
    /// Steamworks integration. Either `Connected` (initialized successfully
    /// and the user is signed into Steam) or `Disabled` (init failed,
    /// Steam isn't running, or `--no-steam` was passed). Every method on
    /// `SteamClient` is safe to call in either state — `Disabled` is a
    /// logged no-op — so no `Option` wrapping is needed at call sites.
    steam: steam::SteamClient,
}

impl App {
    fn saved_resume_scene_for(scene: &Scene) -> Option<persistence::ResumeScene> {
        match scene {
            Scene::Shop(_) => Some(persistence::ResumeScene::Shop),
            Scene::PickBlind(_) => Some(persistence::ResumeScene::PickBlind),
            Scene::Gameplay(_) => Some(persistence::ResumeScene::Gameplay),
            _ => None,
        }
    }

    /// Single source of truth for "is anything modal-like up right now?"
    ///
    /// **The modal-blocking pattern.** Any overlay that should block input
    /// and hover for elements below it is reported here, by ORing together:
    ///   - The app-owned [`ModalQueue`] (toast modals).
    ///   - App-owned debug overlays (`tuning_overlay`, `sfx_test_overlay`).
    ///   - The active scene's own internal overlays, via
    ///     [`Scene::has_blocking_overlay`].
    ///
    /// The main loop also consults this for the **click safety wipe**:
    /// right after the scene populates `active_buttons`, those buttons are
    /// cleared if any modal is up, so scene buttons can never be clicked
    /// through. Overlays that *want* their own clickable surface (e.g.
    /// `ModalQueue`'s full-screen dismiss) write to `active_buttons`
    /// *after* the wipe in their own draw step.
    ///
    /// To make a new overlay modal-blocking by default:
    ///   - If it's app-owned: add it to this OR-chain.
    ///   - If it's scene-owned: report it from the scene's
    ///     `has_blocking_overlay()` method.
    ///
    /// No per-call-site changes are needed — the gates pick it up
    /// automatically.
    fn modal_overlay_active(&self) -> bool {
        self.modals.is_active()
            || self.debug.any_overlay_active()
            || self.scene.has_blocking_overlay()
            || self.overlay_stack.iter().any(|s| s.has_blocking_overlay())
            || !self.overlay_stack.is_empty()
    }

    /// Gameplay-only hover hit-test for the on-table relic row. Uses the
    /// renderer's projected relic rects, so it matches the visible 3D boxes
    /// the same way gameplay focus and tooltips already do.
    fn gameplay_relic_slot_at_cursor(&self, cursor: (f32, f32)) -> Option<usize> {
        if self.modal_overlay_active() || !matches!(self.scene, Scene::Gameplay(_)) {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        renderer
            .projections()
            .relic_rects
            .iter()
            .enumerate()
            .find_map(|(i, rect)| {
                let [x, y, w, h] = *rect;
                (w > 1.0
                    && h > 1.0
                    && x.is_finite()
                    && y.is_finite()
                    && cursor.0 >= x
                    && cursor.0 <= x + w
                    && cursor.1 >= y
                    && cursor.1 <= y + h)
                    .then_some(i)
            })
    }

    fn new(steam: steam::SteamClient) -> Self {
        let t0 = Instant::now();
        let settings = persistence::load_settings();
        let active_profile = settings.active_profile;
        let progress = persistence::load_profile(active_profile);
        // Prefer a saved-on-quit run for this profile (resume). If none
        // exists or it was written by a previous build version, fall back
        // to a fresh demo run. `load_run` deletes stale/corrupt saves.
        let loaded_run = persistence::load_run(active_profile);
        let resume_scene = loaded_run
            .as_ref()
            .map(|saved| saved.scene)
            .unwrap_or(persistence::ResumeScene::Gameplay);
        let mut run = loaded_run
            .map(|saved| saved.run)
            .unwrap_or_else(RunState::new_demo);
        run.set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        run.set_hints_enabled(settings.hints_enabled);
        run.apply_progression(&progress);
        let mut audio = audio::AudioManager::new();
        audio.set_master_volume(settings.master_volume);
        audio.set_sfx_volume(settings.sfx_volume);
        audio.set_music_volume(settings.music_volume);
        if !settings.sfx_enabled {
            audio.set_enabled(false);
        }
        log::info!("App::new() settings + profile loaded in {:?}", t0.elapsed());
        // Steam owns updates for Steam-installed builds: self-replacing the
        // binary fights the Steam content system. Detect Steam two ways —
        // the SDK signal (strongest: Steam *is* hosting us right now) and
        // the env-var signal (catches "launched outside Steam from a Steam
        // library" and offline-mode init failures).
        let is_steam_build = steam.is_connected() || steam::launched_via_steam();
        if is_steam_build {
            log::info!("Steam-hosted build detected; skipping in-app updaters");
        }
        Self {
            window: None,
            renderer: None,
            layout_engine: UiLayout::new(),
            input: None,
            run,
            bus: EventBus::default(),
            anim: AnimationController::new(),
            last_frame: Instant::now(),
            last_frame_dt: 1.0 / 60.0,
            mouse_actions: Vec::new(),
            mouse_button_clicks: Vec::new(),
            mouse_clicked: false,
            mouse_left_down: false,
            scroll_delta: 0.0,
            active_buttons: Vec::new(),
            scene: Scene::Splash(SplashScene::new()),
            resume_scene,
            progress,
            active_profile,
            audio,
            transition_alpha: 1.0,
            transition_speed: 0.08,
            transition_timer: 0.0,
            transition_kind: TransitionKind::Quick,
            pending_scene: None,
            pending_scene_targets_overlay: false,
            overlay_stack: Vec::new(),
            quit_requested: false,
            close_saved: false,
            modals: ModalQueue::default(),
            pending_post_game_over_modals: Vec::new(),
            deferred_round_end: None,
            gfx: RenderSettings {
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                surface_kind: settings.surface_kind,
                tileset_name: settings.tileset_name.clone(),
                gamma: settings.gamma,
                shadows_enabled: settings.shadows_enabled,
                ssr_enabled: settings.ssr_enabled,
                hdr_enabled: settings.hdr_enabled,
                ui_scale: settings.ui_scale,
            },
            // Default: cheap baseline; see `effect_layers.rs`. Use `FULL` or flip
            // flags to restore shadows, SSR, particles, transition FX, HDR, etc.
            effect_layers: crate::effect_layers::EffectLayers::BASELINE,
            debug: DebugState::new(),
            cascade_tuning: CascadeTuning::default(),
            volumetric_tuning: persistence::load_tuning_override::<VolumetricTuning>(
                "VolumetricTuning",
            ),
            update_checker: (!is_steam_build).then(update_check::UpdateChecker::spawn),
            pending_update_prompt: None,
            #[cfg(target_os = "macos")]
            sparkle: (!is_steam_build)
                .then(macos_updater::SparkleUpdater::start)
                .flatten(),
            modifiers: ModifiersState::default(),
            shop_drag_start: None::<(ShopHit, (f32, f32))>,
            steam,
        }
    }

    fn toggle_fullscreen(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = if window.fullscreen().is_some() {
            None
        } else {
            Some(Fullscreen::Borderless(None))
        };
        window.set_fullscreen(fullscreen);
    }

    fn wants_fullscreen_shortcut(&self, event: &KeyEvent) -> bool {
        if event.repeat || event.state != ElementState::Pressed {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };

        #[cfg(target_os = "windows")]
        {
            let no_extra_modifiers = !self.modifiers.control_key()
                && !self.modifiers.shift_key()
                && !self.modifiers.super_key();
            self.modifiers.alt_key()
                && no_extra_modifiers
                && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter)
        }

        #[cfg(target_os = "macos")]
        {
            // `fn` is generally handled below the app layer on macOS and is
            // rarely surfaced by winit, so the practical signal we can bind
            // is the bare `F` keypress that macOS emits for the standard
            // fullscreen shortcut on Apple keyboards.
            self.modifiers.is_empty() && code == KeyCode::KeyF
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = code;
            false
        }
    }

    /// Switch to a different profile, reloading progress.
    fn switch_profile(&mut self, new_index: usize) {
        // Save current profile + any in-progress run before swapping out.
        let _ = persistence::save_profile(self.active_profile, &self.progress);
        self.persist_run_if_in_progress();
        self.active_profile = new_index;
        self.progress = persistence::load_profile(new_index);
        // Resume the new profile's saved run if it has one — otherwise a
        // fresh demo run, exactly like first-launch behavior.
        let loaded_run = persistence::load_run(new_index);
        self.resume_scene = loaded_run
            .as_ref()
            .map(|saved| saved.scene)
            .unwrap_or(persistence::ResumeScene::Gameplay);
        self.run = loaded_run
            .map(|saved| saved.run)
            .unwrap_or_else(RunState::new_demo);
        let mut settings = persistence::load_settings();
        self.run
            .set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        self.run.set_hints_enabled(settings.hints_enabled);
        self.run.apply_progression(&self.progress);
        // Persist the active profile choice.
        settings.active_profile = new_index;
        let _ = persistence::save_settings(&settings);
    }

    /// Persist `self.run` for resume on next launch. Called from every quit
    /// path so the player can resume regardless of how the game was closed.
    /// If the run is fresh (default starting state — e.g. the player started
    /// a new game then quit immediately), the saved-run file is deleted
    /// instead of overwritten. Otherwise the existing save would still
    /// linger and we'd resume into a stale run on next launch.
    fn persist_run_if_in_progress(&self) {
        if self.run.is_in_progress() {
            let scene = Self::saved_resume_scene_for(&self.scene).unwrap_or(self.resume_scene);
            if let Err(e) = persistence::save_run(self.active_profile, &self.run, scene) {
                log::warn!("save_run failed: {e}");
            }
        } else {
            persistence::delete_saved_run(self.active_profile);
        }
    }
}

fn main() -> anyhow::Result<()> {
    crash_guard::install();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    if main_commands::run_cli_command(cli.command)? {
        return Ok(());
    }

    asset_path::log_all_assets();

    let no_steam = cli.no_steam;
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
            // Steam init runs before the window so the SDK can hook the
            // rendering surface as it's created. `--no-steam` short-circuits
            // this for dev runs where you don't want the overlay attaching
            // or Steam claiming the foreground process slot. Init failures
            // (Steam not running, no license, etc.) are logged and we fall
            // back to `Disabled` — the game then runs normally without
            // achievements/overlay.
            let steam = if no_steam {
                log::info!("--no-steam: skipping Steamworks init");
                steam::SteamClient::disabled()
            } else if !steam::steamworks_dll_ready() {
                log::warn!(
                    "steam_api64.dll was not found next to this executable (or failed to load); \
                     Steam achievements and overlay are disabled this session",
                );
                steam::SteamClient::disabled()
            } else {
                steam::SteamClient::init()
            };

            let event_loop = EventLoop::new()?;
            event_loop.set_control_flow(ControlFlow::Poll);

            let mut app = App::new(steam);
            event_loop.run_app(&mut app)?;
            Ok(())
        }));

    match result {
        Ok(inner) => inner,
        Err(_) => {
            crash_guard::show_crash_report();
            std::process::exit(1);
        }
    }
}
