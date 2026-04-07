//! Save / load `PlayerProgress` as JSON — supports up to 3 profiles.

use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::core::progression::PlayerProgress;

const MAX_PROFILES: usize = 3;
const SETTINGS_NAME: &str = "mahjuro_settings.json";

/// Smoke effect intensity levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmokeIntensity {
    Off,
    Subtle,
    Strong,
    OverTheTop,
}

impl SmokeIntensity {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Subtle,
            Self::Subtle => Self::Strong,
            Self::Strong => Self::OverTheTop,
            Self::OverTheTop => Self::Off,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Off => Self::OverTheTop,
            Self::Subtle => Self::Off,
            Self::Strong => Self::Subtle,
            Self::OverTheTop => Self::Strong,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Subtle => "Subtle",
            Self::Strong => "Strong",
            Self::OverTheTop => "Over the Top",
        }
    }
}

fn default_smoke() -> SmokeIntensity {
    SmokeIntensity::Subtle
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
    #[serde(default = "default_smoke")]
    pub smoke_intensity: SmokeIntensity,
}

fn default_volume() -> f32 {
    0.7
}
fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_profile: 0,
            master_volume: 0.7,
            sfx_volume: 0.7,
            music_volume: 0.7,
            sfx_enabled: true,
            smoke_intensity: SmokeIntensity::Subtle,
        }
    }
}

fn settings_path() -> PathBuf {
    PathBuf::from(SETTINGS_NAME)
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

fn profile_path(index: usize) -> PathBuf {
    PathBuf::from(format!("mahjuro_profile_{index}.json"))
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
    pub high_score: u32,
    /// Whether a saved run is on disk for this profile.
    pub has_saved_run: bool,
}

/// Path for a saved in-progress run for a profile.
fn saved_run_path(index: usize) -> PathBuf {
    PathBuf::from(format!("mahjuro_run_{index}.json"))
}

/// Check if a profile has a saved in-progress run.
pub fn has_saved_run(index: usize) -> bool {
    saved_run_path(index).exists()
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

