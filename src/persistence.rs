//! Save / load `PlayerProgress` as JSON — supports up to 3 profiles.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub use mahjuro_gfx_types::{
    EffectsQuality, GraphicsMode, ShadowQuality, TileMaterial, TilePreset, clear_tuning_override,
    has_tuning_override, load_tuning_override, save_tuning_override,
};

use crate::core::progression::{PlayerProgress, RunOutcome};
use crate::game::run::RunState;
use crate::ui::button_prompts::GamepadStyle;

/// Which controller icon atlas to use for on-screen button prompts (Kenney).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlyphPromptSetting {
    /// Match the first connected gamepad (vendor / name heuristics).
    #[default]
    Auto,
    Xbox,
    PlayStation,
    Nintendo,
    NintendoSwitch2,
    SteamDeck,
    SteamController,
    Generic,
}

impl GlyphPromptSetting {
    const ORDER: &[Self] = &[
        Self::Auto,
        Self::Xbox,
        Self::PlayStation,
        Self::Nintendo,
        Self::NintendoSwitch2,
        Self::SteamDeck,
        Self::SteamController,
        Self::Generic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Xbox => "Xbox",
            Self::PlayStation => "PlayStation",
            Self::Nintendo => "Switch",
            Self::NintendoSwitch2 => "Switch 2",
            Self::SteamDeck => "Steam Deck",
            Self::SteamController => "Steam Controller",
            Self::Generic => "Generic",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|&x| x == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|&x| x == self).unwrap_or(0);
        let n = Self::ORDER.len();
        Self::ORDER[(i + n - 1) % n]
    }

    pub fn resolve(self, detected: GamepadStyle) -> GamepadStyle {
        match self {
            Self::Auto => detected,
            Self::Xbox => GamepadStyle::Xbox,
            Self::PlayStation => GamepadStyle::PlayStation,
            Self::Nintendo => GamepadStyle::Nintendo,
            Self::NintendoSwitch2 => GamepadStyle::NintendoSwitch2,
            Self::SteamDeck => GamepadStyle::SteamDeck,
            Self::SteamController => GamepadStyle::SteamController,
            Self::Generic => GamepadStyle::Generic,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ResumeScene {
    #[default]
    Gameplay,
    Shop,
    #[serde(alias = "PickBlind", alias = "PickChamber")]
    Hallway,
}

#[derive(Debug)]
pub struct LoadedRun {
    pub run: RunState,
    pub scene: ResumeScene,
}

const MAX_PROFILES: usize = 3;
const SETTINGS_NAME: &str = "settings.json";

/// Returns the directory where save data lives, creating it if needed.
/// Falls back to the current directory if the platform config dir is
/// unavailable or can't be created.
fn data_dir() -> PathBuf {
    let dir = {
        #[cfg(any(feature = "game", feature = "headless-screenshot"))]
        {
            mahjuro_distribution::PlatformPaths::data_root()
        }
        #[cfg(not(any(feature = "game", feature = "headless-screenshot")))]
        {
            dirs::config_dir()
                .map(|base| base.join("Mahjuro"))
                .unwrap_or_else(|| PathBuf::from("."))
        }
    };
    if fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    PathBuf::from(".")
}

fn default_graphics_mode() -> GraphicsMode {
    GraphicsMode::Visuals
}

fn default_effects_quality() -> EffectsQuality {
    EffectsQuality::High
}

fn default_tile_preset() -> TilePreset {
    TilePreset::Chinese
}

fn default_tile_material() -> TileMaterial {
    TileMaterial::Bamboo
}

fn default_tileset_name() -> String {
    "original".to_string()
}

/// Persistent settings (which profile is active, audio prefs, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub active_profile: usize,
    #[serde(default = "default_volume")]
    pub master_volume: f32,
    #[serde(default = "default_volume")]
    pub sfx_volume: f32,
    #[serde(default = "default_volume")]
    pub music_volume: f32,
    #[serde(default = "default_true")]
    pub sfx_enabled: bool,
    #[serde(default = "default_effects_quality")]
    pub effects_quality: EffectsQuality,
    #[serde(default = "default_tile_preset")]
    pub tile_preset: TilePreset,
    #[serde(default = "default_tile_material")]
    pub tile_material: TileMaterial,
    #[serde(default = "default_tileset_name")]
    pub tileset_name: String,
    #[serde(default = "default_gamma")]
    pub gamma: f32,
    #[serde(default = "default_graphics_mode")]
    pub graphics_mode: GraphicsMode,
    /// When false, the first launch may apply [`GraphicsMode::suggest_for_adapter`].
    #[serde(default)]
    pub graphics_mode_user_set: bool,
    #[serde(default)]
    pub hdr_enabled: bool,
    /// Master kill-switch for the per-scene VHS overlay. Defaults to `true`
    /// so the per-scene Tonemap debug overlay's amounts apply directly;
    /// per-scene amounts of 0 already short-circuit the shader, so this
    /// toggle is mainly for an "everything off" emergency exit. Not yet
    /// surfaced in the Options scene — flip via `app_settings.json`.
    #[serde(default = "default_true")]
    pub vhs_enabled: bool,
    /// Borderless fullscreen when true; windowed (resizable) when false.
    #[serde(default = "default_true")]
    pub borderless_fullscreen: bool,
    /// On-screen controller button art: Auto follows hardware, or force a family.
    #[serde(default)]
    pub glyph_prompt: GlyphPromptSetting,
    #[serde(default)]
    pub swap_ab: bool,
    #[serde(default)]
    pub swap_xy: bool,
    /// True once the player has manually toggled `swap_ab`/`swap_xy` in Options.
    /// Until then, the input layer auto-applies sensible defaults the first
    /// time it sees a real controller this process (Nintendo → both ON, every
    /// other style → both OFF). Once the player takes control, that auto-apply
    /// is suppressed for life.
    #[serde(default)]
    pub controller_layout_user_set: bool,
    #[serde(default = "default_true")]
    pub xy_quick_action: bool,
    /// Controller vibration (shop hold-to-sell, scoring cascade, etc.). Default on.
    #[serde(default = "default_true")]
    pub hold_to_sell_rumble: bool,
    #[serde(default = "default_true")]
    pub auto_cash_in_on_full_structure: bool,
    /// Lift selected melds onto the structure rail as translucent previews before Play.
    #[serde(default = "default_true")]
    pub structure_meld_preview: bool,
    /// Show the post-discard Undo control and allow undoing the last discard.
    /// The snapshot is always recorded; this only gates UI and the undo action.
    #[serde(default)]
    pub discard_undo_enabled: bool,
    /// Per profile slot: `run_history.len()` last time the player opened Archive.
    /// Used for a subtle "new chronicle" hint on the main menu.
    #[serde(default = "default_archive_last_seen_run_len")]
    pub archive_last_seen_run_len: [u32; 3],
}

/// Stored volume gain: `1.0` = 100% (unity), `2.0` = 200% (VLC-style boost cap).
pub const VOLUME_MIN: f32 = 0.0;
pub const VOLUME_MAX: f32 = 2.0;
pub const VOLUME_UNITY: f32 = 1.0;
pub const VOLUME_STEP: f32 = 0.05;

#[inline]
pub fn clamp_volume(vol: f32) -> f32 {
    vol.clamp(VOLUME_MIN, VOLUME_MAX)
}

/// Rounded percentage for options readouts (`1.0` → `100`, `2.0` → `200`).
#[inline]
pub fn volume_display_percent(vol: f32) -> u32 {
    (clamp_volume(vol) * 100.0).round() as u32
}

fn default_volume() -> f32 {
    VOLUME_UNITY
}
fn default_true() -> bool {
    true
}
fn default_gamma() -> f32 {
    1.0
}

fn default_archive_last_seen_run_len() -> [u32; 3] {
    [0, 0, 0]
}

/// Min/max for the user-facing gamma slider.
pub const GAMMA_MIN: f32 = 0.5;
pub const GAMMA_MAX: f32 = 2.0;

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_profile: 0,
            master_volume: VOLUME_UNITY,
            sfx_volume: VOLUME_UNITY,
            music_volume: VOLUME_UNITY,
            sfx_enabled: true,
            effects_quality: EffectsQuality::High,
            tile_preset: TilePreset::Chinese,
            tile_material: TileMaterial::Bamboo,
            tileset_name: default_tileset_name(),
            gamma: 1.0,
            graphics_mode: GraphicsMode::Visuals,
            graphics_mode_user_set: false,
            hdr_enabled: false,
            vhs_enabled: true,
            borderless_fullscreen: true,
            glyph_prompt: GlyphPromptSetting::default(),
            swap_ab: false,
            swap_xy: false,
            controller_layout_user_set: false,
            xy_quick_action: true,
            hold_to_sell_rumble: true,
            auto_cash_in_on_full_structure: true,
            structure_meld_preview: true,
            discard_undo_enabled: false,
            archive_last_seen_run_len: [0, 0, 0],
        }
    }
}

fn settings_path() -> PathBuf {
    data_dir().join(SETTINGS_NAME)
}

/// Process-local cache for [`load_settings`]. The settings file is only ever
/// mutated by this process via [`save_settings`], so a single in-memory copy
/// is always authoritative once loaded — no file watcher or stat-on-read
/// needed. Profiling showed `load_settings` was being called 5–7 times per
/// gameplay frame from update / draw paths (discard-undo eligibility checks,
/// etc.), each doing a stat + read + JSON parse + asset-pack tileset scan;
/// caching turned that into a clone of an `Arc<...>`-free `AppSettings`
/// (only `String` allocations) gated by a single mutex acquire.
static SETTINGS_CACHE: OnceLock<Mutex<Option<AppSettings>>> = OnceLock::new();

fn settings_cache() -> &'static Mutex<Option<AppSettings>> {
    SETTINGS_CACHE.get_or_init(|| Mutex::new(None))
}

pub fn load_settings() -> AppSettings {
    if let Ok(guard) = settings_cache().lock()
        && let Some(cached) = guard.as_ref()
    {
        return cached.clone();
    }
    let settings = load_settings_uncached();
    if let Ok(mut guard) = settings_cache().lock() {
        *guard = Some(settings.clone());
    }
    settings
}

fn load_settings_uncached() -> AppSettings {
    let path = settings_path();
    if !path.exists() {
        return AppSettings::default();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return AppSettings::default(),
    };
    let mut settings: AppSettings = serde_json::from_str(&data).unwrap_or_default();
    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&data)
        && raw.get("graphics_mode").is_none()
    {
        let shadow: ShadowQuality = raw
            .get("shadow_quality")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(|| {
                if raw.get("shadows_enabled").and_then(|v| v.as_bool()) == Some(false) {
                    ShadowQuality::Off
                } else {
                    ShadowQuality::High
                }
            });
        let ssr = raw
            .get("ssr_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let hdr = raw
            .get("hdr_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        settings.graphics_mode = GraphicsMode::from_legacy(shadow, ssr);
        settings.hdr_enabled = hdr;
    }
    settings.active_profile = settings.active_profile.min(MAX_PROFILES - 1);
    if crate::asset_path::is_internal_only_tileset(&settings.tileset_name) {
        settings.tileset_name = default_tileset_name();
    }
    let player_tilesets = crate::asset_path::list_player_tilesets();
    if !player_tilesets.iter().any(|n| n == &settings.tileset_name) {
        settings.tileset_name = player_tilesets
            .first()
            .cloned()
            .unwrap_or_else(default_tileset_name);
    }
    settings
}

pub fn save_settings(settings: &AppSettings) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(settings).context("serialize settings")?;
    fs::write(settings_path(), json).context("write settings")?;
    // Update the cache so the next `load_settings` (often the very next
    // frame) sees the new values without re-reading or re-parsing.
    if let Ok(mut guard) = settings_cache().lock() {
        *guard = Some(settings.clone());
    }
    Ok(())
}

fn profile_path(index: usize) -> PathBuf {
    data_dir().join(format!("profile_{index}.json"))
}

/// Process-local cache for [`load_profile`], one slot per profile index.
/// Callers churn through `load_profile` on profile switches and during
/// gameplay-event bookkeeping; the in-memory copy is authoritative because
/// the app is the only writer (via [`save_profile`]).
///
/// Note: [`load_run`] is intentionally *not* cached. `RunState` does not
/// derive `Clone` and threading `Clone` through the deep tree is invasive,
/// while `load_run` is only invoked at scene transitions and profile
/// switches — well off the per-frame hot path that motivated the
/// settings-cache fix.
static PROFILE_CACHE: OnceLock<Mutex<[Option<PlayerProgress>; MAX_PROFILES]>> = OnceLock::new();

fn profile_cache() -> &'static Mutex<[Option<PlayerProgress>; MAX_PROFILES]> {
    PROFILE_CACHE.get_or_init(|| Mutex::new([const { None }; MAX_PROFILES]))
}

pub fn load_profile(index: usize) -> PlayerProgress {
    let idx = index.min(MAX_PROFILES - 1);
    if let Ok(guard) = profile_cache().lock()
        && let Some(cached) = guard[idx].as_ref()
    {
        return cached.clone();
    }
    let progress = load_profile_uncached(idx);
    if let Ok(mut guard) = profile_cache().lock() {
        guard[idx] = Some(progress.clone());
    }
    progress
}

fn load_profile_uncached(index: usize) -> PlayerProgress {
    let path = profile_path(index);
    if !path.exists() {
        return PlayerProgress::new();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return PlayerProgress::new(),
    };
    let mut progress: PlayerProgress =
        serde_json::from_str(&data).unwrap_or_else(|_| PlayerProgress::new());
    // Meta-level points landed after older profiles were already on disk.
    // Re-derive once from run history (or run count fallback) when absent.
    progress.backfill_level_progress_points_from_history();
    // L1 content is the baseline — ensure it's marked as already-known so
    // `check_level_up` never surfaces it as a new-unlock modal.
    progress.backfill_level_1_unlocks();
    // Seasons landed after some profiles already had victories on disk. Re-derive
    // `unlocked_seasons` from history every load so older saves see the right
    // ladder without requiring a manual migration — the backfill is idempotent.
    progress.backfill_seasons_from_history();
    progress.backfill_material_unlocks_from_history();
    progress
}

pub fn save_profile(index: usize, progress: &PlayerProgress) -> anyhow::Result<()> {
    write_profile_to_disk(index, progress)?;
    update_profile_cache(index, progress);
    Ok(())
}

fn write_profile_to_disk(index: usize, progress: &PlayerProgress) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(progress).context("serialize")?;
    fs::write(profile_path(index), json).context("write save")?;
    Ok(())
}

fn update_profile_cache(index: usize, progress: &PlayerProgress) {
    let idx = index.min(MAX_PROFILES - 1);
    if let Ok(mut guard) = profile_cache().lock() {
        guard[idx] = Some(progress.clone());
    }
}

/// Drop the in-memory snapshot for one slot so the next [`load_profile`] reads disk.
///
/// Call after deleting a save file or before switching profiles so a slot that
/// was cleared on disk is not resurrected from a stale cache entry.
pub fn clear_profile_cache_slot(index: usize) {
    let idx = index.min(MAX_PROFILES - 1);
    if let Ok(mut guard) = profile_cache().lock() {
        guard[idx] = None;
    }
}

/// Background-thread profile saver. Pairs with `App::mark_profile_dirty`
/// + a frame-end flush so per-event saves (relic activation, boss
///   bookkeeping, yaku tally, …) don't stall the frame on slow disks
///   (Steam Deck SD card installs are the motivating case).
///
/// `enqueue` updates the in-process [`profile_cache`] synchronously so
/// any subsequent `load_profile` sees fresh state immediately, then
/// hands a snapshot to the worker for serialization + write. The
/// worker coalesces back-to-back queued saves per profile index — a
/// scoring cascade that fires N save events the same frame turns into
/// at most one write per profile.
pub struct ProfileSaver {
    tx: Option<mpsc::Sender<(usize, PlayerProgress)>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ProfileSaver {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<(usize, PlayerProgress)>();
        let handle = std::thread::Builder::new()
            .name("profile-saver".to_string())
            .spawn(move || profile_saver_loop(rx))
            .expect("spawn profile-saver thread");
        Self {
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Update the in-memory cache synchronously, then send a snapshot
    /// for background disk write. Best-effort — channel send failure
    /// (worker exited) is logged but never propagated.
    pub fn enqueue(&self, index: usize, progress: &PlayerProgress) {
        update_profile_cache(index, progress);
        if let Some(tx) = self.tx.as_ref()
            && let Err(e) = tx.send((index.min(MAX_PROFILES - 1), progress.clone()))
        {
            log::warn!("profile-saver enqueue failed: {e}");
        }
    }

    /// Drop the sender and join the worker so any pending writes land
    /// before the process exits. Idempotent; called from `Drop`.
    pub fn shutdown(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take()
            && let Err(e) = handle.join()
        {
            log::warn!("profile-saver thread join failed: {e:?}");
        }
    }
}

impl Drop for ProfileSaver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn profile_saver_loop(rx: mpsc::Receiver<(usize, PlayerProgress)>) {
    while let Ok((idx, progress)) = rx.recv() {
        // Coalesce: drain anything already buffered and keep the
        // latest snapshot per profile index. Multiple saves landing
        // on the same frame collapse into one write.
        let mut latest: [Option<PlayerProgress>; MAX_PROFILES] = [const { None }; MAX_PROFILES];
        latest[idx.min(MAX_PROFILES - 1)] = Some(progress);
        while let Ok((idx2, progress2)) = rx.try_recv() {
            latest[idx2.min(MAX_PROFILES - 1)] = Some(progress2);
        }
        for (j, snap) in latest.iter().enumerate() {
            if let Some(p) = snap
                && let Err(e) = write_profile_to_disk(j, p)
            {
                log::warn!("profile-saver: write_profile_to_disk({j}) failed: {e}");
            }
        }
    }
}

/// Check if a profile has any save data on disk.
pub fn profile_exists(index: usize) -> bool {
    profile_path(index).exists()
}

/// Summary info for displaying a profile slot on the start screen.
pub struct ProfileSummary {
    pub exists: bool,
    pub level: u32,
    pub runs_completed: u32,
    pub high_score: u64,
    /// Whether a saved run is on disk for this profile.
    pub has_saved_run: bool,
    pub victories: u32,
    pub relics_unlocked: u32,
    pub yaku_discovered: u32,
    pub second_high_score: u64,
    pub third_high_score: u64,
}

/// Path for a saved in-progress run for a profile.
fn saved_run_path(index: usize) -> PathBuf {
    data_dir().join(format!("run_{index}.json"))
}

/// Check if a profile has a saved in-progress run.
pub fn has_saved_run(index: usize) -> bool {
    saved_run_path(index).exists()
}

/// Destination for an Options-menu "export play stats" HTML report. Uses
/// `Downloads/Mahjuro/` when available, then Documents, then local data dir.
pub fn play_stats_export_path(profile_index: usize) -> PathBuf {
    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
    {
        return mahjuro_distribution::PlatformPaths::play_stats_export_path(profile_index);
    }
    #[cfg(not(any(feature = "game", feature = "headless-screenshot")))]
    {
        let base = dirs::download_dir()
            .or_else(dirs::document_dir)
            .map(|p| p.join(APP_DIR))
            .unwrap_or_else(data_dir);
        let _ = fs::create_dir_all(&base);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        base.join(format!("play_stats_profile{}_{ts}.html", profile_index + 1))
    }
}

/// Wrapper that stamps each saved run with the build version. On load we
/// reject any save whose version doesn't match the current binary so an
/// update can ship breaking changes to `RunState` without having to write a
/// migration — the player simply starts a fresh run after upgrading. The
/// stale file is deleted so the "Continue" affordance disappears.
#[derive(Serialize)]
struct SavedRunRef<'a> {
    version: &'a str,
    scene: ResumeScene,
    run: &'a RunState,
}

#[derive(Deserialize)]
struct SavedRunOwned {
    version: String,
    #[serde(default)]
    scene: ResumeScene,
    run: RunState,
}

fn current_save_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Persist an in-progress run for the given profile. Best-effort: any IO or
/// serialization failure is logged and swallowed so quit paths never block.
pub fn save_run(index: usize, run: &RunState, scene: ResumeScene) -> anyhow::Result<()> {
    let payload = SavedRunRef {
        version: current_save_version(),
        scene,
        run,
    };
    let json = serde_json::to_string_pretty(&payload).context("serialize saved run")?;
    fs::write(saved_run_path(index), json).context("write saved run")
}

/// Load the saved run for `index`. Returns `None` if no save exists, the
/// file is corrupt, or the save was written by a different build version
/// (in which case the stale file is deleted on the spot).
pub fn load_run(index: usize) -> Option<LoadedRun> {
    let path = saved_run_path(index);
    if !path.exists() {
        return None;
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("load_run: read failed: {e}");
            return None;
        }
    };
    let saved: SavedRunOwned = match serde_json::from_str(&data) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("load_run: parse failed (deleting stale save): {e}");
            let _ = fs::remove_file(&path);
            return None;
        }
    };
    if saved.version != current_save_version() {
        log::debug!(
            "load_run: save version {} != current {} (deleting)",
            saved.version,
            current_save_version()
        );
        let _ = fs::remove_file(&path);
        return None;
    }
    // Rehydrate the resolved boss effect — it's `#[serde(skip)]`, so on
    // reload `upcoming_ordeal_effect` is `None`. Reactive bosses re-run their
    // `on_reveal` hook against current state; since neither relics nor gold
    // change between save and reload, the result matches the original pick.
    let mut run = saved.run;
    run.resolve_upcoming_ordeal();
    let mut scene = saved.scene;
    // Repair stale/bad scene markers from older builds: a gameplay resume
    // must have an active dealt hand. If the snapshot is still in the
    // pre-blind state, land in the shop instead of the black gameplay shell.
    if matches!(scene, ResumeScene::Gameplay)
        && run.hand().is_empty()
        && run.chamber == run.upcoming_chamber
    {
        log::warn!(
            "load_run: repairing saved scene marker from Gameplay to Shop (profile {}, round {}, blind {:?})",
            index,
            run.run_number,
            run.chamber
        );
        scene = ResumeScene::Shop;
    }
    Some(LoadedRun { run, scene })
}

/// Remove the saved run for a profile (e.g. after a run ends or a new run
/// is started). No-op if no save exists.
pub fn delete_saved_run(index: usize) {
    let path = saved_run_path(index);
    if path.exists()
        && let Err(e) = fs::remove_file(&path)
    {
        log::warn!("delete_saved_run: {e}");
    }
}

pub fn profile_summary(index: usize) -> ProfileSummary {
    if !profile_exists(index) {
        return ProfileSummary {
            exists: false,
            level: 0,
            runs_completed: 0,
            high_score: 0,
            has_saved_run: false,
            victories: 0,
            relics_unlocked: 0,
            yaku_discovered: 0,
            second_high_score: 0,
            third_high_score: 0,
        };
    }
    let progress = load_profile(index);
    ProfileSummary {
        exists: true,
        level: progress.current_level(),
        runs_completed: progress.runs_completed,
        high_score: progress.high_scores.first().copied().unwrap_or(0),
        has_saved_run: has_saved_run(index),
        victories: progress
            .run_history
            .iter()
            .filter(|r| matches!(r.outcome, RunOutcome::Victory))
            .count() as u32,
        relics_unlocked: progress.unlocked_relics.len() as u32,
        yaku_discovered: progress
            .yaku_times_scored
            .values()
            .filter(|&&c| c > 0)
            .count() as u32,
        second_high_score: progress.high_scores.get(1).copied().unwrap_or(0),
        third_high_score: progress.high_scores.get(2).copied().unwrap_or(0),
    }
}

pub fn all_profile_summaries() -> Vec<ProfileSummary> {
    (0..MAX_PROFILES).map(profile_summary).collect()
}

/// Delete all data for a profile slot — the progress file and any saved run.
pub fn delete_profile(index: usize) {
    let path = profile_path(index);
    if path.exists()
        && let Err(e) = fs::remove_file(&path)
    {
        log::warn!("delete_profile: {e}");
    }
    delete_saved_run(index);
    clear_profile_cache_slot(index);
}

#[cfg(test)]
mod glyph_prompt_setting_tests {
    use super::GlyphPromptSetting;
    use crate::ui::button_prompts::GamepadStyle;

    #[test]
    fn auto_uses_detected() {
        assert_eq!(
            GlyphPromptSetting::Auto.resolve(GamepadStyle::PlayStation),
            GamepadStyle::PlayStation
        );
    }

    #[test]
    fn next_cycles_back_to_auto() {
        let start = GlyphPromptSetting::default();
        let mut v = start;
        for _ in 0..32 {
            v = v.next();
            if v == start {
                return;
            }
        }
        panic!("glyph prompt cycle did not return to Auto");
    }
}
