use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Serialize, de::DeserializeOwned};

use crate::ui::placement::Placement;

const APP_DIR: &str = "Mahjuro";
const LAYOUTS_SUBDIR: &str = "layouts";

pub(super) fn layouts_dir() -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(LAYOUTS_SUBDIR);
    let _ = fs::create_dir_all(&base);
    base
}

pub(super) fn load_positions<T>(file_name: &str) -> T
where
    T: DeserializeOwned + Default,
{
    let path = layouts_dir().join(file_name);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub(super) fn save_positions<T>(file_name: &str, label: &str, pos: &T) -> anyhow::Result<()>
where
    T: Serialize,
{
    let json = serde_json::to_string_pretty(pos)?;
    let path = layouts_dir().join(file_name);
    fs::write(&path, json)?;
    log::info!("[Layout] Saved {label} positions → {}", path.display());
    Ok(())
}

/// Remove every `*.json` file under [`layouts_dir`]. Used by the debug menu
/// to drop arrange-mode overrides so compiled-in defaults apply again.
pub fn clear_saved_layout_files() -> anyhow::Result<usize> {
    let dir = layouts_dir();
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            log::info!("[Layout] Removed {}", path.display());
            removed += 1;
        }
    }
    Ok(removed)
}

pub(super) fn sanitize_placements<T, F>(
    scene: &str,
    target: &mut T,
    fields: &[F],
    mut field_mut: impl FnMut(&mut T, F) -> &mut Placement,
) where
    T: Default,
    F: Copy + std::fmt::Debug,
{
    let mut defaults = T::default();
    for &field in fields {
        if !field_mut(target, field).is_finite() {
            log::warn!(
                "[Layout] {scene} placement {:?} had non-finite values, restoring defaults",
                field
            );
            *field_mut(target, field) = *field_mut(&mut defaults, field);
        }
    }
}
