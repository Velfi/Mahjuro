//! Save / load `PlayerProgress` as JSON — supports up to 3 profiles.

use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::core::progression::PlayerProgress;
use crate::game::run::RunState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ResumeScene {
    #[default]
    Gameplay,
    Shop,
    PickBlind,
}

#[derive(Debug)]
pub struct LoadedRun {
    pub run: RunState,
    pub scene: ResumeScene,
}

const MAX_PROFILES: usize = 3;
const SETTINGS_NAME: &str = "settings.json";
const APP_DIR: &str = "Mahjuro";

/// Returns the directory where save data lives, creating it if needed.
/// Falls back to the current directory if the platform config dir is
/// unavailable or can't be created.
fn data_dir() -> PathBuf {
    if let Some(base) = dirs::config_dir() {
        let dir = base.join(APP_DIR);
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    PathBuf::from(".")
}

/// Smoke simulation quality. Bundles grid resolution, raymarch step floor,
/// and offscreen-target downsampling into a single perf knob. `Off`
/// short-circuits the simulation entirely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmokeQuality {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl SmokeQuality {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Ultra,
            Self::Ultra => Self::Off,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Off => Self::Ultra,
            Self::Low => Self::Off,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
            Self::Ultra => Self::High,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    /// Linear divisor for the offscreen raymarch target. Smoke is a
    /// low-frequency field — the bilateral composite upsample hides the
    /// divisor cleanly, and the raymarch is fullscreen-fragment-bound, so
    /// halving the target is a ~4× win. Temporal reprojection in the
    /// raymarch reconstructs detail across frames, so even Ultra can run
    /// at half-res without visible softening.
    pub fn target_divisor(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::Low => 4,
            Self::Medium => 2,
            Self::High => 2,
            Self::Ultra => 2,
        }
    }
}

fn default_smoke_quality() -> SmokeQuality {
    SmokeQuality::High
}

/// How much smoke the cursor injects per puff. Pure look knob — doesn't
/// touch sim cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmokeAmount {
    Light,
    Medium,
    Heavy,
}

impl SmokeAmount {
    pub fn next(self) -> Self {
        match self {
            Self::Light => Self::Medium,
            Self::Medium => Self::Heavy,
            Self::Heavy => Self::Light,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Light => Self::Heavy,
            Self::Medium => Self::Light,
            Self::Heavy => Self::Medium,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Heavy => "Heavy",
        }
    }

    /// Multiplier applied to per-puff injection density.
    pub fn density_mul(self) -> f32 {
        match self {
            Self::Light => 0.55,
            Self::Medium => 1.0,
            Self::Heavy => 1.6,
        }
    }

    /// Maximum alpha the volumetric composite is allowed to reach. Heavier
    /// amounts let dense plumes silhouette properly.
    pub fn max_alpha(self) -> f32 {
        match self {
            Self::Light => 0.50,
            Self::Medium => 0.70,
            Self::Heavy => 0.88,
        }
    }
}

fn default_smoke_amount() -> SmokeAmount {
    SmokeAmount::Medium
}

/// Controls the quality of fullscreen vignette effects (starfield, ember
/// drift, golden dust, shooting-star cascade). Lower levels reduce or skip
/// procedural layers to save GPU ALU on weaker hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectsQuality {
    Off,
    Low,
    Medium,
    High,
}

impl EffectsQuality {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::Low => Self::Off,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// Numeric quality level uploaded to the GPU globals uniform.
    /// The cascade shader uses this to gate layer groups.
    pub fn quality_level_f32(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Low => 0.0,
            Self::Medium => 1.0,
            Self::High => 2.0,
        }
    }
}

fn default_effects_quality() -> EffectsQuality {
    EffectsQuality::High
}

/// Mahjong tile size preset. Proportions are taken from Wikipedia's
/// "Mahjong tiles" article and reflect the canonical real-world dimensions
/// of three common regional sets. Each preset controls the face aspect
/// (long edge / short edge) and the slab thickness relative to the short
/// edge — i.e. it changes the *shape* of every rendered tile, not just a
/// uniform size scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TilePreset {
    /// Chinese standard, ~30 × 20 × 15 mm.
    Chinese,
    /// Japanese riichi, ~26 × 19 × 16 mm — chunkier and squarer.
    Japanese,
    /// American mah jongg, ~32 × 25 × 19 mm — largest.
    American,
}

impl TilePreset {
    pub fn next(self) -> Self {
        match self {
            Self::Chinese => Self::Japanese,
            Self::Japanese => Self::American,
            Self::American => Self::Chinese,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Chinese => Self::American,
            Self::Japanese => Self::Chinese,
            Self::American => Self::Japanese,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chinese => "Chinese (30×20×15mm)",
            Self::Japanese => "Japanese (26×19×16mm)",
            Self::American => "American (32×25×19mm)",
        }
    }

    /// Face long edge divided by short edge.
    pub fn face_long_ratio(self) -> f32 {
        match self {
            Self::Chinese => 30.0 / 20.0,
            Self::Japanese => 26.0 / 19.0,
            Self::American => 32.0 / 25.0,
        }
    }

    /// Slab thickness divided by short edge.
    pub fn thickness_ratio(self) -> f32 {
        match self {
            Self::Chinese => 15.0 / 20.0,
            Self::Japanese => 16.0 / 19.0,
            Self::American => 19.0 / 25.0,
        }
    }
}

fn default_tile_preset() -> TilePreset {
    TilePreset::Chinese
}

/// Tile material / colour scheme. Controls the procedural surface
/// appearance in the tile shader — ivory+bamboo, plastic, etc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileMaterial {
    /// Traditional ivory face on a bamboo body.
    Bamboo,
    /// Mint-green plastic with a bright white face (common mass-produced set).
    Plastic,
}

impl Default for TileMaterial {
    fn default() -> Self {
        Self::Bamboo
    }
}

impl TileMaterial {
    pub fn next(self) -> Self {
        match self {
            Self::Bamboo => Self::Plastic,
            Self::Plastic => Self::Bamboo,
        }
    }

    pub fn prev(self) -> Self {
        self.next() // only two variants
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bamboo => "Bamboo & Ivory",
            Self::Plastic => "Plastic",
        }
    }

    pub fn bonus_description(self) -> &'static str {
        match self {
            Self::Bamboo => "+1 Hand per round",
            Self::Plastic => "+1 Discard per round",
        }
    }

    /// Material ID passed to the tile shader via `base_color_factor.w`.
    /// 0.0 = bamboo/ivory (default), 1.0 = plastic.
    pub fn shader_id(self) -> f32 {
        match self {
            Self::Bamboo => 0.0,
            Self::Plastic => 1.0,
        }
    }
}

fn default_tile_material() -> TileMaterial {
    TileMaterial::Bamboo
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
    #[serde(default = "default_smoke_quality")]
    pub smoke_quality: SmokeQuality,
    #[serde(default = "default_smoke_amount")]
    pub smoke_amount: SmokeAmount,
    #[serde(default = "default_effects_quality")]
    pub effects_quality: EffectsQuality,
    #[serde(default = "default_tile_preset")]
    pub tile_preset: TilePreset,
    #[serde(default = "default_tile_material")]
    pub tile_material: TileMaterial,
    #[serde(default = "default_gamma")]
    pub gamma: f32,
    #[serde(default = "default_true")]
    pub shadows_enabled: bool,
    #[serde(default = "default_true")]
    pub ssr_enabled: bool,
    #[serde(default)]
    pub hdr_enabled: bool,
    #[serde(default)]
    pub swap_ab: bool,
    #[serde(default = "default_true")]
    pub auto_cash_in_on_full_structure: bool,
    #[serde(default)]
    pub hints_enabled: bool,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

fn default_volume() -> f32 {
    0.7
}
fn default_true() -> bool {
    true
}
fn default_gamma() -> f32 {
    1.0
}
fn default_ui_scale() -> f32 {
    1.0
}

/// Min/max for the user-facing gamma slider.
pub const GAMMA_MIN: f32 = 0.5;
pub const GAMMA_MAX: f32 = 2.0;

/// Min/max for the user-facing UI scale slider.
pub const UI_SCALE_MIN: f32 = 0.75;
pub const UI_SCALE_MAX: f32 = 2.0;

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_profile: 0,
            master_volume: 0.7,
            sfx_volume: 0.7,
            music_volume: 0.7,
            sfx_enabled: true,
            smoke_quality: SmokeQuality::High,
            smoke_amount: SmokeAmount::Medium,
            effects_quality: EffectsQuality::High,
            tile_preset: TilePreset::Chinese,
            tile_material: TileMaterial::Bamboo,
            gamma: 1.0,
            shadows_enabled: true,
            ssr_enabled: true,
            hdr_enabled: false,
            swap_ab: false,
            auto_cash_in_on_full_structure: true,
            hints_enabled: false,
            ui_scale: 1.0,
        }
    }
}

fn settings_path() -> PathBuf {
    data_dir().join(SETTINGS_NAME)
}

pub fn load_settings() -> AppSettings {
    let path = settings_path();
    if !path.exists() {
        return AppSettings::default();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return AppSettings::default(),
    };
    let mut settings: AppSettings = serde_json::from_str(&data).unwrap_or_default();
    settings.active_profile = settings.active_profile.min(MAX_PROFILES - 1);
    settings
}

pub fn save_settings(settings: &AppSettings) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(settings).context("serialize settings")?;
    fs::write(settings_path(), json).context("write settings")
}

// ── Tuning overrides ────────────────────────────────────────────────────
//
// A small persistence layer that lets the debug overlays promote a live-
// edited tuning struct to a "user default" that survives restarts. Values
// live in `tuning_overrides.json` keyed by struct name. `load_or_default`
// reads the override if present, falling back to `Default::default()`.
// When a value is promoted into code (copied into the `Default` impl),
// clear the override with `clear_tuning_override` so the code default
// takes over again.

const TUNING_OVERRIDES_NAME: &str = "tuning_overrides.json";

fn tuning_overrides_path() -> PathBuf {
    data_dir().join(TUNING_OVERRIDES_NAME)
}

fn read_tuning_overrides() -> serde_json::Map<String, serde_json::Value> {
    let path = tuning_overrides_path();
    if !path.exists() {
        return serde_json::Map::new();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return serde_json::Map::new(),
    };
    match serde_json::from_str::<serde_json::Value>(&data) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

fn write_tuning_overrides(map: &serde_json::Map<String, serde_json::Value>) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(map).context("serialize tuning overrides")?;
    fs::write(tuning_overrides_path(), json).context("write tuning overrides")
}

/// Load an override for `T` keyed by `name`, or fall back to `Default`.
/// `name` should uniquely identify the struct (typically its type name).
pub fn load_tuning_override<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    let map = read_tuning_overrides();
    match map.get(name) {
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_default(),
        None => T::default(),
    }
}

/// Promote the current value of `T` to the persistent override.
pub fn save_tuning_override<T: serde::Serialize>(name: &str, value: &T) -> anyhow::Result<()> {
    let mut map = read_tuning_overrides();
    let v = serde_json::to_value(value).context("serialize tuning value")?;
    map.insert(name.to_string(), v);
    write_tuning_overrides(&map)
}

/// Remove a persisted override so the code default applies on next load.
pub fn clear_tuning_override(name: &str) -> anyhow::Result<()> {
    let mut map = read_tuning_overrides();
    if map.remove(name).is_some() {
        write_tuning_overrides(&map)
    } else {
        Ok(())
    }
}

fn profile_path(index: usize) -> PathBuf {
    data_dir().join(format!("profile_{index}.json"))
}

pub fn load_profile(index: usize) -> PlayerProgress {
    let path = profile_path(index);
    if !path.exists() {
        return PlayerProgress::new();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return PlayerProgress::new(),
    };
    serde_json::from_str(&data).unwrap_or_else(|_| PlayerProgress::new())
}

pub fn save_profile(index: usize, progress: &PlayerProgress) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(progress).context("serialize")?;
    fs::write(profile_path(index), json).context("write save")
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
}

/// Path for a saved in-progress run for a profile.
fn saved_run_path(index: usize) -> PathBuf {
    data_dir().join(format!("run_{index}.json"))
}

/// Check if a profile has a saved in-progress run.
pub fn has_saved_run(index: usize) -> bool {
    saved_run_path(index).exists()
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
        log::info!(
            "load_run: save version {} != current {} (deleting)",
            saved.version,
            current_save_version()
        );
        let _ = fs::remove_file(&path);
        return None;
    }
    // Rehydrate the resolved boss effect — it's `#[serde(skip)]`, so on
    // reload `upcoming_boss_effect` is `None`. Reactive bosses re-run their
    // `on_reveal` hook against current state; since neither relics nor gold
    // change between save and reload, the result matches the original pick.
    let mut run = saved.run;
    run.resolve_upcoming_boss();
    let mut scene = saved.scene;
    // Repair stale/bad scene markers from older builds: a gameplay resume
    // must have an active dealt hand. If the snapshot is still in the
    // pre-blind state, land in the shop instead of the black gameplay shell.
    if matches!(scene, ResumeScene::Gameplay)
        && run.hand.is_empty()
        && run.blind == run.upcoming_blind
    {
        log::warn!(
            "load_run: repairing saved scene marker from Gameplay to Shop (profile {}, round {}, blind {:?})",
            index,
            run.run_number,
            run.blind
        );
        scene = ResumeScene::Shop;
    }
    Some(LoadedRun { run, scene })
}

/// Remove the saved run for a profile (e.g. after a run ends or a new run
/// is started). No-op if no save exists.
pub fn delete_saved_run(index: usize) {
    let path = saved_run_path(index);
    if path.exists() {
        if let Err(e) = fs::remove_file(&path) {
            log::warn!("delete_saved_run: {e}");
        }
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
        };
    }
    let progress = load_profile(index);
    ProfileSummary {
        exists: true,
        level: progress.current_level(),
        runs_completed: progress.runs_completed,
        high_score: progress.high_scores.first().copied().unwrap_or(0),
        has_saved_run: has_saved_run(index),
    }
}

pub fn all_profile_summaries() -> Vec<ProfileSummary> {
    (0..MAX_PROFILES).map(profile_summary).collect()
}

/// Delete all data for a profile slot — the progress file and any saved run.
pub fn delete_profile(index: usize) {
    let path = profile_path(index);
    if path.exists() {
        if let Err(e) = fs::remove_file(&path) {
            log::warn!("delete_profile: {e}");
        }
    }
    delete_saved_run(index);
}
