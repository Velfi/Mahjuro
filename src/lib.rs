//! Mahjuro — UI-first shell: SDL3 + wgpu + input + scene system.
//!
//! The executable entry point is [`run`]; this library exists so benchmarks
//! (`cargo bench`) can link the game logic.

#![deny(unused_imports)]

pub use mahjuro_assets::asset_path;
pub use mahjuro_assets::asset_sources;
pub mod core;
pub use mahjuro_render as render;

/// Ordeal presentation helpers (`def` / `name` / `tier`) live in the game crate.
pub use game::ordeal::OrdealKindExt;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
mod audio;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
pub mod bot;
#[cfg(feature = "game")]
mod cascade_tuning_timeline;
#[cfg(feature = "game")]
mod crash_guard;
#[cfg(feature = "game")]
mod win_log_console;
#[cfg(feature = "game")]
mod debug_menu;
#[cfg(feature = "game")]
mod debug_overlays;
pub mod effect_layers;
pub mod game;
#[cfg(all(feature = "game", target_os = "macos"))]
mod macos_fullscreen_shortcut;
#[cfg(feature = "game")]
#[path = "main/cli.rs"]
mod main_cli;
#[cfg(feature = "game")]
#[path = "main/commands.rs"]
mod main_commands;
#[cfg(all(feature = "game", debug_menu_enabled))]
#[path = "main/debug_actions.rs"]
mod main_debug_actions;
#[cfg(feature = "game")]
#[path = "main/debug_state.rs"]
mod main_debug_state;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
#[path = "main/draw.rs"]
pub mod main_draw;
#[cfg(feature = "game")]
#[path = "main/event_loop.rs"]
mod main_event_loop;
#[cfg(feature = "game")]
#[path = "main/frame_tick.rs"]
mod main_frame_tick;
#[cfg(feature = "game")]
#[path = "main/perf_watchdog.rs"]
mod main_perf_watchdog;
#[path = "main/render_settings.rs"]
pub mod main_render_settings;
#[cfg(feature = "game")]
#[path = "main/room_gltf_brownout.rs"]
mod main_room_gltf_brownout;
pub mod persistence;
pub mod sfx_id;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
mod shell_open;
#[cfg(feature = "game")]
mod trailer_mode;
pub use mahjuro_render::physical_size;
#[cfg(feature = "game")]
#[path = "main/scene_transition.rs"]
mod scene_transition;
pub mod scenes;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
mod sdl_shell;
pub use mahjuro_render::startup_profile;
pub mod steam;
#[cfg(feature = "game")]
pub(crate) use steam::DistributionBackend;
#[cfg(test)]
mod shader_fxc_lints;
pub mod ui;

#[cfg(feature = "game")]
#[path = "main/app.rs"]
mod app;
#[cfg(feature = "game")]
use std::time::Instant;

#[cfg(feature = "game")]
use crate::game::cascade::CascadeTuning;
#[cfg(feature = "game")]
use crate::game::event_bus::{EventBus, GameEvent};
#[cfg(feature = "game")]
use crate::game::run::RunState;
#[cfg(feature = "game")]
use crate::game::scene_look_tuning::SceneLookTuningSet;
#[cfg(feature = "game")]
use crate::main_debug_state::DebugState;
#[cfg(feature = "game")]
use crate::main_render_settings::RenderSettings;
#[cfg(feature = "game")]
use crate::physical_size::PhysicalSize;
#[cfg(feature = "game")]
use crate::render::animation::AnimationController;
#[cfg(feature = "game")]
use crate::render::wgpu_renderer::WgpuRenderer;
#[cfg(feature = "game")]
use crate::scene_transition::{PendingSceneDestination, TransitionKind};
#[cfg(feature = "game")]
use crate::scenes::{ButtonDef, Scene, SceneBehavior};
#[cfg(feature = "game")]
use crate::ui::input::{InputMode, InputState, UiAction};
#[cfg(feature = "game")]
use crate::ui::layout::UiLayout;
#[cfg(feature = "game")]
use crate::ui::modal::{Modal, ModalQueue};
#[cfg(feature = "game")]
use sdl3::keyboard::Mod;

#[cfg(all(feature = "game", debug_menu_enabled))]
use crate::debug_menu::DebugMenuBar;
#[cfg(feature = "game")]
use crate::render::draw_cmd::{CameraParams, UiFrame, apply_modal_relic_staging};
#[cfg(feature = "game")]
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
#[cfg(feature = "game")]
use crate::scenes::{ButtonAction, DrawCtx};
#[cfg(feature = "game")]
use clap::{ArgAction, Args, Parser, Subcommand};

#[cfg(feature = "game")]
pub(crate) struct App {
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
    /// True on the frame a right mouse button press landed (shop inspect).
    mouse_right_clicked: bool,
    /// Scene button id deferred until LMB release when shop uses drag-to-orbit.
    deferred_lmb_button_click: Option<u32>,
    /// Cursor at LMB press for drag-vs-click discrimination in the shop.
    mouse_left_press_cursor: Option<(f32, f32)>,
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
    /// Destination built at full black from [`pending_scene_intent`].
    pending_scene_intent: Option<crate::scenes::SceneIntent>,
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
    /// Platform distribution backend (Steam, Game Center, Xbox, or disabled).
    /// Every method is safe when disabled — logged no-op — so no `Option`
    /// wrapping is needed at call sites.
    dist: steam::DistributionClient,
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
    /// Rare flicker + brownout on shop / pick_chamber / collection room lighting.
    room_gltf_brownout: main_room_gltf_brownout::RoomGltfBrownout,
    /// Frame-scoped scene-pick cache. Computed once at the top of
    /// `frame_tick` and reused by both `frame_tick` (for `update`) and
    /// `draw` (for `DrawCtx::new`). Without this, every gameplay frame
    /// pays for ~4 separate ray-vs-AABB walks against the full per-class
    /// matrix lists. Per-event mouse-motion picks in
    /// [`crate::main::event_loop`] are intentionally not cached — they
    /// run with a freshly-moved cursor and only test hand tiles.
    frame_picks: FramePicks,
    /// Hub menu loading gates; refreshed each `frame_tick` for draw + update.
    hub_loading: crate::scenes::main_menu::HubMenuLoading,
    /// One-shot watchdog that logs a single warning when frame pacing has
    /// collapsed to <20 FPS for >3 s of steady-state rendering. See
    /// [`main_perf_watchdog`] for the rationale.
    perf_watchdog: main_perf_watchdog::FramePerfWatchdog,
}

/// Cached output of the four scene picks that `frame_tick` consumes.
/// Each field is `None` when the renderer is missing or the cursor is
/// outside the relevant pickable surface this frame. Recomputed every
/// frame_tick from the latest cursor position.
#[cfg(feature = "game")]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FramePicks {
    pub(crate) hand: Option<usize>,
    pub(crate) shop: Option<crate::render::wgpu_renderer::ShopHit>,
    pub(crate) gameplay: Option<crate::render::wgpu_renderer::GameplayPick>,
}

#[cfg(feature = "game")]
use main_cli::Cli;

/// When `MAHJURO_LOG_FILE` is set, send `log` output to that path (append) instead
/// of stderr. Steam and other GUI launchers often discard stderr, so this is the
/// reliable way to capture `RUST_LOG` when launching from Steam.
///
/// On Windows release builds (`windows_subsystem = "windows"`), stderr has no
/// console unless we attach to the parent terminal or redirect to a file.
#[cfg(feature = "game")]
fn init_env_logger() {
    use std::fs::OpenOptions;
    use std::io::{LineWriter, Write};
    use std::path::PathBuf;

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    match if let Some(path_raw) = std::env::var_os("MAHJURO_LOG_FILE") {
        if !mahjuro_distribution::PlatformPaths::allows_external_log_file() {
            let container = mahjuro_distribution::PlatformPaths::data_root().join("mahjuro.log");
            eprintln!(
                "MAHJURO_LOG_FILE ignored on store builds; use container log: {}",
                container.display()
            );
            win_log_console::RustLogOutput::File(container)
        } else {
            win_log_console::RustLogOutput::File(PathBuf::from(path_raw))
        }
    } else {
        win_log_console::prepare_rust_log_output()
    } {
        win_log_console::RustLogOutput::Console(writer) => {
            builder.target(env_logger::Target::Pipe(Box::new(writer)));
        }
        win_log_console::RustLogOutput::File(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(mut f) => {
                    if std::env::var_os("MAHJURO_LOG_FILE").is_none() {
                        let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
                        let _ = writeln!(
                            f,
                            "{} Windows release: RUST_LOG={rust_log:?}; logging to {}",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                            path.display()
                        );
                    }
                    builder.target(env_logger::Target::Pipe(Box::new(LineWriter::new(f))));
                }
                Err(e) => {
                    eprintln!(
                        "Mahjuro: could not open log file {}: {e}",
                        path.display()
                    );
                }
            }
        }
        win_log_console::RustLogOutput::None => {}
    }
    builder.init();
}

/// Non-panic startup failures never run the panic hook / `crash_guard` dialog.
/// If the user set `MAHJURO_LOG_FILE`, mirror the final error there too.
#[cfg(feature = "game")]
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
#[cfg(feature = "game")]
pub fn run() -> anyhow::Result<()> {
    crash_guard::install();
    init_env_logger();
    let cli = Cli::parse();
    let no_platform = cli.no_platform_services();

    if main_commands::run_cli_command(cli.command)? {
        return Ok(());
    }

    {
        let _assets = crate::startup_profile::scope("assets.init");
        asset_path::init();
        asset_path::log_all_assets();
    }
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
            // Platform services init runs before the window (Steam hooks the
            // rendering surface as it's created). `--no-platform-services` /
            // `--no-steam` short-circuit for dev runs.
            let dist = {
                let _scope = crate::startup_profile::scope("dist.init");
                if no_platform {
                    log::debug!("platform services disabled via CLI");
                }
                mahjuro_distribution::init(steam::DistributionConfig {
                    platform_services_disabled: no_platform,
                })
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
                App::new(dist)
            };
            app.run_sdl_main(&mut shell)?;
            Ok(())
        }));

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
