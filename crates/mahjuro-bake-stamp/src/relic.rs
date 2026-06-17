//! Relic RLC2 bake stamp.
//!
//! Inputs: `assets/data/relics.json`, every PNG under `assets/textures/relics/`,
//! and the three runtime-side relic codec files (`relic_pipeline.rs`, `relic_bake.rs`,
//! `relic_dish.rs`). Bumping `relic-rlc2-vN` invalidates every committed bake.
//!
//! Per-relic sidecars (`<slug>.rlc.stamp`) let `mahjuro-bake-relics` skip unchanged relics
//! when only a subset of inputs is stale.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{
    BakeKind, Fnv64, git_tracked_files_under, hash_paths, read_stamp_line, write_stamp_line,
};

pub struct Relic;

/// Rebake every relic even when sidecars match (also `MAHJURO_FORCE_RELIC_BAKE=1`).
pub const FORCE_RELIC_BAKE_ENV: &str = "MAHJURO_FORCE_RELIC_BAKE";

const CODEC_VERSION: &[u8] = b"relic-rlc2-v2\n";

const CODEC_PATHS: &[&str] = &[
    "crates/mahjuro-render/src/relic_pipeline.rs",
    "crates/mahjuro-render/src/relic_bake.rs",
    "crates/mahjuro-render/src/relic_dish.rs",
];

impl BakeKind for Relic {
    const LABEL: &'static str = "relic RLC2 bake";
    const STAMP_PATH: &'static str = "assets/data/relic_baked/.inputs_stamp";
    const OUT_DIR: &'static str = "assets/data/relic_baked";
    const SKIP_ENV: &'static str = "MAHJURO_SKIP_RELIC_BAKE";
    const SCRIPT_REBAKE_CMD: &'static str = "scripts/rebake-offline.sh relic";
    const BUILD_TOOL_CMD: &'static str =
        "cargo build -p mahjuro-render --bin mahjuro-bake-relics --features relic_bc7_bake";
    const REBAKE_CMD: &'static str =
        "cargo run -p mahjuro-render --bin mahjuro-bake-relics --features relic_bc7_bake";
    const COMMIT_PATHS: &'static str = "assets/data/relic_baked/*.rlc assets/data/relic_baked/*.rlc.stamp assets/data/relic_baked/.inputs_stamp";

    fn stamp_input_paths(repo: &Path) -> Vec<PathBuf> {
        [
            CODEC_PATHS[0],
            CODEC_PATHS[1],
            CODEC_PATHS[2],
            "assets/data/relics.json",
            "assets/textures/relics",
        ]
        .into_iter()
        .map(|p| repo.join(p))
        .collect()
    }

    fn compute_inputs_hash(repo: &Path) -> String {
        let mut h = Fnv64::new();
        h.write(CODEC_VERSION);
        for path in Self::stamp_input_paths(repo) {
            if path.is_file() {
                hash_paths(&mut h, repo, std::slice::from_ref(&path));
            } else if path.is_dir() {
                let files = git_tracked_files_under(repo, "assets/textures/relics");
                hash_paths(&mut h, repo, &files);
            }
        }
        h.finish_hex()
    }

    fn outputs_ok(repo: &Path) -> bool {
        let expected = relic_def_count(repo);
        if expected == 0 {
            return false;
        }
        let dir = repo.join(Self::OUT_DIR);
        let Ok(read) = fs::read_dir(&dir) else {
            return false;
        };
        let baked = read
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "rlc"))
            .count();
        baked == expected
    }
}

/// FNV-1a digest of the relic RLC2 codec (version prefix + pipeline/bake/dish sources).
pub fn compute_codec_hash(repo: &Path) -> String {
    let mut h = Fnv64::new();
    h.write(CODEC_VERSION);
    let paths: Vec<PathBuf> = CODEC_PATHS.iter().map(|rel| repo.join(rel)).collect();
    hash_paths(&mut h, repo, &paths);
    h.finish_hex()
}

/// Repo-relative paths for one relic's bake-time PNG inputs (under `assets/`).
pub fn relic_asset_rel_paths(slug: &str) -> Vec<String> {
    let base = format!("assets/textures/relics/{slug}");
    [
        format!("{base}.png"),
        format!("{base}_object.png"),
        format!("{base}_mask.png"),
        format!("{base}_height.png"),
        format!("{base}_specular.png"),
    ]
    .into_iter()
    .collect()
}

/// Git-tracked relic PNG inputs for `slug` that exist on disk.
pub fn relic_asset_paths(repo: &Path, slug: &str) -> Vec<PathBuf> {
    let tracked: HashSet<String> = git_tracked_files_under(repo, "assets/textures/relics")
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(repo)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    relic_asset_rel_paths(slug)
        .into_iter()
        .filter(|rel| tracked.contains(rel))
        .map(|rel| repo.join(rel))
        .filter(|path| path.is_file())
        .collect()
}

/// Per-relic bake fingerprint: codec hash + tracked PNG bytes for `slug`.
pub fn compute_entry_hash(repo: &Path, slug: &str) -> String {
    let mut h = Fnv64::new();
    h.write(compute_codec_hash(repo).as_bytes());
    h.write(b"\n");
    hash_paths(&mut h, repo, &relic_asset_paths(repo, slug));
    h.finish_hex()
}

/// Sidecar next to the baked RLC2 payload (`assets/data/relic_baked/<slug>.rlc.stamp`).
pub fn relic_sidecar_path(out_dir: &Path, slug: &str) -> PathBuf {
    out_dir.join(format!("{slug}.rlc.stamp"))
}

pub fn read_relic_sidecar(path: &Path) -> Option<String> {
    read_stamp_line(path)
}

pub fn write_relic_sidecar(path: &Path, hash: &str) -> io::Result<()> {
    write_stamp_line(path, hash)
}

/// Write missing per-relic sidecars without re-encoding RLC2 payloads.
pub fn bootstrap_missing_sidecars(
    repo: &Path,
    out_dir: &Path,
    slugs: &[&str],
) -> io::Result<usize> {
    let mut bootstrapped = 0usize;
    for slug in slugs {
        let sidecar_path = relic_sidecar_path(out_dir, slug);
        if read_relic_sidecar(&sidecar_path).is_some() {
            continue;
        }
        let hash = compute_entry_hash(repo, slug);
        write_relic_sidecar(&sidecar_path, &hash)?;
        bootstrapped += 1;
    }
    Ok(bootstrapped)
}

pub fn force_relic_bake() -> bool {
    crate::skip_env_set(FORCE_RELIC_BAKE_ENV)
}

/// Count `"id":` lines in `relics.json` as a cheap stand-in for "how many bakes
/// should exist". Avoids parsing JSON in `build.rs`. Mirrors the previous
/// `build/relic_bake.rs` heuristic.
fn relic_def_count(repo: &Path) -> usize {
    let relics_json = repo.join("assets/data/relics.json");
    let Ok(raw) = fs::read_to_string(relics_json) else {
        return 0;
    };
    raw.lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with("\"id\"") && t.contains(':')
        })
        .count()
}

/// Paths the build script should announce via `cargo:rerun-if-changed`.
pub fn rerun_if_changed_paths() -> &'static [&'static str] {
    &[
        "crates/mahjuro-render/src/relic_pipeline.rs",
        "crates/mahjuro-render/src/relic_bake.rs",
        "crates/mahjuro-render/src/relic_dish.rs",
        "assets/data/relics.json",
        "assets/textures/relics",
    ]
}
