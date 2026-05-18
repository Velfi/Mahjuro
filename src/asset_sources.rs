//! Runtime asset loading: multi-pack ZIP (manifest), or a loose tree if `MAHJURO_ASSETS` is set.
//!
//! **Boot loading:** **`shared`** (`fonts/`, `textures/tile_sets/`, all `audio/` SFX) and
//! **`gameplay`** packs are **eager** — opened and indexed during `PacksState::new` (before
//! `WgpuRenderer::new` pulls the tree). **`audio/music/`** is **lazy** — that zip stays closed
//! until first BGM play, or until [`prefetch_lazy_packs`] /
//! [`prefetch_lazy_packs_after_menu_once`].

use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock, RwLock};

const MANIFEST_NAME: &str = "pack_manifest.json";

/// Bytes for one asset (replaces `rust_embed::EmbeddedFile` for call sites).
pub struct AssetFile {
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LoadTier {
    Eager,
    Lazy,
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestPack {
    id: String,
    file: String,
    load_tier: LoadTier,
    #[serde(default)]
    path_prefixes: Vec<String>,
    #[serde(default)]
    root_files: Vec<String>,
    #[serde(default)]
    root_globs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackManifest {
    #[allow(dead_code)]
    schema_version: u32,
    game_version: String,
    packs: Vec<ManifestPack>,
}

/// Per-file index: case-folded lookup key → which pack + **exact** ZIP entry name (`by_name`).
type PathIndex = HashMap<String, (usize, String)>;

struct PackSlot {
    spec: ManifestPack,
    /// `None` until mounted (lazy) or immediately opened (eager).
    archive: Mutex<Option<zip::ZipArchive<File>>>,
}

struct PacksState {
    pack_dir: PathBuf,
    manifest: PackManifest,
    slots: Vec<PackSlot>,
    index: RwLock<PathIndex>,
}

enum AssetsState {
    Loose(PathBuf),
    Packs(PacksState),
}

static STATE: OnceLock<AssetsState> = OnceLock::new();

pub(crate) fn normalize_key(path: &str) -> String {
    path.trim_start_matches("./")
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

/// Case-folded key for pack index and routing (ZIP + `get()` lookups).
pub(crate) fn normalize_lookup_key(path: &str) -> String {
    normalize_key(path).to_lowercase()
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

fn macos_resources_dir(exe: &Path) -> Option<PathBuf> {
    let p = exe.to_string_lossy();
    let needle = ".app/Contents/MacOS/";
    if let Some(idx) = p.find(needle) {
        let app_root = PathBuf::from(&p[..idx + 4]);
        return Some(app_root.join("Contents/Resources"));
    }
    None
}

fn resolve_pack_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MAHJURO_ASSETS_PACK_DIR") {
        let pb = PathBuf::from(p);
        if pb.join(MANIFEST_NAME).is_file() {
            return Some(pb);
        }
    }
    if let Some(dir) = exe_dir() {
        if dir.join(MANIFEST_NAME).is_file() {
            return Some(dir);
        }
        // `cargo test`: exe is under `target/.../deps/`; build.rs writes packs in the parent profile dir.
        if dir.file_name() == Some(std::ffi::OsStr::new("deps"))
            && let Some(parent) = dir.parent()
                && parent.join(MANIFEST_NAME).is_file() {
                    return Some(parent.to_path_buf());
                }
        if let Ok(exe) = std::env::current_exe()
            && let Some(res) = macos_resources_dir(&exe)
                && res.join(MANIFEST_NAME).is_file() {
                    return Some(res);
                }
    }
    None
}

/// Loose tree only when explicitly requested (no implicit repo `assets/` path).
fn try_loose_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MAHJURO_ASSETS") {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return Some(pb);
        }
    }
    None
}

fn loose_or_panic() -> AssetsState {
    if let Some(root) = try_loose_root() {
        log::warn!("assets: loose tree {}", root.display());
        return AssetsState::Loose(root);
    }
    panic!(
        "Mahjuro assets not found: expected {} next to the executable, under MAHJURO_ASSETS_PACK_DIR, \
         or in the parent of `deps/` when running tests — or set MAHJURO_ASSETS to a loose assets/ directory. \
         Run `cargo build` (build.rs runs tools/bake_assets/bake_assets.py) or see tools/bake_assets/README.md.",
        MANIFEST_NAME
    );
}

/// Reject zip-slip / odd paths when indexing a trusted-but-modifiable pack file.
fn zip_entry_name_allowed(name: &str) -> bool {
    if name.is_empty() || name.ends_with('/') {
        return false;
    }
    if name.starts_with('/') || name.contains('\\') {
        return false;
    }
    !name.split('/').any(|seg| seg == "..")
}

fn root_name_matches_glob(name: &str, pat: &str) -> bool {
    match pat {
        "*.glb" => name.ends_with(".glb"),
        "*.png" => name.ends_with(".png"),
        _ => {
            if let Some(rest) = pat.strip_prefix('*') {
                return name.ends_with(rest);
            }
            name == pat
        }
    }
}

fn route_lazy_pack(manifest: &PackManifest, lookup_key: &str) -> Option<usize> {
    let norm = lookup_key.to_string();
    let mut best: Option<(usize, usize)> = None;
    for (idx, p) in manifest.packs.iter().enumerate() {
        if !matches!(p.load_tier, LoadTier::Lazy) {
            continue;
        }
        for rf in &p.root_files {
            if norm == normalize_lookup_key(rf) {
                let score = usize::MAX;
                best = Some(match best {
                    None => (score, idx),
                    Some((s, _i)) if score > s => (score, idx),
                    Some(b) => b,
                });
            }
        }
        for pref in &p.path_prefixes {
            let pref_n = normalize_lookup_key(pref);
            if norm.starts_with(&pref_n) {
                let score = pref_n.len();
                best = Some(match best {
                    None => (score, idx),
                    Some((s, _i)) if score > s => (score, idx),
                    Some(b) => b,
                });
            }
        }
        if !norm.contains('/') {
            for pat in &p.root_globs {
                if root_name_matches_glob(&norm, pat) {
                    let score = 1000 + pat.len();
                    best = Some(match best {
                        None => (score, idx),
                        Some((s, _i)) if score > s => (score, idx),
                        Some(b) => b,
                    });
                }
            }
        }
    }
    best.map(|(_, i)| i)
}

fn open_zip(pack_dir: &Path, file: &str) -> Result<zip::ZipArchive<File>, String> {
    let zip_path = pack_dir.join(file);
    let f = File::open(&zip_path).map_err(|e| format!("open {}: {e}", zip_path.display()))?;
    zip::ZipArchive::new(f).map_err(|e| format!("bad zip {}: {e}", zip_path.display()))
}

fn index_archive(archive: &mut zip::ZipArchive<File>, pack_idx: usize, index: &mut PathIndex) {
    for i in 0..archive.len() {
        let name = match archive.by_index(i) {
            Ok(f) => f.name().to_string(),
            Err(_) => continue,
        };
        if !zip_entry_name_allowed(&name) {
            log::warn!("skipping unsafe zip entry: {name:?}");
            continue;
        }
        let lk = normalize_lookup_key(&name);
        if let Some((prev, _)) = index.insert(lk.clone(), (pack_idx, name.clone()))
            && prev != pack_idx {
                log::warn!(
                    "duplicate asset key after case-fold: {lk} (pack indices {prev} vs {pack_idx})"
                );
            }
    }
}

fn read_exact(archive: &mut zip::ZipArchive<File>, entry_name: &str) -> Option<Vec<u8>> {
    let mut f = archive.by_name(entry_name).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

impl PacksState {
    fn new(pack_dir: PathBuf, manifest: PackManifest) -> Result<Self, String> {
        let mut slots = Vec::with_capacity(manifest.packs.len());
        let mut index = PathIndex::new();

        for spec in &manifest.packs {
            log::trace!(
                "asset pack `{}` file={} tier={:?}",
                spec.id,
                spec.file,
                spec.load_tier
            );
            let archive = if matches!(spec.load_tier, LoadTier::Eager) {
                Some(open_zip(&pack_dir, &spec.file)?)
            } else {
                None
            };
            let pack_idx = slots.len();
            slots.push(PackSlot {
                spec: spec.clone(),
                archive: Mutex::new(archive),
            });
            if matches!(spec.load_tier, LoadTier::Eager) {
                let mut guard = slots[pack_idx]
                    .archive
                    .lock()
                    .map_err(|_| "pack mutex poisoned".to_string())?;
                if let Some(ref mut arc) = *guard {
                    index_archive(arc, pack_idx, &mut index);
                }
            }
        }

        Ok(Self {
            pack_dir,
            manifest,
            slots,
            index: RwLock::new(index),
        })
    }

    /// Lock order: take this pack's `archive` mutex first, then `index` write lock.
    /// Do not hold `index` read lock while waiting on `archive` for a different pack.
    fn ensure_mounted(&self, pack_idx: usize) -> Result<(), String> {
        let slot = self
            .slots
            .get(pack_idx)
            .ok_or_else(|| "bad pack index".to_string())?;
        let mut guard = slot
            .archive
            .lock()
            .map_err(|_| "pack mutex poisoned".to_string())?;
        if guard.is_some() {
            return Ok(());
        }
        let arc = open_zip(&self.pack_dir, &slot.spec.file)?;
        *guard = Some(arc);
        let mut idx = self
            .index
            .write()
            .map_err(|_| "index lock poisoned".to_string())?;
        if let Some(ref mut a) = *guard {
            index_archive(a, pack_idx, &mut idx);
        }
        Ok(())
    }

    fn get(&self, path: &str) -> Option<Vec<u8>> {
        let lk = normalize_lookup_key(path);
        let resolved = {
            let idx = self.index.read().ok()?;
            idx.get(&lk).cloned()
        };
        let (pack_idx, entry_name) = if let Some(pair) = resolved {
            pair
        } else {
            let lazy_idx = route_lazy_pack(&self.manifest, &lk)?;
            self.ensure_mounted(lazy_idx).ok()?;
            let idx = self.index.read().ok()?;
            idx.get(&lk).cloned()?
        };

        self.ensure_mounted(pack_idx).ok()?;
        let slot = self.slots.get(pack_idx)?;
        let mut guard = slot.archive.lock().ok()?;
        let arc = guard.as_mut()?;
        read_exact(arc, &entry_name)
    }

    fn index_len(&self) -> usize {
        self.index.read().map(|g| g.len()).unwrap_or(0)
    }

    fn sample_index_paths(&self, max: usize) -> Vec<String> {
        let Ok(idx) = self.index.read() else {
            return Vec::new();
        };
        idx.values()
            .map(|(_, name)| name.clone())
            .take(max)
            .collect()
    }

    fn tileset_names_from_index(&self) -> Vec<String> {
        let Ok(idx) = self.index.read() else {
            return Vec::new();
        };
        idx.values()
            .filter_map(|(_, name)| {
                let mut it = name.split('/');
                if !it.next()?.eq_ignore_ascii_case("textures") {
                    return None;
                }
                if !it.next()?.eq_ignore_ascii_case("tile_sets") {
                    return None;
                }
                let set_name = it.next()?;
                let file = it.next()?;
                if !file.eq_ignore_ascii_case("atlas.toml") {
                    return None;
                }
                Some(set_name.to_string())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn prefetch_lazy(&self) {
        for i in 0..self.slots.len() {
            if matches!(self.slots[i].spec.load_tier, LoadTier::Lazy) {
                let _ = self.ensure_mounted(i);
            }
        }
    }
}

fn verify_manifest_version(manifest: &PackManifest) {
    let expected = env!("CARGO_PKG_VERSION");
    if manifest.game_version != expected {
        log::warn!(
            "pack_manifest game_version ({}) != binary {} — install may be mismatched",
            manifest.game_version,
            expected
        );
        if std::env::var_os("MAHJURO_STRICT_PACK_VERSION").is_some() {
            panic!(
                "MAHJURO_STRICT_PACK_VERSION: pack game_version {} != {}",
                manifest.game_version, expected
            );
        }
    }
}

fn init_state() -> AssetsState {
    if let Some(dir) = resolve_pack_dir() {
        let path = dir.join(MANIFEST_NAME);
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("pack manifest unreadable ({}): {e}", path.display());
                return loose_or_panic();
            }
        };
        let manifest: PackManifest = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("pack manifest JSON: {e}");
                return loose_or_panic();
            }
        };
        verify_manifest_version(&manifest);
        match PacksState::new(dir, manifest) {
            Ok(p) => {
                log::info!(
                    "asset packs: {} ({} packs, {} indexed paths)",
                    p.pack_dir.display(),
                    p.slots.len(),
                    p.index_len()
                );
                return AssetsState::Packs(p);
            }
            Err(e) => log::warn!("asset packs init failed: {e}"),
        }
    }
    loose_or_panic()
}

/// Initialize asset backend (packs or loose). Idempotent; safe to call multiple times.
pub fn init() {
    let _ = STATE.get_or_init(init_state);
}

/// Background-load all lazy packs (e.g. after main menu).
pub fn prefetch_lazy_packs() {
    let Some(state) = STATE.get() else {
        return;
    };
    if let AssetsState::Packs(p) = state {
        p.prefetch_lazy();
    }
}

static PREFETCH_AFTER_MENU: Once = Once::new();

/// Spawn a one-time background thread to mount lazy packs after the hub is reachable.
pub fn prefetch_lazy_packs_after_menu_once() {
    PREFETCH_AFTER_MENU.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("mahjuro-asset-prefetch".into())
            .spawn(|| {
                prefetch_lazy_packs();
            });
    });
}

pub fn get(path: &str) -> Option<AssetFile> {
    init();
    let state = STATE.get()?;
    let data = match state {
        AssetsState::Loose(root) => std::fs::read(root.join(normalize_key(path))).ok()?,
        AssetsState::Packs(p) => p.get(path)?,
    };
    Some(AssetFile { data })
}

pub fn log_all_assets() {
    init();
    let Some(state) = STATE.get() else {
        return;
    };
    match state {
        AssetsState::Loose(root) => {
            log::trace!("Loose assets under {}", root.display());
        }
        AssetsState::Packs(p) => {
            let n = p.index_len();
            log::trace!("Packed assets: {n} entries (sample up to 500 paths)");
            for name in p.sample_index_paths(500) {
                log::trace!("  {name}");
            }
            if n > 500 {
                log::trace!("  … {} more omitted", n - 500);
            }
        }
    }
}

/// Enumerate tileset names (`textures/tile_sets/*/atlas.toml` + `atlas.png`). With packs,
/// `textures/tile_sets/` lives in the eager **shared** pack, so this does not mount lazy packs
/// in normal layouts.
pub fn list_tilesets() -> Vec<String> {
    init();
    let Some(state) = STATE.get() else {
        return Vec::new();
    };
    let names_src: Vec<String> = match state {
        AssetsState::Loose(root) => {
            let sets = root.join("textures").join("tile_sets");
            let Ok(rd) = std::fs::read_dir(&sets) else {
                return Vec::new();
            };
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        }
        AssetsState::Packs(p) => p.tileset_names_from_index(),
    };
    let mut names: Vec<String> = names_src
        .into_iter()
        .filter(|name| get(&format!("textures/tile_sets/{name}/atlas.png")).is_some())
        .collect();
    names.sort();
    names.dedup();
    if let Some(pos) = names.iter().position(|n| n == "original") {
        names.swap(0, pos);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::{normalize_key, normalize_lookup_key, zip_entry_name_allowed};

    #[test]
    fn normalize_key_trims() {
        assert_eq!(normalize_key("./foo/bar"), "foo/bar");
        assert_eq!(normalize_key(r"a\b"), "a/b");
    }

    #[test]
    fn normalize_lookup_key_folds_case() {
        assert_eq!(normalize_lookup_key("Textures/Foo.PNG"), "textures/foo.png");
    }

    #[test]
    fn zip_entry_name_rejects_traversal() {
        assert!(!zip_entry_name_allowed("../evil"));
        assert!(!zip_entry_name_allowed("a/../b"));
        assert!(!zip_entry_name_allowed("/abs"));
        assert!(zip_entry_name_allowed("textures/a.png"));
    }
}
