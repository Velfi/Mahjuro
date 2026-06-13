//! Shared input fingerprinting + `.inputs_stamp` writer for committed bake outputs.
//!
//! Both `build.rs` (freshness check at compile time) and the offline bake binaries
//! (`mahjuro-bake`, `mahjuro-bake-relics`) link against this crate so they agree on
//! exactly which input files feed each stamp and exactly how their FNV-1a digest is
//! computed. Without this, the bake binary could write outputs whose hash does not
//! match what `build.rs` will recompute on the next `cargo build`, so the freshness
//! check would still panic and the user would have to hand-edit the stamp.
//!
//! Each per-bake submodule (`relic`, `room_gi`, `room_shadow`) exposes:
//! - `stamp_input_paths(repo)` — every file/dir whose contents affect the bake
//! - `compute_inputs_hash(repo)` — FNV-1a 64-bit hex digest, the value written to the stamp
//! - `bake_status(repo)` — `(hash, stamp_ok, outputs_ok)` for the build-time check
//! - `write_stamp(repo)` — recompute the digest and persist it (called by the bake binary)
//! - `skip_bake_env()` — `MAHJURO_SKIP_OFFLINE_BAKES` or per-bake `MAHJURO_SKIP_*_BAKE`
//! - `STAMP_PATH` / `OUT_DIR` / `SKIP_ENV` / `LABEL` / `BUILD_TOOL_CMD` / `REBAKE_CMD` /
//!   `COMMIT_PATHS` — used by the `build.rs` panic message so the fix instructions stay
//!   in sync with the actual paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub mod relic;
pub mod room_gi;
pub mod room_shadow;
pub mod room_slugs;
pub mod shader_program;
pub mod showcase_decal;

/// FNV-1a 64-bit (stable across toolchains for build stamps).
pub struct Fnv64 {
    state: u64,
}

impl Default for Fnv64 {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    pub fn new() -> Self {
        Self {
            state: Self::OFFSET,
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    pub fn write_path_key(&mut self, path: &Path) {
        self.write(path.to_string_lossy().as_bytes());
        self.write(b"\0");
    }

    pub fn finish(self) -> u64 {
        self.state
    }

    pub fn finish_hex(self) -> String {
        format!("{:016x}", self.finish())
    }
}

fn write_rel_path_key(h: &mut Fnv64, path: &Path) {
    h.write(path.to_string_lossy().replace('\\', "/").as_bytes());
    h.write(b"\0");
}

/// Mix file bytes into `h`, normalizing CRLF/CR → LF for text so stamps match on Windows CI.
fn hash_bytes_for_stamp(h: &mut Fnv64, rel: &str, bytes: &[u8]) {
    if !is_text_rel(rel) || !bytes.contains(&b'\r') {
        h.write(bytes);
        return;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            if bytes.get(i + 1) == Some(&b'\n') {
                out.push(b'\n');
                i += 2;
            } else {
                out.push(b'\n');
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    h.write(&out);
}

fn is_text_rel(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    [
        ".rs", ".toml", ".txt", ".svg", ".wgsl", ".json", ".md", ".xml", ".html", ".css",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Mix a single file into `h` under a repo-relative key (`/` separators).
pub fn hash_file_at_rel(h: &mut Fnv64, rel: &str, path: &Path) {
    write_rel_path_key(h, Path::new(rel));
    if path.is_file()
        && let Ok(bytes) = fs::read(path)
    {
        hash_bytes_for_stamp(h, rel, &bytes);
    }
}

/// Mix every listed path into `h` with repo-relative keys.
/// Missing paths contribute only the path key.
pub fn hash_paths(h: &mut Fnv64, repo: &Path, paths: &[PathBuf]) {
    for path in paths {
        let rel = repo_relative(repo, path);
        hash_file_at_rel(h, &rel, path);
    }
}

/// Repo-relative path with `/` separators; stable when `strip_prefix` fails on Windows.
pub(crate) fn repo_relative(repo: &Path, path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(repo) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    let repo_s = repo.to_string_lossy().replace('\\', "/");
    let path_s = path.to_string_lossy().replace('\\', "/");
    if let Some(suffix) = path_s
        .strip_prefix(repo_s.as_str())
        .map(|s| s.trim_start_matches(['/', '\\']))
    {
        return suffix.to_string();
    }
    path_s
}

/// Sorted file list under `root`, honoring `.gitignore` (matches committed tree on CI).
pub fn hashable_git_files(root: &Path) -> Vec<PathBuf> {
    let mut walk = ignore::WalkBuilder::new(root);
    walk.hidden(false);
    walk.git_ignore(true);
    walk.git_global(true);
    walk.git_exclude(true);
    walk.parents(true);
    walk.require_git(false);

    let mut files: Vec<PathBuf> = walk
        .build()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

pub fn read_stamp_line(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let line = s.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

pub fn write_stamp_line(path: &Path, hash: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{hash}\n"))
}

pub fn outputs_present(out_dir: &Path, slugs: &[&str], ext: &str) -> bool {
    slugs
        .iter()
        .all(|s| out_dir.join(format!("{s}.{ext}")).is_file())
}

pub fn skip_env_set(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

/// Skip every committed offline bake freshness check (room GI/shadow, decal, relic).
pub const SKIP_OFFLINE_BAKES_ENV: &str = "MAHJURO_SKIP_OFFLINE_BAKES";

pub fn skip_offline_bakes() -> bool {
    skip_env_set(SKIP_OFFLINE_BAKES_ENV)
}

/// Skip every committed `.inputs_stamp` freshness check (e.g. while compiling `mahjuro-bake`).
pub const SKIP_COMMITTED_BAKE_CHECKS_ENV: &str = "MAHJURO_SKIP_COMMITTED_BAKE_CHECKS";

pub fn skip_committed_bake_checks() -> bool {
    skip_env_set(SKIP_COMMITTED_BAKE_CHECKS_ENV)
}

pub const BUILD_BAKER_CMD: &str = "MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1 cargo build -p mahjuro-headless --bin mahjuro-bake --features bake";

#[derive(Debug, Clone)]
pub struct BakeStatus {
    pub hash: String,
    pub stamp_ok: bool,
    pub outputs_ok: bool,
}

/// Result of [`BakeKind::write_stamp`].
#[derive(Debug, Clone)]
pub struct StampWritten {
    pub stamp_path: PathBuf,
    pub hash: String,
}

/// Common interface every per-bake module exposes; lets `build.rs` and the
/// panic-message helpers stay generic.
pub trait BakeKind {
    /// Human label, e.g. `"room GI bake"`.
    const LABEL: &'static str;
    const STAMP_PATH: &'static str;
    const OUT_DIR: &'static str;
    const SKIP_ENV: &'static str;
    /// Suggested `cargo build` command for the offline baker.
    const BUILD_TOOL_CMD: &'static str;
    /// Suggested `cargo run` command to refresh the stamp.
    const REBAKE_CMD: &'static str;
    /// Glob list to pass to `git add` after rebaking.
    const COMMIT_PATHS: &'static str;

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf>;
    fn compute_inputs_hash(repo: &Path) -> String;
    fn outputs_ok(repo: &Path) -> bool;

    fn skip_bake_env() -> bool {
        skip_offline_bakes() || skip_env_set(Self::SKIP_ENV)
    }

    fn stamp_file(repo: &Path) -> PathBuf {
        repo.join(Self::STAMP_PATH)
    }

    fn bake_status(repo: &Path) -> BakeStatus {
        let hash = Self::compute_inputs_hash(repo);
        let stamp_ok = read_stamp_line(&Self::stamp_file(repo)).is_some_and(|s| s == hash);
        let outputs_ok = Self::outputs_ok(repo);
        BakeStatus {
            hash,
            stamp_ok,
            outputs_ok,
        }
    }

    /// Recompute the inputs hash and persist it next to the bake outputs.
    /// Call this from the bake binary after every output is on disk.
    fn write_stamp(repo: &Path) -> io::Result<StampWritten> {
        let hash = Self::compute_inputs_hash(repo);
        let stamp_path = Self::stamp_file(repo);
        write_stamp_line(&stamp_path, &hash)?;
        Ok(StampWritten { stamp_path, hash })
    }
}

/// Format the panic message the build script uses when committed bakes are stale.
pub fn out_of_date_message<K: BakeKind>(status: &BakeStatus) -> String {
    let detail = if !status.stamp_ok {
        format!(
            "  {} is missing or stale (expected hash {})",
            K::STAMP_PATH,
            status.hash
        )
    } else {
        format!(
            "  baked outputs missing or incomplete under {}/",
            K::OUT_DIR
        )
    };
    format!(
        concat!(
            "{label} is out of date.\n\n",
            "{detail}\n\n",
            "To fix (needs a GPU):\n\n",
            "1. Build the offline baker:\n",
            "   {build_baker_cmd}\n\n",
            "2. Rebake (the binary refreshes the stamp on success):\n",
            "   {rebake_cmd}\n\n",
            "3. Commit the baked files + stamp:\n",
            "   git add {commit_paths}",
        ),
        label = K::LABEL,
        detail = detail,
        build_baker_cmd = BUILD_BAKER_CMD,
        rebake_cmd = K::REBAKE_CMD,
        commit_paths = K::COMMIT_PATHS,
    )
}

pub fn log_bake_timing(label: &str, start: Instant) {
    let secs = start.elapsed().as_secs_f64();
    println!("cargo:info=bake timing: {label} {secs:.2}s");
}

/// `cargo:rerun-if-changed` lines shared by `build/*.rs` freshness modules.
pub fn emit_rerun_if_changed<K: BakeKind>(rerun_paths: &[&str]) {
    println!("cargo:rerun-if-env-changed={SKIP_COMMITTED_BAKE_CHECKS_ENV}");
    println!("cargo:rerun-if-env-changed={SKIP_OFFLINE_BAKES_ENV}");
    println!("cargo:rerun-if-env-changed={}", K::SKIP_ENV);
    println!("cargo:rerun-if-changed={}", K::STAMP_PATH);
    for path in rerun_paths {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// Panic with [`out_of_date_message`] when committed outputs are stale (build.rs).
pub fn assert_bake_current<K: BakeKind>(repo: &Path) {
    if skip_committed_bake_checks() || K::skip_bake_env() {
        let via = if skip_committed_bake_checks() {
            SKIP_COMMITTED_BAKE_CHECKS_ENV
        } else if skip_offline_bakes() {
            SKIP_OFFLINE_BAKES_ENV
        } else {
            K::SKIP_ENV
        };
        println!("cargo:info={via}: skipping {} freshness check", K::LABEL);
        return;
    }

    let status = K::bake_status(repo);
    if status.stamp_ok && status.outputs_ok {
        println!("cargo:info={}: committed bake matches inputs", K::LABEL);
        return;
    }

    panic!("{}", out_of_date_message::<K>(&status));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_gi::RoomGi;
    use crate::room_shadow::RoomShadow;
    use crate::showcase_decal::ShowcaseDecal;
    use std::path::PathBuf;

    fn assert_stamp_matches<K: BakeKind>() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hash = K::compute_inputs_hash(&repo);
        let stamp = std::fs::read_to_string(repo.join(K::STAMP_PATH)).expect("stamp file");
        assert_eq!(stamp.trim(), hash, "{}", K::REBAKE_CMD);
    }

    #[test]
    fn showcase_decal_hash_matches_committed_stamp() {
        assert_stamp_matches::<ShowcaseDecal>();
    }

    #[test]
    fn room_gi_hash_matches_committed_stamp() {
        assert_stamp_matches::<RoomGi>();
    }

    #[test]
    fn room_shadow_hash_matches_committed_stamp() {
        assert_stamp_matches::<RoomShadow>();
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn hash_of(repo: &Path, paths: &[PathBuf]) -> String {
        let mut h = Fnv64::new();
        hash_paths(&mut h, repo, paths);
        h.finish_hex()
    }

    /// A prepended/composed WGSL file must be in the stamp input list *and* its
    /// bytes must move the hash, so editing it invalidates the committed bake.
    fn assert_input_contributes<K: BakeKind>(rel: &str) {
        let repo = repo_root();
        let target = repo.join(rel);
        let all = K::stamp_input_paths(&repo);
        assert!(
            all.iter().filter(|p| **p == target).count() == 1,
            "{rel} must appear exactly once in {} stamp inputs",
            K::LABEL
        );
        let reduced: Vec<PathBuf> = all.iter().filter(|p| **p != target).cloned().collect();
        assert_ne!(
            hash_of(&repo, &all),
            hash_of(&repo, &reduced),
            "{rel} must contribute to the {} stamp hash",
            K::LABEL
        );
    }

    #[test]
    fn room_shadow_hash_includes_prepended_wgsl() {
        for rel in crate::shader_program::SHADOW {
            assert_input_contributes::<RoomShadow>(rel);
        }
    }

    #[test]
    fn room_gi_hash_includes_prepended_wgsl() {
        for rel in crate::shader_program::scene_pbr_with_hallway_warp("shaders/room_glb.wgsl") {
            assert_input_contributes::<RoomGi>(rel);
        }
    }

    #[test]
    fn room_gi_hash_includes_shop_glb_bytes() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let shop = repo.join("assets/3d/Shop.glb");
        assert!(
            shop.is_file() && shop.metadata().unwrap().len() > 1_000_000,
            "Shop.glb must be materialized locally"
        );
        let full = RoomGi::compute_inputs_hash(&repo);
        let mut paths = RoomGi::stamp_input_paths(&repo);
        paths.retain(|p| p != &shop);
        let mut h = Fnv64::new();
        h.write(format!("rlm-v{}\n", crate::room_gi::ROOM_LIGHTMAP_FORMAT_VERSION).as_bytes());
        h.write(
            format!(
                "bake-{}x{}\n",
                crate::room_gi::BAKE_WIDTH,
                crate::room_gi::BAKE_HEIGHT
            )
            .as_bytes(),
        );
        h.write(format!("lightmap-{}\n", crate::room_gi::ROOM_LIGHTMAP_SIZE).as_bytes());
        hash_paths(&mut h, &repo, &paths);
        let without_shop = h.finish_hex();
        assert_ne!(
            full, without_shop,
            "shop GLB must contribute to the room GI stamp"
        );
    }

    #[test]
    fn repo_relative_normalizes_windows_paths() {
        let repo = PathBuf::from(r"D:\a\Mahjuro\Mahjuro");
        let path = PathBuf::from(r"D:\a\Mahjuro\Mahjuro\assets\fonts\foo.ttf");
        assert_eq!(repo_relative(&repo, &path), "assets/fonts/foo.ttf");
    }

    #[test]
    fn hash_bytes_for_stamp_normalizes_crlf_text() {
        let lf = b"line one\nline two\n";
        let crlf = b"line one\r\nline two\r\n";
        let mut h_lf = Fnv64::new();
        hash_bytes_for_stamp(&mut h_lf, "assets/fonts/OFL.txt", lf);
        let mut h_crlf = Fnv64::new();
        hash_bytes_for_stamp(&mut h_crlf, "assets/fonts/OFL.txt", crlf);
        assert_eq!(h_lf.finish_hex(), h_crlf.finish_hex());

        let png = b"\x89PNG\r\n\x1a\nbinary";
        let mut h_png = Fnv64::new();
        hash_bytes_for_stamp(&mut h_png, "assets/textures/foo.png", png);
        let mut h_png_raw = Fnv64::new();
        h_png_raw.write(png);
        assert_eq!(h_png.finish_hex(), h_png_raw.finish_hex());
    }
}
