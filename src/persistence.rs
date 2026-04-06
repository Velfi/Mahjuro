//! Save / load `PlayerProgress` as JSON next to cwd.

use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::core::progression::PlayerProgress;

const SAVE_NAME: &str = "mahjuro_save.json";

pub fn default_save_path() -> std::path::PathBuf {
    Path::new(SAVE_NAME).to_path_buf()
}

pub fn load_or_new() -> PlayerProgress {
    let path = default_save_path();
    if !path.exists() {
        return PlayerProgress::new();
    }
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return PlayerProgress::new(),
    };
    serde_json::from_str(&data).unwrap_or_else(|_| PlayerProgress::new())
}

pub fn save(progress: &PlayerProgress) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(progress).context("serialize")?;
    fs::write(default_save_path(), json).context("write save")
}
