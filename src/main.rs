//! Mahjuro — UI-first shell: winit + wgpu + cassowary + input + scene system.

pub mod asset_path;
mod audio;
mod bot;
mod core;
pub mod crash_guard;
mod debug_menu;
mod debug_overlays;
mod game;
mod persistence;
mod render;
mod scenes;
mod ui;
mod update_check;

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Instant;

use clap::{ArgAction, Args, Parser, Subcommand};
use debug_menu::{DebugAction, DebugMenuBar};
use debug_overlays::{
    CameraDebugOverlay, DebugVisResult, DebugVisibilityOverlay, SfxTestOverlay, TuningOverlay,
    TuningResult,
};
use game::cascade::CascadeTuning;
use game::event_bus::{EventBus, GameEvent};
use game::run::RunState;
use render::animation::AnimationController;
use render::draw_cmd::{CameraParams, UiFrame};
use render::wgpu_renderer::{GpuInstance, TextLabel, WgpuRenderer};
use scenes::game_over::GameOverScene;
use scenes::gameplay::GameplayScene;
use scenes::shop::ShopScene;
use scenes::splash::SplashScene;
use scenes::tutorial_recap::TutorialRecapScene;
use scenes::tutorial_summary::TutorialSummaryScene;
use scenes::{ButtonAction, ButtonDef, DrawCtx, Scene, SceneBehavior, UpdateCtx};
use serde::{Deserialize, Serialize};
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
use ui::modal::{Modal, ModalQueue, ModalTheme, UnlockPage};
use ui::tooltip::TooltipState;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

// Debug overlays (visibility toggles, cascade tuning, SFX test, camera
// params) live in `debug_overlays.rs`.  See `mod debug_overlays` above.

/// Persisted visual/audio settings mirrored from the options screen.
/// Grouped so they can be synced in one go from `OptionsScene` state.
struct RenderSettings {
    smoke_intensity: crate::persistence::SmokeIntensity,
    smoke_detail: crate::persistence::SmokeDetail,
    effects_quality: crate::persistence::EffectsQuality,
    tile_preset: crate::persistence::TilePreset,
    tile_material: crate::persistence::TileMaterial,
    gamma: f32,
    shadows_enabled: bool,
    ssr_enabled: bool,
    hdr_enabled: bool,
    ui_scale: f32,
}

/// Debug-only state: overlays, visibility toggles, FPS counter, and the
/// one-shot object-hit-test picker.
struct DebugState {
    menu: Option<DebugMenuBar>,
    show_fps: bool,
    fps_smoothed: f32,
    hide_tiles: bool,
    hide_candles: bool,
    hide_blind_plaque: bool,
    hide_scoring_placard: bool,
    hide_inventory: bool,
    visibility_overlay: Option<DebugVisibilityOverlay>,
    tuning_overlay: Option<TuningOverlay>,
    sfx_test_overlay: Option<SfxTestOverlay>,
    camera_debug_overlay: Option<CameraDebugOverlay>,
    /// One-shot debug picker armed by the "Object Hit Test" debug menu
    /// item.
    object_hit_test_armed: bool,
    /// Effective 3D camera after the scene's `draw_frame` (override or table
    /// default), updated each paint — used to seed camera debug overlay.
    last_effective_camera: CameraParams,
}

impl DebugState {
    fn new() -> Self {
        Self {
            menu: None,
            show_fps: false,
            fps_smoothed: 60.0,
            hide_tiles: false,
            hide_candles: false,
            hide_blind_plaque: false,
            hide_scoring_placard: false,
            hide_inventory: false,
            visibility_overlay: None,
            tuning_overlay: None,
            sfx_test_overlay: None,
            camera_debug_overlay: None,
            object_hit_test_armed: false,
            last_effective_camera: CameraParams::default_table_camera(800.0),
        }
    }

    /// Whether any debug overlay is blocking input.
    fn any_overlay_active(&self) -> bool {
        self.tuning_overlay.is_some()
            || self.sfx_test_overlay.is_some()
            || self.camera_debug_overlay.is_some()
            || self.visibility_overlay.is_some()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransitionKind {
    /// Default fast fade (~0.2 s).
    Quick,
    /// Dramatic shooting-star cascade (~1.7 s total before extra score steps).
    ShootingStarCascade,
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the headless bot for tuning/balance checks.
    Bot(BotCli),
    /// Run headless bots, update docs snapshot JSON, and regenerate graphs.
    BotGraph(BotGraphCli),
    /// Run the default tuning sweep grid.
    Sweep(SweepCli),
}

#[derive(Debug, Args)]
struct BotCli {
    /// Number of runs to execute.
    #[arg(default_value_t = 100)]
    runs: u32,
    /// Override the standard mode's base blind target.
    #[arg(long)]
    base_target: Option<u32>,
    /// Override the standard mode's target scaling.
    #[arg(long)]
    target_scale: Option<f32>,
    /// Override the standard mode's starting plays.
    #[arg(long)]
    plays: Option<u32>,
    /// Override the standard mode's starting discards.
    #[arg(long)]
    discards: Option<u32>,
    /// Override the standard mode's starting gold.
    #[arg(long)]
    gold: Option<u32>,
    /// Print per-run bot logs in addition to the aggregate summary.
    #[arg(long, action = ArgAction::SetTrue)]
    bot_log: bool,
    /// Export aggregate bot results to a JSON file.
    #[arg(long)]
    export_json: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SweepCli {
    /// Number of runs to execute per sweep cell.
    #[arg(long, default_value_t = 40)]
    runs: u32,
    /// Export sweep results to a JSON file.
    #[arg(long)]
    export_json: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BotGraphCli {
    /// Number of runs to execute.
    #[arg(default_value_t = 10_000)]
    runs: u32,
    /// Stable slug used to insert or replace the snapshot in docs JSON.
    #[arg(long)]
    slug: Option<String>,
    /// Chart label. Supports literal \n for multi-line chart labels.
    #[arg(long)]
    label: Option<String>,
    /// Override the standard mode's base blind target.
    #[arg(long)]
    base_target: Option<u32>,
    /// Override the standard mode's target scaling.
    #[arg(long)]
    target_scale: Option<f32>,
    /// Override the standard mode's starting plays.
    #[arg(long)]
    plays: Option<u32>,
    /// Override the standard mode's starting discards.
    #[arg(long)]
    discards: Option<u32>,
    /// Override the standard mode's starting gold.
    #[arg(long)]
    gold: Option<u32>,
    /// Print per-run bot logs in addition to the aggregate summary.
    #[arg(long, action = ArgAction::SetTrue)]
    bot_log: bool,
}

impl BotCli {
    fn bot_config(&self) -> bot::BotConfig {
        bot::BotConfig {
            base_target: self.base_target,
            target_scaling: self.target_scale,
            starting_plays: self.plays,
            starting_discards: self.discards,
            starting_gold: self.gold,
        }
    }
}

impl BotGraphCli {
    fn bot_config(&self) -> bot::BotConfig {
        bot::BotConfig {
            base_target: self.base_target,
            target_scaling: self.target_scale,
            starting_plays: self.plays,
            starting_discards: self.discards,
            starting_gold: self.gold,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BotGraphSnapshot {
    slug: String,
    label: String,
    runs: u32,
    win_rate: f64,
    avg_blinds: f64,
    avg_antes: f64,
    avg_total_score_m: f64,
    avg_plays: f64,
    avg_discards: f64,
    avg_skips: f64,
    avg_relics: f64,
    avg_gold_spent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_gold_earned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_base: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_plays: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_interest: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_relics: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_final_gold: Option<f64>,
    deaths_by_ante: std::collections::BTreeMap<u32, u32>,
}

fn avg_u64(total: u64, runs: u32) -> f64 {
    if runs == 0 {
        0.0
    } else {
        total as f64 / runs as f64
    }
}

fn avg_i64(total: i64, runs: u32) -> f64 {
    if runs == 0 {
        0.0
    } else {
        total as f64 / runs as f64
    }
}

fn default_snapshot_slug(mode: &game::game_mode::GameMode, runs: u32) -> String {
    format!(
        "bt{}_ts{}_p{}_d{}_g{}_{}",
        mode.base_target,
        format!("{:.2}", mode.target_scaling).replace('.', "_"),
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        runs
    )
}

fn default_snapshot_label(mode: &game::game_mode::GameMode, runs: u32) -> String {
    format!(
        "Base {}\nScale {:.2}\nP{} D{} G{}\n({} runs)",
        mode.base_target,
        mode.target_scaling,
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        runs
    )
}

fn build_bot_graph_snapshot(
    agg: &bot::AggregateStats,
    slug: String,
    label: String,
) -> BotGraphSnapshot {
    let runs = agg.runs;
    BotGraphSnapshot {
        slug,
        label,
        runs,
        win_rate: if runs == 0 {
            0.0
        } else {
            agg.victories as f64 * 100.0 / runs as f64
        },
        avg_blinds: avg_u64(agg.blinds_cleared_total, runs),
        avg_antes: avg_u64(agg.antes_cleared_total, runs),
        avg_total_score_m: avg_u64(agg.total_score, runs) / 1_000_000.0,
        avg_plays: avg_u64(agg.total_plays, runs),
        avg_discards: avg_u64(agg.total_discards, runs),
        avg_skips: avg_u64(agg.total_blinds_skipped, runs),
        avg_relics: avg_u64(agg.total_relics_bought, runs),
        avg_gold_spent: avg_u64(agg.total_gold_spent, runs),
        avg_gold_earned: Some(avg_u64(
            agg.total_gold_from_clears + agg.total_gold_from_skip_tags,
            runs,
        )),
        clear_base: Some(avg_u64(agg.total_gold_from_clear_base, runs)),
        clear_plays: Some(avg_u64(agg.total_gold_from_unused_plays, runs)),
        clear_interest: Some(avg_u64(agg.total_gold_from_interest, runs)),
        clear_relics: Some(avg_u64(agg.total_gold_from_clear_relics, runs)),
        avg_final_gold: Some(avg_i64(agg.total_final_gold, runs)),
        deaths_by_ante: agg.deaths_by_ante.clone(),
    }
}

fn load_snapshots(path: &Path) -> anyhow::Result<Vec<BotGraphSnapshot>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_snapshots(path: &Path, snapshots: &[BotGraphSnapshot]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(snapshots)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn upsert_snapshot(path: &Path, snapshot: BotGraphSnapshot) -> anyhow::Result<()> {
    let mut snapshots = load_snapshots(path)?;
    if let Some(existing) = snapshots.iter_mut().find(|item| item.slug == snapshot.slug) {
        *existing = snapshot;
    } else {
        snapshots.push(snapshot);
    }
    write_snapshots(path, &snapshots)
}

fn render_bot_graphs(repo_root: &Path) -> anyhow::Result<()> {
    let status = ProcessCommand::new("python3")
        .arg("tools/plot_bot_balance.py")
        .current_dir(repo_root)
        .status()?;
    anyhow::ensure!(status.success(), "graph render failed with status {status}");
    Ok(())
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
    scroll_delta: f32,
    active_buttons: Vec<ButtonDef>,
    scene: Scene,
    progress: crate::core::progression::PlayerProgress,
    active_profile: usize,
    audio: audio::AudioManager,
    transition_alpha: f32,
    transition_speed: f32,
    transition_timer: f32,
    transition_kind: TransitionKind,
    pending_scene: Option<Scene>,
    /// Scene suspended while the Meld Guide is open. Restored when the guide
    /// signals `wants_back`.
    suspended_scene: Option<Scene>,
    quit_requested: bool,
    close_saved: bool,
    modals: ModalQueue,
    pending_post_game_over_modals: Vec<Modal>,
    gfx: RenderSettings,
    debug: DebugState,
    #[allow(dead_code)]
    prev_cursor: (f32, f32),
    tooltips: TooltipState,
    cascade_tuning: CascadeTuning,
    deferred_round_end: Option<GameEvent>,
    update_checker: update_check::UpdateChecker,
    modifiers: ModifiersState,
}

impl App {
    /// Single source of truth for "is anything modal-like up right now?"
    ///
    /// **The modal-blocking pattern.** Any overlay that should block input
    /// and hover for elements below it is reported here, by ORing together:
    ///   - The app-owned [`ModalQueue`] (toast modals).
    ///   - App-owned debug overlays (`tuning_overlay`, `sfx_test_overlay`).
    ///   - The active scene's own internal overlays, via
    ///     [`Scene::has_blocking_overlay`].
    ///
    /// Two universal gates in the main loop consult this:
    ///   1. **Tooltip/hover gate** — `skip_tooltips` in the redraw path
    ///      suppresses the global tooltip overlay so hover effects never
    ///      leak through a modal.
    ///   2. **Click safety wipe** — right after the scene populates
    ///      `active_buttons`, those buttons are cleared if any modal is up,
    ///      so scene buttons can never be clicked through. Overlays that
    ///      *want* their own clickable surface (e.g. `ModalQueue`'s full-
    ///      screen dismiss) write to `active_buttons` *after* the wipe in
    ///      their own draw step.
    ///
    /// To make a new overlay modal-blocking by default:
    ///   - If it's app-owned: add it to this OR-chain.
    ///   - If it's scene-owned: report it from the scene's
    ///     `has_blocking_overlay()` method.
    /// No per-call-site changes are needed — the gates pick it up
    /// automatically.
    fn modal_overlay_active(&self) -> bool {
        self.modals.is_active()
            || self.debug.any_overlay_active()
            || self.scene.has_blocking_overlay()
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

    fn new() -> Self {
        let t0 = Instant::now();
        let settings = persistence::load_settings();
        let active_profile = settings.active_profile;
        let progress = persistence::load_profile(active_profile);
        // Prefer a saved-on-quit run for this profile (resume). If none
        // exists or it was written by a previous build version, fall back
        // to a fresh demo run. `load_run` deletes stale/corrupt saves.
        let mut run = persistence::load_run(active_profile).unwrap_or_else(RunState::new_demo);
        run.set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        run.available_yaku = progress.available_yaku();
        run.available_rules = progress.available_rules();
        let mut audio = audio::AudioManager::new();
        audio.set_master_volume(settings.master_volume);
        audio.set_sfx_volume(settings.sfx_volume);
        audio.set_music_volume(settings.music_volume);
        if !settings.sfx_enabled {
            audio.set_enabled(false);
        }
        log::info!("App::new() settings + profile loaded in {:?}", t0.elapsed());
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
            scroll_delta: 0.0,
            active_buttons: Vec::new(),
            scene: Scene::Splash(SplashScene::new()),
            progress,
            active_profile,
            audio,
            transition_alpha: 1.0,
            transition_speed: 0.08,
            transition_timer: 0.0,
            transition_kind: TransitionKind::Quick,
            pending_scene: None,
            suspended_scene: None,
            quit_requested: false,
            close_saved: false,
            modals: ModalQueue::default(),
            pending_post_game_over_modals: Vec::new(),
            deferred_round_end: None,
            gfx: RenderSettings {
                smoke_intensity: settings.smoke_intensity,
                smoke_detail: settings.smoke_detail,
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                gamma: settings.gamma,
                shadows_enabled: settings.shadows_enabled,
                ssr_enabled: settings.ssr_enabled,
                hdr_enabled: settings.hdr_enabled,
                ui_scale: settings.ui_scale,
            },
            debug: DebugState::new(),
            prev_cursor: (0.0, 0.0),
            tooltips: TooltipState::new(),
            cascade_tuning: CascadeTuning::default(),
            update_checker: update_check::UpdateChecker::spawn(),
            modifiers: ModifiersState::default(),
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
        self.run = persistence::load_run(new_index).unwrap_or_else(RunState::new_demo);
        let mut settings = persistence::load_settings();
        self.run
            .set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        self.run.available_yaku = self.progress.available_yaku();
        self.run.available_rules = self.progress.available_rules();
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
            if let Err(e) = persistence::save_run(self.active_profile, &self.run) {
                log::warn!("save_run failed: {e}");
            }
        } else {
            persistence::delete_saved_run(self.active_profile);
        }
    }

    /// Process a `RoundComplete` or `GameOver` event that was held while the
    /// scoring cascade was still playing. Pushes celebration modals, plays the
    /// appropriate sting, and queues the next scene.
    fn handle_round_end_event(&mut self, ev: GameEvent) {
        let win_size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(PhysicalSize::new(800, 600));
        let ww = win_size.width as f32;
        let wh = win_size.height as f32;
        match ev {
            GameEvent::RoundComplete { payout, .. } => {
                if self.run.onboarding_active() {
                    self.run.gold = self.run.gold.saturating_add(payout.total as i32);
                    self.progress.tutorial_completed = true;
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    persistence::delete_saved_run(self.active_profile);
                    self.pending_scene =
                        Some(Scene::TutorialSummary(TutorialSummaryScene::new(true)));
                    self.transition_alpha = 1.0;
                    return;
                }
                // Apply the gold payout now that the scoring cascade has
                // finished — kept deferred so the UI doesn't jump early.
                self.run.gold = self.run.gold.saturating_add(payout.total as i32);
                self.audio.play_sfx(audio::SfxId::RoundWin);
                let final_score = self.run.round_score;
                let target = self.run.target_score;
                // Capture the tutorial lesson *before* advancing so the
                // recap scene can show what was just learned.
                let tutorial_lesson_before = self
                    .run
                    .tutorial
                    .as_ref()
                    .filter(|t| t.is_active())
                    .map(|t| t.current_lesson);
                self.run.advance_round(&mut self.bus);

                // First-encounter tooltip: gold payout.
                if let Some(ref mut tut) = self.run.tutorial {
                    if tut.is_active()
                        && payout.total > 0
                        && tut.encounter(crate::game::tutorial::FirstEncounter::GoldPayout)
                    {
                        self.modals.push(Modal::new(
                            crate::game::tutorial::FirstEncounter::GoldPayout.title(),
                            crate::game::tutorial::FirstEncounter::GoldPayout.message(),
                            ModalTheme::Success,
                        ));
                    }
                }

                // After Lesson 5 (Chips x Mult), grant a free relic to
                // introduce the concept before the shop appears. This
                // bridges the gap between learning scoring and discovering
                // the relic/shop meta-loop.
                if tutorial_lesson_before == Some(5)
                    && !self
                        .run
                        .relics
                        .active
                        .contains(&crate::core::relic::RelicId::PairPower)
                {
                    self.run
                        .relics
                        .active
                        .push(crate::core::relic::RelicId::PairPower);
                    let modal = Modal::new(
                        "Relic Earned!",
                        "You found Pair Power! Relics give passive bonuses for the rest of your run. Pairs now score +30 chips and +1 mult.",
                        ModalTheme::Success,
                    );
                    self.modals.push(modal);
                }

                // Skip the "Round Complete" modal during tutorials — the
                // TutorialRecap scene already shows the round outcome and
                // rendering both causes them to overlap.
                if tutorial_lesson_before.is_none() {
                    let mut lines = vec![format!(
                        "Score: {} / {}",
                        self.run.round_score, self.run.target_score
                    )];
                    lines.push(format!("Base reward  +${}", payout.base_reward));
                    if payout.unused_play_bonus > 0 {
                        lines.push(format!("Unused plays  +${}", payout.unused_play_bonus));
                    }
                    if payout.interest > 0 {
                        lines.push(format!("Interest  +${}", payout.interest));
                    }
                    if payout.green_luck_bonus > 0 {
                        lines.push(format!("Green Luck  +${}", payout.green_luck_bonus));
                    }
                    lines.push(format!("Total  +${}", payout.total));
                    let modal =
                        Modal::new("Round Complete!", lines.join("\n"), ModalTheme::Success)
                            .with_fireworks(ww * 0.5, wh * 0.8, ww * 0.6, 5);
                    self.modals.push(modal);
                }

                self.pending_scene = Some(if self.run.is_run_complete() {
                    // Victory — save progress (mirrors the GameOver loss path).
                    self.progress.has_won = true;
                    self.progress.runs_completed += 1;
                    self.progress.record_score(self.run.round_score);
                    let _ = self.progress.check_level_up();
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    persistence::delete_saved_run(self.active_profile);
                    Scene::GameOver(GameOverScene::victory(final_score, target))
                } else if let Some(lesson) = tutorial_lesson_before {
                    // Tutorial: show a recap of the completed lesson.
                    let shop_follows = self.run.tutorial_shop_enabled();
                    Scene::TutorialRecap(TutorialRecapScene::new(lesson, shop_follows))
                } else if !self.run.tutorial_shop_enabled() {
                    Scene::Gameplay(GameplayScene::new())
                } else {
                    Scene::Shop(ShopScene::new(self.run.run_number, &mut self.run))
                });
                self.transition_alpha = 1.0;
            }
            GameEvent::GameOver { .. } => {
                if self.run.onboarding_active() {
                    let round_score = self.run.round_score;
                    let target_score = self.run.target_score;
                    let discards_left = self.run.discards_remaining;
                    let last = self.run.last_breakdown.as_ref();
                    let feedback = crate::game::onboarding::finale_failure_feedback(
                        round_score,
                        target_score,
                        discards_left,
                        last,
                    );
                    self.run.retry_onboarding_finale();
                    self.audio.play_sfx(audio::SfxId::GameOver);
                    let modal = Modal::new("Try Again!", &feedback, ModalTheme::Info);
                    self.modals.push(modal);
                    self.pending_scene = Some(Scene::Gameplay(GameplayScene::new()));
                    self.transition_alpha = 1.0;
                    return;
                }
                // Tutorial retry: if the tutorial is active and the player
                // hasn't reached the graduation zone, restart the current
                // blind with adaptive difficulty instead of ending the run.
                let tutorial_retry = self
                    .run
                    .tutorial
                    .as_ref()
                    .is_some_and(|t| t.is_active() && t.current_lesson < 8);
                if tutorial_retry {
                    // Capture stats before retry resets them.
                    let round_score = self.run.round_score;
                    let target_score = self.run.target_score;
                    let plays_left = self.run.plays_remaining;
                    let discards_left = self.run.discards_remaining;
                    let lesson = self
                        .run
                        .tutorial
                        .as_ref()
                        .map(|t| t.current_lesson)
                        .unwrap_or(1);

                    self.run.retry_tutorial_blind();

                    let feedback = crate::game::tutorial::failure_feedback(
                        round_score.min(u32::MAX as u64) as u32,
                        target_score,
                        plays_left,
                        discards_left,
                        lesson,
                    );
                    let modal = Modal::new("Try Again!", &feedback, ModalTheme::Success);
                    self.modals.push(modal);
                    self.pending_scene = Some(Scene::Gameplay(GameplayScene::new()));
                    self.transition_alpha = 1.0;
                    return;
                }

                // Mark tutorial as completed if the player reached graduation
                // (or finished the tutorial run regardless of outcome).
                if let Some(ref tutorial) = self.run.tutorial {
                    if tutorial.finished || tutorial.current_lesson >= 8 {
                        self.progress.tutorial_completed = true;
                    }
                }
                self.progress.runs_completed += 1;
                self.progress.record_score(self.run.round_score);
                let level_up = self.progress.check_level_up();
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                // Run is over — drop any saved-on-quit snapshot so the
                // player isn't offered "Continue" into a finished game.
                persistence::delete_saved_run(self.active_profile);

                if let Some(result) = level_up {
                    log::info!("Level up! Now level {}", result.new_level);
                    let mut pages = Vec::new();

                    // Yaku pages.
                    for yk in &result.yaku {
                        pages.push(UnlockPage {
                            category: "New Yaku".into(),
                            name: yk.name().into(),
                            description: yk.description().into(),
                            relic_id: None,
                            accent_color: render::theme::color::TWILIGHT,
                        });
                    }

                    // Relic pages.
                    let relic_defs = core::relic::all_relic_defs();
                    for rid in &result.relics {
                        if let Some(def) = relic_defs.iter().find(|d| d.id == *rid) {
                            let accent = match def.rarity {
                                core::relic::Rarity::Common => render::theme::color::rarity(0),
                                core::relic::Rarity::Uncommon => render::theme::color::rarity(1),
                                core::relic::Rarity::Rare => render::theme::color::rarity(2),
                                core::relic::Rarity::Legendary => render::theme::color::rarity(3),
                            };
                            pages.push(UnlockPage {
                                category: "New Relic".into(),
                                name: def.name.into(),
                                description: def.description.into(),
                                relic_id: Some(*rid),
                                accent_color: accent,
                            });
                        }
                    }

                    // Rule pages.
                    for rm in &result.rules {
                        pages.push(UnlockPage {
                            category: "New Rule".into(),
                            name: rm.name().into(),
                            description: rm.description().into(),
                            relic_id: None,
                            accent_color: render::theme::color::AMBER,
                        });
                    }

                    // Dora page.
                    if result.dora {
                        pages.push(UnlockPage {
                            category: "Bonus".into(),
                            name: "Dora Tiles".into(),
                            description: "Red-highlighted tiles now appear in the wall, granting bonus chips when scored.".into(),
                            relic_id: None,
                            accent_color: render::theme::color::CHAMPAGNE,
                        });
                    }

                    if !pages.is_empty() {
                        let modal = Modal::new(
                            format!("Level Up! — Level {}", result.new_level),
                            "",
                            ModalTheme::Success,
                        )
                        .with_pages(pages)
                        .with_fireworks(ww * 0.5, wh * 0.7, ww * 0.7, 8);
                        self.pending_post_game_over_modals.push(modal);
                    }
                }

                self.audio.play_sfx(audio::SfxId::GameOver);
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(
                    self.run.round_score,
                    self.run.target_score,
                )));
                self.transition_alpha = 1.0;
            }
            _ => {}
        }
    }

    fn draw(&mut self) {
        // Cache once up front so the borrow checker doesn't have to reason
        // about us calling `&self` methods while `self.renderer` is held
        // mutably below.
        let modal_active = self.modal_overlay_active();
        // The button-wipe below must only fire for *app-owned* overlays
        // (modals, tuning, sfx test). Scene-owned overlays like the pause
        // menu push their own clickable buttons through `frame.buttons`,
        // so wiping `active_buttons` for them would nuke the pause-menu
        // buttons themselves and clicks would land on nothing. Scenes are
        // responsible for suppressing their own non-overlay buttons while
        // their overlay is up (see e.g. `GameplayScene::draw_frame`).
        let app_overlay_wipe = self.modals.is_active()
            || self.debug.tuning_overlay.is_some()
            || self.debug.sfx_test_overlay.is_some()
            || self.debug.camera_debug_overlay.is_some();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(win) = self.window.as_ref() else {
            return;
        };

        let size = win.inner_size();
        let layout = self
            .layout_engine
            .solve(size.width as f32, size.height as f32);

        let ctx = DrawCtx {
            layout: &layout,
            anim: &self.anim,
            run: &self.run,
            progress: &self.progress,
            active_profile: self.active_profile,
            game_in_progress: self.run.is_in_progress(),
            proj: renderer.projections(),
            picked_gameplay_object: self
                .input
                .as_ref()
                .and_then(|i| renderer.pick_gameplay_object(i.last_cursor.0, i.last_cursor.1)),
            picked_shop_object: self
                .input
                .as_ref()
                .and_then(|i| renderer.pick_shop_object(i.last_cursor.0, i.last_cursor.1)),
            debug_visibility: scenes::DebugVisibility {
                hide_candles: self.debug.hide_candles,
                hide_blind_plaque: self.debug.hide_blind_plaque,
                hide_scoring_placard: self.debug.hide_scoring_placard,
            },
            ui_scale: self.gfx.ui_scale,
            modal_active,
        };
        // Build the scene's frame in canonical push-order. For migrated
        // scenes (gameplay) this calls their direct `draw_frame` impl;
        // for legacy scenes the default impl forwards through `draw()` +
        // `into_frame()`. Either way we get back a single ordered
        // `UiFrame.cmds` list whose push order is z-order.
        let mut frame: UiFrame = self.scene.draw_frame(ctx);

        let h = size.height as f32;
        self.debug.last_effective_camera = frame
            .camera_override
            .unwrap_or_else(|| CameraParams::default_table_camera(h));

        // Index of the last cmd produced by the scene itself, captured
        // BEFORE any modal/tuning/sfx/fps/tooltip overlay is appended
        // below. Used by the tooltip-overlay snapshot a few lines down so
        // glossary-hover scanning only sees scene content (not modal text
        // or fps debug labels).
        let scene_cmds_end = frame.cmds.len();

        win.set_title(&frame.window_title);
        self.active_buttons = frame
            .buttons
            .iter()
            .map(|b| ButtonDef {
                rect: b.rect,
                action: b.action,
            })
            .collect();

        // Click-safety wipe: if any modal-like overlay is up, scene buttons
        // must not be clickable through it. Overlays that want their own
        // clickable surface (e.g. `ModalQueue`'s full-screen dismiss button)
        // write to `active_buttons` *after* this point in their draw step.
        // See `App::modal_overlay_active` for the contract.
        if app_overlay_wipe {
            self.active_buttons.clear();
        }

        // Spawn departure animations before updating hand tiles (old data still in renderer).
        if !frame.departing_indices.is_empty() {
            let depart_lifetime = self.cascade_tuning.depart_lifetime_ms as f32 / 1000.0;
            renderer.depart_tiles(
                &frame.departing_indices,
                depart_lifetime,
                self.gfx.tile_preset,
            );
        }
        renderer.update_hand_tiles(&frame.hand_tiles);

        // Snapshot the scene's text labels and relic icons for the
        // tooltip overlay's glossary-hover scanning, by walking the
        // scene's portion of `frame.cmds` (everything pushed up to
        // `scene_cmds_end`). This works uniformly for migrated scenes
        // (which push directly into `frame.cmds`) AND legacy scenes
        // (whose `into_frame()` lands their `text_labels` / `relic_icons`
        // as `DrawCmd::Text` / `DrawCmd::RelicIcon` in the same list).
        // Walking the cmds list — instead of snapshotting separate
        // `output.text_labels` / `output.relic_icons` vecs — is what
        // makes the migration transparent to the tooltip system.
        let scene_text_labels: Vec<TextLabel> = frame.cmds[..scene_cmds_end]
            .iter()
            .filter_map(|c| {
                if let crate::render::draw_cmd::DrawCmd::Text(l) = c {
                    Some(TextLabel {
                        rect: l.rect,
                        text: l.text.clone(),
                        color: l.color,
                        font_px: l.font_px,
                        align: l.align,
                        no_glossary: l.no_glossary,
                        scroll_offset: l.scroll_offset,
                    })
                } else {
                    None
                }
            })
            .collect();
        let scene_relic_icons: Vec<crate::render::wgpu_renderer::RelicIcon> = frame.cmds
            [..scene_cmds_end]
            .iter()
            .filter_map(|c| {
                if let crate::render::draw_cmd::DrawCmd::RelicIcon(i) = c {
                    Some(crate::render::wgpu_renderer::RelicIcon {
                        rect: i.rect,
                        relic_id: i.relic_id,
                    })
                } else {
                    None
                }
            })
            .collect();
        let scene_glossary_anchors: Vec<([f32; 4], &'static str)> = frame.cmds[..scene_cmds_end]
            .iter()
            .filter_map(|c| {
                if let crate::render::draw_cmd::DrawCmd::GlossaryAnchor { rect, term } = c {
                    Some((*rect, *term))
                } else {
                    None
                }
            })
            .collect();
        // Forward the cursor position so the renderer can project it onto
        // the table plane and feed it into the volumetric smoke sim.
        frame.cursor_pos = self.input.as_ref().map(|i| i.last_cursor);

        // Apply transition alpha to everything that's part of the scene
        // (after into_frame so all scene cmds exist; before overlays are
        // appended so they fade in cleanly).
        let alpha = self.transition_alpha;
        frame.apply_alpha(alpha);

        // Overlay the shooting-star cascade effect during dramatic transitions.
        if self.transition_kind == TransitionKind::ShootingStarCascade
            && self.transition_timer > 0.0
        {
            frame.transition_progress = self.transition_timer;
            frame.shooting_star_cascade();
        }

        let size = win.inner_size();
        self.modals.update();
        if let Some((modal_insts, modal_labels, modal_buttons, modal_relic_icons)) = self
            .modals
            .draw(size.width as f32, size.height as f32, self.gfx.ui_scale)
        {
            frame.quads(modal_insts);
            frame.texts(modal_labels);
            frame.relic_icons(modal_relic_icons);
            // Replace scene buttons with modal buttons so only dismiss works.
            self.active_buttons = modal_buttons;
        }

        // Tuning overlay — on top of modals.
        if let Some(ref overlay) = self.debug.tuning_overlay {
            let (tuning_insts, tuning_labels) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(tuning_insts);
            frame.texts(tuning_labels);
            self.active_buttons.clear(); // Block scene buttons.
        }

        // SFX test overlay — on top of modals.
        if let Some(ref mut overlay) = self.debug.sfx_test_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Camera debug overlay — on top of modals.
        if let Some(ref overlay) = self.debug.camera_debug_overlay {
            // Override the scene's camera with the debug values.
            frame.camera_override = Some(overlay.to_camera_params());
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Debug visibility overlay — on top of modals.
        if let Some(ref overlay) = self.debug.visibility_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Tooltip overlay — pushed *after* modals/tuning so it sits on top
        // of all scene/modal content. Suppressed whenever any modal-like
        // overlay is up so hover effects don't leak through to elements
        // below it. See `App::modal_overlay_active` for the contract.
        let skip_tooltips = modal_active || matches!(&self.scene, Scene::Options(_));
        if !skip_tooltips {
            let cursor = self
                .input
                .as_ref()
                .map(|i| i.last_cursor)
                .unwrap_or((0.0, 0.0));
            let ww = size.width as f32;
            let wh = size.height as f32;
            let btn_rects: Vec<(f32, f32, f32, f32)> =
                self.active_buttons.iter().map(|b| b.rect).collect();
            self.tooltips.update_and_draw_into(
                &mut frame,
                cursor,
                &scene_text_labels,
                &btn_rects,
                &scene_relic_icons,
                &scene_glossary_anchors,
                ww,
                wh,
                self.gfx.ui_scale,
            );
        } else {
            self.tooltips.clear();
        }

        // FPS counter overlay (debug) — pushed last so it's always on top.
        if self.debug.show_fps {
            // Use the real frame-to-frame delta captured at the top of
            // RedrawRequested. `self.last_frame.elapsed()` would only see
            // partial CPU work done so far this frame and report inflated FPS.
            let instant_fps = 1.0 / self.last_frame_dt;
            // Exponential moving average for smooth display.
            self.debug.fps_smoothed = self.debug.fps_smoothed * 0.9 + instant_fps * 0.1;
            let w = size.width as f32;
            let h = size.height as f32;
            let label_h = (h * 0.03).max(20.0);
            let label_w = label_h * 4.0;
            let margin = label_h * 0.3;
            frame.quad(GpuInstance {
                rect: [w - label_w - margin, margin, label_w, label_h],
                color: [0.0, 0.0, 0.0, 0.55],
            });
            frame.text(TextLabel {
                rect: [w - label_w - margin, margin, label_w, label_h],
                text: format!("{:.0} FPS", self.debug.fps_smoothed),
                color: [0.9, 0.9, 0.3, 1.0],
                ..Default::default()
            });
        }

        // Debug: drop draw cmds for hidden HUD elements so we can inspect the
        // procedural 3D scene underneath. The blind plaque, scoring placard,
        // and candles are gated at the *call site* in `gameplay.rs` (via
        // `DrawCtx::debug_visibility`) because (a) the two plaques share the
        // same `DrawCmd::Plaque(_)` variant and can't be told apart by a
        // post-process filter, and (b) skipping candle pushes also skips the
        // attached `PointLight`s, which a cmd-only filter would leak. Tiles
        // and inventory items have unambiguous variants and can be safely
        // dropped after the fact.
        let any_hide = self.debug.hide_tiles || self.debug.hide_inventory;
        if any_hide {
            let hide_tiles = self.debug.hide_tiles;
            let hide_inv = self.debug.hide_inventory;
            frame.cmds.retain(|c| {
                use crate::render::draw_cmd::DrawCmd;
                if hide_tiles && matches!(c, DrawCmd::HandTileBackdrop | DrawCmd::HandTileFaces) {
                    return false;
                }
                if hide_inv
                    && matches!(
                        c,
                        DrawCmd::Dish
                            | DrawCmd::DishExplicit(_)
                            | DrawCmd::RelicBatch(_)
                            | DrawCmd::ZodiacBatch(_)
                            | DrawCmd::TalismanBatch(_)
                            | DrawCmd::RelicIcon(_)
                    )
                {
                    return false;
                }
                true
            });
        }

        // Convert settle ms to exponential decay speed (inversely proportional).
        // Default: 500ms → speed 8.0, 400ms → speed 10.0.
        let draw_settle_speed = 8.0 * (500.0 / self.cascade_tuning.draw_settle_ms.max(1) as f32);
        let sort_settle_speed = 10.0 * (400.0 / self.cascade_tuning.sort_settle_ms.max(1) as f32);

        // When a run is active, use its tile material (gameplay choice);
        // otherwise fall back to the options-screen cosmetic setting.
        let active_material = frame.tile_material_override.unwrap_or_else(|| {
            if self.run.is_in_progress() {
                self.run.mode.tile_material
            } else {
                self.gfx.tile_material
            }
        });
        if let Err(e) = renderer.render(
            &frame,
            self.gfx.smoke_intensity,
            self.gfx.smoke_detail,
            self.gfx.effects_quality,
            self.gfx.tile_preset,
            active_material,
            draw_settle_speed,
            sort_settle_speed,
            self.gfx.gamma,
            self.gfx.shadows_enabled,
            self.gfx.ssr_enabled,
        ) {
            log::error!("render: {e:?}");
        }
    }

    fn handle_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetLevel(level) => {
                // Set runs_completed to the minimum value for this level.
                let runs = match level {
                    1 => 0,
                    2 => 1,
                    3 => 3,
                    4 => 6,
                    5 => 10,
                    6 => 15,
                    7 => 20,
                    _ => 0,
                };
                self.progress.runs_completed = runs;
                self.progress.check_level_up();
                self.run.available_yaku = self.progress.available_yaku();
                self.run.available_rules = self.progress.available_rules();
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::info!(
                    "[Debug] Set player level to {} (runs_completed={})",
                    level,
                    runs
                );
            }
            DebugAction::SetGold(amount) => {
                self.run.gold = amount as i32;
                log::info!("[Debug] Set gold to {}", amount);
            }
            DebugAction::AddRelic(relic_id) => {
                if !self.run.relics.active.contains(&relic_id) {
                    if self.run.relics.is_full() {
                        // Expand capacity to fit.
                        self.run.relics.max_slots += 1;
                    }
                    self.run.relics.active.push(relic_id);
                    self.run.recompute_capacities();
                    log::info!("[Debug] Added relic {:?}", relic_id);
                } else {
                    log::info!("[Debug] Relic {:?} already active", relic_id);
                }
            }
            DebugAction::ClearRelics => {
                self.run.relics.active.clear();
                log::info!("[Debug] Cleared all relics");
            }
            DebugAction::AddTalisman(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Talisman(kind));
                log::info!("[Debug] Added talisman {:?}", kind);
            }
            DebugAction::AddZodiac(kind) => {
                use crate::core::consumable::Consumable;
                if self.run.consumables.is_full() {
                    self.run.consumables.capacity += 1;
                }
                self.run.consumables.try_push(Consumable::Zodiac(kind));
                log::info!("[Debug] Added zodiac {:?}", kind);
            }
            DebugAction::ClearConsumables => {
                self.run.consumables.items.clear();
                log::info!("[Debug] Cleared all consumables");
            }
            DebugAction::ToggleShowFps => {
                self.debug.show_fps = !self.debug.show_fps;
                log::info!("[Debug] Show FPS: {}", self.debug.show_fps);
            }
            DebugAction::OpenDebugVisibility => {
                if self.debug.visibility_overlay.is_some() {
                    self.debug.visibility_overlay = None;
                    log::info!("[Debug] Closed debug visibility overlay");
                } else {
                    self.debug.visibility_overlay = Some(DebugVisibilityOverlay::new(
                        self.debug.hide_tiles,
                        self.debug.hide_candles,
                        self.debug.hide_blind_plaque,
                        self.debug.hide_scoring_placard,
                        self.debug.hide_inventory,
                    ));
                    log::info!("[Debug] Opened debug visibility overlay");
                }
            }
            DebugAction::OpenTuning => {
                if self.debug.tuning_overlay.is_none() {
                    self.debug.tuning_overlay = Some(TuningOverlay::new(&self.cascade_tuning));
                    log::info!("[Debug] Opened cascade tuning overlay");
                }
            }
            DebugAction::OpenSfxTest => {
                if self.debug.sfx_test_overlay.is_none() {
                    self.debug.sfx_test_overlay = Some(SfxTestOverlay::new());
                    log::info!("[Debug] Opened SFX test overlay");
                }
            }
            DebugAction::OpenCameraDebug => {
                if self.debug.camera_debug_overlay.is_none() {
                    let seed = self.debug.last_effective_camera;
                    self.debug.camera_debug_overlay = Some(CameraDebugOverlay::new(&seed));
                    log::info!("[Debug] Opened camera debug overlay");
                }
            }
            DebugAction::ProfileGpu => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.start_gpu_profile(100);
                    log::info!("[Debug] GPU profile capture queued (100 frames)");
                } else {
                    log::warn!("[Debug] Cannot start GPU profile: renderer not initialised");
                }
            }
            DebugAction::BlowWindGust => {
                // Inject the same UiAction that pressing `B` would push,
                // so the gameplay scene's existing wind-trigger branch
                // picks it up on the next frame.
                self.mouse_actions.push(UiAction::DebugBlowWind);
                log::info!("[Debug] Blow wind gust queued");
            }
            DebugAction::ToggleWorldAxes => {
                // Forward to the gameplay scene's existing toggle branch
                // via the same UiAction the keyboard binding used to push.
                self.mouse_actions.push(UiAction::DebugToggleAxes);
                log::info!("[Debug] World-axes overlay toggled");
            }
            DebugAction::ArmObjectHitTest => {
                self.debug.object_hit_test_armed = !self.debug.object_hit_test_armed;
                if self.debug.object_hit_test_armed {
                    log::info!(
                        "[Debug] Object hit test ARMED — click anywhere in the world to identify the object under the cursor"
                    );
                } else {
                    log::info!("[Debug] Object hit test disarmed");
                }
            }
            DebugAction::RerollShop => {
                if let Scene::Shop(shop) = &mut self.scene {
                    shop.debug_reroll(&self.run);
                    log::info!("[Debug] Rerolled shop stock (free)");
                } else {
                    log::warn!("[Debug] Reroll Shop ignored — not in shop scene");
                }
            }
            DebugAction::OpenPack => {
                if let Scene::Shop(shop) = &mut self.scene {
                    shop.debug_open_pack(&mut self.run);
                    log::info!("[Debug] Opened tile pack celebration");
                } else {
                    log::warn!("[Debug] Open Pack ignored — not in shop scene");
                }
            }
            DebugAction::SetBoss(kind) => {
                // Replace the current ante's boss and rebuild the resolved
                // effect. resolve_upcoming_boss handles both static (wraps
                // BossDef::effect) and reactive (calls on_reveal) cases —
                // and zeros tax_collector_cost so leftover state from a
                // prior boss doesn't leak through.
                self.run.boss.upcoming = Some(kind);
                self.run.resolve_upcoming_boss();
                log::info!("[Debug] Set boss to {}", kind.name());
            }
            DebugAction::TestOverlay => {
                let modal = Modal::new(
                    "Test Overlay",
                    "This is a blank test modal.\nClick anywhere or press Enter to continue.",
                    ModalTheme::Info,
                );
                self.modals.push(modal);
                log::info!("[Debug] Spawned test overlay modal");
            }
            DebugAction::ShowVictoryScreen => {
                while self.modals.dismiss() {}
                self.pending_scene = Some(Scene::GameOver(GameOverScene::victory(
                    self.run.round_score,
                    self.run.target_score,
                )));
                self.transition_alpha = 1.0;
                log::info!("[Debug] Showing victory screen");
            }
        }
        // Request redraw to reflect changes immediately.
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let t_resumed = Instant::now();

        let mut attrs = Window::default_attributes();
        attrs.title = "Mahjuro".to_string();
        attrs.inner_size = Some(PhysicalSize::new(1920, 1080).into());

        let t0 = Instant::now();
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());
        log::info!("window created in {:?}", t0.elapsed());

        let renderer = WgpuRenderer::new(window.clone(), self.gfx.hdr_enabled).expect("wgpu");
        self.renderer = Some(renderer);

        let t0 = Instant::now();
        self.input = Some(InputState::new().expect("input"));
        self.debug.menu = Some(DebugMenuBar::new());
        log::info!("input + debug menu init in {:?}", t0.elapsed());

        log::info!("App::resumed() total: {:?}", t_resumed.elapsed());
        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if self.close_saved {
                    log::info!("CloseRequested received again — exiting immediately");
                    event_loop.exit();
                } else {
                    log::info!("CloseRequested — saving profile and exiting");
                    self.progress.record_score(self.run.round_score);
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    self.persist_run_if_in_progress();
                    self.close_saved = true;
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // Advance animation clock once per presented frame. Doing this
                // at the top of `window_event` instead would tick animations on
                // every input event (CursorMoved fires faster than vsync), so
                // the game would effectively run faster than the monitor can
                // render. RedrawRequested is gated by the Fifo presenter, which
                // blocks at vsync, so this caps the tick to refresh rate.
                let now = Instant::now();
                self.last_frame_dt = now
                    .saturating_duration_since(self.last_frame)
                    .as_secs_f32()
                    .max(0.0001);
                self.last_frame = now;
                self.anim.update(now);

                // 1. Drain event bus — bus events can trigger scene transitions.
                for ev in self.bus.drain() {
                    match ev {
                        GameEvent::TileDrawn(_) => {
                            self.audio.play_sfx(audio::SfxId::TilePlace);
                        }
                        GameEvent::TileDiscarded { .. } => {
                            self.audio.play_sfx(audio::SfxId::TileDiscard);
                        }
                        GameEvent::ScoreUpdated(_) => {
                            self.audio.play_sfx(audio::SfxId::ScoreReveal);
                        }
                        GameEvent::ScoreStepRevealed { index } => {
                            // Cycle three pre-recorded tick pitches per
                            // step so the cascade audibly climbs through
                            // its reveal sequence (rodio doesn't support
                            // runtime pitch shifting). Layer the existing
                            // ScoreStep "rollover" sound on top to keep the
                            // soft confirmation that's already wired into
                            // the game.
                            let tick = match index % 3 {
                                0 => audio::SfxId::ScoreTickA,
                                1 => audio::SfxId::ScoreTickB,
                                _ => audio::SfxId::ScoreTickC,
                            };
                            self.audio.play_sfx(tick);
                            self.audio.play_sfx(audio::SfxId::ScoreStep);
                        }
                        GameEvent::ScoreCascadeFinal => {
                            // Crescendo: brassy hit jingle layered over the
                            // existing confirmation sting so the closing
                            // beat lands with weight.
                            self.audio.play_sfx(audio::SfxId::ScoreFinal);
                            self.audio.play_sfx(audio::SfxId::ScoreCrescendo);
                        }
                        GameEvent::GoldChanged { .. } => {
                            self.audio.play_sfx(audio::SfxId::CoinDrop);
                        }
                        ev @ GameEvent::RoundComplete { .. } => {
                            // Hold the win sting + scene transition until the
                            // scoring cascade has finished playing out — the
                            // player should get to watch the winning hand pop.
                            self.deferred_round_end = Some(ev);
                        }
                        ev @ GameEvent::GameOver { .. } => {
                            // Same as RoundComplete: hold until the final
                            // cascade has finished animating.
                            self.deferred_round_end = Some(ev);
                        }
                        GameEvent::PackBought => {
                            self.audio.play_sfx(audio::SfxId::PackBuy);
                        }
                        GameEvent::PackOpened => {
                            self.audio.play_sfx(audio::SfxId::PackOpen);
                        }
                        GameEvent::PackTileRevealed => {
                            self.audio.play_sfx(audio::SfxId::PackTileReveal);
                        }
                        GameEvent::ZodiacReveal => {
                            self.audio.play_sfx(audio::SfxId::ZodiacReveal);
                        }
                        GameEvent::ZodiacLevelUp => {
                            self.audio.play_sfx(audio::SfxId::ZodiacLevelUp);
                        }
                        GameEvent::CandleFlare => {
                            self.audio.play_sfx(audio::SfxId::CandleFlareWhoosh);
                            self.audio.play_sfx(audio::SfxId::CandleFlareImpact);
                        }
                        GameEvent::StructureCommitted => {
                            self.audio.play_sfx(audio::SfxId::StructureCommit);
                        }
                        GameEvent::InvalidAction => {
                            self.audio.play_sfx(audio::SfxId::InvalidAction);
                        }
                        GameEvent::UiSound(id) => {
                            self.audio.play_sfx(id);
                        }
                        GameEvent::TutorialMilestone(milestone) => {
                            use crate::game::tutorial::TutorialMilestone;
                            let (title, body) = match milestone {
                                TutorialMilestone::FirstPair => (
                                    "First Pair!",
                                    "Two matching tiles \u{2014} the foundation of every hand.",
                                ),
                                TutorialMilestone::FirstTriplet => (
                                    "First Triplet!",
                                    "Three of a kind scores big chips. Keep building!",
                                ),
                                TutorialMilestone::FirstSequence => (
                                    "First Sequence!",
                                    "Three in a row, same suit. Smooth and versatile.",
                                ),
                                TutorialMilestone::FirstDiscard => {
                                    ("First Discard!", "Out with the old, in with the new.")
                                }
                                TutorialMilestone::FirstFullHand => (
                                    "First Full Hand!",
                                    "4 melds + 1 pair \u{2014} the ultimate yaku. Huge multiplier!",
                                ),
                                TutorialMilestone::FirstShopBuy => (
                                    "First Relic!",
                                    "Relics power up your scoring for the rest of the run.",
                                ),
                                TutorialMilestone::FirstTrigger => (
                                    "First Trigger!",
                                    "Structure cashed in \u{2014} watch the chip and mult piles build, then bank more melds to multiply your score!",
                                ),
                            };
                            let win_size = self
                                .window
                                .as_ref()
                                .map(|w| w.inner_size())
                                .unwrap_or(PhysicalSize::new(800, 600));
                            let ww = win_size.width as f32;
                            let wh = win_size.height as f32;
                            let modal = crate::ui::modal::Modal::new(
                                title,
                                body,
                                crate::ui::modal::ModalTheme::Success,
                            )
                            .with_fireworks(
                                ww * 0.5,
                                wh * 0.8,
                                ww * 0.5,
                                3,
                            );
                            self.modals.push(modal);
                            self.audio.play_sfx(audio::SfxId::ScoreFinal);
                        }
                        GameEvent::RelicActivated(_) => {
                            // Visual feedback (glow + wiggle) is handled by the
                            // active scene; audio is a soft chime layered on top.
                            self.audio.play_sfx(audio::SfxId::ScoreStep);
                        }
                        other => log::info!("event: {other:?}"),
                    }
                }

                // 1a. Poll background update check.
                if let Some(result) = self.update_checker.poll() {
                    let modal = match result {
                        update_check::UpdateResult::Updated { new_version } => Modal::new(
                            "Updated!",
                            format!("v{new_version} installed.\nRestart to use the new version."),
                            ModalTheme::Info,
                        ),
                        update_check::UpdateResult::UpdateFailed {
                            new_version,
                            release_url,
                            error,
                        } => {
                            log::warn!("auto-update to v{new_version} failed: {error}");
                            Modal::new(
                                "Update Available",
                                format!(
                                    "v{new_version} is available but auto-update failed.\n\n{release_url}"
                                ),
                                ModalTheme::Info,
                            )
                        }
                    };
                    self.modals.push(modal);
                }

                // 1b. Poll debug menu actions.
                if let Some(ref debug_menu) = self.debug.menu {
                    for action in debug_menu.poll() {
                        self.handle_debug_action(action);
                    }
                }

                // 2. Collect input actions.
                let mut actions = Vec::new();
                let mut button_clicks: Vec<u32> = Vec::new();
                button_clicks.append(&mut self.mouse_button_clicks);
                let mut hide_cursor = false;
                if let Some(input) = self.input.as_mut() {
                    if input.poll_gamepads(&mut actions) {
                        hide_cursor = true;
                    }
                    actions.append(&mut self.mouse_actions);

                    let size = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or(PhysicalSize::new(800, 600));
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    // Hit-test by raycasting from the camera through the
                    // cursor against each tile's OBB (last-frame snapshot).
                    // We feed `update_pointer_hover` synthetic slots so only
                    // the picked tile contains the cursor — the rest are
                    // collapsed off-screen so they can't compete.
                    let mut slots: Vec<(f32, f32, f32, f32)> = layout
                        .hand_slots
                        .iter()
                        .map(|_| (-9999.0, -9999.0, 0.0, 0.0))
                        .collect();
                    let picked = self
                        .renderer
                        .as_ref()
                        .and_then(|r| r.pick_hand_tile(input.last_cursor.0, input.last_cursor.1));
                    if let Some(idx) = picked {
                        if let Some(s) = slots.get_mut(idx) {
                            *s = (
                                input.last_cursor.0 - 1.0,
                                input.last_cursor.1 - 1.0,
                                2.0,
                                2.0,
                            );
                        }
                    }
                    input.update_pointer_hover(input.last_cursor, &slots);

                    // 3. Update focus slot (App-level, shared across scenes).
                    for a in &actions {
                        match a {
                            UiAction::FocusNext => {
                                input.focus_slot = (input.focus_slot + 1)
                                    .min(game::run::HAND_SIZE.saturating_sub(1));
                            }
                            UiAction::FocusPrev => {
                                input.focus_slot = input.focus_slot.saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                }

                if hide_cursor {
                    if let Some(w) = self.window.as_ref() {
                        w.set_cursor_visible(false);
                    }
                }

                // 3b. If the tuning overlay is open, intercept input.
                if let Some(ref mut overlay) = self.debug.tuning_overlay {
                    match overlay.update(&actions) {
                        TuningResult::Stay => {
                            // Apply live tuning changes.
                            self.cascade_tuning = overlay.tuning.clone();
                        }
                        TuningResult::Close => {
                            // Apply final tuning and close.
                            self.cascade_tuning = overlay.tuning.clone();
                            self.debug.tuning_overlay = None;
                            log::info!("[Debug] Closed cascade tuning overlay");
                        }
                        TuningResult::Export => {
                            let json = serde_json::to_string_pretty(&overlay.tuning)
                                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                            let path = "cascade_tuning.json";
                            match std::fs::write(path, &json) {
                                Ok(()) => log::info!("[Debug] Exported tuning to {path}"),
                                Err(e) => log::error!("[Debug] Failed to export tuning: {e}"),
                            }
                        }
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b'. If the SFX test overlay is open, intercept input.
                if let Some(mut overlay) = self.debug.sfx_test_overlay.take() {
                    let mouse = self.input.as_ref().map(|i| {
                        let (mx, my) = i.last_cursor;
                        (mx, my, self.mouse_clicked)
                    });
                    let close = overlay.update(&actions, &self.audio, mouse);
                    self.mouse_clicked = false;
                    if !close {
                        self.debug.sfx_test_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed SFX test overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b'''. If the camera debug overlay is open, intercept input.
                if let Some(mut overlay) = self.debug.camera_debug_overlay.take() {
                    let wh = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size().height as f32)
                        .unwrap_or(800.0);
                    let close = overlay.update(&actions, wh);
                    if !close {
                        self.debug.camera_debug_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed camera debug overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3b''. If the debug visibility overlay is open, intercept
                // input. Mirror the toggle state back to App fields each
                // frame so the gameplay scene + retain filter pick up live
                // changes immediately.
                if let Some(mut overlay) = self.debug.visibility_overlay.take() {
                    let result = overlay.update(&actions);
                    self.debug.hide_tiles = overlay.hide_tiles;
                    self.debug.hide_candles = overlay.hide_candles;
                    self.debug.hide_blind_plaque = overlay.hide_blind_plaque;
                    self.debug.hide_scoring_placard = overlay.hide_scoring_placard;
                    self.debug.hide_inventory = overlay.hide_inventory;
                    if result == DebugVisResult::Stay {
                        self.debug.visibility_overlay = Some(overlay);
                    } else {
                        log::info!("[Debug] Closed debug visibility overlay");
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3c. If a modal is active, intercept input.
                if self.modals.is_active() {
                    for a in &actions {
                        match a {
                            UiAction::Confirm => {
                                self.modals.advance_page();
                                break;
                            }
                            UiAction::Cancel => {
                                self.modals.dismiss();
                                break;
                            }
                            UiAction::FocusNext => {
                                self.modals.navigate(1);
                                break;
                            }
                            UiAction::FocusPrev => {
                                self.modals.navigate(-1);
                                break;
                            }
                            _ => {}
                        }
                    }
                    // Block all actions from reaching the scene.
                    actions.clear();
                    button_clicks.clear();
                }

                // Clear one-shot mouse click flag so it doesn't bleed into
                // the next frame if no overlay consumed it.
                self.mouse_clicked = false;

                // 4. Delegate actions to the active scene.
                let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
                let win_size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or(PhysicalSize::new(800, 600));
                let update_layout = self
                    .layout_engine
                    .solve(win_size.width as f32, win_size.height as f32);
                let mut quit_requested = false;
                let mut switch_profile_req: Option<usize> = None;
                let mut delete_profile_req: Option<usize> = None;
                let mut complete_onboarding = false;
                let cursor_pos = self
                    .input
                    .as_ref()
                    .map(|i| i.last_cursor)
                    .unwrap_or((0.0, 0.0));
                let loading_done = self.renderer.as_ref().map_or(true, |r| !r.is_loading());
                let picked_shop_object = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_shop_object(cursor_pos.0, cursor_pos.1));
                let picked_gameplay_object = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_gameplay_object(cursor_pos.0, cursor_pos.1));
                let picked_hand_tile_for_update = self
                    .renderer
                    .as_ref()
                    .and_then(|r| r.pick_hand_tile(cursor_pos.0, cursor_pos.1));
                let scroll_lines = std::mem::take(&mut self.scroll_delta);
                let ctx = UpdateCtx {
                    actions: &actions,
                    button_clicks: &button_clicks,
                    run: &mut self.run,
                    bus: &mut self.bus,
                    anim: &mut self.anim,
                    layout: &update_layout,
                    focus_tile_index: focus,
                    quit_requested: &mut quit_requested,
                    switch_profile: &mut switch_profile_req,
                    delete_profile: &mut delete_profile_req,
                    complete_onboarding: &mut complete_onboarding,
                    cursor_pos,
                    loading_done,
                    cascade_tuning: &self.cascade_tuning,
                    picked_shop_object,
                    picked_gameplay_object,
                    input_mode: self
                        .input
                        .as_ref()
                        .map(|i| i.mode)
                        .unwrap_or(crate::ui::input::InputMode::Cursor),
                    picked_hand_tile: picked_hand_tile_for_update,
                    scroll_lines,
                    ui_scale: self.gfx.ui_scale,
                    tutorial_eligible: self.progress.runs_completed == 0
                        && !self.progress.tutorial_completed,
                    multiple_materials: self.progress.plastic_unlocked(),
                    transitioning: self.pending_scene.is_some(),
                };
                if let Some(mut next_scene) = self.scene.update(ctx) {
                    // When transitioning *to* the Meld Guide from a game scene,
                    // suspend the current scene so we can restore it later.
                    if matches!(next_scene, Scene::MeldGuide(_))
                        && !matches!(self.scene, Scene::StartScreen(_))
                    {
                        let old = std::mem::replace(
                            &mut self.scene,
                            Scene::Splash(scenes::SplashScene::new()), // placeholder
                        );
                        self.suspended_scene = Some(old);
                    }
                    // When leaving the Meld Guide, restore the suspended scene
                    // instead of going where the guide wanted (start screen).
                    if matches!(self.scene, Scene::MeldGuide(_)) {
                        if let Some(restored) = self.suspended_scene.take() {
                            next_scene = restored;
                        }
                    }
                    // Choose transition style: dramatic cascade for
                    // new-game flows, quick fade for everything else.
                    let use_cascade = matches!(
                        (&self.scene, &next_scene),
                        (Scene::StartScreen(_), Scene::TileSelect(_))
                            | (Scene::StartScreen(_), Scene::Shop(_))
                            | (Scene::TileSelect(_), Scene::Shop(_))
                            | (Scene::TileSelect(_), Scene::TutorialCampaign(_))
                    );
                    if use_cascade {
                        self.transition_kind = TransitionKind::ShootingStarCascade;
                        self.transition_speed = 0.012;
                    } else {
                        self.transition_kind = TransitionKind::Quick;
                        self.transition_speed = 0.08;
                    }
                    self.transition_timer = 0.0;
                    // Start fade-out transition.
                    self.pending_scene = Some(next_scene);
                    self.transition_alpha = 1.0;
                }

                if complete_onboarding {
                    self.progress.tutorial_completed = true;
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                }

                // Sync live audio/graphics settings whenever the player has
                // an options menu open — either the standalone Options scene
                // (from the start screen) or the embedded options overlay
                // inside the in-game pause menu.
                let active_options_overlay = match &self.scene {
                    // Standalone Options scene IS the options screen, so its
                    // own state is what we sync. Every other scene defers to
                    // its `SceneBehavior::pause_options_overlay()` (default
                    // `None` for scenes without an embedded pause menu).
                    Scene::Options(opts) => Some(opts),
                    other => other.pause_options_overlay(),
                };
                if let Some(opts) = active_options_overlay {
                    self.audio.set_master_volume(opts.master_volume);
                    self.audio.set_sfx_volume(opts.sfx_volume);
                    self.audio.set_music_volume(opts.music_volume);
                    self.audio.set_enabled(opts.sfx_enabled);
                    self.gfx.smoke_intensity = opts.smoke_intensity;
                    self.gfx.smoke_detail = opts.smoke_detail;
                    self.gfx.effects_quality = opts.effects_quality;
                    self.gfx.tile_preset = opts.tile_preset;
                    self.gfx.tile_material = opts.tile_material;
                    self.gfx.gamma = opts.gamma;
                    self.gfx.shadows_enabled = opts.shadows_enabled;
                    self.gfx.ssr_enabled = opts.ssr_enabled;
                    self.gfx.hdr_enabled = opts.hdr_enabled;
                    self.gfx.ui_scale = opts.ui_scale;
                    self.run
                        .set_auto_cash_in_on_full_structure(opts.auto_cash_in_on_full_structure);
                    if let Some(ref mut input) = self.input {
                        input.swap_ab = opts.swap_ab;
                    }
                }

                // Handle profile switch request.
                if let Some(idx) = switch_profile_req {
                    let new_idx = if idx == usize::MAX {
                        // Previous profile (wrapping), from start screen arrows.
                        (self.active_profile + 3 - 1) % 3
                    } else if idx == usize::MAX - 1 {
                        // Next profile (wrapping), from start screen arrows.
                        (self.active_profile + 1) % 3
                    } else {
                        // Absolute index, from profile select scene.
                        idx.min(2)
                    };
                    if new_idx != self.active_profile {
                        self.switch_profile(new_idx);
                    }
                }

                // Handle profile delete request.
                if let Some(idx) = delete_profile_req {
                    let idx = idx.min(2);
                    persistence::delete_profile(idx);
                    // If we just deleted the active profile, reload it (now
                    // returns a fresh default since the file is gone).
                    if idx == self.active_profile {
                        self.progress = persistence::load_profile(idx);
                        self.run = persistence::load_run(idx)
                            .unwrap_or_else(crate::game::run::RunState::new_demo);
                        self.run.available_yaku = self.progress.available_yaku();
                        self.run.available_rules = self.progress.available_rules();
                    }
                }

                // If we deferred a round-end event so the player could watch
                // the scoring cascade play out, fire it now that the gameplay
                // scene has gone idle.
                if self.deferred_round_end.is_some() {
                    let cascade_done = match &self.scene {
                        Scene::Gameplay(g) => !g.is_animating(),
                        _ => true,
                    };
                    if cascade_done {
                        if let Some(ev) = self.deferred_round_end.take() {
                            self.handle_round_end_event(ev);
                        }
                    }
                }

                // Advance transition animation using the animation controller.
                // Pause the transition while a modal is active so the player
                // must dismiss milestone / celebration modals before the scene
                // change proceeds (e.g. "First Pair!" before the recap screen).
                if self.pending_scene.is_some() && !self.modals.is_active() {
                    self.transition_alpha -= self.transition_speed;
                    // Map alpha 1→0 onto timer 0→0.5 (first half of transition).
                    self.transition_timer =
                        (1.0 - self.transition_alpha.max(0.0)).clamp(0.0, 1.0) * 0.5;
                    if self.transition_alpha <= 0.0 {
                        self.transition_alpha = 0.0;
                        if let Some(next) = self.pending_scene.take() {
                            // If we're transitioning out of the GameOver scene,
                            // surface any deferred celebration modals now.
                            if matches!(self.scene, Scene::GameOver(_))
                                && !self.pending_post_game_over_modals.is_empty()
                            {
                                for modal in self.pending_post_game_over_modals.drain(..) {
                                    self.modals.push(modal);
                                }
                            }
                            // Clear residual smoke when entering the shop
                            // or the shrine-select screen so the new scene
                            // starts with a clean atmosphere.
                            let clear_smoke = matches!(
                                (&self.scene, &next),
                                (Scene::TileSelect(_), Scene::Shop(_))
                                    | (Scene::TutorialCampaign(_), Scene::Shop(_))
                                    | (Scene::Shop(_), Scene::PickBlind(_))
                            );
                            self.scene = next;
                            if clear_smoke {
                                if let Some(r) = self.renderer.as_mut() {
                                    r.clear_smoke();
                                }
                            }
                            if let Some(input) = self.input.as_mut() {
                                input.focus_slot = 0;
                            }
                            // Fade score panel in for the new scene.
                            self.anim
                                .fade(render::animation::ENTITY_SCORE_PANEL, 0.0, 1.0, 300);
                            // Slide hand strip up from below.
                            self.anim.slide_to(
                                render::animation::ENTITY_HAND_STRIP,
                                0.0,
                                -20.0,
                                400,
                            );
                        }
                    }
                } else if self.transition_alpha < 1.0 {
                    self.transition_alpha =
                        (self.transition_alpha + self.transition_speed).min(1.0);
                    // Map alpha 0→1 onto timer 0.5→1.0 (second half).
                    self.transition_timer = 0.5 + (self.transition_alpha.clamp(0.0, 1.0)) * 0.5;
                    // Reset transition kind once fully faded in.
                    if self.transition_alpha >= 1.0 {
                        self.transition_timer = 0.0;
                        self.transition_kind = TransitionKind::Quick;
                        self.transition_speed = 0.08;
                    }
                }

                // Handle quit request from scene.
                if quit_requested {
                    self.quit_requested = true;
                }

                // Cursor → smoke impulses are now injected by the renderer
                // itself (it has the gameplay camera matrices required to
                // unproject the cursor onto the table plane).
                self.draw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let cursor = self
                        .input
                        .as_ref()
                        .map(|i| i.last_cursor)
                        .unwrap_or((0.0, 0.0));

                    if state == ElementState::Pressed {
                        self.mouse_clicked = true;

                        // Debug "Object Hit Test" one-shot picker. If armed,
                        // consume this click: hit-test the cursor against
                        // every known scene object and log the match. Skip
                        // all the normal click dispatch (buttons, tiles,
                        // drag) so the click can't accidentally fire a
                        // gameplay action while we're just probing.
                        if self.debug.object_hit_test_armed {
                            self.debug.object_hit_test_armed = false;
                            let name = self
                                .renderer
                                .as_ref()
                                .and_then(|r| r.pick_debug_object(cursor.0, cursor.1));
                            match name {
                                Some(n) => log::info!(
                                    "[Debug] Object hit test: {n} at ({:.0}, {:.0})",
                                    cursor.0,
                                    cursor.1
                                ),
                                None => log::info!(
                                    "[Debug] Object hit test: (no object) at ({:.0}, {:.0})",
                                    cursor.0,
                                    cursor.1
                                ),
                            }
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                            return;
                        }

                        // Check if click hit any button.
                        let mut hit = false;
                        for btn in &self.active_buttons {
                            let (bx, by, bw, bh) = btn.rect;
                            if cursor.0 >= bx
                                && cursor.0 <= bx + bw
                                && cursor.1 >= by
                                && cursor.1 <= by + bh
                            {
                                match btn.action {
                                    ButtonAction::Ui(a) => self.mouse_actions.push(a),
                                    ButtonAction::Scene(id) => self.mouse_button_clicks.push(id),
                                }
                                hit = true;
                                break;
                            }
                        }
                        if !hit {
                            // Check if we're clicking on a hand tile to start drag.
                            let clicked_relic_slot = self.gameplay_relic_slot_at_cursor(cursor);
                            if let Some(input) = self.input.as_mut() {
                                if let Some(slot) = input.pointer_slot {
                                    input.drag = Some(ui::input::DragState {
                                        subject: ui::input::DragSubject::HandTile,
                                        from_slot: slot,
                                        start_pos: cursor,
                                        current_pos: cursor,
                                    });
                                    // Only confirm (toggle-select tile) if a hand tile was clicked.
                                    self.mouse_actions.push(UiAction::Confirm);
                                } else if let Some(slot) = clicked_relic_slot {
                                    input.drag = Some(ui::input::DragState {
                                        subject: ui::input::DragSubject::Relic,
                                        from_slot: slot,
                                        start_pos: cursor,
                                        current_pos: cursor,
                                    });
                                }
                            }
                        }
                    } else if state == ElementState::Released {
                        // End drag — swap tiles if dropped on a different slot.
                        // Require minimum drag distance to avoid accidental swaps.
                        let dropped_relic_slot = self.gameplay_relic_slot_at_cursor(cursor);
                        if let Some(input) = self.input.as_mut() {
                            if let Some(drag) = input.drag.take() {
                                let dx = cursor.0 - drag.start_pos.0;
                                let dy = cursor.1 - drag.start_pos.1;
                                let dist = (dx * dx + dy * dy).sqrt();
                                if dist > 10.0 {
                                    match drag.subject {
                                        ui::input::DragSubject::HandTile => {
                                            if let Some(target_slot) = input.pointer_slot {
                                                if target_slot != drag.from_slot {
                                                    self.run
                                                        .swap_tiles(drag.from_slot, target_slot);
                                                }
                                            }
                                        }
                                        ui::input::DragSubject::Relic => {
                                            if let Some(target_slot) = dropped_relic_slot {
                                                if target_slot != drag.from_slot {
                                                    self.run
                                                        .relics
                                                        .swap_relics(drag.from_slot, target_slot);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(input), Some(win)) = (self.input.as_mut(), self.window.as_ref()) {
                    let was_hidden = input.mode != InputMode::Cursor;
                    input.mode = InputMode::Cursor;
                    input.last_cursor = (position.x as f32, position.y as f32);
                    let size = win.inner_size();
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    // Same raycast-based pick as the per-frame update path.
                    let mut slots: Vec<(f32, f32, f32, f32)> = layout
                        .hand_slots
                        .iter()
                        .map(|_| (-9999.0, -9999.0, 0.0, 0.0))
                        .collect();
                    let picked = self
                        .renderer
                        .as_ref()
                        .and_then(|r| r.pick_hand_tile(input.last_cursor.0, input.last_cursor.1));
                    if let Some(idx) = picked {
                        if let Some(s) = slots.get_mut(idx) {
                            *s = (
                                input.last_cursor.0 - 1.0,
                                input.last_cursor.1 - 1.0,
                                2.0,
                                2.0,
                            );
                        }
                    }
                    input.update_pointer_hover(input.last_cursor, &slots);
                    // Update drag position if dragging.
                    if let Some(ref mut drag) = input.drag {
                        drag.current_pos = input.last_cursor;
                    }
                    if was_hidden {
                        win.set_cursor_visible(true);
                    }
                    win.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        // Convert pixel delta to approximate line units.
                        (pos.y as f32) / 40.0
                    }
                };
                self.scroll_delta += lines;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.wants_fullscreen_shortcut(&event) {
                    self.toggle_fullscreen();
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                } else if event.state == ElementState::Pressed {
                    let mut v = Vec::new();
                    let mode_changed = if let Some(input) = self.input.as_mut() {
                        input.on_key(event.physical_key, &mut v)
                    } else {
                        false
                    };
                    self.mouse_actions.extend(v);
                    if mode_changed {
                        if let Some(w) = self.window.as_ref() {
                            w.set_cursor_visible(false);
                        }
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.quit_requested {
            let _ = persistence::save_profile(self.active_profile, &self.progress);
            let _ = persistence::save_settings(&persistence::load_settings());
            self.persist_run_if_in_progress();
            _event_loop.exit();
            return;
        }
        let cascade_active = matches!(&self.scene, Scene::Gameplay(g) if g.is_animating());
        let collection_3d = matches!(&self.scene, Scene::Collection(c) if c.has_3d_tab());
        let transitioning = self.pending_scene.is_some() || self.transition_alpha < 1.0;
        let needs_redraw = !self.anim.is_idle()
            || self
                .renderer
                .as_ref()
                .map(|r| r.is_spinning())
                .unwrap_or(false)
            || cascade_active
            || collection_3d
            || transitioning
            || self.modals.needs_redraw()
            || self.gfx.smoke_intensity != crate::persistence::SmokeIntensity::Off
            || self.tooltips.is_active();
        if needs_redraw {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    crash_guard::install();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    // Headless tuning subcommands. Examples:
    //   mahjuro bot
    //   mahjuro bot 200 --base-target 250 --target-scale 1.3 --plays 5
    //   mahjuro bot --export-json /tmp/bot.json
    //   mahjuro bot-graph 10000 --slug baseline_10k --label "Baseline\n(10k runs)"
    //   mahjuro sweep
    //   mahjuro sweep --runs 50 --export-json /tmp/sweep.json
    match cli.command {
        Some(Command::Sweep(sweep)) => {
            let bases: &[u32] = &[200, 250, 300, 350];
            let scales: &[f32] = &[1.20, 1.30, 1.40, 1.50];
            let plays: &[u32] = &[4, 5];
            bot::run_sweep(
                sweep.runs,
                bases,
                scales,
                plays,
                sweep.export_json.as_deref(),
            );
            return Ok(());
        }
        Some(Command::Bot(bot_cli)) => {
            bot::run_headless(
                bot_cli.runs,
                bot_cli.bot_config(),
                bot::BotRunOptions {
                    log: bot_cli.bot_log,
                    export_json: bot_cli.export_json.clone(),
                },
            );
            return Ok(());
        }
        Some(Command::BotGraph(bot_graph)) => {
            let config = bot_graph.bot_config();
            let mode = config.clone().into_mode();
            let slug = bot_graph
                .slug
                .clone()
                .unwrap_or_else(|| default_snapshot_slug(&mode, bot_graph.runs));
            let label = bot_graph
                .label
                .clone()
                .unwrap_or_else(|| default_snapshot_label(&mode, bot_graph.runs))
                .replace("\\n", "\n");
            let agg = bot::run_headless_aggregate(
                bot_graph.runs,
                config,
                bot::BotRunOptions {
                    log: bot_graph.bot_log,
                    export_json: None,
                },
            );
            agg.print_summary();

            let snapshot = build_bot_graph_snapshot(&agg, slug.clone(), label);
            let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let snapshot_path = repo_root.join("docs").join("bot_balance_runs.json");
            upsert_snapshot(&snapshot_path, snapshot)?;
            println!(
                "updated bot graph snapshot '{}' in {}",
                slug,
                snapshot_path.display()
            );
            render_bot_graphs(repo_root)?;
            println!("regenerated docs bot balance graphs");
            return Ok(());
        }
        None => {}
    }

    asset_path::log_all_assets();

    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
            let event_loop = EventLoop::new()?;
            event_loop.set_control_flow(ControlFlow::Poll);

            let mut app = App::new();
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
