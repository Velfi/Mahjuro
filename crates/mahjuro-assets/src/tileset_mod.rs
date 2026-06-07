//! Player-installed tileset mods under the platform config directory.

use std::fs;
use std::path::{Path, PathBuf};
use image::GenericImageView;

use crate::atlas_toml::parse_atlas_toml;

const APP_DIR: &str = "Mahjuro";
const MODS_DIR: &str = "mods";
const TILESETS_DIR: &str = "tilesets";
const CACHE_DIR: &str = "cache";

/// Internal tileset id prefix for player mods (`mod:my_theme`).
pub const MOD_PREFIX: &str = "mod:";

const MAX_MOD_FOLDER_NAME_LEN: usize = 64;

/// Starter folder — not listed in Options; copy and rename to create a mod.
pub const TEMPLATE_FOLDER: &str = "_template";

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

const ROOT_README: &str = include_str!("tileset_mod_root_README.md");
const FOLDER_README: &str = include_str!("tileset_mod_folder_README.md");
const TEMPLATE_ATLAS_TOML: &str = include_str!("tileset_mod_template_atlas.toml");

/// Test-only: redirect mod tileset scans to a temp directory (per test thread).
#[cfg(test)]
pub fn set_mod_tilesets_root_for_tests(root: PathBuf) {
    TEST_CONFIG_ROOT.with(|cell| *cell.borrow_mut() = Some(root));
}

fn config_data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = TEST_CONFIG_ROOT.with(|cell| cell.borrow().clone()) {
        return root;
    }
    if let Some(base) = dirs::config_dir() {
        let dir = base.join(APP_DIR);
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    PathBuf::from(".")
}

/// `{config_dir}/Mahjuro/mods/tilesets`
pub fn mod_tilesets_root() -> PathBuf {
    config_data_dir().join(MODS_DIR).join(TILESETS_DIR)
}

/// `{config_dir}/Mahjuro/mods/cache/tilesets/{folder_name}/`
pub fn mod_tileset_cache_root(folder_name: &str) -> PathBuf {
    config_data_dir()
        .join(MODS_DIR)
        .join(CACHE_DIR)
        .join(TILESETS_DIR)
        .join(folder_name)
}

/// Cached offline showcase decal atlas for a mod tileset.
pub fn mod_showcase_cache_path(tileset_id: &str) -> Option<PathBuf> {
    let folder = mod_folder_name(tileset_id)?;
    Some(
        mod_tileset_cache_root(folder)
            .join("showcase_decal_atlas.png"),
    )
}

pub fn mod_showcase_cache_exists(tileset_id: &str) -> bool {
    mod_showcase_cache_path(tileset_id)
        .is_some_and(|p| p.is_file())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModTilesetEntry {
    pub folder_name: String,
    pub id: String,
}

pub fn mod_id(folder_name: &str) -> String {
    format!("{MOD_PREFIX}{folder_name}")
}

pub fn is_mod_tileset(tileset_id: &str) -> bool {
    tileset_id.starts_with(MOD_PREFIX)
}

/// Folder name from a mod tileset id (`mod:my_theme` → `my_theme`).
pub fn mod_folder_name(tileset_id: &str) -> Option<&str> {
    tileset_id.strip_prefix(MOD_PREFIX).filter(|s| !s.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TilesetId<'a> {
    Builtin(&'a str),
    Mod { folder_name: &'a str },
}

pub fn parse_tileset_id(tileset_id: &str) -> Option<TilesetId<'_>> {
    if let Some(folder) = mod_folder_name(tileset_id) {
        if is_valid_mod_folder_name(folder) {
            return Some(TilesetId::Mod { folder_name: folder });
        }
        return None;
    }
    if tileset_id.is_empty() || tileset_id.contains('/') || tileset_id.contains('\\') {
        return None;
    }
    Some(TilesetId::Builtin(tileset_id))
}

/// Options label for a tileset id.
pub fn tileset_display_name(tileset_id: &str) -> String {
    match parse_tileset_id(tileset_id) {
        Some(TilesetId::Mod { folder_name }) => format!("{folder_name} (mod)"),
        _ => tileset_id.to_string(),
    }
}

fn is_valid_mod_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MOD_FOLDER_NAME_LEN
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name != "." && name != ".."
}

/// Folders reserved for docs/templates — never offered as playable tilesets.
fn is_reserved_mod_folder(name: &str) -> bool {
    name.starts_with('_') || name.starts_with('.')
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn file_bytes_hash(path: &Path) -> Option<u64> {
    fs::read(path).ok().map(|bytes| fnv1a64(&bytes))
}

fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if file_bytes_hash(path) == Some(fnv1a64(bytes)) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

fn write_text_if_changed(path: &Path, contents: &str) -> std::io::Result<()> {
    write_bytes_if_changed(path, contents.as_bytes())
}

fn write_if_missing(path: &Path, contents: &str) -> std::io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    write_text_if_changed(path, contents)
}

fn template_atlas_png_bytes() -> std::io::Result<Vec<u8>> {
    // Keep in sync with tileset_mod_template_atlas.toml (9 cols × 5 rows, 128×192 cells).
    const TILE_W: u32 = 128;
    const TILE_H: u32 = 192;
    const COLS: u32 = 9;
    const ROWS: u32 = 5;
    let img = image::RgbaImage::new(COLS * TILE_W, ROWS * TILE_H);
    let mut bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| std::io::Error::other(e))?;
    Ok(bytes)
}

fn write_template_atlas_png_if_changed(path: &Path) -> std::io::Result<()> {
    let bytes = template_atlas_png_bytes()?;
    write_bytes_if_changed(path, &bytes)
}

/// Create the mod install tree and refresh shipped README / `_template/` when content changes.
pub fn ensure_mod_tilesets_scaffold() {
    #[cfg(test)]
    if TEST_CONFIG_ROOT.with(|cell| cell.borrow().is_some()) {
        return;
    }
    let root = mod_tilesets_root();
    if fs::create_dir_all(&root).is_err() {
        return;
    }
    let template = root.join(TEMPLATE_FOLDER);
    let _ = write_text_if_changed(&root.join("README.md"), ROOT_README);
    let _ = write_text_if_changed(&template.join("README.md"), FOLDER_README);
    let _ = write_text_if_changed(&template.join("atlas.toml"), TEMPLATE_ATLAS_TOML);
    let _ = write_template_atlas_png_if_changed(&template.join("atlas.png"));
}

fn ensure_folder_readme(dir: &Path) {
    let readme = dir.join("README.md");
    let _ = write_if_missing(&readme, FOLDER_README);
}

#[derive(Debug)]
pub enum ModTilesetValidationError {
    InvalidFolderName,
    MissingAtlasToml,
    MissingAtlasPng,
    AtlasTomlNotUtf8,
    AtlasTomlParseFailed,
    AtlasTomlInvalidDimensions,
    AtlasPngDecodeFailed,
    AtlasPngDimensionMismatch { expected: (u32, u32), actual: (u32, u32) },
}

impl std::fmt::Display for ModTilesetValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFolderName => write!(f, "invalid mod folder name"),
            Self::MissingAtlasToml => write!(f, "missing atlas.toml"),
            Self::MissingAtlasPng => write!(f, "missing atlas.png"),
            Self::AtlasTomlNotUtf8 => write!(f, "atlas.toml is not valid UTF-8"),
            Self::AtlasTomlParseFailed => write!(f, "atlas.toml failed to parse"),
            Self::AtlasTomlInvalidDimensions => {
                write!(f, "atlas.toml has zero tile_width, tile_height, or columns")
            }
            Self::AtlasPngDecodeFailed => write!(f, "atlas.png failed to decode"),
            Self::AtlasPngDimensionMismatch { expected, actual } => write!(
                f,
                "atlas.png size {actual:?} does not match atlas.toml grid {expected:?}"
            ),
        }
    }
}

pub fn validate_mod_tileset(dir: &Path) -> Result<(), ModTilesetValidationError> {
    let folder_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(ModTilesetValidationError::InvalidFolderName)?;
    if !is_valid_mod_folder_name(folder_name) {
        return Err(ModTilesetValidationError::InvalidFolderName);
    }

    let toml_path = dir.join("atlas.toml");
    let png_path = dir.join("atlas.png");
    if !toml_path.is_file() {
        return Err(ModTilesetValidationError::MissingAtlasToml);
    }
    if !png_path.is_file() {
        return Err(ModTilesetValidationError::MissingAtlasPng);
    }

    let toml_src = fs::read_to_string(&toml_path)
        .map_err(|_| ModTilesetValidationError::AtlasTomlNotUtf8)?;
    let (tile_w, tile_h, columns, layout) = parse_atlas_toml(&toml_src)
        .ok_or(ModTilesetValidationError::AtlasTomlParseFailed)?;
    if tile_w == 0 || tile_h == 0 || columns == 0 {
        return Err(ModTilesetValidationError::AtlasTomlInvalidDimensions);
    }

    let png_bytes = fs::read(&png_path).map_err(|_| ModTilesetValidationError::AtlasPngDecodeFailed)?;
    let img = image::ImageReader::new(std::io::Cursor::new(&png_bytes))
        .with_guessed_format()
        .map_err(|_| ModTilesetValidationError::AtlasPngDecodeFailed)?
        .decode()
        .map_err(|_| ModTilesetValidationError::AtlasPngDecodeFailed)?;
    let (png_w, png_h) = img.dimensions();

    let rows = layout.len().div_ceil(columns as usize) as u32;
    let expected_w = columns * tile_w;
    let expected_h = rows * tile_h;
    if png_w != expected_w || png_h != expected_h {
        return Err(ModTilesetValidationError::AtlasPngDimensionMismatch {
            expected: (expected_w, expected_h),
            actual: (png_w, png_h),
        });
    }

    Ok(())
}

/// Scan `{config_dir}/Mahjuro/mods/tilesets/*` for valid mod tilesets.
pub fn list_mod_tilesets() -> Vec<ModTilesetEntry> {
    ensure_mod_tilesets_scaffold();
    let root = mod_tilesets_root();
    let Ok(rd) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(folder_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_reserved_mod_folder(folder_name) {
            continue;
        }
        match validate_mod_tileset(&path) {
            Ok(()) => out.push(ModTilesetEntry {
                folder_name: folder_name.to_string(),
                id: mod_id(folder_name),
            }),
            Err(e) => {
                ensure_folder_readme(&path);
                log::warn!("skipping tileset mod '{folder_name}': {e}");
            }
        }
    }
    out.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    out
}

/// Read a file from an installed mod tileset folder.
pub fn read_mod_file(tileset_id: &str, filename: &str) -> Option<Vec<u8>> {
    let TilesetId::Mod { folder_name } = parse_tileset_id(tileset_id)? else {
        return None;
    };
    if filename.contains('/') || filename.contains('\\') {
        return None;
    }
    let path = mod_tilesets_root().join(folder_name).join(filename);
    fs::read(&path).ok()
}

/// Write bytes to the mod showcase cache (creates parent dirs).
pub fn write_mod_showcase_cache(tileset_id: &str, png_bytes: &[u8]) -> std::io::Result<()> {
    let path = mod_showcase_cache_path(tileset_id)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a mod tileset"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, png_bytes)
}

/// Read cached showcase decal atlas bytes for a mod tileset.
pub fn read_mod_showcase_cache(tileset_id: &str) -> Option<Vec<u8>> {
    let path = mod_showcase_cache_path(tileset_id)?;
    fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_valid_mod(dir: &Path, tile_w: u32, tile_h: u32, columns: u32) {
        fs::create_dir_all(dir).unwrap();
        let layout = r#"layout = [
    "B1","B2","B3","B4","B5","B6","B7","B8","B9",
]"#;
        fs::write(
            dir.join("atlas.toml"),
            format!(
                "tile_width = {tile_w}\ntile_height = {tile_h}\ncolumns = {columns}\n{layout}\n"
            ),
        )
        .unwrap();
        let rows = 1u32;
        let img = image::RgbaImage::new(columns * tile_w, rows * tile_h);
        img.save(dir.join("atlas.png")).unwrap();
    }

    #[test]
    fn mod_id_round_trip() {
        let id = mod_id("my_theme");
        assert_eq!(id, "mod:my_theme");
        assert!(is_mod_tileset(&id));
        assert_eq!(mod_folder_name(&id), Some("my_theme"));
        assert_eq!(
            parse_tileset_id(&id),
            Some(TilesetId::Mod {
                folder_name: "my_theme"
            })
        );
        assert_eq!(tileset_display_name(&id), "my_theme (mod)");
    }

    #[test]
    fn validate_accepts_good_mod_and_rejects_bad_dimensions() {
        let base = std::env::temp_dir().join("mahjuro_tileset_mod_test");
        let _ = fs::remove_dir_all(&base);
        set_mod_tilesets_root_for_tests(base.clone());

        let good = base.join("mods/tilesets/good");
        write_valid_mod(&good, 10, 20, 9);
        assert!(validate_mod_tileset(&good).is_ok());

        let bad = base.join("mods/tilesets/bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(
            bad.join("atlas.toml"),
            "tile_width = 10\ntile_height = 20\ncolumns = 9\nlayout = [\"B1\"]\n",
        )
        .unwrap();
        let img = image::RgbaImage::new(5, 5);
        img.save(bad.join("atlas.png")).unwrap();
        assert!(matches!(
            validate_mod_tileset(&bad),
            Err(ModTilesetValidationError::AtlasPngDimensionMismatch { .. })
        ));

        let listed = list_mod_tilesets();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "mod:good");
    }

    #[test]
    fn list_mod_tilesets_skips_reserved_folders() {
        let base = std::env::temp_dir().join(format!(
            "mahjuro_tileset_mod_reserved_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        set_mod_tilesets_root_for_tests(base.clone());

        write_valid_mod(&base.join("mods/tilesets/_template"), 128, 192, 9);
        write_valid_mod(&base.join("mods/tilesets/playable"), 10, 20, 9);

        let listed = list_mod_tilesets();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].folder_name, "playable");
    }

    #[test]
    fn write_bytes_if_changed_skips_identical_content() {
        let path = std::env::temp_dir().join(format!(
            "mahjuro_tileset_mod_hash_{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        write_bytes_if_changed(&path, b"same").unwrap();
        let before = fs::read(&path).unwrap();
        write_bytes_if_changed(&path, b"same").unwrap();
        let after = fs::read(&path).unwrap();
        assert_eq!(before, after);
        write_bytes_if_changed(&path, b"different").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"different");
    }

    #[test]
    fn incomplete_mod_folder_gets_readme() {
        let base = std::env::temp_dir().join(format!(
            "mahjuro_tileset_mod_readme_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        set_mod_tilesets_root_for_tests(base.clone());

        let incomplete = base.join("mods/tilesets/my_work_in_progress");
        fs::create_dir_all(&incomplete).unwrap();
        assert!(!incomplete.join("README.md").exists());

        let listed = list_mod_tilesets();
        assert!(listed.is_empty());
        assert!(incomplete.join("README.md").is_file());
    }
}
