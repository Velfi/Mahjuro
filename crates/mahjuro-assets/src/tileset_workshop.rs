//! Steam Workshop tileset installs — populated at runtime by the Steam backend.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::tileset_mod::{resolve_mod_content_dir, validate_mod_tileset};

/// Internal tileset id prefix for Workshop items (`workshop:1234567890`).
pub const WORKSHOP_PREFIX: &str = "workshop:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopTilesetInstall {
    pub published_file_id: u64,
    pub title: Option<String>,
    pub content_dir: PathBuf,
}

struct Registry {
    installs: Vec<WorkshopTilesetInstall>,
    revision: u64,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            installs: Vec::new(),
            revision: 0,
        })
    })
}

pub fn workshop_id(published_file_id: u64) -> String {
    format!("{WORKSHOP_PREFIX}{published_file_id}")
}

pub fn is_workshop_tileset(tileset_id: &str) -> bool {
    tileset_id.starts_with(WORKSHOP_PREFIX)
}

pub fn parse_workshop_file_id(tileset_id: &str) -> Option<u64> {
    tileset_id
        .strip_prefix(WORKSHOP_PREFIX)?
        .parse()
        .ok()
}

pub fn registry_revision() -> u64 {
    registry()
        .lock()
        .map(|r| r.revision)
        .unwrap_or(0)
}

pub fn list_workshop_tilesets() -> Vec<WorkshopTilesetInstall> {
    registry()
        .lock()
        .map(|r| r.installs.clone())
        .unwrap_or_default()
}

pub fn workshop_content_dir(tileset_id: &str) -> Option<PathBuf> {
    let file_id = parse_workshop_file_id(tileset_id)?;
    registry()
        .lock()
        .ok()?
        .installs
        .iter()
        .find(|e| e.published_file_id == file_id)
        .map(|e| e.content_dir.clone())
}

pub fn workshop_display_title(tileset_id: &str) -> Option<String> {
    let file_id = parse_workshop_file_id(tileset_id)?;
    registry().lock().ok()?.installs.iter().find_map(|e| {
        if e.published_file_id == file_id {
            e.title.clone()
        } else {
            None
        }
    })
}

/// Replace the installed Workshop tileset list (called from the Steam sync layer).
pub fn set_workshop_installs(installs: Vec<WorkshopTilesetInstall>) {
    let mut reg = match registry().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if reg.installs != installs {
        reg.revision = reg.revision.saturating_add(1);
        reg.installs = installs;
    }
}

pub fn update_workshop_title(published_file_id: u64, title: String) {
    let mut reg = match registry().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if let Some(entry) = reg
        .installs
        .iter_mut()
        .find(|e| e.published_file_id == published_file_id)
    {
        if entry.title.as_deref() != Some(title.as_str()) {
            entry.title = Some(title);
            reg.revision = reg.revision.saturating_add(1);
        }
    }
}

/// Read a file from an installed Workshop tileset folder.
pub fn read_workshop_file(tileset_id: &str, filename: &str) -> Option<Vec<u8>> {
    if filename.contains('/') || filename.contains('\\') {
        return None;
    }
    let dir = workshop_content_dir(tileset_id)?;
    std::fs::read(dir.join(filename)).ok()
}

/// Resolve a Workshop install root to the folder containing `atlas.toml` + `atlas.png`.
pub fn resolve_workshop_content_dir(install_root: &Path) -> Option<PathBuf> {
    resolve_mod_content_dir(install_root)
}

/// Validate that `install_root` (or an immediate child) is a playable tileset.
pub fn validate_workshop_install(install_root: &Path) -> Option<PathBuf> {
    resolve_mod_content_dir(install_root).filter(|dir| validate_mod_tileset(dir).is_ok())
}

/// Showcase decal cache folder name for a Workshop tileset id.
pub fn workshop_cache_folder_name(tileset_id: &str) -> Option<String> {
    parse_workshop_file_id(tileset_id).map(|id| format!("workshop_{id}"))
}

mod tests {
    use super::*;
    use std::fs;

    fn write_valid_mod(dir: &Path, tile_w: u32, tile_h: u32, columns: u32) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("atlas.toml"),
            format!(
                "tile_width = {tile_w}\ntile_height = {tile_h}\ncolumns = {columns}\nlayout = [\"B1\"]\n"
            ),
        )
        .unwrap();
        let img = image::RgbaImage::new(columns * tile_w, tile_h);
        img.save(dir.join("atlas.png")).unwrap();
    }

    #[test]
    fn workshop_id_round_trip() {
        let id = workshop_id(42);
        assert_eq!(id, "workshop:42");
        assert!(is_workshop_tileset(&id));
        assert_eq!(parse_workshop_file_id(&id), Some(42));
        assert!(crate::tileset_mod::is_player_tileset(&id));
    }

    #[test]
    fn resolve_nested_workshop_content() {
        let base = std::env::temp_dir().join(format!(
            "mahjuro_workshop_resolve_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let nested = base.join("pack");
        write_valid_mod(&nested, 10, 20, 9);
        assert_eq!(
            resolve_workshop_content_dir(&base).as_deref(),
            Some(nested.as_path())
        );
    }

    #[test]
    fn registry_revision_bumps_on_update() {
        let base = std::env::temp_dir().join(format!(
            "mahjuro_workshop_registry_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        write_valid_mod(&base, 10, 20, 9);

        set_workshop_installs(vec![WorkshopTilesetInstall {
            published_file_id: 99,
            title: None,
            content_dir: base.clone(),
        }]);
        let rev1 = registry_revision();
        set_workshop_installs(vec![WorkshopTilesetInstall {
            published_file_id: 99,
            title: None,
            content_dir: base.clone(),
        }]);
        assert_eq!(registry_revision(), rev1);
        update_workshop_title(99, "Cool Tiles".into());
        assert!(registry_revision() > rev1);
        assert_eq!(
            read_workshop_file("workshop:99", "atlas.toml"),
            fs::read(base.join("atlas.toml")).ok()
        );
    }
}
