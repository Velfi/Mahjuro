use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::de::DeserializeOwned;

const TUNING_OVERRIDES_NAME: &str = "tuning_overrides.json";
const APP_DIR: &str = "Mahjuro";

/// Returns the directory where save data lives, creating it if needed.
/// Falls back to the current directory if the platform config dir is
/// unavailable or can't be created.
pub fn data_dir() -> PathBuf {
    if let Some(base) = dirs::config_dir() {
        let dir = base.join(APP_DIR);
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    PathBuf::from(".")
}
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
pub fn load_tuning_override<T: DeserializeOwned + Default>(name: &str) -> T {
    let map = read_tuning_overrides();
    match map.get(name) {
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_default(),
        None => T::default(),
    }
}

/// True iff a tuning override exists on disk for `name`. Used by the
/// per-scene tonemap loader to distinguish "no entry" (fall back to the
/// default tuning) from "entry that happens to match defaults".
pub fn has_tuning_override(name: &str) -> bool {
    read_tuning_overrides().contains_key(name)
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
