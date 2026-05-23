//! Mahjuro — UI-first shell: SDL3 + wgpu + cassowary + input + scene system.
//!
//! The executable entry point is [`run`]; this library exists so benchmarks
//! (`cargo bench`) can link the game logic.

#![deny(unused_imports)]

pub mod asset_path;
mod asset_sources;
mod audio;
pub mod bot;
pub mod core;
pub mod crash_guard;
mod debug_menu;
mod debug_overlays;
mod effect_layers;
pub mod game;
#[cfg(target_os = "macos")]
mod macos_fullscreen_shortcut;
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
#[path = "main/frame_tick.rs"]
mod main_frame_tick;
#[path = "main/headless.rs"]
mod main_headless;
#[path = "main/perf_watchdog.rs"]
mod main_perf_watchdog;
#[path = "main/render_settings.rs"]
mod main_render_settings;
#[path = "main/room_gltf_brownout.rs"]
mod main_room_gltf_brownout;
mod persistence;
mod physical_size;
mod render;
#[path = "main/scene_transition.rs"]
mod scene_transition;
mod scenes;
mod sdl_shell;
mod startup_profile;
mod steam;
mod ui;

use std::time::Instant;

use clap::{ArgAction, Args, Parser, Subcommand};
use debug_menu::DebugAction;
#[cfg(debug_menu_enabled)]
use debug_menu::DebugMenuBar;
use debug_overlays::{
    CameraDebugOverlay, DebugVisResult, DebugVisibilityOverlay, SfxTestOverlay, TuningOverlay,
    TuningResult,
};
use game::cascade::CascadeTuning;
use game::event_bus::{EventBus, GameEvent};
use game::run::RunState;
use game::scene_look_tuning::SceneLookTuningSet;
use render::animation::AnimationController;
use render::draw_cmd::{CameraParams, UiFrame, apply_modal_relic_staging};
use render::wgpu_renderer::{GpuInstance, TextLabel, WgpuRenderer};
use scenes::game_over::GameOverScene;
use scenes::gameplay::GameplayScene;
use scenes::material_viewer::MaterialViewerScene;
use scenes::rumble_lab::RumbleLabScene;
use scenes::splash::SplashScene;
use scenes::transition_playground::TransitionPlaygroundScene;
use scenes::tutorial_summary::TutorialSummaryScene;
use scenes::{ButtonAction, ButtonDef, DrawCtx, Scene, SceneBehavior, UpdateCtx};
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
use ui::modal::{Modal, ModalQueue, ModalTheme, UnlockPage};

use crate::physical_size::PhysicalSize;
use sdl3::keyboard::{Mod, Scancode};

use main_cli::Cli;
use main_debug_state::DebugState;
use main_render_settings::RenderSettings;

// Debug overlays (visibility toggles, cascade tuning, SFX test, camera
// params) live in `debug_overlays.rs`.  See `mod debug_overlays` above.
// `DebugState` and `RenderSettings` live in
// `main/debug_state.rs` and `main/render_settings.rs`.

use scene_transition::{DEFAULT_QUICK_SPEC, PendingSceneDestination, TransitionKind};

struct App {
    /// Last known drawable size in pixels (updated each SDL frame).
    last_drawable_px: PhysicalSize,
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
    /// Captured when `pending_scene` was set: where the next scene is
    /// written when the fade completes. Decoupled from live stack depth so
    /// overlays pushed mid-transition are not clobbered.
    pending_scene_destination: PendingSceneDestination,
    /// Pushdown stack for overlay scenes (e.g. zodiac/pack celebrations,
    /// guide from in-game).
    /// When non-empty, the top of the stack is the active scene: only it
    /// ticks and draws. `self.scene` is the root; overlays stack above.
    overlay_stack: Vec<Scene>,
    /// Snapshot of "any controller currently connected?" from the previous
    /// frame, used to detect the falling edge (controller unplugged) so we
    /// can auto-pause while the player is in [`InputMode::Controller`].
    prev_controller_present: bool,
    quit_requested: bool,
    close_saved: bool,
    modals: ModalQueue,
    /// Meta profile level-up after game over — stinger + [`Scene::Showcase`]
    /// (`MetaLevelUpPresenter`) once the main menu fade-in finishes.
    pending_post_game_over_level_up: Option<Modal>,
    gfx: RenderSettings,
    effect_layers: crate::effect_layers::EffectLayers,
    debug: DebugState,
    cascade_tuning: CascadeTuning,
    /// Per-scene tonemap + room GLB look, resolved each frame from
    /// `active_scene_key`. Loaded from `tuning_overrides.json` on startup;
    /// the debug overlay edits this in place and writes back via
    /// [`persistence`].
    scene_look: SceneLookTuningSet,
    deferred_round_end: Option<GameEvent>,
    modifiers: Mod,
    /// Steamworks integration. Either `Connected` (initialized successfully
    /// and the user is signed into Steam) or `Disabled` (init failed,
    /// Steam isn't running, or `--no-steam` was passed). Every method on
    /// `SteamClient` is safe to call in either state — `Disabled` is a
    /// logged no-op — so no `Option` wrapping is needed at call sites.
    steam: steam::SteamClient,
    /// Mirrors `AppSettings::archive_last_seen_run_len` for menu hints without disk reads.
    archive_last_seen_run_len: [u32; 3],
    /// Optional CPU-side frame timer; sibling of the renderer's `GpuProfiler`.
    /// Started on demand from the Debug menu via `Profile (CPU + GPU)`.
    cpu_profiler: render::cpu_profiler::CpuProfiler,
    /// Background saver for `progress`. Per-frame event handlers set
    /// `profile_dirty` instead of doing a synchronous write; the
    /// frame-end flush hands a snapshot to the saver thread. Quit
    /// paths still call `save_profile` directly so a forced exit
    /// can't drop pending state.
    profile_saver: persistence::ProfileSaver,
    /// Set by `mark_profile_dirty` whenever an event mutates `progress`.
    /// Cleared after the frame-end flush enqueues a snapshot.
    profile_dirty: bool,
    /// Last `frame.window_title` we pushed to the SDL window. Cached so
    /// we only call `set_title` when it actually changes — Wayland/X11
    /// charges a syscall for each call.
    last_window_title: String,
    /// Rare flicker + brownout on shop / pick_blind / collection room lighting.
    room_gltf_brownout: main_room_gltf_brownout::RoomGltfBrownout,
    /// Frame-scoped scene-pick cache. Computed once at the top of
    /// `frame_tick` and reused by both `frame_tick` (for `update`) and
    /// `draw` (for `DrawCtx::new`). Without this, every gameplay frame
    /// pays for ~4 separate ray-vs-AABB walks against the full per-class
    /// matrix lists. Per-event mouse-motion picks in
    /// [`crate::main::event_loop`] are intentionally not cached — they
    /// run with a freshly-moved cursor and only test hand tiles.
    frame_picks: FramePicks,
    /// One-shot watchdog that logs a single warning when frame pacing has
    /// collapsed to <20 FPS for >3 s of steady-state rendering. See
    /// [`main_perf_watchdog`] for the rationale.
    perf_watchdog: main_perf_watchdog::FramePerfWatchdog,
}

/// Cached output of the four scene picks that `frame_tick` consumes.
/// Each field is `None` when the renderer is missing or the cursor is
/// outside the relevant pickable surface this frame. Recomputed every
/// frame_tick from the latest cursor position.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FramePicks {
    pub hand: Option<usize>,
    pub shop: Option<render::wgpu_renderer::ShopHit>,
    pub gameplay: Option<render::wgpu_renderer::GameplayPick>,
    pub collection: Option<u32>,
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
        steam.sync_profile_stats(&progress);
        let mut audio = audio::AudioManager::new();
        audio.set_master_volume(settings.master_volume);
        audio.set_sfx_volume(settings.sfx_volume);
        audio.set_music_volume(settings.music_volume);
        if !settings.sfx_enabled {
            audio.set_enabled(false);
        }
        let scene_look = SceneLookTuningSet::load();
        Self {
            last_drawable_px: PhysicalSize::new(1920, 1080),
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
            transition_speed: DEFAULT_QUICK_SPEC.speed,
            transition_timer: 0.0,
            transition_kind: TransitionKind::Quick,
            pending_scene: None,
            pending_scene_destination: PendingSceneDestination::default(),
            overlay_stack: Vec::new(),
            prev_controller_present: false,
            quit_requested: false,
            close_saved: false,
            modals: ModalQueue::default(),
            pending_post_game_over_level_up: None,
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
                vhs_enabled: settings.vhs_enabled,
            },
            // Default: cheap baseline; see `effect_layers.rs`. Use `FULL` or flip
            // flags to restore shadows, SSR, particles, transition FX, HDR, etc.
            effect_layers: crate::effect_layers::EffectLayers::BASELINE,
            debug: DebugState::new(),
            cascade_tuning: CascadeTuning::default(),
            scene_look,
            modifiers: Mod::NOMOD,
            steam,
            archive_last_seen_run_len: settings.archive_last_seen_run_len,
            cpu_profiler: render::cpu_profiler::CpuProfiler::new(),
            profile_saver: persistence::ProfileSaver::spawn(),
            profile_dirty: false,
            last_window_title: String::new(),
            room_gltf_brownout: main_room_gltf_brownout::RoomGltfBrownout::new(),
            frame_picks: FramePicks::default(),
            perf_watchdog: main_perf_watchdog::FramePerfWatchdog::new(),
        }
    }

    /// Flag `progress` for a background save at frame end. Cheap — the
    /// actual JSON serialize + write happens off-thread via
    /// [`persistence::ProfileSaver`]. The cache is updated when the
    /// flush enqueues a snapshot, which is fine because nothing
    /// `load_profile`s mid-frame between event handlers.
    pub(crate) fn mark_profile_dirty(&mut self) {
        self.profile_dirty = true;
    }

    /// Frame-end flush: hand a snapshot of `progress` to the saver
    /// thread iff something marked it dirty. Resets the flag.
    pub(crate) fn flush_dirty_profile(&mut self) {
        if self.profile_dirty {
            self.profile_saver
                .enqueue(self.active_profile, &self.progress);
            self.profile_dirty = false;
        }
    }

    /// Quit / window-close hand-off: synchronously persist `progress`
    /// after stopping the background saver. The saver is shut down
    /// first so any pending older snapshot in its channel can't land
    /// on disk after the synchronous write.
    pub(crate) fn save_profile_sync_for_exit(&mut self) {
        self.profile_saver.shutdown();
        if let Err(e) = persistence::save_profile(self.active_profile, &self.progress) {
            log::warn!("save_profile (exit) failed: {e}");
        }
        self.profile_dirty = false;
    }

    fn toggle_fullscreen(&mut self, shell: &mut sdl_shell::SdlShell) -> anyhow::Result<()> {
        let on = shell.desktop_fullscreen_on();
        shell.set_desktop_fullscreen(!on)?;
        let mut settings = persistence::load_settings();
        settings.borderless_fullscreen = shell.desktop_fullscreen_on();
        let _ = persistence::save_settings(&settings);
        Ok(())
    }

    fn wants_fullscreen_shortcut(
        &self,
        scancode: Option<Scancode>,
        keymod: Mod,
        repeat: bool,
    ) -> bool {
        if repeat {
            return false;
        }
        let Some(code) = scancode else {
            return false;
        };

        #[cfg(target_os = "windows")]
        {
            let no_extra_modifiers = !(keymod.contains(Mod::LCTRLMOD | Mod::RCTRLMOD)
                || keymod.contains(Mod::LSHIFTMOD | Mod::RSHIFTMOD)
                || keymod.contains(Mod::LGUIMOD | Mod::RGUIMOD));
            keymod.contains(Mod::LALTMOD | Mod::RALTMOD)
                && no_extra_modifiers
                && matches!(code, Scancode::Return | Scancode::KpEnter)
        }

        #[cfg(target_os = "macos")]
        {
            if code != Scancode::F {
                return false;
            }
            let disallowed_mod = keymod.contains(Mod::LCTRLMOD)
                || keymod.contains(Mod::RCTRLMOD)
                || keymod.contains(Mod::LALTMOD)
                || keymod.contains(Mod::RALTMOD)
                || keymod.contains(Mod::LGUIMOD)
                || keymod.contains(Mod::RGUIMOD)
                || keymod.contains(Mod::LSHIFTMOD)
                || keymod.contains(Mod::RSHIFTMOD);
            if disallowed_mod {
                return false;
            }
            crate::macos_fullscreen_shortcut::fn_modifier_held()
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = code;
            let _ = keymod;
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
        self.steam.sync_profile_stats(&self.progress);
        // Persist the active profile choice.
        settings.active_profile = new_index;
        let _ = persistence::save_settings(&settings);
        self.archive_last_seen_run_len = settings.archive_last_seen_run_len;
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

/// When `MAHJURO_LOG_FILE` is set, send `log` output to that path (append) instead
/// of stderr. Steam and other GUI launchers often discard stderr, so this is the
/// reliable way to capture `RUST_LOG` when launching from Steam.
fn init_env_logger() {
    use std::fs::OpenOptions;
    use std::io::LineWriter;
    use std::path::Path;

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(path_raw) = std::env::var_os("MAHJURO_LOG_FILE") {
        let path = Path::new(&path_raw);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => {
                builder.target(env_logger::Target::Pipe(Box::new(LineWriter::new(f))));
            }
            Err(e) => {
                eprintln!(
                    "Mahjuro: could not open MAHJURO_LOG_FILE {}: {e}",
                    path.display()
                );
            }
        }
    }
    builder.init();
}

/// Non-panic startup failures never run the panic hook / `crash_guard` dialog.
/// If the user set `MAHJURO_LOG_FILE`, mirror the final error there too.
fn append_startup_error_to_log_file(msg: &str) {
    use std::io::Write;

    let Some(path_raw) = std::env::var_os("MAHJURO_LOG_FILE") else {
        return;
    };
    let path = std::path::Path::new(&path_raw);
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(
        f,
        "{} [startup error] {msg}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
}

/// Interactive / CLI entry used by the `mahjuro` binary.
pub fn run() -> anyhow::Result<()> {
    crash_guard::install();
    init_env_logger();
    let cli = Cli::parse();

    if main_commands::run_cli_command(cli.command)? {
        return Ok(());
    }

    {
        let _assets = crate::startup_profile::scope("assets.init");
        asset_path::init();
        asset_path::log_all_assets();
    }

    let no_steam = cli.no_steam;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> anyhow::Result<()> {
            // Steam init runs before the window so the SDK can hook the
            // rendering surface as it's created. `--no-steam` short-circuits
            // this for dev runs where you don't want the overlay attaching
            // or Steam claiming the foreground process slot. Init failures
            // (Steam not running, no license, etc.) are logged and we fall
            // back to `Disabled` — the game then runs normally without
            // achievements/overlay.
            let steam = {
                let _steam_scope = crate::startup_profile::scope("steam.init");
                if no_steam {
                    log::debug!("--no-steam: skipping Steamworks init");
                    steam::SteamClient::disabled()
                } else if !steam::steamworks_dll_ready() {
                    log::warn!(
                        "steam_api64.dll was not found next to this executable (or failed to load); \
                         Steam achievements and overlay are disabled this session",
                    );
                    steam::SteamClient::disabled()
                } else {
                    steam::SteamClient::init()
                }
            };

            let settings = persistence::load_settings();
            let tenfoot = std::env::var_os("SteamTenfoot").is_some();
            let launch_borderless = !tenfoot && settings.borderless_fullscreen;
            let mut shell = {
                let _sdl = crate::startup_profile::scope("sdl.window");
                sdl_shell::SdlShell::new("Mahjuro", 1920, 1080, launch_borderless)?
            };
            let app = {
                let _app = crate::startup_profile::scope("app.new");
                App::new(steam)
            };
            app.run_sdl_main(&mut shell)?;
            Ok(())
        },
    ));

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            append_startup_error_to_log_file(&format!("{e:#}"));
            Err(e)
        }
        Err(_) => {
            crash_guard::show_crash_report();
            std::process::exit(1);
        }
    }
}
