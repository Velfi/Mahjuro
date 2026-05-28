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
//! - `skip_bake_env()` — read the `MAHJURO_SKIP_*_BAKE` env var
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

/// Mix every listed path into `h`. Missing paths contribute only the path key.
pub fn hash_paths(h: &mut Fnv64, paths: &[PathBuf]) {
    for path in paths {
        h.write_path_key(path);
        if path.is_file()
            && let Ok(bytes) = fs::read(path)
        {
            h.write(&bytes);
        }
    }
}

/// Depth-first walk of `root`, skipping paths where `skip(rel_from_root)` is true.
/// `rel_from_root` uses `/` separators and is empty at `root`.
pub fn hash_tree(
    h: &mut Fnv64,
    root: &Path,
    rel_prefix: &str,
    skip: &impl Fn(&str) -> bool,
) {
    if skip(rel_prefix) {
        return;
    }
    if root.is_file() {
        h.write_path_key(root);
        if let Ok(bytes) = fs::read(root) {
            h.write(&bytes);
        }
        return;
    }
    if !root.is_dir() {
        return;
    }
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    let mut children: Vec<_> = read.filter_map(|e| e.ok()).collect();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let rel = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        hash_tree(h, &entry.path(), &rel, skip);
    }
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
        skip_env_set(Self::SKIP_ENV)
    }

    fn stamp_file(repo: &Path) -> PathBuf {
        repo.join(Self::STAMP_PATH)
    }

    fn bake_status(repo: &Path) -> BakeStatus {
        let hash = Self::compute_inputs_hash(repo);
        let stamp_ok =
            read_stamp_line(&Self::stamp_file(repo)).is_some_and(|s| s == hash);
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
        format!("  baked outputs missing or incomplete under {}/", K::OUT_DIR)
    };
    format!(
        concat!(
            "{label} is out of date.\n\n",
            "{detail}\n\n",
            "To fix (needs a GPU):\n\n",
            "1. Build the offline baker:\n",
            "   {skip_env}=1 {build_tool_cmd}\n\n",
            "2. Rebake (the binary refreshes the stamp on success):\n",
            "   {rebake_cmd}\n\n",
            "3. Commit the baked files + stamp:\n",
            "   git add {commit_paths}",
        ),
        label = K::LABEL,
        detail = detail,
        skip_env = K::SKIP_ENV,
        build_tool_cmd = K::BUILD_TOOL_CMD,
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
    println!("cargo:rerun-if-env-changed={}", K::SKIP_ENV);
    println!("cargo:rerun-if-changed={}", K::STAMP_PATH);
    for path in rerun_paths {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// Panic with [`out_of_date_message`] when committed outputs are stale (build.rs).
pub fn assert_bake_current<K: BakeKind>(repo: &Path) {
    if K::skip_bake_env() {
        println!(
            "cargo:warning={}: skipping {} freshness check",
            K::SKIP_ENV,
            K::LABEL
        );
        return;
    }

    let status = K::bake_status(repo);
    if status.stamp_ok && status.outputs_ok {
        println!("cargo:info={}: committed bake matches inputs", K::LABEL);
        return;
    }

    panic!("{}", out_of_date_message::<K>(&status));
}
